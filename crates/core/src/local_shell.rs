//! 本机终端：openpty + 用户的登录 shell（macOS 上同 Ghostty 经 /usr/bin/login 起，见 [`login_launch`]）。
//! 对界面的接口同 SSH 终端：键盘输入、改尺寸、流控确认、关闭都经 [`ShellHandle`] 进来，输出交给 [`TerminalSink`]。
//!
//! 每个会话四个线程：读（poll 主端 → sink，积压到 [`BACKLOG_LIMIT`] 就停下等确认）、写（键盘输入写主端，
//! 程序不读输入时写会阻塞，所以单独一个线程）、分派（收 ShellHandle 的消息：确认 / 改尺寸 / 关闭当场处理，
//! 输入转给写线程，不排在卡住的写后面）、等待（waitpid，shell 退出后叫醒读线程收尾）。

use std::ffi::{CStr, OsStr};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};

use crate::ssh::{Outbound, ShellReceiver, TerminalSink};

/// 读出来、终端线程还没解析完的字节到这么多就先不读（同 SSH 终端的 2MB 窗口）：
/// 刷屏时 ^C 之后要滚完的尾巴以它为界。
const BACKLOG_LIMIT: u64 = 2 * 1024 * 1024;

pub struct LocalOptions {
    /// 起始目录；None 或不存在 = 用户主目录。
    pub cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
    /// 测试用：指定程序和参数。None = 用户的登录 shell（argv[0] 带 `-`，读 .zprofile 等）。
    pub program: Option<(PathBuf, Vec<String>)>,
}

/// 运行中的本机 shell。drop 不结束进程：关闭走 [`crate::ssh::ShellHandle::close`]。
pub struct LocalShell {
    /// 子进程：经 login 起的是 login，否则就是 shell。
    pid: libc::pid_t,
    via_login: bool,
    master: Arc<OwnedFd>,
    exit: Arc<Mutex<Option<Option<u32>>>>,
}

impl LocalShell {
    pub fn pid(&self) -> u32 {
        self.pid as u32
    }

    /// shell 本身的进程号（[`shell_of`]）。
    fn shell_pid(&self) -> Option<libc::pid_t> {
        shell_of(self.pid, self.via_login)
    }

    /// 前台在跑的不是 shell 自己（vim、ssh、npm run dev…）时返回那个程序名：关标签前要问一声。
    /// shell 刚起来、还没把自己设成前台进程组时，前台是 login 那一组，也不算在忙。
    pub fn foreground_program(&self) -> Option<String> {
        if self.exit.lock().unwrap().is_some() {
            return None;
        }
        let fg = foreground_group(self.master.as_raw_fd())?;
        if fg == self.pid || Some(fg) == self.shell_pid() {
            return None;
        }
        Some(process_name(fg).unwrap_or_else(|| crate::tr!("程序").into()))
    }

    /// 前台进程（在提示符下就是 shell 自己）的当前目录：新开的本机标签从这里开始。
    pub fn cwd(&self) -> Option<PathBuf> {
        let fg = foreground_group(self.master.as_raw_fd());
        fg.and_then(process_cwd).or_else(|| self.shell_pid().and_then(process_cwd))
    }
}

/// 起什么：程序、参数、argv[0]，以及是不是经 login 起的（那样 shell 是子进程下面的那个）。
struct Launch {
    program: PathBuf,
    args: Vec<String>,
    arg0: Option<String>,
    via_login: bool,
}

/// 用户的登录 shell。macOS 上同 Ghostty / 系统终端经 /usr/bin/login 起（[`login_launch`]），起不了才直接起
/// （argv[0] 带 `-`，算登录 shell，读 .zprofile 等）。
fn shell_launch() -> Launch {
    let shell = login_shell();
    #[cfg(target_os = "macos")]
    if let Some(launch) = login_launch(&shell) {
        return launch;
    }
    let name = shell.file_name().unwrap_or(OsStr::new("sh")).to_string_lossy().to_string();
    Launch { program: shell, args: Vec::new(), arg0: Some(format!("-{name}")), via_login: false }
}

/// 经 /usr/bin/login 起 shell（同 Ghostty、系统终端、iTerm）：login 先打一行「Last login: … on ttys000」，在 utmpx 里记下这次登录
/// （`who` / `last` 看得到），再起 shell（用户 2026-11-23：Ghostty 第一行是 Last login，SSHL 第一行是空的——ys 这类主题在提示符前先换一行）。
#[cfg(target_os = "macos")]
fn login_launch(shell: &Path) -> Option<Launch> {
    let login = Path::new("/usr/bin/login");
    if !login.exists() {
        return None;
    }
    let (user, home) = current_user()?;
    let hush = home.join(".hushlogin").exists();
    Some(Launch { program: login.into(), args: login_args(&user, shell, hush), arg0: None, via_login: true })
}

/// login 的参数，同 Ghostty：`-f` 自己登录自己不要密码，`-p` 留着我们给的环境，`-l` 不切到主目录（新标签从当前目录开始）。
/// `-l` 同时让 login 不给 argv[0] 加「-」，所以中间套一层 bash 的 `exec -l`，shell 照旧是登录 shell。
/// `-l` 时 login 在当前目录里找 .hushlogin，所以主目录里有 .hushlogin 时自己加 `-q`。
fn login_args(user: &str, shell: &Path, hush: bool) -> Vec<String> {
    let quoted = format!("'{}'", shell.to_string_lossy().replace('\'', r"'\''"));
    [if hush { "-qflp" } else { "-flp" }, user, "/bin/bash", "--noprofile", "--norc", "-c"]
        .into_iter()
        .map(String::from)
        .chain([format!("exec -l {quoted}")])
        .collect()
}

/// 当前用户的账户名和主目录（getpwuid，不信环境变量）。
#[cfg(target_os = "macos")]
fn current_user() -> Option<(String, PathBuf)> {
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_name.is_null() || (*pw).pw_dir.is_null() {
            return None;
        }
        let name = CStr::from_ptr((*pw).pw_name).to_string_lossy().to_string();
        let home = PathBuf::from(CStr::from_ptr((*pw).pw_dir).to_string_lossy().to_string());
        Some((name, home))
    }
}

/// shell 本身的进程号：直接起的就是子进程；经 login 起的是 login 下面那个（login fork 出 shell，自己留着等它退出、收尾 utmpx）。
fn shell_of(pid: libc::pid_t, via_login: bool) -> Option<libc::pid_t> {
    if via_login { children(pid).first().copied() } else { Some(pid) }
}

#[cfg(target_os = "macos")]
fn children(pid: libc::pid_t) -> Vec<libc::pid_t> {
    let mut buf = [0 as libc::pid_t; 64];
    let n = unsafe { libc::proc_listchildpids(pid, buf.as_mut_ptr().cast(), std::mem::size_of_val(&buf) as libc::c_int) };
    // 有的系统版本返回个数，有的返回字节数：都按上限截，空位是 0
    let n = usize::try_from(n).unwrap_or(0).min(buf.len());
    buf[..n].iter().copied().filter(|p| *p > 0).collect()
}

#[cfg(not(target_os = "macos"))]
fn children(_: libc::pid_t) -> Vec<libc::pid_t> {
    Vec::new()
}

/// 起本机 shell。`shell` 是 [`crate::ssh::shell_channel`] 的接收端，写端交给终端线程。
pub fn spawn(mut options: LocalOptions, shell: ShellReceiver, sink: Arc<dyn TerminalSink>) -> io::Result<LocalShell> {
    let launch = match options.program.take() {
        Some((program, args)) => Launch { program, args, arg0: None, via_login: false },
        None => shell_launch(),
    };
    spawn_launch(launch, options, shell, sink)
}

fn spawn_launch(launch: Launch, options: LocalOptions, mut shell: ShellReceiver, sink: Arc<dyn TerminalSink>) -> io::Result<LocalShell> {
    let (master, slave) = open_pty(options.cols, options.rows)?;
    let mut command = Command::new(&launch.program);
    command.args(&launch.args);
    if let Some(arg0) = &launch.arg0 {
        command.arg0(arg0);
    }
    let via_login = launch.via_login;
    let cwd = options.cwd.filter(|dir| dir.is_dir()).or_else(dirs::home_dir);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let (remove, set) = child_env(std::env::vars_os().map(|(k, _)| k.to_string_lossy().to_string()), default_lang);
    for key in remove {
        command.env_remove(key);
    }
    command.envs(set);
    command
        .stdin(Stdio::from(File::from(slave.try_clone()?)))
        .stdout(Stdio::from(File::from(slave.try_clone()?)))
        .stderr(Stdio::from(File::from(slave)));
    // 新会话、以这个 PTY 为控制终端：^C / ^Z / 作业控制才对，关掉主端时整个会话收到 SIGHUP
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    // 父进程手里的从端副本都在 command 里：放掉，shell 退出后主端才读得到 EOF
    drop(command);
    let pid = child.id() as libc::pid_t;

    let master = Arc::new(master);
    let reader = File::from(master.try_clone()?);
    let mut writer = File::from(master.try_clone()?);
    let (wake_rx, mut wake_tx) = pipe()?;
    let flow = Arc::new(Flow::default());
    let exit: Arc<Mutex<Option<Option<u32>>>> = Arc::default();

    let waiter_exit = exit.clone();
    std::thread::Builder::new().name("local-shell-wait".into()).spawn(move || {
        let code = child.wait().ok().and_then(|status| status.code().or_else(|| status.signal().map(|s| 128 + s))).map(|c| c as u32);
        *waiter_exit.lock().unwrap() = Some(code);
        let _ = wake_tx.write_all(&[1]);
    })?;

    let reader_flow = flow.clone();
    let reader_exit = exit.clone();
    std::thread::Builder::new().name("local-shell-read".into()).spawn(move || {
        pump(&reader, &wake_rx, &reader_flow, sink.as_ref());
        // 主端读完了（shell 关了终端）但进程可能还没退出：等它退出再报关闭
        wait_readable(wake_rx.as_raw_fd(), -1);
        let code = reader_exit.lock().unwrap().unwrap_or(None);
        sink.closed(code, "local shell exited".into());
    })?;

    let (input_tx, input_rx) = mpsc::channel::<Vec<u8>>();
    std::thread::Builder::new().name("local-shell-write".into()).spawn(move || {
        for bytes in input_rx {
            if writer.write_all(&bytes).is_err() {
                break;
            }
        }
    })?;

    let control = master.clone();
    let control_exit = exit.clone();
    std::thread::Builder::new().name("local-shell-control".into()).spawn(move || {
        while let Some(message) = shell.blocking_recv() {
            match message {
                Outbound::Data(bytes) => {
                    let _ = input_tx.send(bytes);
                }
                Outbound::Resize { cols, rows } => {
                    let _ = set_size(control.as_raw_fd(), cols as u16, rows as u16);
                }
                Outbound::Ack(bytes) => flow.ack(bytes),
                Outbound::Close => {
                    if control_exit.lock().unwrap().is_none() {
                        hang_up(control.as_raw_fd(), pid);
                    }
                }
            }
        }
        // 界面那头没了：读线程别再等确认
        flow.close();
    })?;

    Ok(LocalShell { pid, via_login, master, exit })
}

/// 输出积压记账：读线程交出去的字节，终端线程解析完确认回来。
#[derive(Default)]
struct Flow {
    state: Mutex<(u64, bool)>,
    room: Condvar,
}

impl Flow {
    fn emitted(&self, bytes: usize) {
        self.state.lock().unwrap().0 += bytes as u64;
    }

    fn ack(&self, bytes: u32) {
        let mut state = self.state.lock().unwrap();
        state.0 = state.0.saturating_sub(u64::from(bytes));
        self.room.notify_all();
    }

    fn close(&self) {
        self.state.lock().unwrap().1 = true;
        self.room.notify_all();
    }

    fn wait_for_room(&self) {
        let state = self.state.lock().unwrap();
        let _unused = self.room.wait_while(state, |(backlog, closed)| *backlog >= BACKLOG_LIMIT && !*closed).unwrap();
    }
}

/// 读主端直到 shell 关了终端（EOF / EIO）或退出（wake 可读）。退出时把缓冲里剩的读完。
fn pump(master: &File, wake: &File, flow: &Flow, sink: &dyn TerminalSink) {
    let mut buf = vec![0u8; 64 * 1024];
    let mut master_ref = master;
    loop {
        flow.wait_for_room();
        let mut fds = [
            libc::pollfd { fd: master.as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: wake.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) } == -1 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return;
        }
        if fds[0].revents != 0 {
            match master_ref.read(&mut buf) {
                Ok(0) => return,
                Ok(n) => {
                    flow.emitted(n);
                    sink.data(&buf[..n]);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                // EIO：从端全关了
                Err(_) => return,
            }
            continue;
        }
        if fds[1].revents != 0 {
            while wait_readable(master.as_raw_fd(), 0) {
                match master_ref.read(&mut buf) {
                    Ok(n) if n > 0 => sink.data(&buf[..n]),
                    _ => return,
                }
            }
            return;
        }
    }
}

/// fd 在 `timeout_ms` 内可读（或挂断）。-1 = 一直等。
fn wait_readable(fd: RawFd, timeout_ms: i32) -> bool {
    let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    loop {
        match unsafe { libc::poll(&mut pfd, 1, timeout_ms) } {
            -1 if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => continue,
            n => return n > 0,
        }
    }
}

/// 关标签：前台作业和子进程都发 SIGHUP（同关掉终端窗口）。经 login 起的子进程是 login：它是这个终端的控制进程，
/// 退出时系统挂断前台作业，shell 也跟着退（有测试）。
fn hang_up(master: RawFd, pid: libc::pid_t) {
    if let Some(fg) = foreground_group(master).filter(|fg| *fg != pid) {
        unsafe { libc::kill(-fg, libc::SIGHUP) };
    }
    unsafe { libc::kill(pid, libc::SIGHUP) };
}

fn foreground_group(master: RawFd) -> Option<libc::pid_t> {
    let fg = unsafe { libc::tcgetpgrp(master) };
    (fg > 0).then_some(fg)
}

fn open_pty(cols: u16, rows: u16) -> io::Result<(OwnedFd, OwnedFd)> {
    let (mut master, mut slave) = (-1, -1);
    let mut size = winsize(cols, rows);
    if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    set_cloexec(master.as_raw_fd())?;
    set_cloexec(slave.as_raw_fd())?;
    // IUTF8：行编辑（cat、read 这类规范模式）退格按 UTF-8 字符删，不会把中文删成半个
    unsafe {
        let mut termios: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(slave.as_raw_fd(), &mut termios) == 0 {
            termios.c_iflag |= libc::IUTF8;
            libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &termios);
        }
    }
    Ok((master, slave))
}

fn winsize(cols: u16, rows: u16) -> libc::winsize {
    libc::winsize { ws_row: rows.max(1), ws_col: cols.max(1), ws_xpixel: 0, ws_ypixel: 0 }
}

fn set_size(master: RawFd, cols: u16, rows: u16) -> io::Result<()> {
    let size = winsize(cols, rows);
    if unsafe { libc::ioctl(master, libc::TIOCSWINSZ as _, &size) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn set_cloexec(fd: RawFd) -> io::Result<()> {
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn pipe() -> io::Result<(File, File)> {
    let mut fds = [-1; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    set_cloexec(fds[0])?;
    set_cloexec(fds[1])?;
    Ok(unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) })
}

/// 用户的登录 shell：$SHELL，没有就查账户（getpwuid），再没有用 /bin/zsh。
fn login_shell() -> PathBuf {
    if let Some(shell) = std::env::var_os("SHELL").map(PathBuf::from).filter(|p| p.is_absolute() && p.exists()) {
        return shell;
    }
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if !pw.is_null() && !(*pw).pw_shell.is_null() {
            let shell = PathBuf::from(CStr::from_ptr((*pw).pw_shell).to_string_lossy().to_string());
            if shell.exists() {
                return shell;
            }
        }
    }
    PathBuf::from("/bin/zsh")
}

/// 子进程环境要去掉的和要设的。终端类型同 SSH 那边（xterm-256color）；从别的终端里启动 SSHL 时
/// 带进来的那个终端的变量去掉（[`from_launcher`]）。没设语言时补一个 UTF-8 的 LANG（从 Finder 打开的应用没有），
/// 否则 zsh 里中文显示成 <ffffffe4> 这类转义。
fn child_env(keys: impl Iterator<Item = String>, lang: impl FnOnce() -> String) -> (Vec<String>, Vec<(String, String)>) {
    let mut remove = Vec::new();
    let mut has_locale = false;
    for key in keys {
        if matches!(key.as_str(), "LANG" | "LC_ALL" | "LC_CTYPE") {
            has_locale = true;
        }
        if from_launcher(&key) {
            remove.push(key);
        }
    }
    let mut set = vec![
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
        ("TERM_PROGRAM".to_string(), "SSHL".to_string()),
        ("TERM_PROGRAM_VERSION".to_string(), env!("CARGO_PKG_VERSION").to_string()),
    ];
    if !has_locale {
        set.push(("LANG".to_string(), lang()));
    }
    (remove, set)
}

/// 说的是启动 SSHL 的那个程序、不是这个新终端的变量：别的终端（Ghostty、iTerm）的，开发测试口的，
/// 还有 Claude Code 给它的命令设的会话标记（在 Claude Code 里 `open -a SSHL` 时整个带进来：本机终端里再开的 claude
/// 会以为自己是那次会话的子会话，不存对话记录）。用户的 Claude 配置（CLAUDE_CONFIG_DIR、settings.json 里的）不在其中。
fn from_launcher(key: &str) -> bool {
    key.starts_with("GHOSTTY_")
        || key.starts_with("ITERM_")
        || key.starts_with("SSHL_DEV_")
        || key.starts_with("CLAUDE_CODE_")
        || matches!(key, "TERMINFO" | "TERM_SESSION_ID" | "CLAUDECODE" | "CLAUDE_PID" | "CLAUDE_EFFORT" | "AI_AGENT")
}

/// 系统语言地区对应的 UTF-8 locale（同 Terminal / Ghostty），系统里没有这个 locale 就用 en_US.UTF-8。
fn default_lang() -> String {
    lang_for(&system_locale().unwrap_or_default(), |name| Path::new("/usr/share/locale").join(name).is_dir())
}

/// `zh-Hans_CN` / `zh_CN` / `en_FR@rg=frzzzz` → `zh_CN.UTF-8` / `en_FR.UTF-8`（存在的话）。
fn lang_for(identifier: &str, exists: impl Fn(&str) -> bool) -> String {
    let identifier = identifier.split('@').next().unwrap_or_default();
    let language = identifier.split(['-', '_']).next().unwrap_or_default();
    let region = identifier.rsplit_once('_').map(|(_, r)| r).unwrap_or_default();
    let candidate = format!("{language}_{region}.UTF-8");
    if !language.is_empty() && !region.is_empty() && exists(&candidate) {
        candidate
    } else {
        "en_US.UTF-8".into()
    }
}

#[cfg(target_os = "macos")]
fn system_locale() -> Option<String> {
    use core_foundation::base::{CFRelease, TCFType};
    use core_foundation::string::{CFString, CFStringRef};

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFLocaleCopyCurrent() -> *const std::ffi::c_void;
        fn CFLocaleGetIdentifier(locale: *const std::ffi::c_void) -> CFStringRef;
    }
    unsafe {
        let locale = CFLocaleCopyCurrent();
        if locale.is_null() {
            return None;
        }
        let id = CFLocaleGetIdentifier(locale);
        let text = (!id.is_null()).then(|| CFString::wrap_under_get_rule(id).to_string());
        CFRelease(locale);
        text
    }
}

#[cfg(not(target_os = "macos"))]
fn system_locale() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn process_name(pid: libc::pid_t) -> Option<String> {
    let mut buf = [0u8; 256];
    let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).to_string())
}

#[cfg(not(target_os = "macos"))]
fn process_name(pid: libc::pid_t) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|s| s.trim().to_string())
}

#[cfg(target_os = "macos")]
fn process_cwd(pid: libc::pid_t) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;

    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, (&mut info as *mut libc::proc_vnodepathinfo).cast(), size) };
    if n != size {
        return None;
    }
    let raw: &[u8; 1024] = unsafe { &*(info.pvi_cdir.vip_path.as_ptr().cast()) };
    let path = CStr::from_bytes_until_nul(raw).ok()?;
    let path = PathBuf::from(OsStr::from_bytes(path.to_bytes()));
    (!path.as_os_str().is_empty()).then_some(path)
}

#[cfg(not(target_os = "macos"))]
fn process_cwd(pid: libc::pid_t) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::{shell_channel, ShellHandle};
    use std::time::{Duration, Instant};

    /// 收输出的 sink：攒起来，关闭时记下退出码。
    #[derive(Default)]
    struct Collect {
        out: Mutex<Vec<u8>>,
        closed: Mutex<Option<Option<u32>>>,
    }

    impl TerminalSink for Collect {
        fn data(&self, bytes: &[u8]) {
            self.out.lock().unwrap().extend_from_slice(bytes);
        }

        fn closed(&self, code: Option<u32>, _: String) {
            *self.closed.lock().unwrap() = Some(code);
        }
    }

    impl Collect {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.out.lock().unwrap()).to_string()
        }

        fn wait(&self, what: &str, until: impl Fn(&Self) -> bool) {
            let start = Instant::now();
            while !until(self) {
                assert!(start.elapsed() < Duration::from_secs(10), "等 {what} 超时，输出：{:?}", self.text());
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    fn start(script: &str, cwd: Option<PathBuf>) -> (LocalShell, ShellHandle, Arc<Collect>) {
        let (handle, rx) = shell_channel();
        let sink = Arc::new(Collect::default());
        let options = LocalOptions { cwd, cols: 100, rows: 30, program: Some(("/bin/sh".into(), vec!["-c".into(), script.into()])) };
        let shell = spawn(options, rx, sink.clone()).expect("spawn");
        (shell, handle, sink)
    }

    #[test]
    fn output_and_exit_code_reach_the_sink() {
        let (_shell, _handle, sink) = start("printf 'hello 中文'; exit 3", None);
        sink.wait("退出", |s| s.closed.lock().unwrap().is_some());
        assert!(sink.text().contains("hello 中文"), "{:?}", sink.text());
        assert_eq!(*sink.closed.lock().unwrap(), Some(Some(3)));
    }

    #[test]
    fn the_child_runs_on_a_terminal_of_the_requested_size_and_follows_resizes() {
        let (_shell, handle, sink) = start("stty size; read line; stty size", None);
        sink.wait("首个尺寸", |s| s.text().contains("30 100"));
        handle.resize(120, 40);
        handle.write(b"go\r".to_vec());
        sink.wait("新尺寸", |s| s.text().contains("40 120"));
        assert!(sink.text().contains("go"), "输入经过 PTY 回显");
    }

    #[test]
    fn output_pauses_at_the_backlog_limit_until_acknowledged() {
        let (_shell, handle, sink) = start("head -c 6000000 /dev/zero | tr '\\0' x", None);
        let len = |s: &Collect| s.out.lock().unwrap().len() as u64;
        sink.wait("读满积压上限", |s| len(s) >= BACKLOG_LIMIT);
        std::thread::sleep(Duration::from_millis(200));
        let stalled = len(&sink);
        assert!(stalled < BACKLOG_LIMIT + 64 * 1024, "没确认时最多多读一块：{stalled}");
        handle.ack(stalled as u32);
        sink.wait("确认后继续读", |s| len(s) > stalled);
    }

    #[test]
    fn close_hangs_up_the_shell() {
        let (_shell, handle, sink) = start("sleep 30", None);
        std::thread::sleep(Duration::from_millis(100));
        handle.close();
        sink.wait("挂断后退出", |s| s.closed.lock().unwrap().is_some());
        assert_eq!(*sink.closed.lock().unwrap(), Some(Some(128 + libc::SIGHUP as u32)));
    }

    #[test]
    fn reports_the_working_directory_and_a_busy_foreground_job() {
        let dir = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        // 交互 sh：有作业控制，前台命令有自己的进程组
        let (handle, rx) = shell_channel();
        let sink = Arc::new(Collect::default());
        let options = LocalOptions { cwd: Some(dir.clone()), cols: 80, rows: 24, program: Some(("/bin/sh".into(), vec!["-i".into()])) };
        let shell = spawn(options, rx, sink.clone()).expect("spawn");
        handle.write(b"echo ready\r".to_vec());
        sink.wait("提示符", |s| s.text().matches("ready").count() >= 2);
        assert_eq!(shell.foreground_program(), None, "提示符下不算在忙");
        assert_eq!(shell.cwd().as_deref(), Some(dir.as_path()));
        handle.write(b"sleep 30\r".to_vec());
        let start = Instant::now();
        while shell.foreground_program().is_none() {
            assert!(start.elapsed() < Duration::from_secs(5), "sleep 没进前台");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(shell.foreground_program().as_deref(), Some("sleep"));
        handle.close();
        sink.wait("关闭", |s| s.closed.lock().unwrap().is_some());
    }

    /// 同 Ghostty：`login -flp 用户 /bin/bash --noprofile --norc -c "exec -l shell"`，主目录里有 .hushlogin 时加 -q。
    #[test]
    fn the_shell_is_started_through_login_with_the_same_arguments_as_ghostty() {
        assert_eq!(
            login_args("qfdk", Path::new("/bin/zsh"), false),
            ["-flp", "qfdk", "/bin/bash", "--noprofile", "--norc", "-c", "exec -l '/bin/zsh'"]
        );
        assert_eq!(login_args("qfdk", Path::new("/bin/zsh"), true)[0], "-qflp");
        assert_eq!(login_args("a", Path::new("/opt/it's/zsh"), false)[6], r"exec -l '/opt/it'\''s/zsh'", "路径里的单引号转义");
    }

    /// 经 login 起：login 打「Last login」，shell 在 login 下面（login 自己 fork 出 shell、在一边等它退出）。
    /// 提示符下不算在忙、跑着 sleep 算；-l 不切到主目录，起始目录照旧；关闭能把整个会话挂断。
    #[cfg(target_os = "macos")]
    #[test]
    fn a_session_through_login_keeps_its_directory_and_knows_its_shell() {
        let dir = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let launch = login_launch(Path::new("/bin/sh")).expect("/usr/bin/login");
        let (handle, rx) = shell_channel();
        let sink = Arc::new(Collect::default());
        let options = LocalOptions { cwd: Some(dir.clone()), cols: 80, rows: 24, program: None };
        let shell = spawn_launch(launch, options, rx, sink.clone()).expect("spawn");
        handle.write(b"echo ready-$((40+2))\r".to_vec());
        sink.wait("提示符", |s| s.text().contains("ready-42"));
        if !dirs::home_dir().unwrap().join(".hushlogin").exists() {
            assert!(sink.text().contains("Last login"), "{:?}", sink.text());
        }
        let sh = shell.shell_pid().expect("login 下面的 shell");
        assert_ne!(sh, shell.pid, "子进程是 login，shell 是它下面的");
        assert_eq!(shell.foreground_program(), None, "提示符下不算在忙");
        assert_eq!(shell.cwd().as_deref(), Some(dir.as_path()), "没被切到主目录");
        handle.write(b"sleep 30\r".to_vec());
        let start = Instant::now();
        while shell.foreground_program().is_none() {
            assert!(start.elapsed() < Duration::from_secs(5), "sleep 没进前台");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(shell.foreground_program().as_deref(), Some("sleep"));
        handle.close();
        sink.wait("挂断后退出", |s| s.closed.lock().unwrap().is_some());
        assert_eq!(unsafe { libc::kill(sh, 0) }, -1, "shell 也退出了");
    }

    #[test]
    fn child_env_sets_the_terminal_and_a_utf8_locale_only_when_missing() {
        let keys = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter();
        let (remove, set) = child_env(keys(&["HOME", "GHOSTTY_RESOURCES_DIR", "TERMINFO", "SSHL_DEV_SOCKET"]), || "zh_CN.UTF-8".into());
        assert_eq!(remove, ["GHOSTTY_RESOURCES_DIR", "TERMINFO", "SSHL_DEV_SOCKET"]);
        assert!(set.contains(&("TERM".into(), "xterm-256color".into())));
        assert!(set.contains(&("LANG".into(), "zh_CN.UTF-8".into())));
        let (_, set) = child_env(keys(&["LC_ALL"]), || unreachable!("已有语言设置就不问系统"));
        assert!(!set.iter().any(|(k, _)| k == "LANG"));
    }

    /// SSHL 是在 Claude Code 的会话里打开的（比如部署完 `open -a SSHL`）时，它的环境里带着那次会话的标记；
    /// 原样传下去的话，本机终端里再开的 claude 以为自己是那次会话的子会话，不存对话记录（「Transcript saving is off」）。
    /// 用户自己的配置留着。
    #[test]
    fn child_env_drops_the_markers_of_a_claude_code_session_that_launched_sshl() {
        let keys = ["HOME", "CLAUDECODE", "CLAUDE_CODE_CHILD_SESSION", "CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_MESSAGING_SOCKET", "CLAUDE_PID", "CLAUDE_EFFORT", "AI_AGENT", "CLAUDE_CONFIG_DIR", "ANTHROPIC_API_KEY"];
        let (remove, _) = child_env(keys.iter().map(|k| k.to_string()), || "en_US.UTF-8".into());
        assert_eq!(remove, ["CLAUDECODE", "CLAUDE_CODE_CHILD_SESSION", "CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_MESSAGING_SOCKET", "CLAUDE_PID", "CLAUDE_EFFORT", "AI_AGENT"]);
    }

    #[test]
    fn lang_follows_the_system_region_when_that_locale_exists() {
        let have = |name: &str| ["zh_CN.UTF-8", "en_US.UTF-8", "fr_FR.UTF-8"].contains(&name);
        assert_eq!(lang_for("zh-Hans_CN", have), "zh_CN.UTF-8");
        assert_eq!(lang_for("fr_FR@rg=frzzzz", have), "fr_FR.UTF-8");
        assert_eq!(lang_for("en_FR", have), "en_US.UTF-8", "没有这个组合就退到 en_US");
        assert_eq!(lang_for("", have), "en_US.UTF-8");
    }
}
