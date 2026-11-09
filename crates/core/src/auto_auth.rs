//! 免密登录：依次试服务器免认证（none）、SSH Agent、私钥文件。私钥和 Agent 按 `~/.ssh/config` 来
//! （`ssh -G` 解析出这台主机的 IdentityFile / IdentityAgent / IdentitiesOnly，和终端里 `ssh` 一致），
//! 解析不了（没装 ssh、超时）就用 OpenSSH 的默认私钥和 `SSH_AUTH_SOCK`。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use russh_keys::key::{KeyPair, PublicKey};

use crate::error::{AppError, AppResult};

/// 去哪找 Agent。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSource {
    /// `SSH_AUTH_SOCK`。
    Env,
    Socket(PathBuf),
    /// `IdentityAgent none`。
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub files: Vec<PathBuf>,
    pub agent: AgentSource,
    /// `IdentitiesOnly yes`：Agent 里只用和私钥文件对得上的那几把。
    pub identities_only: bool,
}

const DEFAULT_KEYS: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

fn default_plan(home: &Path) -> Plan {
    Plan { files: DEFAULT_KEYS.iter().map(|name| home.join(".ssh").join(name)).collect(), agent: AgentSource::Env, identities_only: false }
}

/// `~`、`%d`（家目录）、`%u`（本机用户）、`%h`（主机）、`%r`（远端用户）、`%%`。
fn expand(value: &str, home: &Path, host: &str, user: &str) -> PathBuf {
    let value = match value.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", home.display()),
        None if value == "~" => home.display().to_string(),
        None => value.to_string(),
    };
    let local_user = std::env::var("USER").unwrap_or_default();
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('d') => out.push_str(&home.display().to_string()),
            Some('u') => out.push_str(&local_user),
            Some('h') => out.push_str(host),
            Some('r') => out.push_str(user),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    PathBuf::from(out)
}

/// 解析 `ssh -G` 的输出（每行 `关键字 值`，关键字小写）。
pub fn parse_ssh_g(out: &str, home: &Path, host: &str, user: &str) -> Plan {
    let mut plan = Plan { files: Vec::new(), agent: AgentSource::Env, identities_only: false };
    for line in out.lines() {
        let Some((keyword, value)) = line.split_once(' ') else { continue };
        let value = value.trim();
        match keyword {
            "identityfile" => plan.files.push(expand(value, home, host, user)),
            "identitiesonly" => plan.identities_only = value == "yes",
            "identityagent" => {
                plan.agent = match value {
                    "none" => AgentSource::Disabled,
                    "SSH_AUTH_SOCK" | "$SSH_AUTH_SOCK" => AgentSource::Env,
                    path => AgentSource::Socket(expand(path, home, host, user)),
                }
            }
            _ => {}
        }
    }
    if plan.files.is_empty() {
        plan.files = default_plan(home).files;
    }
    plan
}

/// 主机名直接交给 `ssh -G`：只放行主机名 / IP 里会出现的字符。
fn plain_host(host: &str) -> bool {
    !host.is_empty() && !host.starts_with('-') && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']' | '%'))
}

pub async fn plan_for(host: &str, port: u16, user: &str) -> Plan {
    let home = dirs::home_dir().unwrap_or_default();
    if !plain_host(host) {
        return default_plan(&home);
    }
    let run = tokio::process::Command::new("ssh")
        .args(["-G", "-p", &port.to_string(), "-l", user, "--", host])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(Duration::from_secs(3), run).await {
        Ok(Ok(out)) if out.status.success() => parse_ssh_g(&String::from_utf8_lossy(&out.stdout), &home, host, user),
        other => {
            tracing::debug!("ssh -G {host}: {other:?}; using default keys");
            default_plan(&home)
        }
    }
}

/// 试过了什么，失败时据此说明。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tried {
    pub agent_keys: usize,
    pub files: Vec<String>,
    /// 设了密码、免密用不了的私钥。
    pub encrypted: Vec<String>,
}

pub fn failure_message(tried: &Tried) -> String {
    let mut parts = Vec::new();
    if tried.agent_keys > 0 {
        parts.push(format!("SSH Agent 里的 {} 把密钥", tried.agent_keys));
    }
    if !tried.files.is_empty() {
        parts.push(tried.files.join("、"));
    }
    let mut message = if parts.is_empty() {
        "免密登录没有可用的密钥：SSH Agent 里没有密钥，也没找到可用的私钥文件。".to_string()
    } else {
        format!("免密登录没有成功：服务器没有接受{}。", parts.join("和"))
    };
    if !tried.encrypted.is_empty() {
        message.push_str(&format!(
            "{} 设了密码，免密用不了：可以先 ssh-add 加进 SSH Agent，或改用「私钥」方式并填写私钥密码。",
            tried.encrypted.join("、")
        ));
    }
    message
}

fn file_label(path: &Path) -> String {
    let home = dirs::home_dir().unwrap_or_default();
    match path.strip_prefix(home.join(".ssh")) {
        Ok(rest) => rest.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

enum Loaded {
    Key(Box<KeyPair>),
    Encrypted,
    Unusable,
}

fn load(path: &Path) -> Loaded {
    let Ok(pem) = std::fs::read_to_string(path) else { return Loaded::Unusable };
    match russh_keys::decode_secret_key(&pem, None) {
        Ok(key) => Loaded::Key(Box::new(key)),
        // OpenSSH / PKCS#5 格式报 KeyIsEncrypted；加密的 PKCS#8 只能从 PEM 头看出来
        Err(russh_keys::Error::KeyIsEncrypted) => Loaded::Encrypted,
        Err(_) if pem.contains("ENCRYPTED") => Loaded::Encrypted,
        Err(_) => Loaded::Unusable,
    }
}

/// 私钥文件对应的公钥：先看同名 .pub（加密的私钥也能对上 Agent 里的那把），没有再从私钥算。
fn public_of(path: &Path) -> Option<PublicKey> {
    let mut pub_path = path.as_os_str().to_owned();
    pub_path.push(".pub");
    let from_pub = std::fs::read_to_string(&pub_path)
        .ok()
        .and_then(|line| line.split_whitespace().nth(1).and_then(|b64| russh_keys::parse_public_key_base64(b64).ok()));
    if from_pub.is_some() {
        return from_pub;
    }
    match load(path) {
        Loaded::Key(key) => key.clone_public_key().ok(),
        _ => None,
    }
}

/// 免密登录。成功返回 Ok(())；都不行返回说明试了什么的错误。
pub async fn authenticate<H: russh::client::Handler>(
    handle: &mut russh::client::Handle<H>,
    host: &str,
    port: u16,
    user: &str,
) -> AppResult<()> {
    if handle.authenticate_none(user).await.unwrap_or(false) {
        return Ok(());
    }
    let plan = plan_for(host, port, user).await;
    authenticate_with(handle, user, &plan).await
}

/// 按给定的私钥 / Agent 依次试（不含免认证）。
pub async fn authenticate_with<H: russh::client::Handler>(
    handle: &mut russh::client::Handle<H>,
    user: &str,
    plan: &Plan,
) -> AppResult<()> {
    let files: Vec<&PathBuf> = plan.files.iter().filter(|path| path.is_file()).collect();
    let mut tried = Tried::default();
    let mut offered: Vec<PublicKey> = Vec::new();

    let agent = match &plan.agent {
        AgentSource::Env => russh_keys::agent::client::AgentClient::connect_env().await.ok(),
        AgentSource::Socket(path) => russh_keys::agent::client::AgentClient::connect_uds(path).await.ok(),
        AgentSource::Disabled => None,
    };
    if let Some(mut agent) = agent {
        let mut keys = agent.request_identities().await.unwrap_or_default();
        if plan.identities_only {
            let allowed: Vec<PublicKey> = files.iter().filter_map(|path| public_of(path)).collect();
            keys.retain(|key| allowed.contains(key));
        }
        for key in keys {
            let (back, result) = handle.authenticate_future(user, key.clone(), agent).await;
            agent = back;
            tried.agent_keys += 1;
            offered.push(key);
            match result {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(error) => tracing::debug!("agent key rejected: {error}"),
            }
            if handle.is_closed() {
                return Err(AppError::Ssh(failure_message(&tried)));
            }
        }
    }

    for path in files {
        match load(path) {
            Loaded::Key(key) => {
                if key.clone_public_key().is_ok_and(|public| offered.contains(&public)) {
                    continue;
                }
                tried.files.push(file_label(path));
                if handle.authenticate_publickey(user, Arc::new(*key)).await.unwrap_or(false) {
                    return Ok(());
                }
                if handle.is_closed() {
                    break;
                }
            }
            // Agent 里已经有它（同名 .pub 对得上）就不算「用不了」
            Loaded::Encrypted if !public_of(path).is_some_and(|public| offered.contains(&public)) => {
                tried.encrypted.push(file_label(path));
            }
            _ => {}
        }
    }
    Err(AppError::Ssh(failure_message(&tried)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实 sshd 上的免密登录：私钥文件、Agent、私钥不对、私钥设了密码、IdentitiesOnly。
    /// 测试故意认证失败多次：OpenSSH 9.8+ 的 sshd 要配 `PerSourcePenalties no`，否则 127.0.0.1 会被暂时拒连。
    /// `SSHL_AUTO_AUTH_IT=127.0.0.1:2222 SSHL_AUTO_AUTH_IT_USER=me SSHL_AUTO_AUTH_IT_KEY=/path/authorized_key \
    ///  cargo test --lib auto_auth_against_real_sshd -- --ignored`
    #[tokio::test]
    #[ignore = "需要 SSHL_AUTO_AUTH_IT=host:port 指向认可 SSHL_AUTO_AUTH_IT_KEY 的 sshd"]
    async fn auto_auth_against_real_sshd() {
        use crate::known_hosts::HostKeyCheck;
        use std::process::Command;
        let (Ok(target), Ok(user), Ok(key)) =
            (std::env::var("SSHL_AUTO_AUTH_IT"), std::env::var("SSHL_AUTO_AUTH_IT_USER"), std::env::var("SSHL_AUTO_AUTH_IT_KEY"))
        else {
            return;
        };
        let (host, port) = target.split_once(':').unwrap();
        let port: u16 = port.parse().unwrap();
        let dir = std::env::temp_dir().join(format!("sshl-auto-it-{}", &uuid::Uuid::new_v4().to_string()[..8]));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good");
        std::fs::copy(&key, &good).unwrap();
        let locked = dir.join("locked");
        std::fs::copy(&key, &locked).unwrap();
        for path in [&good, &locked] {
            std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
        }
        assert!(Command::new("ssh-keygen").args(["-q", "-p", "-N", "secret", "-P", "", "-f"]).arg(&locked).status().unwrap().success());
        let other = dir.join("other");
        assert!(Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&other).status().unwrap().success());
        let run = |plan: Plan| {
            let user = user.clone();
            async move {
                let mut handle = crate::ssh::connect_unauthenticated(host, port, HostKeyCheck::trust_any_for_tests()).await.unwrap();
                authenticate_with(&mut handle, &user, &plan).await
            }
        };
        let files = |paths: &[&PathBuf]| Plan { files: paths.iter().map(|p| p.to_path_buf()).collect(), agent: AgentSource::Disabled, identities_only: false };

        run(files(&[&other, &good])).await.expect("第二把私钥对得上就登录");
        let err = run(files(&[&other])).await.unwrap_err().to_string();
        assert!(err.contains("免密登录没有成功") && err.contains("other"), "{err}");
        let err = run(files(&[&locked])).await.unwrap_err().to_string();
        assert!(err.contains("设了密码"), "{err}");

        // 自己起一个 ssh-agent，加进那把对的私钥
        let sock = dir.join("agent.sock");
        let out = Command::new("ssh-agent").arg("-a").arg(&sock).output().unwrap();
        let pid = String::from_utf8_lossy(&out.stdout).split("SSH_AGENT_PID=").nth(1).and_then(|s| s.split(';').next()).unwrap().to_string();
        assert!(Command::new("ssh-add").arg("-q").arg(&good).env("SSH_AUTH_SOCK", &sock).status().unwrap().success());
        let agent = |files: Vec<PathBuf>, identities_only| Plan { files, agent: AgentSource::Socket(sock.clone()), identities_only };
        run(agent(vec![], false)).await.expect("Agent 里的密钥登录");
        // 设了密码的私钥文件，Agent 里已有它：用 Agent 登录，不报「设了密码」
        std::fs::write(dir.join("locked.pub"), std::fs::read_to_string(format!("{key}.pub")).unwrap()).unwrap();
        run(agent(vec![locked.clone()], true)).await.expect("IdentitiesOnly：Agent 里和私钥文件对得上的可以用");
        let err = run(agent(vec![other.clone()], true)).await.unwrap_err().to_string();
        assert!(!err.contains("SSH Agent"), "IdentitiesOnly：Agent 里对不上的不用：{err}");
        let _ = Command::new("kill").arg(&pid).status();

        // 完整流程（Credentials::Auto）：免认证 → ssh -G → 默认私钥。家目录换成只放了那把对的私钥的临时目录
        let home = dir.join("home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::copy(&good, home.join(".ssh").join("id_ed25519")).unwrap();
        std::env::set_var("HOME", &home);
        std::env::remove_var("SSH_AUTH_SOCK");
        crate::ssh::do_handshake(host, port, &user, crate::ssh::Credentials::Auto, HostKeyCheck::trust_any_for_tests())
            .await
            .expect("免密：~/.ssh/id_ed25519 登录");
        std::fs::remove_file(home.join(".ssh").join("id_ed25519")).unwrap();
        let err = crate::ssh::do_handshake(host, port, &user, crate::ssh::Credentials::Auto, HostKeyCheck::trust_any_for_tests())
            .await
            .err()
            .expect("没有私钥应失败")
            .to_string();
        assert!(err.contains("没有可用的密钥"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn follows_identity_settings_from_ssh_config() {
        let home = Path::new("/Users/me");
        let out = "user deploy\nhostname 10.0.0.5\nport 22\nidentitiesonly yes\nidentityfile ~/.ssh/id_work\nidentityfile %d/keys/%r@%h\nidentityagent ~/.1password/agent.sock\n";
        let plan = parse_ssh_g(out, home, "10.0.0.5", "deploy");
        assert_eq!(plan.files, [PathBuf::from("/Users/me/.ssh/id_work"), PathBuf::from("/Users/me/keys/deploy@10.0.0.5")]);
        assert_eq!(plan.agent, AgentSource::Socket(PathBuf::from("/Users/me/.1password/agent.sock")));
        assert!(plan.identities_only);
    }

    #[test]
    fn falls_back_to_default_keys_and_the_environment_agent() {
        let home = Path::new("/Users/me");
        let plan = parse_ssh_g("user me\nidentitiesonly no\n", home, "h", "me");
        assert_eq!(plan, default_plan(home));
        assert_eq!(plan.files[0], PathBuf::from("/Users/me/.ssh/id_ed25519"));
        assert_eq!(parse_ssh_g("identityagent none\n", home, "h", "me").agent, AgentSource::Disabled);
        assert_eq!(parse_ssh_g("identityagent SSH_AUTH_SOCK\n", home, "h", "me").agent, AgentSource::Env);
    }

    #[test]
    fn only_plain_host_names_reach_ssh() {
        for ok in ["example.com", "10.0.0.5", "fe80::1%en0", "[::1]", "my_host-1"] {
            assert!(plain_host(ok), "{ok}");
        }
        for bad in ["", "-oProxyCommand=x", "a b", "a;b", "$(x)", "a\nb"] {
            assert!(!plain_host(bad), "{bad:?}");
        }
    }

    #[test]
    fn explains_what_was_tried() {
        let none = failure_message(&Tried::default());
        assert!(none.contains("没有可用的密钥"), "{none}");
        let both = failure_message(&Tried { agent_keys: 2, files: vec!["id_ed25519".into(), "id_rsa".into()], encrypted: vec![] });
        assert!(both.contains("SSH Agent 里的 2 把密钥和id_ed25519、id_rsa"), "{both}");
        let locked = failure_message(&Tried { agent_keys: 0, files: vec![], encrypted: vec!["id_work".into()] });
        assert!(locked.contains("id_work 设了密码") && locked.contains("ssh-add"), "{locked}");
    }

    #[test]
    fn tells_usable_encrypted_and_foreign_files_apart() {
        let dir = std::env::temp_dir().join(format!("sshl-auto-auth-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = KeyPair::generate_ed25519().unwrap();
        let mut pem = Vec::new();
        russh_keys::encode_pkcs8_pem(&key, &mut pem).unwrap();
        std::fs::write(dir.join("plain"), &pem).unwrap();
        let mut locked = Vec::new();
        russh_keys::encode_pkcs8_pem_encrypted(&key, b"secret", 100, &mut locked).unwrap();
        std::fs::write(dir.join("locked"), &locked).unwrap();
        std::fs::write(dir.join("notes"), "hello").unwrap();
        assert!(matches!(load(&dir.join("plain")), Loaded::Key(_)));
        assert!(matches!(load(&dir.join("locked")), Loaded::Encrypted));
        // ssh-keygen 默认的 OpenSSH 格式，设密码 / 不设密码
        for (name, pass) in [("openssh", ""), ("openssh_locked", "secret")] {
            let ok = std::process::Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", pass, "-f"])
                .arg(dir.join(name))
                .status()
                .is_ok_and(|s| s.success());
            assert!(ok, "ssh-keygen");
        }
        assert!(matches!(load(&dir.join("openssh")), Loaded::Key(_)));
        assert!(matches!(load(&dir.join("openssh_locked")), Loaded::Encrypted));
        assert!(public_of(&dir.join("openssh_locked")).is_some(), "加密的也能从 .pub 拿到公钥");
        assert!(matches!(load(&dir.join("notes")), Loaded::Unusable));
        assert!(matches!(load(&dir.join("missing")), Loaded::Unusable));
        // 加密私钥靠同名 .pub 认出 Agent 里的那把
        assert_eq!(public_of(&dir.join("locked")), None);
        let public = key.clone_public_key().unwrap();
        std::fs::write(dir.join("locked.pub"), format!("{} {} me@host\n", public.name(), russh_keys::PublicKeyBase64::public_key_base64(&public))).unwrap();
        assert_eq!(public_of(&dir.join("locked")), Some(public.clone()));
        assert_eq!(public_of(&dir.join("plain")), Some(public));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
