//! 「系统」页的服务：systemd 的 service 单元，启动 / 停止 / 重启 / 开机启动。
//! 和进程列表一样以文件管理当前身份跑；不是 root 时 systemctl 要认证、会失败（`needs_root`），
//! 界面就提议改在终端里用 sudo 执行（`terminal_command`）。

use std::collections::HashMap;

use crate::error::AppResult;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInfo {
    /// 完整单元名，如 `nginx.service`。
    pub unit: String,
    pub description: String,
    /// ACTIVE 列：active / inactive / failed / activating / deactivating / reloading。
    pub active: String,
    /// SUB 列：running / exited / dead / failed …
    pub sub: String,
    /// 开机启动：enabled / disabled / static / masked …；读不到为空。
    pub enabled: String,
}

impl ServiceInfo {
    /// 列表里显示的名字（去掉 `.service`）。
    pub fn name(&self) -> &str {
        self.unit.strip_suffix(".service").unwrap_or(&self.unit)
    }

    pub fn is_running(&self) -> bool {
        matches!(self.active.as_str(), "active" | "activating" | "reloading")
    }

    pub fn starts_at_boot(&self) -> bool {
        self.enabled.starts_with("enabled")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
    Enable,
    Disable,
}

impl ServiceAction {
    fn verb(self) -> &'static str {
        match self {
            ServiceAction::Start => "start",
            ServiceAction::Stop => "stop",
            ServiceAction::Restart => "restart",
            ServiceAction::Enable => "enable",
            ServiceAction::Disable => "disable",
        }
    }
}

/// 没有 systemd（不是它起的系统、容器里、macOS）时只输出 `@@none`。
/// list-units 只有载入内存的单元；没被引用的已停服务（比如 disable 掉的 nginx）只在 list-unit-files 里，两份合起来。
pub const SERVICES_COMMAND: &str = "if [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1; then \
     echo @@units; LC_ALL=C systemctl list-units --type=service --all --no-legend --no-pager --plain 2>/dev/null; \
     echo @@files; LC_ALL=C systemctl list-unit-files --type=service --no-legend --no-pager 2>/dev/null; \
     else echo @@none; fi";

/// None = 这台机器没有 systemd。排序：失败的在最前，再是在跑的，然后其余；同类按名字。
pub fn parse_services(out: &str) -> Option<Vec<ServiceInfo>> {
    let mut section = "";
    let mut services: Vec<ServiceInfo> = Vec::new();
    let mut files: Vec<(String, String)> = Vec::new();
    for line in out.lines() {
        if let Some(name) = line.trim().strip_prefix("@@") {
            if name == "none" {
                return None;
            }
            section = if name == "units" { "units" } else if name == "files" { "files" } else { "" };
            continue;
        }
        // 老版本（systemd 245 等）即使 --plain 也在失败的单元前画 ●
        let line = line.trim_start().trim_start_matches(['●', '*']).trim_start();
        let mut fields = line.split_whitespace();
        match section {
            "units" => {
                let (Some(unit), Some(load), Some(active), Some(sub)) = (fields.next(), fields.next(), fields.next(), fields.next()) else { continue };
                if load == "not-found" || is_template(unit) || !unit.ends_with(".service") {
                    continue;
                }
                services.push(ServiceInfo {
                    unit: unit.into(),
                    description: fields.collect::<Vec<_>>().join(" "),
                    active: active.into(),
                    sub: sub.into(),
                    enabled: String::new(),
                });
            }
            "files" => {
                if let (Some(unit), Some(state)) = (fields.next(), fields.next()) {
                    files.push((unit.into(), state.into()));
                }
            }
            _ => {}
        }
    }
    let loaded: HashMap<String, usize> = services.iter().enumerate().map(|(i, s)| (s.unit.clone(), i)).collect();
    for (unit, state) in files {
        match loaded.get(&unit) {
            Some(&i) => services[i].enabled = state,
            // 没载入的：只列出能开关的（enabled / disabled），alias、static、模板这些不是用户要管的
            None if !is_template(&unit) && matches!(state.as_str(), "enabled" | "disabled") => services.push(ServiceInfo {
                unit,
                description: String::new(),
                active: "inactive".into(),
                sub: "dead".into(),
                enabled: state,
            }),
            None => {}
        }
    }
    services.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.unit.to_lowercase().cmp(&b.unit.to_lowercase())));
    Some(services)
}

fn is_template(unit: &str) -> bool {
    unit.ends_with("@.service")
}

/// 跑完就退的一次性服务（active / exited，如 apparmor）排在常驻的后面，和停了的放一起。
fn rank(service: &ServiceInfo) -> u8 {
    match (service.active.as_str(), service.sub.as_str()) {
        ("failed", _) => 0,
        ("active", "running") | ("activating" | "reloading", _) => 1,
        _ => 2,
    }
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// 这里执行用的命令。`--no-ask-password`：不是 root 时直接失败，不去等一个没人回答的认证。
pub fn action_command(action: ServiceAction, unit: &str) -> String {
    format!("systemctl --no-ask-password {} {}", action.verb(), quote(unit))
}

/// 没权限时发到终端里的命令（用户在那里输 sudo 密码，或点「填充密码」）。
pub fn terminal_command(action: ServiceAction, unit: &str) -> String {
    format!("sudo systemctl {} {}", action.verb(), quote(unit))
}

/// 在终端里看日志：最近 100 行并接着跟（Ctrl-C 退出）。
pub fn logs_command(unit: &str) -> String {
    format!("journalctl -u {} -n 100 -f", quote(unit))
}

/// systemctl / kill 的报错是不是因为不是 root。
pub fn needs_root(error: &str) -> bool {
    let lower = error.to_lowercase();
    ["access denied", "authentication is required", "interactive authentication required", "not permitted", "permission denied"]
        .iter()
        .any(|needle| lower.contains(needle))
}

pub async fn services(state: &AppState, session_id: String) -> AppResult<Option<Vec<ServiceInfo>>> {
    Ok(parse_services(&crate::server_info::run(state, &session_id, SERVICES_COMMAND).await?))
}

/// systemctl 默认等任务做完才返回：返回时服务已经起来 / 停下了（或失败）。
pub async fn act(state: &AppState, session_id: String, action: ServiceAction, unit: String) -> AppResult<()> {
    crate::server_info::run(state, &session_id, &action_command(action, &unit)).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ubuntu 20.04（systemd 245）：--plain 下失败的单元前仍有 ●；最后一列说明里有空格。
    const UBUNTU_2004: &str = "@@units
  accounts-daemon.service   loaded    active   running Accounts Service
  ModemManager.service      loaded    active   running Modem Manager
  apparmor.service          loaded    active   exited  Load AppArmor profiles
● auditd.service            not-found inactive dead    auditd.service
  cron.service              loaded    active   running Regular background program processing daemon
● mysql.service             loaded    failed   failed  MySQL Community Server
  nginx.service             loaded    active   running A high performance web server and a reverse proxy server
  getty@tty1.service        loaded    active   running Getty on tty1
@@files
accounts-daemon.service                enabled
apparmor.service                       enabled
autovt@.service                        enabled
cron.service                           enabled
mysql.service                          enabled
nginx.service                          disabled
redis-server.service                   disabled
dbus-org.freedesktop.timesync1.service alias
systemd-fsck-root.service              static
getty@.service                         enabled
";

    /// Debian 12（systemd 252）：没有 ●，list-unit-files 多一列 VENDOR PRESET。
    const DEBIAN_12: &str = "@@units
ssh.service loaded active running OpenBSD Secure Shell server
docker.service loaded activating start-pre Docker Application Container Engine
@@files
docker.service enabled enabled
ssh.service enabled enabled
nftables.service disabled enabled
";

    fn units(services: &[ServiceInfo]) -> Vec<&str> {
        services.iter().map(|s| s.unit.as_str()).collect()
    }

    #[test]
    fn failed_first_then_running_then_stopped_with_boot_state() {
        let services = parse_services(UBUNTU_2004).unwrap();
        assert_eq!(
            units(&services),
            ["mysql.service", "accounts-daemon.service", "cron.service", "getty@tty1.service", "ModemManager.service", "nginx.service", "apparmor.service", "redis-server.service"],
            "名字不分大小写排"
        );
        let nginx = services.iter().find(|s| s.unit == "nginx.service").unwrap();
        assert_eq!(nginx.description, "A high performance web server and a reverse proxy server");
        assert!(nginx.is_running() && !nginx.starts_at_boot());
        assert_eq!(nginx.name(), "nginx");
        let redis = services.iter().find(|s| s.unit == "redis-server.service").unwrap();
        assert_eq!((redis.active.as_str(), redis.description.as_str(), redis.enabled.as_str()), ("inactive", "", "disabled"), "没载入的已停服务也列出来");
        let mysql = &services[0];
        assert_eq!((mysql.active.as_str(), mysql.starts_at_boot()), ("failed", true), "● 去掉了");
        // auditd（not-found）、模板（@.service）、alias、static 不列
        assert!(!services.iter().any(|s| s.unit.starts_with("auditd") || s.unit.contains("@.") || s.unit.starts_with("dbus-org") || s.unit.starts_with("systemd-fsck")));
    }

    #[test]
    fn newer_systemd_output_has_a_vendor_preset_column() {
        let services = parse_services(DEBIAN_12).unwrap();
        assert_eq!(units(&services), ["docker.service", "ssh.service", "nftables.service"], "activating 算在跑");
        assert_eq!(services[1].enabled, "enabled");
        assert_eq!(services[2].enabled, "disabled");
    }

    #[test]
    fn no_systemd_is_not_an_empty_list() {
        assert_eq!(parse_services("@@none\n"), None);
        assert_eq!(parse_services("@@units\n@@files\n"), Some(Vec::new()));
    }

    #[test]
    fn commands_quote_the_unit_and_never_wait_for_a_password_prompt() {
        assert_eq!(action_command(ServiceAction::Restart, "nginx.service"), "systemctl --no-ask-password restart 'nginx.service'");
        assert_eq!(action_command(ServiceAction::Disable, "a'b.service"), r"systemctl --no-ask-password disable 'a'\''b.service'");
        assert_eq!(terminal_command(ServiceAction::Stop, "nginx.service"), "sudo systemctl stop 'nginx.service'");
        assert_eq!(logs_command("nginx.service"), "journalctl -u 'nginx.service' -n 100 -f");
    }

    /// 真实的 systemd 机器（用户能免密 sudo，如 lima 虚拟机）：列得出 ssh 服务；普通用户启停被拒且认得出是权限问题；
    /// root（sudo -n）停了再起 cron 真的生效。
    /// `SSHL_SYSTEMD_IT=127.0.0.1:60022 SSHL_SYSTEMD_IT_USER=me SSHL_SYSTEMD_IT_KEY=~/.lima/_config/user cargo test --lib services_against_real_systemd -- --ignored`
    #[tokio::test]
    #[ignore = "需要 SSHL_SYSTEMD_IT / _USER / _KEY 指向一台 systemd 的 Linux"]
    async fn services_against_real_systemd() {
        let (Ok(target), Ok(user), Ok(key)) =
            (std::env::var("SSHL_SYSTEMD_IT"), std::env::var("SSHL_SYSTEMD_IT_USER"), std::env::var("SSHL_SYSTEMD_IT_KEY"))
        else {
            return;
        };
        let (host, port) = target.split_once(':').unwrap();
        let handle = crate::ssh::do_handshake(
            host,
            port.parse().unwrap(),
            &user,
            crate::ssh::Credentials::Key { key: &key, passphrase: None },
            crate::known_hosts::HostKeyCheck::trust_any_for_tests(),
        )
        .await
        .unwrap();
        let exec = |cmd: String| {
            let handle = handle.clone();
            async move { crate::sftp::exec_capture(handle.channel_open_session().await.unwrap(), &cmd, None).await.unwrap() }
        };
        let list = || async { parse_services(&exec(SERVICES_COMMAND.into()).await.1).expect("有 systemd") };
        let find = |services: &[ServiceInfo], unit: &str| services.iter().find(|s| s.unit == unit).cloned().unwrap_or_else(|| panic!("没有 {unit}"));

        let services = list().await;
        let ssh = find(&services, "ssh.service");
        assert!(ssh.is_running() && !ssh.description.is_empty(), "{ssh:?}");
        assert!(rank(&services[0]) <= rank(&ssh));

        let (status, _, err) = exec(action_command(ServiceAction::Restart, "cron.service")).await;
        assert_ne!(status, 0);
        assert!(needs_root(&err), "普通用户被拒，认得出是权限：{err}");

        let (status, _, err) = exec(format!("sudo -n {}", action_command(ServiceAction::Stop, "cron.service"))).await;
        assert_eq!(status, 0, "{err}");
        assert_eq!(find(&list().await, "cron.service").active, "inactive");
        let (status, _, err) = exec(format!("sudo -n {}", action_command(ServiceAction::Start, "cron.service"))).await;
        assert_eq!(status, 0, "{err}");
        assert!(find(&list().await, "cron.service").is_running());
    }

    #[test]
    fn recognizes_errors_that_mean_not_root() {
        assert!(needs_root("Failed to restart nginx.service: Access denied"));
        assert!(needs_root("Failed to stop nginx.service: Interactive authentication required.\nSee system logs and 'systemctl status nginx.service' for details."));
        assert!(needs_root("kill: (1) - Operation not permitted"));
        assert!(!needs_root("Failed to start foo.service: Unit foo.service not found."));
    }
}
