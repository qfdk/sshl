// AES-256-GCM 凭据加密。密钥在独立的 ~/.sshl/master.key（32B, 0600）；密文存进
// SQLite 的 secrets 表（见 db.rs），key 形如 "id:kind"，value = base64(nonce(12) || ciphertext)。
//
// 密钥与密文分离：master.key 独立保管，库文件即便被拷走/同步，没有密钥也解不出密码。

use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::RngCore;

use crate::db;
use crate::error::{AppError, AppResult};

fn key_path() -> AppResult<PathBuf> {
    Ok(db::dir()?.join("master.key"))
}

/// 把主密钥写进文件：建出来就是 0600。先按默认权限写、写完再改的话，中间有一下别的用户读得到。
fn write_key(path: &Path, key: &[u8; 32]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    // 已有的文件（长度不对、要重写的）不会按上面的权限重建
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(key)
}

fn load_or_create_key() -> AppResult<[u8; 32]> {
    let path = key_path()?;
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() == 32 {
            let mut k = [0u8; 32];
            k.copy_from_slice(&bytes);
            return Ok(k);
        }
    }
    db::private_dir(&db::dir()?)?;
    let mut k = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut k);
    write_key(&path, &k)?;
    Ok(k)
}

fn cipher() -> AppResult<Aes256Gcm> {
    let key = load_or_create_key()?;
    Ok(Aes256Gcm::new((&key).into()))
}

fn encrypt(plain: &str) -> AppResult<String> {
    let cipher = cipher()?;
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
        .map_err(|e| AppError::Config(format!("encrypt: {e}")))?;
    let mut buf = Vec::with_capacity(12 + ct.len());
    buf.extend_from_slice(&nonce);
    buf.extend_from_slice(&ct);
    Ok(base64::engine::general_purpose::STANDARD.encode(&buf))
}

fn decrypt(encoded: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    if bytes.len() < 12 {
        return None;
    }
    let cipher = cipher().ok()?;
    let pt = cipher
        .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
        .ok()?;
    String::from_utf8(pt).ok()
}

pub fn set(id: &str, kind: &str, value: &str) -> AppResult<()> {
    let encoded = encrypt(value)?;
    let key = format!("{id}:{kind}");
    db::with_db(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO secrets (key,value) VALUES (?1,?2)",
            rusqlite::params![key, encoded],
        )?;
        Ok(())
    })
}

pub fn get(id: &str, kind: &str) -> Option<String> {
    let key = format!("{id}:{kind}");
    let encoded = db::with_db(|conn| {
        conn.query_row(
            "SELECT value FROM secrets WHERE key=?1",
            rusqlite::params![key],
            |r| r.get::<_, String>(0),
        )
    })
    .ok()?;
    decrypt(&encoded)
}

/// 是否存在某条凭据（不解密），用于列出"有密码的连接"。
pub fn has(id: &str, kind: &str) -> bool {
    let key = format!("{id}:{kind}");
    db::with_db(|conn| {
        conn.query_row(
            "SELECT 1 FROM secrets WHERE key=?1",
            rusqlite::params![key],
            |_| Ok(()),
        )
    })
    .is_ok()
}

pub fn delete(id: &str, kind: &str) {
    let key = format!("{id}:{kind}");
    let _ = db::with_db(|conn| {
        conn.execute("DELETE FROM secrets WHERE key=?1", rusqlite::params![key])?;
        Ok(())
    });
}

/// 列出某连接下保存的所有账号标签（key 形如 "{id}:acct:{label}"）。
pub fn list_accounts(id: &str) -> Vec<String> {
    let prefix = format!("{id}:acct:");
    let like = format!("{prefix}%");
    db::with_db(|conn| {
        let mut stmt = conn.prepare("SELECT key FROM secrets WHERE key LIKE ?1 ORDER BY key")?;
        let rows = stmt.query_map(rusqlite::params![like], |r| r.get::<_, String>(0))?;
        let mut v: Vec<String> = rows
            .filter_map(|k| k.ok())
            .filter_map(|k| k.strip_prefix(&prefix).map(str::to_string))
            .collect();
        v.sort();
        Ok(v)
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::write_key;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_master_key_file_is_only_readable_by_its_owner() {
        let dir = std::env::temp_dir().join(format!("sshl-master-key-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mode = |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let fresh = dir.join("master.key");
        write_key(&fresh, &[7; 32]).unwrap();
        assert_eq!((mode(&fresh), std::fs::read(&fresh).unwrap()), (0o600, vec![7; 32]));
        // 已有一个别人读得到的（坏掉的）文件：重写后也收紧
        let loose = dir.join("loose.key");
        std::fs::write(&loose, b"short").unwrap();
        std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_key(&loose, &[9; 32]).unwrap();
        assert_eq!((mode(&loose), std::fs::read(&loose).unwrap()), (0o600, vec![9; 32]));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
