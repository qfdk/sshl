// Persistent connection storage — replaces electron-store + safeStorage.
// Plaintext metadata in ~/.sshl/connections.json; passwords/passphrases in macOS Keychain.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::error::{AppError, AppResult};

const KEYRING_SERVICE: &str = "com.qfdk.sshl";

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
    /// Password is never written to disk — only flagged here, value lives in Keychain.
    #[serde(default)]
    pub has_password: bool,
    #[serde(default)]
    pub has_passphrase: bool,
}

fn default_port() -> u16 {
    22
}

fn config_dir() -> AppResult<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| AppError::Config("no home dir".into()))?;
    Ok(home.join(".sshl"))
}

fn config_path() -> AppResult<PathBuf> {
    Ok(config_dir()?.join("connections.json"))
}

pub struct ConfigStore {
    lock: Mutex<()>,
}

impl ConfigStore {
    pub fn new() -> Self {
        Self {
            lock: Mutex::new(()),
        }
    }

    async fn read_all(&self) -> AppResult<Vec<StoredConnection>> {
        let _g = self.lock.lock().await;
        let path = config_path()?;
        if !path.exists() {
            return Ok(Vec::new());
        }
        let data = tokio::fs::read(&path).await?;
        Ok(serde_json::from_slice(&data).unwrap_or_default())
    }

    async fn write_all(&self, items: &[StoredConnection]) -> AppResult<()> {
        let _g = self.lock.lock().await;
        let dir = config_dir()?;
        tokio::fs::create_dir_all(&dir).await?;
        let path = config_path()?;
        let data = serde_json::to_vec_pretty(items)?;
        tokio::fs::write(&path, data).await?;
        Ok(())
    }
}

impl Default for ConfigStore {
    fn default() -> Self {
        Self::new()
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
    store: tauri::State<'_, ConfigStore>,
    connection: SaveConnectionInput,
) -> AppResult<StoredConnection> {
    let id = connection
        .id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let has_password = connection.password.is_some();
    let has_passphrase = connection.passphrase.is_some();

    if let Some(pw) = &connection.password {
        keyring::Entry::new(KEYRING_SERVICE, &format!("{}:password", id))?
            .set_password(pw)?;
    }
    if let Some(pp) = &connection.passphrase {
        keyring::Entry::new(KEYRING_SERVICE, &format!("{}:passphrase", id))?
            .set_password(pp)?;
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

    let mut all = store.read_all().await?;
    if let Some(pos) = all.iter().position(|c| c.id == id) {
        all[pos] = stored.clone();
    } else {
        all.push(stored.clone());
    }
    store.write_all(&all).await?;

    let _ = app.emit("connections:updated", ());
    Ok(stored)
}

#[tauri::command]
pub async fn config_delete_connection(
    app: AppHandle,
    store: tauri::State<'_, ConfigStore>,
    id: String,
) -> AppResult<()> {
    let _ = keyring::Entry::new(KEYRING_SERVICE, &format!("{}:password", id))
        .and_then(|e| e.delete_credential());
    let _ = keyring::Entry::new(KEYRING_SERVICE, &format!("{}:passphrase", id))
        .and_then(|e| e.delete_credential());

    let mut all = store.read_all().await?;
    all.retain(|c| c.id != id);
    store.write_all(&all).await?;

    let _ = app.emit("connections:updated", ());
    Ok(())
}

/// Used by ssh::connect — pulls password/passphrase from Keychain if not provided inline.
pub fn load_secret(connection_id: &str, kind: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, &format!("{}:{}", connection_id, kind))
        .ok()
        .and_then(|e| e.get_password().ok())
}
