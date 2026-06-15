// 连接存储 —— 元数据存在 ~/.sshl/sshl.db 的 connections 表；密码/账号密码经 crypto_store
// 加密后存进同库的 secrets 表（密钥仍在独立的 master.key）。旧的 JSON 文件由 db.rs 自动迁移。

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::crypto_store;
use crate::db;
use crate::error::{AppError, AppResult};

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
}

fn default_port() -> u16 {
    22
}

/// 保留为 Tauri State 句柄（命令签名沿用），实际存储走 db。
#[derive(Default)]
pub struct ConfigStore;

impl ConfigStore {
    pub fn new() -> Self {
        Self
    }

    pub(crate) async fn read_all(&self) -> AppResult<Vec<StoredConnection>> {
        db::with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id,name,host,port,username,private_key,has_password,has_passphrase
                 FROM connections ORDER BY sort_order, rowid",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(StoredConnection {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    host: r.get(2)?,
                    port: r.get::<_, i64>(3)? as u16,
                    username: r.get(4)?,
                    private_key: r.get(5)?,
                    has_password: r.get::<_, i64>(6)? != 0,
                    has_passphrase: r.get::<_, i64>(7)? != 0,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }
}

#[derive(Debug, Deserialize)]
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
}

#[tauri::command]
pub async fn config_get_connections(
    store: tauri::State<'_, ConfigStore>,
) -> AppResult<Vec<StoredConnection>> {
    store.read_all().await
}

#[tauri::command]
pub async fn config_save_connection(
    app: AppHandle,
    _store: tauri::State<'_, ConfigStore>,
    connection: SaveConnectionInput,
) -> AppResult<StoredConnection> {
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
                (id,name,host,port,username,private_key,has_password,has_passphrase,sort_order)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(id) DO UPDATE SET
                name=excluded.name, host=excluded.host, port=excluded.port,
                username=excluded.username, private_key=excluded.private_key,
                has_password=excluded.has_password, has_passphrase=excluded.has_passphrase",
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
            ],
        )?;
        Ok(())
    })?;

    let _ = app.emit("connections:updated", ());
    Ok(stored)
}

#[tauri::command]
pub async fn config_delete_connection(
    app: AppHandle,
    _store: tauri::State<'_, ConfigStore>,
    id: String,
) -> AppResult<()> {
    // 连同该连接名下所有密文（password / passphrase / acct:*）一并删除
    db::with_db(|conn| {
        conn.execute("DELETE FROM connections WHERE id=?1", rusqlite::params![id])?;
        conn.execute(
            "DELETE FROM secrets WHERE key LIKE ?1",
            rusqlite::params![format!("{id}:%")],
        )?;
        Ok(())
    })?;

    let _ = app.emit("connections:updated", ());
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

#[tauri::command]
pub fn cred_list(connection_id: String) -> CredList {
    CredList {
        has_password: crypto_store::has(&connection_id, "password"),
        accounts: crypto_store::list_accounts(&connection_id),
    }
}

/// 保存（或更新）某连接下一个账号的密码。account 不可为空。
#[tauri::command]
pub fn cred_set(connection_id: String, account: String, password: String) -> AppResult<()> {
    let account = account.trim();
    if account.is_empty() {
        return Err(AppError::Config("账号名不能为空".into()));
    }
    crypto_store::set(&connection_id, &format!("acct:{account}"), &password)
}

#[tauri::command]
pub fn cred_delete(connection_id: String, account: String) -> AppResult<()> {
    crypto_store::delete(&connection_id, &format!("acct:{}", account.trim()));
    Ok(())
}
