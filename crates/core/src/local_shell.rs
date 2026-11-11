//! 本机终端：openpty + 用户的登录 shell。对界面的接口同 SSH 终端：键盘输入、改尺寸、流控确认、关闭
//! 都经 [`ShellHandle`] 进来，输出交给 [`TerminalSink`]。
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
    pid: libc::pid_t,
    master: Arc<OwnedFd>,
    exit: Arc<Mutex<Option<Option<u32>>>>,
}

impl LocalShell {
    pub fn pid(&self) -> u32 {
        self.pid as u32
    }

    /// 前台在跑的不是 shell 自己（vim、ssh、npm run dev…）时返回那个程序名：关标签前要问一声。
    pub fn foreground_program(&self) -> Option<String> {
        if self.exit.lock().unwrap().is_some() {
            return None;
        }
        let fg = foreground_group(self.master.as_raw_fd())?;
        if fg == self.pid {
            return None;
        }
        Some(process_name(fg).unwrap_or_else(|| "程序".into()))
    }

    /// 前台进程（在提示符下就是 shell 自己）的当前目录：新开的本机标签从这里开始。
    pub fn cwd(&self) -> Option<PathBuf> {
        let fg = foreground_group(self.master.as_raw_fd());
        fg.and_then(process_cwd).or_else(|| process_cwd(self.pid))
    }
}

/// 起本机 shell。`shell` 是 [`crate::ssh::shell_channel`] 的接收端，写端交给终端线程。
pub fn spawn(options: LocalOptions, mut shell: ShellReceiver, sink: Arc<dyn TerminalSink>) -> io::Result<LocalShell> {
    let (master, slave) = open_pty(options.cols, options.rows)?;
    let mut command = match options.program {
        Some((program, args)) => {
            let mut command = Command::new(program);
            command.args(args);
            command
        }
        None => {
            let shell = login_shell();
            let name = shell.file_name().unwrap_or(OsStr::new("sh")).to_string_lossy().to_string();
            let mut command = Command::new(&shell);
            command.arg0(format!("-{name}"));
            command
        }
    };
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

    Ok(LocalShell { pid, master, exit })
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

/// 关标签：前台作业和 shell 都发 SIGHUP（同关掉终端窗口）。
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
/// 带进来的那个终端的变量去掉。没设语言时补一个 UTF-8 的 LANG（从 Finder 打开的应用没有），
/// 否则 zsh 里中文显示成 <ffffffe4> 这类转义。
fn child_env(keys: impl Iterator<Item = String>, lang: impl FnOnce() -> String) -> (Vec<String>, Vec<(String, String)>) {
    let mut remove = Vec::new();
    let mut has_locale = false;
    for key in keys {
        if matches!(key.as_str(), "LANG" | "LC_ALL" | "LC_CTYPE") {
            has_locale = true;
        }
        if key.starts_with("GHOSTTY_") || key.starts_with("ITERM_") || key.starts_with("SSHL_DEV_") || key == "TERMINFO" || key == "TERM_SESSION_ID" {
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

    #[test]
    fn lang_follows_the_system_region_when_that_locale_exists() {
        let have = |name: &str| ["zh_CN.UTF-8", "en_US.UTF-8", "fr_FR.UTF-8"].contains(&name);
        assert_eq!(lang_for("zh-Hans_CN", have), "zh_CN.UTF-8");
        assert_eq!(lang_for("fr_FR@rg=frzzzz", have), "fr_FR.UTF-8");
        assert_eq!(lang_for("en_FR", have), "en_US.UTF-8", "没有这个组合就退到 en_US");
        assert_eq!(lang_for("", have), "en_US.UTF-8");
    }
}
