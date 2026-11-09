// 连接存储 —— 元数据存在 ~/.sshl/sshl.db 的 connections 表；密码/账号密码经 crypto_store
// 加密后存进同库的 secrets 表（密钥仍在独立的 master.key）。旧的 JSON 文件由 db.rs 自动迁移。

use serde::{Deserialize, Serialize};

use crate::crypto_store;
use crate::db;
use crate::error::{AppError, AppResult};
use crate::events::{CoreEvent, Events};

/// 认证方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthKind {
    #[default]
    Password,
    Key,
    /// 免密：依次试服务器免认证、SSH Agent、私钥文件（按 ~/.ssh/config），见 auto_auth。
    Auto,
}

impl AuthKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthKind::Password => "password",
            AuthKind::Key => "key",
            AuthKind::Auto => "auto",
        }
    }

    /// 库里的值；加这一列之前存的连接（空）按有没有私钥推断。
    pub fn from_db(value: &str, private_key: Option<&str>) -> Self {
        match value {
            "password" => AuthKind::Password,
            "key" => AuthKind::Key,
            "auto" => AuthKind::Auto,
            _ if private_key.is_some() => AuthKind::Key,
            _ => AuthKind::Password,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredConnection {
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub private_key: Option<String>,
    /// 密码本身从不入元数据，只记一个标记；明文密文在 secrets 表里。
    #[serde(default)]
    pub has_password: bool,
    #[serde(default)]
    pub has_passphrase: bool,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub auth: AuthKind,
}

fn default_port() -> u16 {
    22
}

/// 全部保存的连接，按用户排的顺序。
pub fn connections() -> AppResult<Vec<StoredConnection>> {
        db::with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id,name,host,port,username,private_key,has_password,has_passphrase,group_name,auth
                 FROM connections ORDER BY sort_order, rowid",
            )?;
            let rows = stmt.query_map([], |r| {
                let private_key: Option<String> = r.get(5)?;
                let auth = AuthKind::from_db(&r.get::<_, String>(9)?, private_key.as_deref());
                Ok(StoredConnection {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    host: r.get(2)?,
                    port: r.get::<_, i64>(3)? as u16,
                    username: r.get(4)?,
                    private_key,
                    has_password: r.get::<_, i64>(6)? != 0,
                    has_passphrase: r.get::<_, i64>(7)? != 0,
                    group: r.get(8)?,
                    auth,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
}

/// 分组名称，按用户排的顺序（空分组也在这里）。
pub fn groups() -> AppResult<Vec<String>> {
        db::with_db(|conn| {
            let mut stmt =
                conn.prepare("SELECT name FROM connection_groups ORDER BY sort_order, rowid")?;
            let rows = stmt.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveConnectionInput {
    pub id: Option<String>,
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub auth: AuthKind,
}

pub fn save_connection(events: &Events, connection: SaveConnectionInput) -> AppResult<StoredConnection> {
    let id = connection
        .id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let has_password = connection.password.is_some();
    let has_passphrase = connection.passphrase.is_some();

    if let Some(pw) = &connection.password {
        crypto_store::set(&id, "password", pw)?;
    }
    if let Some(pp) = &connection.passphrase {
        crypto_store::set(&id, "passphrase", pp)?;
    }

    let stored = StoredConnection {
        id: id.clone(),
        name: connection.name,
        host: connection.host,
        port: connection.port,
        username: connection.username,
        private_key: connection.private_key,
        has_password,
        has_passphrase,
        group: connection.group.trim().to_string(),
        auth: connection.auth,
    };

    // upsert：已存在则保留原排序，否则追加到末尾（sort_order = max+1）
    db::with_db(|conn| {
        let order: i64 = conn
            .query_row(
                "SELECT sort_order FROM connections WHERE id=?1",
                rusqlite::params![stored.id],
                |r| r.get(0),
            )
            .or_else(|_| {
                conn.query_row(
                    "SELECT COALESCE(MAX(sort_order)+1,0) FROM connections",
                    [],
                    |r| r.get(0),
                )
            })?;
        conn.execute(
            "INSERT INTO connections
                (id,name,host,port,username,private_key,has_password,has_passphrase,sort_order,group_name,auth)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(id) DO UPDATE SET
                name=excluded.name, host=excluded.host, port=excluded.port,
                username=excluded.username, private_key=excluded.private_key,
                has_password=excluded.has_password, has_passphrase=excluded.has_passphrase,
                group_name=excluded.group_name, auth=excluded.auth",
            rusqlite::params![
                stored.id,
                stored.name,
                stored.host,
                stored.port as i64,
                stored.username,
                stored.private_key,
                stored.has_password as i64,
                stored.has_passphrase as i64,
                order,
                stored.group,
                stored.auth.as_str(),
            ],
        )?;
        if !stored.group.is_empty() {
            conn.execute(
                "INSERT OR IGNORE INTO connection_groups(name, sort_order)
                 SELECT ?1, COALESCE(MAX(sort_order) + 1, 0) FROM connection_groups",
                rusqlite::params![stored.group],
            )?;
        }
        Ok(())
    })?;

    events.emit(CoreEvent::ConnectionsUpdated);
    Ok(stored)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionLayoutInput {
    pub id: String,
    #[serde(default)]
    pub group: String,
}

pub fn apply_connection_layout(
    events: &Events,
    layout: Vec<ConnectionLayoutInput>,
    groups: Option<Vec<String>>,
) -> AppResult<()> {
    db::with_db(|conn| {
        if let Some(groups) = &groups {
            conn.execute("DELETE FROM connection_groups", [])?;
            let mut saved = std::collections::HashSet::new();
            for name in groups {
                let name = name.trim();
                if name.is_empty() || !saved.insert(name.to_string()) {
                    continue;
                }
                conn.execute(
                    "INSERT INTO connection_groups(name, sort_order) VALUES (?1, ?2)",
                    rusqlite::params![name, saved.len() as i64 - 1],
                )?;
            }
        }
        for (order, item) in layout.iter().enumerate() {
            conn.execute(
                "UPDATE connections SET sort_order=?1, group_name=?2 WHERE id=?3",
                rusqlite::params![order as i64, item.group.trim(), item.id],
            )?;
        }
        Ok(())
    })?;
    events.emit(CoreEvent::ConnectionsUpdated);
    Ok(())
}

pub fn delete_connection(events: &Events, id: String) -> AppResult<()> {
    db::with_db(|conn| delete_connection_rows(conn, &id))?;
    events.emit(CoreEvent::ConnectionsUpdated);
    Ok(())
}

/// 连同该连接名下所有密文（password / passphrase / acct:*）和远程目录收藏一并删除。
fn delete_connection_rows(conn: &rusqlite::Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM connections WHERE id=?1", rusqlite::params![id])?;
    conn.execute("DELETE FROM secrets WHERE key LIKE ?1", rusqlite::params![format!("{id}:%")])?;
    conn.execute("DELETE FROM bookmarks WHERE connection_id=?1", rusqlite::params![id])?;
    Ok(())
}

/// 某连接收藏的远程目录，按收藏先后。
pub fn bookmarks(connection_id: &str) -> AppResult<Vec<String>> {
    db::with_db(|conn| bookmarks_in(conn, connection_id))
}

/// 收藏 / 取消收藏一个远程目录。
pub fn set_bookmarked(connection_id: &str, path: &str, bookmarked: bool) -> AppResult<()> {
    db::with_db(|conn| set_bookmarked_in(conn, connection_id, path, bookmarked))
}

fn bookmarks_in(conn: &rusqlite::Connection, connection_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT path FROM bookmarks WHERE connection_id=?1 ORDER BY sort_order, rowid")?;
    let rows = stmt.query_map(rusqlite::params![connection_id], |r| r.get(0))?;
    rows.collect()
}

fn set_bookmarked_in(conn: &rusqlite::Connection, connection_id: &str, path: &str, bookmarked: bool) -> rusqlite::Result<()> {
    if bookmarked {
        conn.execute(
            "INSERT OR IGNORE INTO bookmarks(connection_id, path, sort_order)
             SELECT ?1, ?2, COALESCE(MAX(sort_order) + 1, 0) FROM bookmarks WHERE connection_id=?1",
            rusqlite::params![connection_id, path],
        )?;
    } else {
        conn.execute("DELETE FROM bookmarks WHERE connection_id=?1 AND path=?2", rusqlite::params![connection_id, path])?;
    }
    Ok(())
}

/// Used by ssh::connect — pulls password/passphrase from the encrypted store.
pub fn load_secret(connection_id: &str, kind: &str) -> Option<String> {
    crypto_store::get(connection_id, kind)
}

/// 某连接下可填充的凭据清单：连接主密码是否存在 + 已保存的账号标签。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredList {
    pub has_password: bool,
    pub accounts: Vec<String>,
}

pub fn cred_list(connection_id: String) -> CredList {
    CredList {
        has_password: crypto_store::has(&connection_id, "password"),
        accounts: crypto_store::list_accounts(&connection_id),
    }
}

/// 保存（或更新）某连接下一个账号的密码。account 不可为空。
pub fn cred_set(connection_id: String, account: String, password: String) -> AppResult<()> {
    let account = account.trim();
    if account.is_empty() {
        return Err(AppError::Config("账号名不能为空".into()));
    }
    crypto_store::set(&connection_id, &format!("acct:{account}"), &password)
}

pub fn cred_delete(connection_id: String, account: String) -> AppResult<()> {
    crypto_store::delete(&connection_id, &format!("acct:{}", account.trim()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(db::SCHEMA).unwrap();
        conn
    }

    #[test]
    fn bookmarks_are_per_connection_in_the_order_added() {
        let conn = memory_db();
        set_bookmarked_in(&conn, "a", "/var/log", true).unwrap();
        set_bookmarked_in(&conn, "a", "/etc/nginx", true).unwrap();
        set_bookmarked_in(&conn, "b", "/srv", true).unwrap();
        // 重复收藏不产生第二条，也不改顺序
        set_bookmarked_in(&conn, "a", "/var/log", true).unwrap();
        assert_eq!(bookmarks_in(&conn, "a").unwrap(), ["/var/log", "/etc/nginx"]);
        assert_eq!(bookmarks_in(&conn, "b").unwrap(), ["/srv"]);
        set_bookmarked_in(&conn, "a", "/var/log", false).unwrap();
        assert_eq!(bookmarks_in(&conn, "a").unwrap(), ["/etc/nginx"]);
        set_bookmarked_in(&conn, "a", "/var/log", true).unwrap();
        assert_eq!(bookmarks_in(&conn, "a").unwrap(), ["/etc/nginx", "/var/log"], "重新收藏排到最后");
    }

    #[test]
    fn deleting_a_connection_forgets_its_bookmarks_and_secrets() {
        let conn = memory_db();
        conn.execute("INSERT INTO connections(id,name,host,username) VALUES ('a','web','h','root'), ('b','db','h','root')", []).unwrap();
        conn.execute("INSERT INTO secrets(key,value) VALUES ('a:password','x'), ('b:password','y')", []).unwrap();
        set_bookmarked_in(&conn, "a", "/var/log", true).unwrap();
        set_bookmarked_in(&conn, "b", "/srv", true).unwrap();
        delete_connection_rows(&conn, "a").unwrap();
        assert!(bookmarks_in(&conn, "a").unwrap().is_empty());
        assert_eq!(bookmarks_in(&conn, "b").unwrap(), ["/srv"], "别的连接不受影响");
        let secrets: i64 = conn.query_row("SELECT COUNT(*) FROM secrets", [], |r| r.get(0)).unwrap();
        assert_eq!(secrets, 1);
    }
}
