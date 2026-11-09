//! 终端此刻是谁。用户常在终端里 `su -` / `sudo -i` 切成别的用户（多半是 root），
//! 文件管理上传应以这个用户的身份进行，而不是 SSH 登录用户。
//!
//! 终端和文件管理共用一条 SSH 连接，连接上所有通道的进程都挂在同一个 sshd 会话进程下：
//! exec 命令里的 `$PPID` 就是它，它下面带 tty 的子进程就是终端的登录 shell。
//! 从登录 shell 顺着通往前台进程的那条链往下走，最深的那个 shell 的有效 uid 就是终端当前用户；
//! 链上第一个 su / sudo 说明是怎么切过去的（决定要谁的密码）。

/// 在 exec 通道里跑：第一行是 sshd 会话进程 pid，后面是进程表。
pub const PROBE_COMMAND: &str = "echo \"$PPID\"; LC_ALL=C ps -eo pid=,ppid=,uid=,tty=,stat=,comm=";

#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub has_tty: bool,
    /// ps stat 带 `+`：在其终端的前台进程组里。
    pub foreground: bool,
    pub comm: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Su,
    Sudo,
}

impl Via {
    pub fn as_str(self) -> &'static str {
        match self {
            Via::Su => "su",
            Via::Sudo => "sudo",
        }
    }
}

/// 解析 PROBE_COMMAND 的输出：(sshd 会话 pid, 进程表)。comm 可能带空格，放在最后整段取。
pub fn parse_probe(out: &str) -> Option<(u32, Vec<Proc>)> {
    let mut lines = out.lines();
    let sshd = lines.next()?.trim().parse().ok()?;
    let procs = lines
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let ppid = it.next()?.parse().ok()?;
            let uid = it.next()?.parse().ok()?;
            let tty = it.next()?;
            let stat = it.next()?;
            let comm = it.collect::<Vec<_>>().join(" ");
            Some(Proc {
                pid,
                ppid,
                uid,
                has_tty: !matches!(tty, "?" | "??" | "-"),
                foreground: stat.contains('+'),
                comm,
            })
        })
        .collect();
    Some((sshd, procs))
}

/// 进程名去掉登录 shell 的 `-` 前缀和路径。
fn base_name(comm: &str) -> &str {
    let name = comm.trim_start_matches('-');
    name.rsplit('/').next().unwrap_or(name)
}

fn is_shell(comm: &str) -> bool {
    matches!(base_name(comm), "sh" | "bash" | "dash" | "zsh" | "ksh" | "mksh" | "ash" | "fish" | "tcsh" | "csh" | "yash")
}

fn children(procs: &[Proc], pid: u32) -> impl Iterator<Item = &Proc> {
    procs.iter().filter(move |p| p.ppid == pid)
}

/// 子树里有没有前台进程：分叉时（后台任务 + su）沿着通往前台的那支走。
fn leads_to_foreground(procs: &[Proc], start: &Proc) -> bool {
    let mut stack = vec![start];
    for _ in 0..=procs.len() {
        let Some(p) = stack.pop() else { return false };
        if p.foreground {
            return true;
        }
        stack.extend(children(procs, p.pid));
    }
    false
}

/// 终端当前用户：(有效 uid, 怎么切过去的)。没切换时 via 为 None。
/// 找不到终端 shell（比如服务器开了 hidepid、进程不可见）返回 None，调用方按登录用户处理。
pub fn terminal_user(sshd: u32, procs: &[Proc]) -> Option<(u32, Option<Via>)> {
    let login = procs.iter().find(|p| p.ppid == sshd && p.has_tty && is_shell(&p.comm))?;
    let (mut cur, mut shell, mut via) = (login, login, None);
    for _ in 0..64 {
        let kids: Vec<&Proc> = children(procs, cur.pid).collect();
        let next = match kids.as_slice() {
            [] => break,
            [only] => *only,
            many => match many.iter().find(|p| leads_to_foreground(procs, p)) {
                Some(p) => *p,
                None => break,
            },
        };
        if via.is_none() {
            via = match base_name(&next.comm) {
                "su" => Some(Via::Su),
                "sudo" => Some(Via::Sudo),
                _ => None,
            };
        }
        if is_shell(&next.comm) {
            shell = next;
        }
        cur = next;
    }
    if shell.uid == login.uid {
        Some((login.uid, None))
    } else {
        // 链上没有 su/sudo（runuser、machinectl 等）：按 su 处理，要的是目标用户自己的密码
        Some((shell.uid, Some(via.unwrap_or(Via::Su))))
    }
}

/// 文件管理当前以谁的身份运行（终端切换了用户之后）。密码只留在后端内存里，会话结束即丢弃。
pub struct Elevation {
    pub user: String,
    pub via: Via,
    pub password: Option<String>,
}

const SFTP_SERVERS: &str = "/usr/lib/openssh/sftp-server /usr/libexec/openssh/sftp-server /usr/lib/ssh/sftp-server \
                            /usr/libexec/sftp-server /usr/lib/sftp-server /usr/local/libexec/sftp-server /usr/sbin/sftp-server";
/// 切换成功、即将 exec sftp-server 的标记（走 stderr，不污染 SFTP 用的 stdout）。
pub const SFTP_READY: &str = "SSHL_SFTP_READY";
pub const NO_SFTP_SERVER: &str = "SSHL_NO_SFTP_SERVER";
/// sudo 的密码提示，用固定串便于识别「要密码了」和「密码错了又问」。
pub const SUDO_PROMPT: &str = "SSHL_PW:";
/// LC_ALL=C 下 su 的密码提示。
pub const SU_PROMPT: &str = "Password:";

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// 以 `user` 身份跑 `cmd`，密码从 stdin 给：sudo 要登录用户的密码，su 要目标用户的密码
/// （util-linux su 没有 tty 也从 stdin 读）。LC_ALL=C 让提示和报错保持英文，便于识别。
pub fn run_as_command(user: &str, via: Via, with_password: bool, cmd: &str) -> String {
    match via {
        Via::Sudo => {
            let auth = if with_password { format!("-S -p {}", quote(SUDO_PROMPT)) } else { "-n".into() };
            format!("LC_ALL=C sudo {auth} -u {} sh -c {}", quote(user), quote(cmd))
        }
        Via::Su => format!("LC_ALL=C su {} -c {}", quote(user), quote(&format!("sh -c {}", quote(cmd)))),
    }
}

/// 切到目标用户后起 sftp-server：先进他的家目录（文件管理的「主目录」随之变成 /root 等），
/// 找到 sftp-server 就在 stderr 打就绪标记再 exec，之后 stdout/stdin 就是 SFTP 协议流。
pub fn sftp_server_script() -> String {
    format!(
        "h=$(getent passwd \"$(id -un)\" 2>/dev/null | cut -d: -f6); cd \"${{h:-$HOME}}\" 2>/dev/null || cd /; \
         for p in {SFTP_SERVERS}; do if [ -x \"$p\" ]; then printf {SFTP_READY} >&2; exec \"$p\"; fi; done; \
         echo {NO_SFTP_SERVER} >&2; exit 127"
    )
}

/// 去掉输出里的密码提示，免得出现在报错信息里。
pub fn clean_auth_noise(s: &str) -> String {
    s.replace(SUDO_PROMPT, "").replace(SU_PROMPT, "").trim().to_string()
}

/// su 失败归类（标记与 sudo 一致，前端按同一套逻辑决定是否再要密码）。
pub fn su_failure(output: &str) -> &'static str {
    let s = output.to_ascii_lowercase();
    if s.contains("authentication failure") || s.contains("incorrect password") || s.contains("sorry") {
        "SUDO_PASSWORD_INCORRECT"
    } else if s.contains("permission denied") {
        "SUDO_NOT_ALLOWED"
    } else {
        "SUDO_FAILED"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with_fakes(cmd: &str, stdin: &[u8]) -> std::process::Output {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let bin = std::env::temp_dir().join(format!("sshl-fake-su-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&bin).unwrap();
        // 假 su / sudo：只认密码 rootpw / alicepw，然后照常执行 -c 后的命令
        let fakes = [
            ("su", "#!/bin/sh\nread -r pw; [ \"$pw\" = rootpw ] || { echo 'su: Authentication failure' >&2; exit 1; }\nshift; [ \"$1\" = -c ] && exec sh -c \"$2\"\n"),
            ("sudo", "#!/bin/sh\nwhile [ $# -gt 0 ]; do case \"$1\" in -S) read -r pw; [ \"$pw\" = alicepw ] || exit 1; shift;; -p|-u) shift 2;; -n) shift;; *) break;; esac; done\nexec \"$@\"\n"),
        ];
        for (name, body) in fakes {
            std::fs::write(bin.join(name), body).unwrap();
            std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
        let mut child = std::process::Command::new("sh").arg("-c").arg(cmd).env("PATH", path)
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
            .spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin).unwrap();
        let out = child.wait_with_output().unwrap();
        let _ = std::fs::remove_dir_all(&bin);
        out
    }

    #[test]
    fn run_as_keeps_awkward_arguments_intact_through_su_and_sudo() {
        // 文件名带单引号、空格、$：经过两层 sh -c 引号仍原样到达
        let cmd = "printf '%s|' \"it's a \\$file\" done";
        for (via, pw) in [(Via::Su, "rootpw\n"), (Via::Sudo, "alicepw\n")] {
            let out = run_with_fakes(&run_as_command("root", via, true, cmd), pw.as_bytes());
            assert!(out.status.success(), "{via:?}: {}", String::from_utf8_lossy(&out.stderr));
            assert_eq!(String::from_utf8_lossy(&out.stdout), "it's a $file|done|", "{via:?}");
        }
        let out = run_with_fakes(&run_as_command("root", Via::Su, true, "true"), b"wrong\n");
        assert!(!out.status.success());
        assert_eq!(su_failure(&String::from_utf8_lossy(&out.stderr)), "SUDO_PASSWORD_INCORRECT");
        // 免密 sudo 用 -n，不读 stdin
        assert!(run_as_command("root", Via::Sudo, false, "true").contains("sudo -n -u 'root'"));
    }

    #[test]
    fn classifies_su_failures_and_strips_prompts() {
        assert_eq!(su_failure("Password: su: Authentication failure"), "SUDO_PASSWORD_INCORRECT");
        assert_eq!(su_failure("su: Sorry"), "SUDO_PASSWORD_INCORRECT");
        assert_eq!(su_failure("su: Permission denied"), "SUDO_NOT_ALLOWED");
        assert_eq!(su_failure("su: user nobody2 does not exist"), "SUDO_FAILED");
        assert_eq!(clean_auth_noise("Password: rm: cannot remove 'x'\n"), "rm: cannot remove 'x'");
        assert_eq!(clean_auth_noise("SSHL_PW:Sorry, try again.\nSSHL_PW:"), "Sorry, try again.");
    }

    // 进程表片段：sshd 会话 100，alice=1001，root=0，bob=1002。最后一行是 exec 通道本身（无 tty）。
    fn probe(rows: &str) -> (u32, Vec<Proc>) {
        parse_probe(&format!("100\n{rows}  300   100  1001 ?        Ss   sh\n  301   300  1001 ?        R    ps\n")).unwrap()
    }

    #[test]
    fn plain_login_shell_is_the_login_user() {
        let (sshd, procs) = probe("  101   100  1001 pts/0    Ss+  bash\n");
        assert_eq!(terminal_user(sshd, &procs), Some((1001, None)));
    }

    #[test]
    fn su_to_root_is_root_via_su() {
        let (sshd, procs) = probe("  101   100  1001 pts/0    Ss   bash\n  102   101     0 pts/0    S    su\n  103   102     0 pts/0    S+   bash\n");
        assert_eq!(terminal_user(sshd, &procs), Some((0, Some(Via::Su))));
    }

    #[test]
    fn sudo_i_with_use_pty_is_root_via_sudo() {
        // sudo 1.9.14+ 默认 use_pty：命令跑在新的 pty 上，链里有两层 sudo
        let (sshd, procs) = probe("  101   100  1001 pts/0    Ss   bash\n  102   101     0 pts/0    S+   sudo\n  103   102     0 pts/1    Ss   sudo\n  104   103     0 pts/1    Ss+  bash\n");
        assert_eq!(terminal_user(sshd, &procs), Some((0, Some(Via::Sudo))));
    }

    #[test]
    fn sudo_su_counts_as_sudo() {
        let (sshd, procs) = probe("  101   100  1001 pts/0    Ss   -bash\n  102   101     0 pts/0    S+   sudo\n  103   102     0 pts/0    S    su\n  104   103     0 pts/0    S+   -bash\n");
        assert_eq!(terminal_user(sshd, &procs), Some((0, Some(Via::Sudo))));
    }

    #[test]
    fn program_running_in_root_shell_still_means_root() {
        let (sshd, procs) = probe("  101   100  1001 pts/0    Ss   bash\n  102   101     0 pts/0    S    su\n  103   102     0 pts/0    S    bash\n  104   103     0 pts/0    S+   vim\n");
        assert_eq!(terminal_user(sshd, &procs), Some((0, Some(Via::Su))));
    }

    #[test]
    fn sudo_vim_from_login_shell_is_still_the_login_user() {
        // 前台是 root 的 vim，但没有 root 的 shell：终端用户仍是 alice
        let (sshd, procs) = probe("  101   100  1001 pts/0    Ss   bash\n  102   101     0 pts/0    S+   sudo\n  103   102     0 pts/0    S+   vim\n");
        assert_eq!(terminal_user(sshd, &procs), Some((1001, None)));
    }

    #[test]
    fn background_job_does_not_hide_the_su_shell() {
        let (sshd, procs) = probe("  101   100  1001 pts/0    Ss   bash\n  105   101  1001 pts/0    S    sleep\n  102   101     0 pts/0    S    su\n  103   102  1002 pts/0    S+   bash\n");
        assert_eq!(terminal_user(sshd, &procs), Some((1002, Some(Via::Su))));
    }

    #[test]
    fn invisible_processes_fall_back_to_login_user() {
        // hidepid：看不到 root 的进程，链在 alice 的 shell 处断掉
        let (sshd, procs) = probe("  101   100  1001 pts/0    Ss   bash\n");
        assert_eq!(terminal_user(sshd, &procs), Some((1001, None)));
        // 进程表里找不到终端 shell
        let (sshd, procs) = probe("");
        assert_eq!(terminal_user(sshd, &procs), None);
    }

    #[test]
    fn parses_bsd_ps_and_comm_with_spaces() {
        let (sshd, procs) = parse_probe("7\n  8 7 501 s000 Ss /bin/zsh\n  9 8 0 s000 S+ Web Content\n 10 7 501 ?? Ss sh\n").unwrap();
        assert_eq!(sshd, 7);
        assert_eq!(procs[1].comm, "Web Content");
        assert!(procs[0].has_tty && !procs[2].has_tty);
        assert_eq!(terminal_user(sshd, &procs), Some((501, None)));
    }
}
