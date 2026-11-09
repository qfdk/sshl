//! 新建 / 编辑连接时自动发现 ~/.ssh 下的私钥（只在对话框里用，不上启动路径）。

use std::io::Read;
use std::path::{Path, PathBuf};

/// 常见默认私钥名，按 OpenSSH 的偏好排在前面；其余按文件名。
const PREFERRED: [&str; 4] = ["id_ed25519", "id_ecdsa", "id_rsa", "id_dsa"];
/// FIDO 硬件密钥的类型：要插着硬件令牌签名，russh 不支持，列出来也用不了。
const SECURITY_KEY_TYPES: [&[u8]; 2] = [b"sk-ssh-ed25519@openssh.com", b"sk-ecdsa-sha2-nistp256@openssh.com"];
const OPENSSH_HEADER: &str = "-----BEGIN OPENSSH PRIVATE KEY-----";
/// 私钥的 PEM / OpenSSH 头就在文件开头，读这么多足够。
const HEADER_BYTES: u64 = 4096;

/// ~/.ssh 下的私钥（绝对路径，排好序）。
pub fn discover() -> Vec<PathBuf> {
    dirs::home_dir().map(|home| scan(&home.join(".ssh"))).unwrap_or_default()
}

/// 扫描 `dir` 这一层（不递归）的私钥文件。目录不存在、读不了的文件都跳过。
pub fn scan(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut keys: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| file_name(path).is_some_and(is_candidate_name))
        .filter(|path| std::fs::metadata(path).is_ok_and(|m| m.is_file()))
        .filter(|path| looks_like_private_key(path))
        .collect();
    keys.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| file_name(a).cmp(&file_name(b))));
    keys
}

/// 家目录下的路径显示成 `~/...`。
pub fn display(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

fn file_name(path: &Path) -> Option<&str> {
    path.file_name()?.to_str()
}

/// 明显不是私钥的文件不读：公钥、known_hosts、config、authorized_keys、agent 套接字目录等。
fn is_candidate_name(name: &str) -> bool {
    !(name.ends_with(".pub")
        || name.starts_with("known_hosts")
        || name.starts_with("config")
        || name.starts_with("authorized_keys")
        || name == "agent")
}

fn looks_like_private_key(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else { return false };
    let mut head = Vec::new();
    if file.take(HEADER_BYTES).read_to_end(&mut head).is_err() {
        return false;
    }
    head.windows(b"PRIVATE KEY".len()).any(|w| w == b"PRIVATE KEY") && !is_security_key(&head)
}

/// OpenSSH 格式私钥开头的公钥部分是明文（设了密码也只加密私钥部分），解开前几百字节就能看到类型。
fn is_security_key(head: &[u8]) -> bool {
    use base64::Engine;
    let text = String::from_utf8_lossy(head);
    let Some((_, body)) = text.split_once(OPENSSH_HEADER) else { return false };
    let b64: String = body.chars().take_while(|c| *c != '-').filter(|c| !c.is_whitespace()).take(512).collect();
    let b64 = &b64[..b64.len() / 4 * 4];
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else { return false };
    SECURITY_KEY_TYPES.iter().any(|kind| bytes.windows(kind.len()).any(|w| w == *kind))
}

fn rank(path: &Path) -> usize {
    file_name(path).and_then(|name| PREFERRED.iter().position(|p| *p == name)).unwrap_or(PREFERRED.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENSSH: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----\n";
    const PEM_RSA: &str = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA\n-----END RSA PRIVATE KEY-----\n";

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let dir = std::env::temp_dir().join(format!("sshl-keys-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, name: &str, body: &str) {
            std::fs::write(self.0.join(name), body).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn names(keys: &[PathBuf]) -> Vec<&str> {
        keys.iter().map(|p| file_name(p).unwrap()).collect()
    }

    #[test]
    fn missing_directory_finds_nothing() {
        let dir = TempDir::new("missing");
        assert!(scan(&dir.0.join("nope")).is_empty());
    }

    #[test]
    fn finds_private_keys_and_skips_everything_else() {
        let dir = TempDir::new("filter");
        dir.write("id_ed25519", OPENSSH);
        dir.write("id_ed25519.pub", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI me@host\n");
        dir.write("deploy.pem", PEM_RSA);
        dir.write("known_hosts", "example.com ssh-ed25519 AAAAC3Nza\n");
        dir.write("config", "Host *\n  IdentityFile ~/.ssh/id_ed25519\n");
        dir.write("notes.txt", "nothing to see here\n");
        dir.write("google_compute_known_hosts", "compute.1 ssh-ed25519 AAAA\n");
        // 子目录里的私钥不算（不递归），子目录本身也不是私钥
        std::fs::create_dir(dir.0.join("nested")).unwrap();
        std::fs::write(dir.0.join("nested").join("id_rsa"), PEM_RSA).unwrap();
        assert_eq!(names(&scan(&dir.0)), ["id_ed25519", "deploy.pem"]);
    }

    #[test]
    fn excluded_names_are_skipped_even_if_they_hold_a_key() {
        let dir = TempDir::new("names");
        dir.write("id_rsa.pub", PEM_RSA);
        dir.write("config.bak", OPENSSH);
        dir.write("known_hosts.old", OPENSSH);
        dir.write("authorized_keys", OPENSSH);
        dir.write("agent", OPENSSH);
        dir.write("work", OPENSSH);
        assert_eq!(names(&scan(&dir.0)), ["work"]);
    }

    #[test]
    fn default_names_come_first_in_openssh_order() {
        let dir = TempDir::new("order");
        for name in ["zeta", "id_rsa", "id_dsa", "aws", "id_ed25519", "id_ecdsa"] {
            dir.write(name, OPENSSH);
        }
        assert_eq!(names(&scan(&dir.0)), ["id_ed25519", "id_ecdsa", "id_rsa", "id_dsa", "aws", "zeta"]);
    }

    /// OpenSSH 私钥：magic + 加密方式 + 1 把公钥（类型 kind）+ 后面一段私钥数据，按 70 列折行。
    fn openssh_key(kind: &str) -> String {
        use base64::Engine;
        let mut blob = b"openssh-key-v1\0".to_vec();
        let mut string = |bytes: &[u8]| {
            blob.extend((bytes.len() as u32).to_be_bytes());
            blob.extend(bytes);
        };
        string(b"none");
        string(b"none");
        string(b"");
        let mut public = Vec::new();
        public.extend((kind.len() as u32).to_be_bytes());
        public.extend(kind.as_bytes());
        public.extend([7u8; 40]);
        blob.extend(1u32.to_be_bytes());
        blob.extend((public.len() as u32).to_be_bytes());
        blob.extend(public);
        blob.extend([9u8; 300]);
        let b64 = base64::engine::general_purpose::STANDARD.encode(blob);
        let lines: Vec<&str> = b64.as_bytes().chunks(70).map(|c| std::str::from_utf8(c).unwrap()).collect();
        format!("{OPENSSH_HEADER}\n{}\n-----END OPENSSH PRIVATE KEY-----\n", lines.join("\n"))
    }

    #[test]
    fn hardware_security_keys_are_left_out() {
        let dir = TempDir::new("sk");
        dir.write("id_ed25519", &openssh_key("ssh-ed25519"));
        dir.write("id_ed25519_sk", &openssh_key("sk-ssh-ed25519@openssh.com"));
        dir.write("yubikey", &openssh_key("sk-ecdsa-sha2-nistp256@openssh.com"));
        dir.write("legacy", PEM_RSA);
        assert_eq!(names(&scan(&dir.0)), ["id_ed25519", "legacy"]);
    }

    #[test]
    fn named_pipes_are_not_opened() {
        // 扫描在界面线程上做：打开 FIFO 会一直阻塞，只读普通文件
        let dir = TempDir::new("fifo");
        dir.write("id_ed25519", OPENSSH);
        assert!(std::process::Command::new("mkfifo").arg(dir.0.join("pipe")).status().unwrap().success());
        let (tx, rx) = std::sync::mpsc::channel();
        let path = dir.0.clone();
        std::thread::spawn(move || tx.send(scan(&path)).unwrap());
        let keys = rx.recv_timeout(std::time::Duration::from_secs(2)).expect("scan blocked on a FIFO");
        assert_eq!(names(&keys), ["id_ed25519"]);
    }

    #[test]
    fn home_paths_are_shown_with_a_tilde() {
        let home = Path::new("/Users/me");
        assert_eq!(display(Path::new("/Users/me/.ssh/id_rsa"), Some(home)), "~/.ssh/id_rsa");
        assert_eq!(display(Path::new("/opt/keys/id_rsa"), Some(home)), "/opt/keys/id_rsa");
        assert_eq!(display(Path::new("/Users/me/.ssh/id_rsa"), None), "/Users/me/.ssh/id_rsa");
    }
}
