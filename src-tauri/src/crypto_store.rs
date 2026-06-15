// File-based AES-256-GCM credential store. Replaces macOS Keychain.
// Why: each rebuild changes the binary cdhash, invalidating Keychain ACL
// "Always Allow" entries and re-prompting on every save/load. For a single-user
// desktop app, encrypting with a 0600 master key in ~/.sshl is sufficient.
//
// Layout:
//   ~/.sshl/master.key   32 random bytes, mode 0600 (one-time generated)
//   ~/.sshl/secrets.json { "id:kind": "base64(nonce(12) || ciphertext)" }

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::RngCore;

use crate::error::{AppError, AppResult};

fn dir() -> AppResult<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| AppError::Config("no home dir".into()))?;
    Ok(home.join(".sshl"))
}

fn key_path() -> AppResult<PathBuf> { Ok(dir()?.join("master.key")) }
fn store_path() -> AppResult<PathBuf> { Ok(dir()?.join("secrets.json")) }

fn load_or_create_key() -> AppResult<[u8; 32]> {
    let path = key_path()?;
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() == 32 {
            let mut k = [0u8; 32];
            k.copy_from_slice(&bytes);
            return Ok(k);
        }
    }
    std::fs::create_dir_all(dir()?)?;
    let mut k = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut k);
    std::fs::write(&path, k)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(k)
}

fn cipher() -> AppResult<Aes256Gcm> {
    let key = load_or_create_key()?;
    Ok(Aes256Gcm::new((&key).into()))
}

fn load_map() -> HashMap<String, String> {
    let Ok(path) = store_path() else { return HashMap::new() };
    std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_map(map: &HashMap<String, String>) -> AppResult<()> {
    std::fs::create_dir_all(dir()?)?;
    let path = store_path()?;
    let data = serde_json::to_vec_pretty(map)?;
    std::fs::write(&path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

static LOCK: Mutex<()> = Mutex::new(());

pub fn set(id: &str, kind: &str, value: &str) -> AppResult<()> {
    let _g = LOCK.lock().unwrap();
    let cipher = cipher()?;
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), value.as_bytes())
        .map_err(|e| AppError::Config(format!("encrypt: {e}")))?;
    let mut buf = Vec::with_capacity(12 + ct.len());
    buf.extend_from_slice(&nonce);
    buf.extend_from_slice(&ct);
    let encoded = base64::engine::general_purpose::STANDARD.encode(&buf);
    let mut map = load_map();
    map.insert(format!("{id}:{kind}"), encoded);
    save_map(&map)
}

pub fn get(id: &str, kind: &str) -> Option<String> {
    let _g = LOCK.lock().unwrap();
    let map = load_map();
    let encoded = map.get(&format!("{id}:{kind}"))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    if bytes.len() < 12 { return None; }
    let cipher = cipher().ok()?;
    let pt = cipher
        .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
        .ok()?;
    String::from_utf8(pt).ok()
}

/// 是否存在某条凭据（不解密，仅查 key），用于列出"有密码的连接"。
pub fn has(id: &str, kind: &str) -> bool {
    let _g = LOCK.lock().unwrap();
    load_map().contains_key(&format!("{id}:{kind}"))
}

/// 列出某连接下保存的所有账号标签（key 形如 "{id}:acct:{label}"）。仅查 key，不解密。
pub fn list_accounts(id: &str) -> Vec<String> {
    let _g = LOCK.lock().unwrap();
    let prefix = format!("{id}:acct:");
    let mut v: Vec<String> = load_map()
        .keys()
        .filter_map(|k| k.strip_prefix(&prefix).map(str::to_string))
        .collect();
    v.sort();
    v
}

pub fn delete(id: &str, kind: &str) {
    let _g = LOCK.lock().unwrap();
    let mut map = load_map();
    if map.remove(&format!("{id}:{kind}")).is_some() {
        let _ = save_map(&map);
    }
}
