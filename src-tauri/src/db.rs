// 单一本地存储：~/.sshl/sshl.db（SQLite）。
//   connections 表 —— 连接元数据（明文）
//   secrets 表     —— 密码/账号密码（AES-256-GCM 密文，base64），密钥仍在独立的 master.key
//
// 进程内用一个全局连接 + Mutex 串行化访问（数据量极小，无并发压力）。首次访问时
// 建表，并把旧的 connections.json / secrets.json 一次性迁移进库后删除，保持目录干净。
// 不开 WAL（默认 rollback journal）→ 静止状态只有 sshl.db 一个文件。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use rusqlite::Connection;

use crate::error::{AppError, AppResult};

fn dir() -> AppResult<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| AppError::Config("no home dir".into()))?;
    Ok(home.join(".sshl"))
}

fn db_path() -> AppResult<PathBuf> {
    Ok(dir()?.join("sshl.db"))
}

static DB: OnceLock<Mutex<Connection>> = OnceLock::new();

fn open() -> AppResult<Connection> {
    let dir = dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = db_path()?;
    let conn = Connection::open(&path)
        .map_err(|e| AppError::Config(format!("open sshl.db: {e}")))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS connections (
            id             TEXT PRIMARY KEY,
            name           TEXT NOT NULL,
            host           TEXT NOT NULL,
            port           INTEGER NOT NULL DEFAULT 22,
            username       TEXT NOT NULL,
            private_key    TEXT,
            has_password   INTEGER NOT NULL DEFAULT 0,
            has_passphrase INTEGER NOT NULL DEFAULT 0,
            sort_order     INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS secrets (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
    .map_err(|e| AppError::Config(format!("init schema: {e}")))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }

    migrate_from_json(&conn)?;
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
                             (id,name,host,port,username,private_key,has_password,has_passphrase,sort_order)
                             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
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
