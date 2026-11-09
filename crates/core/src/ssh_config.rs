//! 从 `~/.ssh/config` 导入主机：列出 `Host` 别名（不含通配），逐个用 `ssh -G 别名` 解析出真正的主机名、
//! 用户、端口、私钥（和终端里 ssh 看到的一样，Match、Include、默认值都由 ssh 自己处理）。

use std::path::Path;
use std::time::Duration;

/// 一个可导入的主机。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportCandidate {
    /// config 里的别名，导入后作连接名。
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// config 里为它写了 IdentityFile 时的私钥（家目录下写成 `~/...`）；没写为 None（导入为「自动」）。
    pub identity: Option<String>,
}

/// config 里的 Host 别名，按出现顺序去重；带 `*` `?` `!` 的是模式，不是一台主机。
pub fn host_aliases(config: &str) -> Vec<String> {
    let mut aliases: Vec<String> = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        let mut words = line.split_whitespace();
        let Some(keyword) = words.next() else { continue };
        // `Host foo` 或 `Host=foo`
        let rest: Vec<&str> = match keyword.split_once('=') {
            Some((k, v)) if k.eq_ignore_ascii_case("host") => std::iter::once(v).chain(words).collect(),
            _ if keyword.eq_ignore_ascii_case("host") => words.collect(),
            _ => continue,
        };
        for alias in rest {
            let alias = alias.trim_matches('"');
            if alias.is_empty() || alias.contains(['*', '?', '!']) || aliases.iter().any(|a| a == alias) {
                continue;
            }
            aliases.push(alias.to_string());
        }
    }
    aliases
}

/// OpenSSH 没配 IdentityFile 时 `ssh -G` 列出的默认私钥（不同版本多几个少几个）。
const DEFAULT_IDENTITIES: [&str; 7] =
    ["~/.ssh/id_rsa", "~/.ssh/id_ecdsa", "~/.ssh/id_ecdsa_sk", "~/.ssh/id_ed25519", "~/.ssh/id_ed25519_sk", "~/.ssh/id_xmss", "~/.ssh/id_dsa"];

/// `ssh -G 别名` 的输出 → 导入项。`home` 用来把家目录下的私钥写成 `~/...`、判断文件在不在。
pub fn candidate_from_ssh_g(alias: &str, out: &str, home: &Path) -> Option<ImportCandidate> {
    let (mut host, mut user, mut port) = (None, None, 22u16);
    let mut identities: Vec<String> = Vec::new();
    for line in out.lines() {
        let Some((keyword, value)) = line.split_once(' ') else { continue };
        let value = value.trim();
        match keyword {
            "hostname" => host = Some(value.to_string()),
            "user" => user = Some(value.to_string()),
            "port" => port = value.parse().unwrap_or(22),
            "identityfile" => identities.push(value.to_string()),
            _ => {}
        }
    }
    let explicit = identities.iter().any(|i| !DEFAULT_IDENTITIES.contains(&i.as_str()));
    let identity = explicit
        .then(|| {
            identities.iter().find_map(|i| {
                let path = match i.strip_prefix("~/") {
                    Some(rest) => home.join(rest),
                    None => Path::new(i).to_path_buf(),
                };
                path.is_file().then(|| match path.strip_prefix(home) {
                    Ok(rest) => format!("~/{}", rest.display()),
                    Err(_) => path.display().to_string(),
                })
            })
        })
        .flatten();
    Some(ImportCandidate { alias: alias.to_string(), host: host?, port, user: user?, identity })
}

/// 读 `~/.ssh/config` 列出可导入的主机（逐个 `ssh -G`，并发）。没有 config 或没装 ssh 时为空。
pub async fn candidates() -> Vec<ImportCandidate> {
    let home = dirs::home_dir().unwrap_or_default();
    let path = home.join(".ssh").join("config");
    let Ok(config) = tokio::fs::read_to_string(&path).await else { return Vec::new() };
    let lookups = host_aliases(&config).into_iter().map(|alias| {
        let (home, path) = (home.clone(), path.clone());
        async move {
            // -F：解析的就是上面读的这份 config
            let run = tokio::process::Command::new("ssh")
                .arg("-G")
                .arg("-F")
                .arg(&path)
                .args(["--", &alias])
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
                .output();
            let out = tokio::time::timeout(Duration::from_secs(3), run).await.ok()?.ok()?;
            out.status.success().then(|| candidate_from_ssh_g(&alias, &String::from_utf8_lossy(&out.stdout), &home)).flatten()
        }
    });
    futures::future::join_all(lookups).await.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_concrete_hosts_but_not_patterns() {
        let config = "Host *\n  ServerAliveInterval 30\nHost web web-2 \"db\"\n  HostName 10.0.0.5\nhost=bastion\nHost *.internal !skip\nHost web\nMatch host foo\n";
        assert_eq!(host_aliases(config), ["web", "web-2", "db", "bastion"]);
    }

    #[test]
    fn resolves_host_user_port_and_an_explicit_key() {
        let dir = std::env::temp_dir().join(format!("sshl-import-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join(".ssh")).unwrap();
        std::fs::write(dir.join(".ssh/work_key"), "x").unwrap();
        let out = "user deploy\nhostname 10.0.0.5\nport 2200\nidentityfile ~/.ssh/missing\nidentityfile ~/.ssh/work_key\n";
        let c = candidate_from_ssh_g("web", out, &dir).unwrap();
        assert_eq!(c, ImportCandidate { alias: "web".into(), host: "10.0.0.5".into(), port: 2200, user: "deploy".into(), identity: Some("~/.ssh/work_key".into()) });
        // 没写 IdentityFile：ssh -G 列的是默认私钥（即使文件在）→ 导入为「自动」
        std::fs::write(dir.join(".ssh/id_rsa"), "x").unwrap();
        let defaults = "user me\nhostname h\nport 22\nidentityfile ~/.ssh/id_rsa\nidentityfile ~/.ssh/id_ecdsa\nidentityfile ~/.ssh/id_ed25519\n";
        assert_eq!(candidate_from_ssh_g("h", defaults, &dir).unwrap().identity, None);
        // 写了但文件都不在：也按「自动」
        assert_eq!(candidate_from_ssh_g("h", "user me\nhostname h\nidentityfile ~/.ssh/gone\n", &dir).unwrap().identity, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
