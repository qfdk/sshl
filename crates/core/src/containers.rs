//! 「系统」页的容器：Docker（没有就 Podman）的容器列表，启动 / 停止 / 重启 / 删除。
//! 看日志、进容器都发到终端里做（`logs_command` / `shell_command`）。和进程列表一样以文件管理当前身份跑：
//! 不在 docker 组里会被拒（`services::needs_root` 认得出），界面提议 sudo。

use crate::error::AppResult;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    /// running / exited / created / paused / restarting / dead
    pub state: String,
    /// docker 的原话，如 `Up 3 hours`、`Exited (137) 2 days ago`。
    pub status: String,
    /// 原样的端口映射，显示用 `short_ports`。
    pub ports: String,
}

impl ContainerInfo {
    pub fn is_running(&self) -> bool {
        matches!(self.state.as_str(), "running" | "restarting" | "paused")
    }

    /// 退出码（`Exited (137) …`）；不是退出状态时 None。
    pub fn exit_code(&self) -> Option<i32> {
        let rest = self.status.strip_prefix("Exited (")?;
        rest.split(')').next()?.trim().parse().ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Containers {
    /// docker 或 podman：之后的操作用同一个。
    pub engine: String,
    pub list: Vec<ContainerInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerAction {
    Start,
    Stop,
    Restart,
    Remove,
}

impl ContainerAction {
    fn verb(self) -> &'static str {
        match self {
            ContainerAction::Start => "start",
            ContainerAction::Stop => "stop",
            ContainerAction::Restart => "restart",
            ContainerAction::Remove => "rm",
        }
    }
}

const FORMAT: &str = "'{{.ID}}\\t{{.Names}}\\t{{.Image}}\\t{{.State}}\\t{{.Status}}\\t{{.Ports}}'";

/// 先 docker 后 podman；都没有输出 `@@none`。没权限时 ps 失败，报错原样带回来。
pub fn containers_command() -> String {
    format!(
        "if command -v docker >/dev/null 2>&1; then echo @@docker; docker ps -a --no-trunc --format {FORMAT}; \
         elif command -v podman >/dev/null 2>&1; then echo @@podman; podman ps -a --no-trunc --format {FORMAT}; \
         else echo @@none; fi"
    )
}

/// None = 没装 Docker / Podman。排序：在跑的在前，同类按名字。
pub fn parse_containers(out: &str) -> Option<Containers> {
    let mut lines = out.lines();
    let engine = match lines.next()?.trim() {
        "@@docker" => "docker",
        "@@podman" => "podman",
        _ => return None,
    };
    let mut list: Vec<ContainerInfo> = lines
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let (id, name, image) = (fields.next()?.trim(), fields.next()?.trim(), fields.next()?.trim());
            if id.is_empty() {
                return None;
            }
            let state = fields.next().unwrap_or_default().trim();
            let status = fields.next().unwrap_or_default().trim().to_string();
            let ports = fields.next().unwrap_or_default().trim().to_string();
            // 老版本没有 .State，从 Status 推
            let state = if state.is_empty() { state_from_status(&status) } else { state.to_lowercase() };
            Some(ContainerInfo { id: id.into(), name: name.into(), image: image.into(), state, status, ports })
        })
        .collect();
    list.sort_by(|a, b| b.is_running().cmp(&a.is_running()).then_with(|| a.name.cmp(&b.name)));
    Some(Containers { engine: engine.into(), list })
}

fn state_from_status(status: &str) -> String {
    let state = if status.starts_with("Up") {
        if status.contains("(Paused)") { "paused" } else { "running" }
    } else if status.starts_with("Exited") {
        "exited"
    } else if status.starts_with("Restarting") {
        "restarting"
    } else if status.starts_with("Created") {
        "created"
    } else if status.starts_with("Dead") {
        "dead"
    } else {
        ""
    };
    state.into()
}

/// `0.0.0.0:8080->80/tcp, [::]:8080->80/tcp, 443/tcp` → `8080→80/tcp, 443/tcp`：去掉监听地址，IPv4 / IPv6 同一条只留一个。
pub fn short_ports(ports: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for mapping in ports.split(", ").map(str::trim).filter(|m| !m.is_empty()) {
        let short = match mapping.split_once("->") {
            Some((host, container)) => format!("{}→{container}", host.rsplit(':').next().unwrap_or(host)),
            None => mapping.to_string(),
        };
        if !out.contains(&short) {
            out.push(short);
        }
    }
    out.join(", ")
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub fn action_command(engine: &str, action: ContainerAction, id: &str) -> String {
    format!("{engine} {} {}", action.verb(), quote(id))
}

/// 没权限时发到终端的命令。
pub fn terminal_command(engine: &str, action: ContainerAction, name: &str) -> String {
    format!("sudo {}", action_command(engine, action, name))
}

/// 在终端里看日志：最近 100 行并接着跟。
pub fn logs_command(engine: &str, name: &str) -> String {
    format!("{engine} logs -f --tail 100 {}", quote(name))
}

/// 在终端里进容器：有 bash 用 bash，没有用 sh（alpine 之类）。
pub fn shell_command(engine: &str, name: &str) -> String {
    format!("{engine} exec -it {} sh -c 'command -v bash >/dev/null 2>&1 && exec bash || exec sh'", quote(name))
}

pub async fn containers(state: &AppState, session_id: String) -> AppResult<Option<Containers>> {
    Ok(parse_containers(&crate::server_info::run(state, &session_id, &containers_command()).await?))
}

pub async fn act(state: &AppState, session_id: String, engine: String, action: ContainerAction, id: String) -> AppResult<()> {
    crate::server_info::run(state, &session_id, &action_command(&engine, action, &id)).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// docker 29：--no-trunc 的完整 ID，端口 IPv4 / IPv6 各一条。
    const DOCKER: &str = "@@docker
b0264835a487fe07f96ebe5f8fab00251aa225cc24708d9c631b1443bc6c237b\tstopped-one\talpine\texited\tExited (0) 3 seconds ago\t
a4700b3e244a6c170d8c01889161e788516b6d8560dd76629db989f34994bd86\tweb\tnginx:alpine\trunning\tUp 4 seconds\t0.0.0.0:8080->80/tcp, [::]:8080->80/tcp
c1\tdb\tpostgres:16\texited\tExited (137) 2 days ago\t
d2\tapi\tghcr.io/acme/api:1.2\trunning\tUp 3 hours (healthy)\t127.0.0.1:3000->3000/tcp, 9229/tcp
";

    #[test]
    fn running_containers_come_first() {
        let c = parse_containers(DOCKER).unwrap();
        assert_eq!(c.engine, "docker");
        let names: Vec<&str> = c.list.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["api", "web", "db", "stopped-one"]);
        let web = &c.list[1];
        assert_eq!((web.image.as_str(), web.state.as_str(), web.is_running()), ("nginx:alpine", "running", true));
        assert_eq!(web.id.len(), 64);
        assert_eq!(c.list[2].exit_code(), Some(137));
        assert_eq!(c.list[3].exit_code(), Some(0));
        assert_eq!(web.exit_code(), None);
    }

    #[test]
    fn old_docker_without_a_state_column_is_read_from_the_status() {
        let c = parse_containers("@@docker\nabc\tweb\tnginx\t\tUp 2 hours (Paused)\t\ndef\tjob\tbusybox\t\tExited (1) 5 minutes ago\t\n").unwrap();
        assert_eq!(c.list.iter().map(|c| c.state.as_str()).collect::<Vec<_>>(), ["paused", "exited"]);
    }

    #[test]
    fn no_engine_and_podman() {
        assert_eq!(parse_containers("@@none\n"), None);
        let c = parse_containers("@@podman\n").unwrap();
        assert_eq!((c.engine.as_str(), c.list.len()), ("podman", 0));
    }

    #[test]
    fn ports_drop_the_listen_address_and_the_ipv6_twin() {
        assert_eq!(short_ports("0.0.0.0:8080->80/tcp, [::]:8080->80/tcp"), "8080→80/tcp");
        assert_eq!(short_ports("127.0.0.1:3000->3000/tcp, 9229/tcp"), "3000→3000/tcp, 9229/tcp");
        assert_eq!(short_ports(""), "");
    }

    #[test]
    fn commands_quote_names_and_use_the_same_engine() {
        assert_eq!(action_command("docker", ContainerAction::Restart, "web"), "docker restart 'web'");
        assert_eq!(action_command("podman", ContainerAction::Remove, "abc"), "podman rm 'abc'");
        assert_eq!(terminal_command("docker", ContainerAction::Stop, "web"), "sudo docker stop 'web'");
        assert_eq!(logs_command("docker", "web"), "docker logs -f --tail 100 'web'");
        assert_eq!(shell_command("docker", "web"), "docker exec -it 'web' sh -c 'command -v bash >/dev/null 2>&1 && exec bash || exec sh'");
        assert!(containers_command().contains("--format '{{.ID}}\\t{{.Names}}"), "制表符交给 docker 的模板，不让 shell 展开");
    }

    #[test]
    fn a_user_outside_the_docker_group_needs_root() {
        assert!(crate::services::needs_root(
            "permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock: Get \"http://%2Fvar%2Frun%2Fdocker.sock/v1.47/containers/json?all=1\": dial unix /var/run/docker.sock: connect: permission denied"
        ));
    }

    /// 真实的 Docker 机器（lima 虚拟机，docker-rootful 模板）：需要一个在跑的 `web`。停了再起、日志命令能跑。
    /// `SSHL_SYSTEMD_IT=127.0.0.1:60022 SSHL_SYSTEMD_IT_USER=me SSHL_SYSTEMD_IT_KEY=~/.lima/_config/user cargo test --lib containers_against_real_docker -- --ignored`
    #[tokio::test]
    #[ignore = "需要 SSHL_SYSTEMD_IT / _USER / _KEY 指向一台装了 Docker、跑着 web 容器的 Linux"]
    async fn containers_against_real_docker() {
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
        let web = || async {
            let (status, out, err) = exec(containers_command()).await;
            assert_eq!(status, 0, "{err}");
            parse_containers(&out).expect("有 docker").list.into_iter().find(|c| c.name == "web").expect("有 web 容器")
        };
        let before = web().await;
        assert!(before.is_running() && short_ports(&before.ports).contains("→80/tcp"), "{before:?}");
        let (status, _, err) = exec(action_command("docker", ContainerAction::Stop, &before.id)).await;
        assert_eq!(status, 0, "{err}");
        assert_eq!(web().await.state, "exited");
        let (status, _, err) = exec(action_command("docker", ContainerAction::Start, &before.id)).await;
        assert_eq!(status, 0, "{err}");
        assert!(web().await.is_running());
        let (status, out, err) = exec(logs_command("docker", "web").replace("-f ", "")).await;
        assert_eq!(status, 0, "{err}");
        assert!(!(out + &err).is_empty(), "有日志");
    }
}
