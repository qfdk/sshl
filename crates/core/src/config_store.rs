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

/// 连同该连接名下所有密文（password / passphrase / acct:*）、远程目录收藏、专属命令片段一并删除。
fn delete_connection_rows(conn: &rusqlite::Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM connections WHERE id=?1", rusqlite::params![id])?;
    conn.execute("DELETE FROM secrets WHERE key LIKE ?1", rusqlite::params![format!("{id}:%")])?;
    conn.execute("DELETE FROM bookmarks WHERE connection_id=?1", rusqlite::params![id])?;
    conn.execute("DELETE FROM snippets WHERE connection_id=?1", rusqlite::params![id])?;
    Ok(())
}

/// 命令片段：常用命令存起来，一点就发到终端。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub id: i64,
    /// 可以不起名（列表里直接显示命令）。
    pub name: String,
    pub command: String,
    /// 只在这个连接里出现；None = 所有服务器通用。
    pub connection_id: Option<String>,
}

/// 新建（id 为 None）或修改一个命令片段。
#[derive(Debug, Clone)]
pub struct SaveSnippetInput {
    pub id: Option<i64>,
    pub name: String,
    pub command: String,
    pub connection_id: Option<String>,
}

/// 某连接能用的命令片段：它专属的在前，再是通用的；各自按添加先后。
pub fn snippets(connection_id: &str) -> AppResult<Vec<Snippet>> {
    db::with_db(|conn| snippets_in(conn, connection_id))
}

pub fn save_snippet(input: SaveSnippetInput) -> AppResult<i64> {
    let command = normalize_command(&input.command);
    if command.trim().is_empty() {
        return Err(AppError::Config(crate::tr!("命令不能为空").into()));
    }
    let input = SaveSnippetInput { name: input.name.trim().to_string(), command, ..input };
    db::with_db(|conn| save_snippet_in(conn, &input))
}

pub fn delete_snippet(id: i64) -> AppResult<()> {
    db::with_db(|conn| conn.execute("DELETE FROM snippets WHERE id=?1", rusqlite::params![id]).map(|_| ()))
}

/// 去掉首尾的空行和末尾空白；行首空格留着（bash 的 HISTCONTROL=ignorespace 靠它不进历史）。
fn normalize_command(command: &str) -> String {
    command.trim_end().trim_start_matches(['\n', '\r']).to_string()
}

fn snippets_in(conn: &rusqlite::Connection, connection_id: &str) -> rusqlite::Result<Vec<Snippet>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, command, connection_id FROM snippets
         WHERE connection_id IS NULL OR connection_id=?1
         ORDER BY connection_id IS NULL, sort_order, id",
    )?;
    let rows = stmt.query_map(rusqlite::params![connection_id], |r| {
        Ok(Snippet { id: r.get(0)?, name: r.get(1)?, command: r.get(2)?, connection_id: r.get(3)? })
    })?;
    rows.collect()
}

fn save_snippet_in(conn: &rusqlite::Connection, input: &SaveSnippetInput) -> rusqlite::Result<i64> {
    match input.id {
        Some(id) => {
            conn.execute(
                "UPDATE snippets SET name=?2, command=?3, connection_id=?4 WHERE id=?1",
                rusqlite::params![id, input.name, input.command, input.connection_id],
            )?;
            Ok(id)
        }
        None => {
            conn.execute(
                "INSERT INTO snippets(name, command, connection_id, sort_order)
                 SELECT ?1, ?2, ?3, COALESCE(MAX(sort_order) + 1, 0) FROM snippets",
                rusqlite::params![input.name, input.command, input.connection_id],
            )?;
            Ok(conn.last_insert_rowid())
        }
    }
}

/// 收藏的目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub path: String,
    /// 自己起的名字；空 = 没起，界面显示目录名。
    pub name: String,
}

/// 某连接收藏的目录，按收藏先后。
pub fn bookmarks(connection_id: &str) -> AppResult<Vec<Bookmark>> {
    db::with_db(|conn| bookmarks_in(conn, connection_id))
}

/// 给收藏起名 / 改名；空 = 恢复成显示目录名。
pub fn rename_bookmark(connection_id: &str, path: &str, name: &str) -> AppResult<()> {
    db::with_db(|conn| rename_bookmark_in(conn, connection_id, path, name))
}

/// 收藏 / 取消收藏一个远程目录。
pub fn set_bookmarked(connection_id: &str, path: &str, bookmarked: bool) -> AppResult<()> {
    db::with_db(|conn| set_bookmarked_in(conn, connection_id, path, bookmarked))
}

fn bookmarks_in(conn: &rusqlite::Connection, connection_id: &str) -> rusqlite::Result<Vec<Bookmark>> {
    let mut stmt = conn.prepare("SELECT path, name FROM bookmarks WHERE connection_id=?1 ORDER BY sort_order, rowid")?;
    let rows = stmt.query_map(rusqlite::params![connection_id], |r| Ok(Bookmark { path: r.get(0)?, name: r.get(1)? }))?;
    rows.collect()
}

fn rename_bookmark_in(conn: &rusqlite::Connection, connection_id: &str, path: &str, name: &str) -> rusqlite::Result<()> {
    conn.execute("UPDATE bookmarks SET name=?3 WHERE connection_id=?1 AND path=?2", rusqlite::params![connection_id, path, name])?;
    Ok(())
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
        return Err(AppError::Config(crate::tr!("账号名不能为空").into()));
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

    fn bookmark_paths(conn: &rusqlite::Connection, connection_id: &str) -> Vec<String> {
        bookmarks_in(conn, connection_id).unwrap().into_iter().map(|b| b.path).collect()
    }

    #[test]
    fn a_bookmark_keeps_the_name_it_was_given_until_it_is_removed() {
        let conn = memory_db();
        set_bookmarked_in(&conn, "a", "/var/log", true).unwrap();
        set_bookmarked_in(&conn, "b", "/var/log", true).unwrap();
        assert_eq!(bookmarks_in(&conn, "a").unwrap(), [Bookmark { path: "/var/log".into(), name: String::new() }]);
        rename_bookmark_in(&conn, "a", "/var/log", "日志").unwrap();
        assert_eq!(bookmarks_in(&conn, "a").unwrap(), [Bookmark { path: "/var/log".into(), name: "日志".into() }]);
        assert_eq!(bookmarks_in(&conn, "b").unwrap()[0].name, "", "别的连接同一路径的收藏不跟着改");
        // 重复收藏不冲掉名字
        set_bookmarked_in(&conn, "a", "/var/log", true).unwrap();
        assert_eq!(bookmarks_in(&conn, "a").unwrap()[0].name, "日志");
        rename_bookmark_in(&conn, "a", "/var/log", "").unwrap();
        assert_eq!(bookmarks_in(&conn, "a").unwrap()[0].name, "");
        // 取消收藏再收藏：名字不留
        rename_bookmark_in(&conn, "a", "/var/log", "日志").unwrap();
        set_bookmarked_in(&conn, "a", "/var/log", false).unwrap();
        set_bookmarked_in(&conn, "a", "/var/log", true).unwrap();
        assert_eq!(bookmarks_in(&conn, "a").unwrap()[0].name, "");
    }

    #[test]
    fn bookmarks_are_per_connection_in_the_order_added() {
        let conn = memory_db();
        set_bookmarked_in(&conn, "a", "/var/log", true).unwrap();
        set_bookmarked_in(&conn, "a", "/etc/nginx", true).unwrap();
        set_bookmarked_in(&conn, "b", "/srv", true).unwrap();
        // 重复收藏不产生第二条，也不改顺序
        set_bookmarked_in(&conn, "a", "/var/log", true).unwrap();
        assert_eq!(bookmark_paths(&conn, "a"), ["/var/log", "/etc/nginx"]);
        assert_eq!(bookmark_paths(&conn, "b"), ["/srv"]);
        set_bookmarked_in(&conn, "a", "/var/log", false).unwrap();
        assert_eq!(bookmark_paths(&conn, "a"), ["/etc/nginx"]);
        set_bookmarked_in(&conn, "a", "/var/log", true).unwrap();
        assert_eq!(bookmark_paths(&conn, "a"), ["/etc/nginx", "/var/log"], "重新收藏排到最后");
    }

    #[test]
    fn deleting_a_connection_forgets_its_bookmarks_and_secrets() {
        let conn = memory_db();
        conn.execute("INSERT INTO connections(id,name,host,username) VALUES ('a','web','h','root'), ('b','db','h','root')", []).unwrap();
        conn.execute("INSERT INTO secrets(key,value) VALUES ('a:password','x'), ('b:password','y')", []).unwrap();
        set_bookmarked_in(&conn, "a", "/var/log", true).unwrap();
        set_bookmarked_in(&conn, "b", "/srv", true).unwrap();
        delete_connection_rows(&conn, "a").unwrap();
        assert!(bookmark_paths(&conn, "a").is_empty());
        assert_eq!(bookmark_paths(&conn, "b"), ["/srv"], "别的连接不受影响");
        let secrets: i64 = conn.query_row("SELECT COUNT(*) FROM secrets", [], |r| r.get(0)).unwrap();
        assert_eq!(secrets, 1);
    }

    fn snippet(name: &str, command: &str, connection_id: Option<&str>) -> SaveSnippetInput {
        SaveSnippetInput { id: None, name: name.into(), command: command.into(), connection_id: connection_id.map(Into::into) }
    }

    fn names(conn: &rusqlite::Connection, connection_id: &str) -> Vec<String> {
        snippets_in(conn, connection_id).unwrap().into_iter().map(|s| s.name).collect()
    }

    #[test]
    fn a_connection_sees_its_own_snippets_first_then_the_shared_ones() {
        let conn = memory_db();
        save_snippet_in(&conn, &snippet("磁盘", "df -h", None)).unwrap();
        save_snippet_in(&conn, &snippet("部署", "cd /srv/app && git pull", Some("a"))).unwrap();
        save_snippet_in(&conn, &snippet("别的机器", "uptime", Some("b"))).unwrap();
        save_snippet_in(&conn, &snippet("日志", "journalctl -fu nginx", None)).unwrap();
        save_snippet_in(&conn, &snippet("重启", "systemctl restart app", Some("a"))).unwrap();
        assert_eq!(names(&conn, "a"), ["部署", "重启", "磁盘", "日志"]);
        assert_eq!(names(&conn, "b"), ["别的机器", "磁盘", "日志"]);
        assert_eq!(names(&conn, "c"), ["磁盘", "日志"]);
    }

    #[test]
    fn editing_a_snippet_keeps_its_place_and_can_change_its_scope() {
        let conn = memory_db();
        let first = save_snippet_in(&conn, &snippet("一", "echo 1", None)).unwrap();
        save_snippet_in(&conn, &snippet("二", "echo 2", None)).unwrap();
        save_snippet_in(&conn, &SaveSnippetInput { id: Some(first), ..snippet("一改", "echo one", Some("a")) }).unwrap();
        assert_eq!(names(&conn, "a"), ["一改", "二"]);
        assert_eq!(names(&conn, "b"), ["二"], "改成专属后别的连接看不到");
        let edited = &snippets_in(&conn, "a").unwrap()[0];
        assert_eq!((edited.command.as_str(), edited.connection_id.as_deref()), ("echo one", Some("a")));
    }

    #[test]
    fn deleting_a_connection_drops_only_its_own_snippets() {
        let conn = memory_db();
        save_snippet_in(&conn, &snippet("通用", "df -h", None)).unwrap();
        save_snippet_in(&conn, &snippet("专属", "uptime", Some("a"))).unwrap();
        delete_connection_rows(&conn, "a").unwrap();
        assert_eq!(names(&conn, "a"), ["通用"]);
    }

    #[test]
    fn snippet_commands_lose_blank_edges_but_keep_a_leading_space() {
        assert_eq!(normalize_command("\n\n ls -la  \n\n"), " ls -la");
        assert_eq!(normalize_command("cd /srv\ngit pull\n"), "cd /srv\ngit pull");
    }
}
