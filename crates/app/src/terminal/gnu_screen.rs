//! GNU screen 不进备用屏：自动做到 `~/.screenrc` 里 `termcapinfo xterm* ti@:te@` 的效果，服务器上不用配。
//!
//! screen 每次连上终端（启动、`screen -r`、`Ctrl-a z` 挂起后 fg）都是先发 is2 再紧跟着发 smcup（`InitTerm`）。
//! xterm-256color 的 is2 是 `\e[!p\e[?3;4l\e[4l\e>`（DECSTR 打头），smcup 是 `\e[?1049h\e[22;0;0t`。
//! 照常进备用屏的话，screen 里滚出屏幕的行哪儿都不留（备用屏没有回滚历史）；滚轮在备用屏里又换成方向键，
//! 发给 screen 窗口里的程序——在 shell 里是翻历史，脚本在跑时被 tty 回显成一串 `^[[B`，还真进了脚本的输入。
//!
//! 认出「DECSTR 后面紧跟 smcup」就把这个 smcup 吞掉：screen 留在主屏上画。它往上滚动时在滚动区底边换行
//! （`ScrollV`：滚动区从第一行开始，最后一行留给 hardstatus 时也是），滚出去的行进回滚历史，滚轮、⌘F 照常用。
//! 之后它退出 / 断开时发的 rmcup（`\e[?1049l`）也吞掉，同 ti@:te@：screen 的画面留在屏幕上，提示符接在下面。
//! vim、less、htop 这类程序的 smcup 前面没有 DECSTR，照常进备用屏。

use std::borrow::Cow;
use std::ops::Range;

/// 软复位（DECSTR），xterm 系 is2 的开头。
const DECSTR: &[u8] = b"\x1b[!p";
const SMCUP: &[u8] = b"\x1b[?1049h";
const RMCUP: &[u8] = b"\x1b[?1049l";
/// DECSTR 结束后这么多字节内来的 smcup 才算 screen 的：中间只隔着 is2 剩下的 13 个字节。
const ARM_WINDOW: usize = 32;

#[derive(Default)]
pub struct GnuScreen {
    /// 吞了 screen 的 smcup、还没见到它的 rmcup。
    active: bool,
    /// DECSTR 之后还剩多少字节内来的 smcup 算 screen 的；0 = 没在等。
    armed: usize,
    /// 上一片末尾可能是 DECSTR / smcup / rmcup 开头的半截，拼上下一片再判断。
    /// 半截的转义序列交给终端也还不起作用，晚一片交过去看不出差别。
    pending: Vec<u8>,
}

impl GnuScreen {
    /// 过滤一片远端输出：去掉 screen 的 smcup / rmcup；末尾可能是半截的留到下一片。没改动时原样借出。
    pub fn filter<'a>(&mut self, bytes: &'a [u8]) -> Cow<'a, [u8]> {
        if self.pending.is_empty() {
            let (cuts, end) = self.scan(bytes);
            if cuts.is_empty() && end == bytes.len() {
                return Cow::Borrowed(bytes);
            }
            self.pending = bytes[end..].to_vec();
            return Cow::Owned(assemble(bytes, &cuts, end));
        }
        let mut data = std::mem::take(&mut self.pending);
        data.extend_from_slice(bytes);
        let (cuts, end) = self.scan(&data);
        self.pending = data[end..].to_vec();
        Cow::Owned(assemble(&data, &cuts, end))
    }

    /// 换了连接（断线原地重连）：旧连接留下的半截和状态都不算数。
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// 返回（要去掉的区间，交给终端到哪儿为止——后面是留到下一片的半截）。
    fn scan(&mut self, data: &[u8]) -> (Vec<Range<usize>>, usize) {
        let mut cuts = Vec::new();
        let mut i = 0;
        while let Some(found) = data[i..].iter().position(|&b| b == 0x1b) {
            let at = i + found;
            self.armed = self.armed.saturating_sub(at - i);
            let rest = &data[at..];
            if rest.starts_with(DECSTR) {
                self.armed = ARM_WINDOW;
                i = at + DECSTR.len();
            } else if self.armed > 0 && rest.starts_with(SMCUP) {
                cuts.push(at..at + SMCUP.len());
                self.active = true;
                self.armed = 0;
                i = at + SMCUP.len();
            } else if self.active && rest.starts_with(RMCUP) {
                cuts.push(at..at + RMCUP.len());
                self.active = false;
                i = at + RMCUP.len();
            } else if self.partial(rest) {
                return (cuts, at);
            } else {
                self.armed = self.armed.saturating_sub(1);
                i = at + 1;
            }
        }
        self.armed = self.armed.saturating_sub(data.len() - i);
        (cuts, data.len())
    }

    /// `rest`（从某个 ESC 到这片末尾）是不是还没收完的、我们要认的序列。
    fn partial(&self, rest: &[u8]) -> bool {
        let prefix_of = |pattern: &[u8]| rest.len() < pattern.len() && pattern.starts_with(rest);
        prefix_of(DECSTR) || (self.armed > 0 && prefix_of(SMCUP)) || (self.active && prefix_of(RMCUP))
    }
}

fn assemble(data: &[u8], cuts: &[Range<usize>], end: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(end);
    let mut from = 0;
    for cut in cuts {
        out.extend_from_slice(&data[from..cut.start]);
        from = cut.end;
    }
    out.extend_from_slice(&data[from..end]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// screen 连上时的开头（is2 + smcup + 后面几样），取自 screen 4.9 / 5.0 的 `InitTerm`，TERM=xterm-256color。
    const ATTACH: &[u8] = b"\x1b[!p\x1b[?3;4l\x1b[4l\x1b>\x1b[?1049h\x1b[22;0;0t\x1b[?1h\x1b=\x1b[H\x1b[2J";
    /// 去掉 smcup 之后应该交给终端的。
    const ATTACH_KEPT: &[u8] = b"\x1b[!p\x1b[?3;4l\x1b[4l\x1b>\x1b[22;0;0t\x1b[?1h\x1b=\x1b[H\x1b[2J";
    /// screen 退出 / 断开时（`FinitTerm`）：rmcup + 恢复标题。
    const DETACH: &[u8] = b"\x1b[?1049l\x1b[23;0;0t";

    /// 一片一片过滤，拼起来就是终端实际收到的。
    fn run(screen: &mut GnuScreen, pieces: &[&[u8]]) -> Vec<u8> {
        pieces.iter().flat_map(|piece| screen.filter(piece).into_owned()).collect()
    }

    #[test]
    fn screen_attaching_does_not_switch_to_the_alternate_screen() {
        let mut screen = GnuScreen::default();
        assert_eq!(run(&mut screen, &[ATTACH]), ATTACH_KEPT);
        assert!(screen.active);
        assert_eq!(run(&mut screen, &[b"output\r\n"]), b"output\r\n");
        assert_eq!(run(&mut screen, &[DETACH, b"[detached]\r\n"]), b"\x1b[23;0;0t[detached]\r\n", "rmcup 也吞掉");
        assert!(!screen.active);
    }

    /// vim / less / htop 直接发 smcup（前面没有 DECSTR）：照常进出备用屏。
    #[test]
    fn other_programs_still_use_the_alternate_screen() {
        let mut screen = GnuScreen::default();
        let vim = b"\x1b[?1049h\x1b[22;0;0t\x1b[?1h\x1b=\x1b[H\x1b[2J~\r\n\x1b[?1049l\x1b[23;0;0t";
        assert_eq!(run(&mut screen, &[vim]), vim);
        assert!(!screen.active);
    }

    /// screen 退出之后再开 vim：vim 的 smcup / rmcup 不受影响。
    #[test]
    fn after_screen_exits_later_programs_are_untouched() {
        let mut screen = GnuScreen::default();
        run(&mut screen, &[ATTACH, b"x", DETACH]);
        let vim = b"\x1b[?1049hvim\x1b[?1049l";
        assert_eq!(run(&mut screen, &[vim]), vim);
    }

    /// `tput init` / `reset` 之后隔了一阵（制表位、提示符……）才开 vim：不算 screen。
    #[test]
    fn a_soft_reset_long_before_smcup_does_not_count() {
        let mut screen = GnuScreen::default();
        let reset = [&b"\x1b[!p\x1b[?3;4l\x1b[4l\x1b>"[..], &b"\x1b[3g\x1bH".repeat(10), b"$ vim\r\n\x1b[?1049h"].concat();
        assert_eq!(run(&mut screen, &[&reset]), reset);
        assert!(!screen.active);
    }

    /// 序列被切在两片输出中间（SSH 包、64KB 解析片的边界）：哪儿切开都认得出，交给终端的字节一样。
    #[test]
    fn sequences_split_across_pieces_are_still_recognized() {
        let whole = [b"$ screen -r\r\n", ATTACH, b"line\r\n", DETACH, b"$ "].concat();
        let expected = [b"$ screen -r\r\n", ATTACH_KEPT, b"line\r\n", b"\x1b[23;0;0t", b"$ "].concat();
        for cut in 1..whole.len() {
            let mut screen = GnuScreen::default();
            let (a, b) = whole.split_at(cut);
            assert_eq!(run(&mut screen, &[a, b]), expected, "切在第 {cut} 个字节");
            assert!(screen.pending.is_empty() && !screen.active, "切在第 {cut} 个字节");
        }
        // 一个字节一片
        let mut screen = GnuScreen::default();
        let pieces: Vec<&[u8]> = whole.chunks(1).collect();
        assert_eq!(run(&mut screen, &pieces), expected);
    }

    /// 没有要认的序列时原样借出，不复制（刷屏时每片都走这里）。
    #[test]
    fn ordinary_output_is_passed_through_without_copying() {
        let mut screen = GnuScreen::default();
        let colored = b"\x1b[31mred\x1b[0m plain\r\n";
        assert!(matches!(screen.filter(colored), Cow::Borrowed(_)));
        // 末尾是半截 ESC：留到下一片
        assert_eq!(screen.filter(b"text\x1b").as_ref(), b"text");
        assert_eq!(screen.filter(b"[0m").as_ref(), b"\x1b[0m");
        assert!(screen.pending.is_empty());
    }

    /// 断线原地重连：旧连接里 screen 还开着，新连接里别的程序的 rmcup 不能被吞。
    #[test]
    fn reconnecting_forgets_the_old_screen() {
        let mut screen = GnuScreen::default();
        run(&mut screen, &[ATTACH, b"\x1b["]);
        screen.reset();
        let vim = b"\x1b[?1049hvim\x1b[?1049l";
        assert_eq!(run(&mut screen, &[vim]), vim);
    }
}
