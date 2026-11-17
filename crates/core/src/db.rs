// 单一本地存储：~/.sshl/sshl.db（SQLite）。
//   connections 表 —— 连接元数据（明文）
//   connection_groups 表 —— 分组名称与显式排序
//   secrets 表           —— 密码/账号密码（AES-256-GCM 密文，base64），密钥仍在独立的 master.key
//
// 进程内用一个全局连接 + Mutex 串行化访问（数据量极小，无并发压力）。首次访问时
// 建表，并把旧的 connections.json / secrets.json 一次性迁移进库后删除，保持目录干净。
// 不开 WAL（默认 rollback journal）→ 静止状态只有 sshl.db 一个文件。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use rusqlite::Connection;

use crate::error::{AppError, AppResult};

pub(crate) fn dir() -> AppResult<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| AppError::Config("no home dir".into()))?;
    Ok(home.join(".sshl"))
}

/// 建好数据目录，并保证只有自己进得去（0700）。macOS 的家目录默认别的用户也能进：目录照默认的 0755 建的话，
/// 这台机器上的其他账号读得到连接列表、设置、连过哪些主机，主密钥和库文件在刚建出来、还没改权限的那一下也读得到。
pub(crate) fn private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    // 以前的版本、或界面先存设置时照默认权限建的：收紧
    if std::fs::metadata(dir)?.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn db_path() -> AppResult<PathBuf> {
    Ok(dir()?.join("sshl.db"))
}

static DB: OnceLock<Mutex<Connection>> = OnceLock::new();

/// 建表（已有的表不动；后加的列见 open 里的迁移）。
pub(crate) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS connections (
            id             TEXT PRIMARY KEY,
            name           TEXT NOT NULL,
            host           TEXT NOT NULL,
            port           INTEGER NOT NULL DEFAULT 22,
            username       TEXT NOT NULL,
            private_key    TEXT,
            has_password   INTEGER NOT NULL DEFAULT 0,
            has_passphrase INTEGER NOT NULL DEFAULT 0,
            sort_order     INTEGER NOT NULL DEFAULT 0,
            group_name     TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS connection_groups (
            name       TEXT PRIMARY KEY,
            sort_order INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS secrets (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS bookmarks (
            connection_id TEXT NOT NULL,
            path          TEXT NOT NULL,
            sort_order    INTEGER NOT NULL DEFAULT 0,
            name          TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (connection_id, path)
        );
        CREATE TABLE IF NOT EXISTS snippets (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            name          TEXT NOT NULL DEFAULT '',
            command       TEXT NOT NULL,
            connection_id TEXT,
            sort_order    INTEGER NOT NULL DEFAULT 0
        );";

fn open() -> AppResult<Connection> {
    private_dir(&dir()?)?;
    let path = db_path()?;
    let conn =
        Connection::open(&path).map_err(|e| AppError::Config(format!("open sshl.db: {e}")))?;
    conn.execute_batch(SCHEMA).map_err(|e| AppError::Config(format!("init schema: {e}")))?;

    let mut columns = conn
        .prepare("PRAGMA table_info(connections)")
        .map_err(|e| AppError::Config(format!("inspect schema: {e}")))?;
    let column_names = columns
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| AppError::Config(format!("inspect schema: {e}")))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AppError::Config(format!("inspect schema: {e}")))?;
    let has_group_name = column_names.iter().any(|name| name == "group_name");
    let has_auth = column_names.iter().any(|name| name == "auth");
    drop(columns);
    if !has_group_name {
        conn.execute(
            "ALTER TABLE connections ADD COLUMN group_name TEXT NOT NULL DEFAULT ''",
            [],
        )
        .map_err(|e| AppError::Config(format!("migrate schema: {e}")))?;
    }
    // 认证方式：空 = 加这一列之前存的，按有没有私钥推断（AuthKind::from_db）
    if !has_auth {
        conn.execute("ALTER TABLE connections ADD COLUMN auth TEXT NOT NULL DEFAULT ''", [])
            .map_err(|e| AppError::Config(format!("migrate schema: {e}")))?;
    }

    // 收藏的名字：空 = 没起名，显示目录名
    let bookmark_has_name = conn
        .prepare("SELECT 1 FROM pragma_table_info('bookmarks') WHERE name='name'")
        .and_then(|mut stmt| stmt.exists([]))
        .map_err(|e| AppError::Config(format!("inspect schema: {e}")))?;
    if !bookmark_has_name {
        conn.execute("ALTER TABLE bookmarks ADD COLUMN name TEXT NOT NULL DEFAULT ''", [])
            .map_err(|e| AppError::Config(format!("migrate schema: {e}")))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }

    migrate_from_json(&conn)?;
    conn.execute(
        "INSERT OR IGNORE INTO connection_groups(name, sort_order)
         SELECT TRIM(group_name), MIN(sort_order)
         FROM connections
         WHERE TRIM(group_name) <> ''
         GROUP BY TRIM(group_name)",
        [],
    )
    .map_err(|e| AppError::Config(format!("migrate groups: {e}")))?;

    Ok(conn)
}

fn db() -> &'static Mutex<Connection> {
    DB.get_or_init(|| Mutex::new(open().expect("init sshl.db")))
}

/// 串行化地拿到底层连接执行一段 SQL。
pub fn with_db<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> AppResult<T> {
    let guard = db().lock().unwrap();
    f(&guard).map_err(|e| AppError::Config(format!("db: {e}")))
}

/// 一次性把旧的 JSON 文件导入库后删除（密文 blob 原样搬运，master.key 不变仍可解密）。
fn migrate_from_json(conn: &Connection) -> AppResult<()> {
    let d = dir()?;

    let conn_json = d.join("connections.json");
    if conn_json.exists() {
        let empty: i64 = conn
            .query_row("SELECT COUNT(*) FROM connections", [], |r| r.get(0))
            .unwrap_or(0);
        if empty == 0 {
            if let Ok(bytes) = std::fs::read(&conn_json) {
                if let Ok(arr) = serde_json::from_slice::<Vec<serde_json::Value>>(&bytes) {
                    for (i, v) in arr.iter().enumerate() {
                        let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("");
                        if id.is_empty() {
                            continue;
                        }
                        let _ = conn.execute(
                            "INSERT OR REPLACE INTO connections
                             (id,name,host,port,username,private_key,has_password,has_passphrase,sort_order,group_name)
                             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                            rusqlite::params![
                                id,
                                v.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                                v.get("host").and_then(|x| x.as_str()).unwrap_or(""),
                                v.get("port").and_then(|x| x.as_u64()).unwrap_or(22) as i64,
                                v.get("username").and_then(|x| x.as_str()).unwrap_or(""),
                                v.get("privateKey").and_then(|x| x.as_str()),
                                v.get("hasPassword").and_then(|x| x.as_bool()).unwrap_or(false) as i64,
                                v.get("hasPassphrase").and_then(|x| x.as_bool()).unwrap_or(false) as i64,
                                i as i64,
                                v.get("group").and_then(|x| x.as_str()).unwrap_or(""),
                            ],
                        );
                    }
                }
            }
        }
        let _ = std::fs::remove_file(&conn_json);
    }

    let secrets_json = d.join("secrets.json");
    if secrets_json.exists() {
        let empty: i64 = conn
            .query_row("SELECT COUNT(*) FROM secrets", [], |r| r.get(0))
            .unwrap_or(0);
        if empty == 0 {
            if let Ok(bytes) = std::fs::read(&secrets_json) {
                if let Ok(map) = serde_json::from_slice::<HashMap<String, String>>(&bytes) {
                    for (k, val) in map {
                        let _ = conn.execute(
                            "INSERT OR REPLACE INTO secrets (key,value) VALUES (?1,?2)",
                            rusqlite::params![k, val],
                        );
                    }
                }
            }
        }
        let _ = std::fs::remove_file(&secrets_json);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::private_dir;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_data_directory_is_only_for_its_owner() {
        let root = std::env::temp_dir().join(format!("sshl-private-dir-{}", uuid::Uuid::new_v4()));
        let mode = |dir: &std::path::Path| std::fs::metadata(dir).unwrap().permissions().mode() & 0o777;
        // 新建的
        let fresh = root.join("fresh");
        private_dir(&fresh).unwrap();
        assert_eq!(mode(&fresh), 0o700);
        // 已经照默认权限建好的：收紧，里面的东西不动
        let loose = root.join("loose");
        std::fs::create_dir_all(&loose).unwrap();
        std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(loose.join("sshl.db"), b"data").unwrap();
        private_dir(&loose).unwrap();
        assert_eq!(mode(&loose), 0o700);
        assert_eq!(std::fs::read(loose.join("sshl.db")).unwrap(), b"data");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
