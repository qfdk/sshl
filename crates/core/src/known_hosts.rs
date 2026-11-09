//! 主机身份核对。信任记在 SSHL 自己的 `~/.sshl/known_hosts`；`~/.ssh/known_hosts` 只读不写，
//! 在那里已认过的主机（用 ssh 连过的）直接信任。没有记录、或密钥和记录不一样（可能被中间人冒充）时
//! 由调用方问用户，用户确认了才写进 SSHL 自己的文件。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::future::BoxFuture;
use russh_keys::PublicKeyBase64;
use russh_keys::key::PublicKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Known,
    /// 没有这台主机这种密钥的记录（第一次连）。
    Unknown,
    /// 记录里同类型的密钥和这次的不一样。
    Changed,
}

/// 要用户确认的主机身份。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyQuestion {
    pub host: String,
    pub port: u16,
    /// 如 `ssh-ed25519`。
    pub key_type: String,
    /// `SHA256:…`，同 `ssh-keygen -lf`。
    pub fingerprint: String,
    pub changed: bool,
}

/// 问用户是否信任；true = 信任并连接。
pub type HostKeyPrompt = Arc<dyn Fn(HostKeyQuestion) -> BoxFuture<'static, bool> + Send + Sync>;

/// 一次握手的核对方式。
#[derive(Clone)]
pub struct HostKeyCheck {
    /// SSHL 自己的信任记录，确认后写这里。
    pub ours: PathBuf,
    /// OpenSSH 的记录，只读。
    pub openssh: Option<PathBuf>,
    /// 没记录 / 变了时问谁；None（后台预热，用户不在场）直接拒绝，等用户点连接时再问。
    pub prompt: Option<HostKeyPrompt>,
}

impl HostKeyCheck {
    pub fn user(prompt: Option<HostKeyPrompt>) -> Self {
        let home = dirs::home_dir().unwrap_or_default();
        Self { ours: home.join(".sshl").join("known_hosts"), openssh: Some(home.join(".ssh").join("known_hosts")), prompt }
    }
}

#[cfg(test)]
impl HostKeyCheck {
    /// 集成测试用：信任任何主机，记录写到临时目录，不碰开发机的 ~/.sshl、~/.ssh。
    pub fn trust_any_for_tests() -> Self {
        Self {
            ours: std::env::temp_dir().join(format!("sshl-it-known-hosts-{}", std::process::id())),
            openssh: None,
            prompt: Some(Arc::new(|_| Box::pin(async { true }))),
        }
    }
}

fn check_file(host: &str, port: u16, key: &PublicKey, path: &Path) -> Verdict {
    match russh_keys::check_known_hosts_path(host, port, key, path) {
        Ok(true) => Verdict::Known,
        Err(russh_keys::Error::KeyChanged { .. }) => Verdict::Changed,
        Ok(false) => Verdict::Unknown,
        Err(error) => {
            tracing::warn!("known_hosts {}: {error}", path.display());
            Verdict::Unknown
        }
    }
}

/// 任一份记录里有这把密钥就信任（OpenSSH 那边换过新密钥、SSHL 这边还是旧的，也以用户在 ssh 里的确认为准）；
/// 都没有、而某份记录里同类型的是另一把，就是变了。
pub fn verdict(host: &str, port: u16, key: &PublicKey, ours: &Path, openssh: Option<&Path>) -> Verdict {
    let verdicts = [Some(check_file(host, port, key, ours)), openssh.map(|path| check_file(host, port, key, path))];
    let verdicts = verdicts.iter().flatten();
    if verdicts.clone().any(|v| *v == Verdict::Known) {
        Verdict::Known
    } else if verdicts.clone().any(|v| *v == Verdict::Changed) {
        Verdict::Changed
    } else {
        Verdict::Unknown
    }
}

pub fn question(host: &str, port: u16, key: &PublicKey, changed: bool) -> HostKeyQuestion {
    HostKeyQuestion {
        host: host.to_string(),
        port,
        key_type: key.name().to_string(),
        fingerprint: format!("SHA256:{}", key.fingerprint()),
        changed,
    }
}

fn entry(field: &str, key: &PublicKey) -> String {
    format!("{field} {} {}\n", key.name(), key.public_key_base64())
}

fn host_field(host: &str, port: u16) -> String {
    if port == 22 { host.to_string() } else { format!("[{host}]:{port}") }
}

/// 用户确认后记下这把密钥：先删掉 SSHL 记录里这台主机同类型的旧密钥，再追加。只写 `ours`。
pub fn trust(host: &str, port: u16, key: &PublicKey, ours: &Path) -> std::io::Result<()> {
    if let Some(dir) = ours.parent() {
        fs::create_dir_all(dir)?;
    }
    let field = host_field(host, port);
    let existing = fs::read_to_string(ours).unwrap_or_default();
    let kept: Vec<&str> = existing
        .lines()
        .filter(|line| {
            let mut parts = line.split_whitespace();
            let (Some(hosts), Some(kind)) = (parts.next(), parts.next()) else { return true };
            !(hosts.split(',').any(|h| h == field) && kind == key.name())
        })
        .collect();
    let mut content = kept.join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    content.push_str(&entry(&field, key));
    // 先写临时文件再改名：写到一半崩了也不丢已有的记录
    let tmp = ours.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, content)?;
    fs::rename(&tmp, ours)
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh_keys::key::KeyPair;

    fn key() -> PublicKey {
        KeyPair::generate_ed25519().unwrap().clone_public_key().unwrap()
    }

    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("sshl-known-hosts-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn line(host: &str, key: &PublicKey) -> String {
        entry(host, key)
    }

    #[test]
    fn first_connection_is_unknown_and_trusting_remembers_it() {
        let dir = Dir::new();
        let ours = dir.0.join("sshl").join("known_hosts");
        let k = key();
        assert_eq!(verdict("example.com", 22, &k, &ours, None), Verdict::Unknown);
        trust("example.com", 22, &k, &ours).unwrap();
        assert_eq!(verdict("example.com", 22, &k, &ours, None), Verdict::Known);
        // 端口不同是另一台主机
        assert_eq!(verdict("example.com", 2222, &k, &ours, None), Verdict::Unknown);
        trust("example.com", 2222, &k, &ours).unwrap();
        assert_eq!(verdict("example.com", 2222, &k, &ours, None), Verdict::Known);
    }

    #[test]
    fn different_key_is_changed_and_trusting_replaces_only_that_entry() {
        let dir = Dir::new();
        let ours = dir.0.join("known_hosts");
        let (old, new, other) = (key(), key(), key());
        trust("a.example", 22, &old, &ours).unwrap();
        trust("b.example", 22, &other, &ours).unwrap();
        assert_eq!(verdict("a.example", 22, &new, &ours, None), Verdict::Changed);
        trust("a.example", 22, &new, &ours).unwrap();
        assert_eq!(verdict("a.example", 22, &new, &ours, None), Verdict::Known);
        assert_eq!(verdict("a.example", 22, &old, &ours, None), Verdict::Changed, "旧密钥不再被信任");
        assert_eq!(verdict("b.example", 22, &other, &ours, None), Verdict::Known, "别的主机不受影响");
        assert_eq!(fs::read_to_string(&ours).unwrap().lines().count(), 2);
    }

    #[test]
    fn openssh_known_hosts_is_trusted_but_never_written() {
        let dir = Dir::new();
        let (ours, openssh) = (dir.0.join("ours"), dir.0.join("openssh"));
        let (k, stale) = (key(), key());
        let original = format!("# comment\n{}", line("[srv.example]:2200", &k));
        fs::write(&openssh, &original).unwrap();
        assert_eq!(verdict("srv.example", 2200, &k, &ours, Some(&openssh)), Verdict::Known);
        // OpenSSH 里是另一把：变了
        assert_eq!(verdict("srv.example", 2200, &key(), &ours, Some(&openssh)), Verdict::Changed);
        // SSHL 里记的是旧的，但用户在 ssh 里已认了新的：信任
        trust("srv.example", 2200, &stale, &ours).unwrap();
        assert_eq!(verdict("srv.example", 2200, &k, &ours, Some(&openssh)), Verdict::Known);
        trust("srv.example", 2200, &key(), &ours).unwrap();
        assert_eq!(fs::read_to_string(&openssh).unwrap(), original, "~/.ssh/known_hosts 原样不动");
    }

    #[test]
    fn hashed_openssh_entries_are_understood() {
        let dir = Dir::new();
        let openssh = dir.0.join("openssh");
        let k = key();
        // HashKnownHosts：|1|salt|HMAC-SHA1(salt, host)
        use hmac::{Hmac, Mac};
        let salt = [7u8; 20];
        let hash = Hmac::<sha1::Sha1>::new_from_slice(&salt).unwrap().chain_update("hashed.example").finalize().into_bytes();
        let b64 = |b: &[u8]| data_encoding::BASE64.encode(b);
        fs::write(&openssh, line(&format!("|1|{}|{}", b64(&salt), b64(&hash)), &k)).unwrap();
        assert_eq!(verdict("hashed.example", 22, &k, &dir.0.join("ours"), Some(&openssh)), Verdict::Known);
    }

    #[test]
    fn question_shows_type_and_sha256_fingerprint() {
        let k = key();
        let q = question("h", 22, &k, true);
        assert_eq!(q.key_type, "ssh-ed25519");
        assert!(q.fingerprint.starts_with("SHA256:") && q.fingerprint.len() == 7 + 43, "{}", q.fingerprint);
        assert!(q.changed);
    }
}
