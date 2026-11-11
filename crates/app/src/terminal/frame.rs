//! 终端线程交给界面的帧：纯数据（Send），界面线程只读它来画。
//! 行用 Arc 共享：没变的行在帧与帧之间是同一个 Arc，界面据此复用排好的字形。

use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn hex(v: u32) -> Self {
        Self { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 }
    }

    pub const fn to_u32(self) -> u32 {
        (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }
}

pub mod flags {
    pub const BOLD: u16 = 1 << 0;
    pub const ITALIC: u16 = 1 << 1;
    pub const FAINT: u16 = 1 << 2;
    pub const UNDERLINE: u16 = 1 << 3;
    pub const UNDERLINE_DOUBLE: u16 = 1 << 4;
    pub const UNDERLINE_CURLY: u16 = 1 << 5;
    pub const STRIKE: u16 = 1 << 6;
    pub const INVISIBLE: u16 = 1 << 7;
    /// 宽字符（占两格）的第一格。
    pub const WIDE: u16 = 1 << 8;
    /// 宽字符的第二格 / 软换行行尾的占位：不画字。
    pub const SPACER: u16 = 1 << 9;
    pub const SELECTED: u16 = 1 << 10;
    pub const OVERLINE: u16 = 1 << 11;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// 格子的第一个码点；空格子是 ' '。
    pub ch: char,
    /// 组合字符、emoji 修饰等第一个码点之后的部分（少见）。
    pub extra: Option<Box<str>>,
    /// 已解析好的前景色（反显、选中、粗体变亮都已算进去）。
    pub fg: Rgb,
    /// 画出来的底色；None = 终端默认底色（不单独画）。
    pub bg: Option<Rgb>,
    /// 不算选中时的底色：内边距取色只看程序画的，不跟着鼠标选区变。
    pub base_bg: Option<Rgb>,
    pub flags: u16,
}

impl Cell {
    pub const BLANK: Cell = Cell { ch: ' ', extra: None, fg: Rgb::hex(0), bg: None, base_bg: None, flags: 0 };

    pub fn has(&self, flag: u16) -> bool {
        self.flags & flag != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// 软换行：这一行排满了，内容接着下一行（长命令、长网址折成几行）。
    pub wrapped: bool,
}

impl Row {
    /// 这一行的文字（宽字符的占位格跳过），用于密码提示检测和复制。
    pub fn text(&self) -> String {
        let mut out = String::with_capacity(self.cells.len());
        for cell in &self.cells {
            if cell.has(flags::SPACER) {
                continue;
            }
            out.push(cell.ch);
            if let Some(extra) = &cell.extra {
                out.push_str(extra);
            }
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Bar,
    Block,
    Underline,
    BlockHollow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub shape: CursorShape,
    pub blinking: bool,
    pub color: Rgb,
    /// 光标所在格的字是什么颜色（方块光标里反显用）。
    pub text_color: Rgb,
}

/// 内边距延伸：边缘一整行都有底色时，把它延伸进卡片的内边距（同 Ghostty 的 window-padding-color = extend）。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct EdgeExtension {
    /// 首行每格的底色（整行都有底色时）。
    pub top: Option<Vec<Rgb>>,
    /// 末行每格的底色（整行都有底色时）。
    pub bottom: Option<Vec<Rgb>>,
    /// 每行最左一格的底色（该行整行都有底色时）。
    pub left: Vec<Option<Rgb>>,
    /// 每行最右一格的底色（该行整行都有底色时）。
    pub right: Vec<Option<Rgb>>,
}

/// 一行里一段搜索匹配（`start..end` 是格子列号，不含 end）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchSpan {
    pub y: u16,
    pub start: u16,
    pub end: u16,
    /// 是不是当前选中的那一个（↵ / ⇧↵ 走到的）。
    pub selected: bool,
}

/// 回滚搜索（⌘F）此刻的状态。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SearchInfo {
    /// 找到的总数（还在搜时是目前为止的）。
    pub total: usize,
    /// 当前选中第几个：0 是最新（最靠下）的那个。
    pub selected: Option<usize>,
    /// 回滚历史还没搜完。
    pub searching: bool,
    /// 屏幕上看得见的匹配，按行切好。
    pub spans: Vec<MatchSpan>,
}

/// 把一个匹配（首尾格子都算在内，视口坐标 `(列, 行)`）按行切成高亮段。
/// 端点不在视口里时是 None：只有头在外面 = 从上面延续下来，只有尾在外面 = 一直延续到视口下面，两头都不在就不画。
pub fn match_spans(start: Option<(u16, u32)>, end: Option<(u16, u32)>, cols: u16, rows: u16, selected: bool, out: &mut Vec<MatchSpan>) {
    if cols == 0 || rows == 0 {
        return;
    }
    let last = (cols - 1, u32::from(rows) - 1);
    let (start, end) = match (start, end) {
        (None, None) => return,
        (start, end) => (start.unwrap_or((0, 0)), end.unwrap_or(last)),
    };
    // 端点可能是反着的（选区保留方向）
    let (start, end) = if (start.1, start.0) <= (end.1, end.0) { (start, end) } else { (end, start) };
    if start.1 > last.1 {
        return;
    }
    let end = if end.1 > last.1 { last } else { end };
    for y in start.1..=end.1 {
        let from = if y == start.1 { start.0.min(cols - 1) } else { 0 };
        let to = if y == end.1 { end.0.min(cols - 1) + 1 } else { cols };
        if from < to {
            out.push(MatchSpan { y: y as u16, start: from, end: to, selected });
        }
    }
}

/// 「从新到旧」的列表被转了一圈时（后一段整体比前一段新），真正最新的那一项的下标；没转是 0。
/// `position` 给出第 i 项的位置（越大越新），二分查找，只问 O(log n) 项。
pub fn newest_index<K: Ord>(len: usize, position: impl Fn(usize) -> Option<K>) -> usize {
    if len < 2 {
        return 0;
    }
    let (Some(first), Some(last)) = (position(0), position(len - 1)) else { return 0 };
    if first > last {
        return 0;
    }
    // 最后一项比第一项新：找第一个比第一项新的
    let (mut low, mut high) = (1, len - 1);
    while low < high {
        let middle = (low + high) / 2;
        if position(middle).is_some_and(|at| at > first) {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    low
}

/// 一行里一段链接的格子（`start..end` 是格子列号，不含 end）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    pub y: u16,
    pub start: u16,
    pub end: u16,
}

/// 屏幕上的一个纯文本网址：按住 ⌘ 时画下划线、点击打开。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    /// 网址占的格子，按行切好（折行的网址有好几段）。
    pub spans: Vec<LinkSpan>,
}

/// 认作网址开头的 scheme（ASCII 字母不分大小写）。
const URL_SCHEMES: &[&str] = &["http://", "https://", "ftp://"];

/// 格子 `(x, y)` 处的纯文本网址。整条逻辑行一起看：上一行是软换行就接着往上找开头，本行是软换行就接着往下，
/// 登录类命令打印的长链接折成几行也是同一个。
pub fn link_at(rows: &[Arc<Row>], x: usize, y: usize) -> Option<Link> {
    if x >= rows.get(y)?.cells.len() {
        return None;
    }
    let mut first = y;
    while first > 0 && rows[first - 1].wrapped {
        first -= 1;
    }
    let mut last = y;
    while last + 1 < rows.len() && rows[last].wrapped {
        last += 1;
    }
    // 逻辑行的字符（宽字符的占位格跳过）和每个字符所在的格子
    let mut chars = Vec::new();
    let mut cells = Vec::new();
    let mut target = None;
    for (row_y, row) in rows.iter().enumerate().take(last + 1).skip(first) {
        for (col, cell) in row.cells.iter().enumerate() {
            if cell.has(flags::SPACER) {
                continue;
            }
            if (row_y, col) == (y, x) {
                target = Some(chars.len());
            }
            chars.push(cell.ch);
            cells.push((row_y as u16, col as u16));
        }
    }
    let target = target?;
    let mut start = 0;
    while start <= target {
        let Some(end) = url_end(&chars, start) else {
            start += 1;
            continue;
        };
        if target < end {
            let url = chars[start..end].iter().collect();
            return Some(Link { url, spans: link_spans(&cells[start..end]) });
        }
        start = end;
    }
    None
}

/// `start` 处是网址开头的话，返回网址（去掉结尾标点后）的结尾下标（不含）。
fn url_end(chars: &[char], start: usize) -> Option<usize> {
    // 前面紧挨着字母数字（xhttps://、nothttp://）不算开头
    if start > 0 && chars[start - 1].is_ascii_alphanumeric() {
        return None;
    }
    let rest = &chars[start..];
    let scheme = URL_SCHEMES
        .iter()
        .find(|scheme| scheme.len() <= rest.len() && scheme.chars().zip(rest).all(|(want, got)| got.eq_ignore_ascii_case(&want)))?;
    let body = start + scheme.len();
    let mut end = body;
    while end < chars.len() && chars[end].is_ascii_graphic() && !"\"<>\\`{}|^".contains(chars[end]) {
        end += 1;
    }
    // 结尾的标点多半属于句子；没配对的右括号是外面套的括号（网址里有对应的左括号时留着，如维基百科的 Rust_(language)）
    while end > body {
        let unpaired = |open: char, close: char| {
            let count = |c: char| chars[body..end].iter().filter(|&&x| x == c).count();
            count(open) < count(close)
        };
        let trim = match chars[end - 1] {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' => true,
            ')' => unpaired('(', ')'),
            ']' => unpaired('[', ']'),
            _ => false,
        };
        if !trim {
            break;
        }
        end -= 1;
    }
    // 只有 scheme（`https://` 单独出现）不算
    (end > body).then_some(end)
}

/// 网址的格子按行切成段。
fn link_spans(cells: &[(u16, u16)]) -> Vec<LinkSpan> {
    let mut spans: Vec<LinkSpan> = Vec::new();
    for &(y, x) in cells {
        match spans.last_mut() {
            Some(span) if span.y == y => span.end = x + 1,
            _ => spans.push(LinkSpan { y, start: x, end: x + 1 }),
        }
    }
    spans
}

/// ⌘ 点击时允许打开的链接：只开网页、FTP 和邮件（ASCII 不分大小写）；file:、javascript:、
/// 打开系统设置之类的 scheme 一律不开（OSC 8 的链接是远端程序写的，不能由它决定打开什么）。
pub fn openable(url: &str) -> bool {
    ["http://", "https://", "ftp://", "mailto:"]
        .iter()
        .any(|scheme| url.get(..scheme.len()).is_some_and(|head| head.eq_ignore_ascii_case(scheme)))
}

#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub cols: u16,
    pub rows: Vec<Arc<Row>>,
    pub cursor: Option<Cursor>,
    /// 终端默认底色（OSC 11 改过就是改后的）。
    pub background: Rgb,
    /// 卡片和内边距的底色：边缘格子的主色（全屏 TUI 的根背景），定不下来时是 background。
    pub padding: Rgb,
    pub extension: EdgeExtension,
    /// 光标行像密码提示（sudo / su 等），界面据此显示「填充密码」。
    pub password_prompt: bool,
    /// 程序开了鼠标上报（vim、htop 等）：滚轮、点击交给程序。
    pub mouse_tracking: bool,
    /// 在备用屏（全屏程序）上：没有回滚历史。
    pub alternate_screen: bool,
    /// 正在搜索（⌘F）时的状态和高亮；没在搜是 None。
    pub search: Option<Arc<SearchInfo>>,
}

fn majority(colors: &[Option<Rgb>], strict: bool) -> Option<Rgb> {
    let mut counts: HashMap<Rgb, usize> = HashMap::new();
    for color in colors.iter().flatten() {
        *counts.entry(*color).or_default() += 1;
    }
    let (best, max) = counts.into_iter().max_by_key(|(_, count)| *count)?;
    let enough = if strict { max * 2 > colors.len() } else { max * 2 >= colors.len() };
    enough.then_some(best)
}

/// 终端内边距取什么底色：看贴着内边距的边缘格子。
/// 先看左右两列：光标行、标签栏、状态栏这类横贯整行的条带在两列里各只占一格，两列最能代表 TUI 的根背景。
/// 只看四条边时，宽屏上首末两行（各 cols 格）压过两侧：vim 光标停在首行时首行是 cursorline 色、末行是状态栏，
/// 根背景只剩约 24%，内边距回落默认色，四周露出一圈黑框。
/// 两列凑不出严格过半（左右分属不同面板）再按四条边取众数（≥50%）。
/// 四条边也定不下来（左边文件树、右边编辑区、首行标签栏、末行状态栏各一种色）时，取屏幕中央的主色：
/// 中央是正文区，内边距跟正文同色；回落默认色反而四周一圈黑框。
pub fn pick_edge_bg(cols: usize, rows: usize, color_at: impl Fn(usize, usize) -> Option<Rgb>) -> Option<Rgb> {
    if cols == 0 || rows == 0 {
        return None;
    }
    let mut sides = Vec::with_capacity(rows * 2);
    for y in 0..rows {
        sides.push(color_at(0, y));
        sides.push(color_at(cols - 1, y));
    }
    if let Some(side) = majority(&sides, true) {
        return Some(side);
    }
    let mut edges = sides;
    for x in 0..cols {
        edges.push(color_at(x, 0));
        edges.push(color_at(x, rows - 1));
    }
    if let Some(edge) = majority(&edges, false) {
        return Some(edge);
    }
    // 中央一半区域隔格取样，够定主色，也免得每帧读上千个格子
    let mut center = Vec::new();
    let mut y = rows / 4;
    while y < (rows * 3).div_ceil(4) {
        let mut x = cols / 4;
        while x < (cols * 3).div_ceil(4) {
            center.push(color_at(x, y));
            x += 2;
        }
        y += 2;
    }
    majority(&center, false)
}

/// 只延伸「整行都有底色」的行：shell 提示符、ls 着色只占半行，抹进内边距反而难看。
/// `color_at` 对默认底色返回 None。
pub fn edge_extension(cols: usize, rows: usize, color_at: impl Fn(usize, usize) -> Option<Rgb>) -> EdgeExtension {
    let mut ext = EdgeExtension { left: Vec::with_capacity(rows), right: Vec::with_capacity(rows), ..Default::default() };
    for y in 0..rows {
        let edge_row = y == 0 || y + 1 == rows;
        let mut colors: Vec<Rgb> = Vec::new();
        let mut first = None;
        let mut last = None;
        let mut full = cols > 0;
        for x in 0..cols {
            let Some(color) = color_at(x, y) else {
                full = false;
                break;
            };
            if edge_row {
                colors.push(color);
            }
            if x == 0 {
                first = Some(color);
            }
            if x + 1 == cols {
                last = Some(color);
            }
        }
        ext.left.push(if full { first } else { None });
        ext.right.push(if full { last } else { None });
        if full && y == 0 {
            ext.top = Some(colors.clone());
        }
        if full && y + 1 == rows {
            ext.bottom = Some(colors);
        }
    }
    ext
}

const PASSWORD_WORDS: &[&str] = &["password", "mot de passe", "passwort", "contraseña", "пароль"];

/// 光标所在行像不像密码提示：含「password」等词，并以冒号结尾（sudo、su、ssh 的 keyboard-interactive）。
pub fn is_password_prompt(line: &str) -> bool {
    let trimmed = line.trim_end();
    if !(trimmed.ends_with(':') || trimmed.ends_with('：')) {
        return false;
    }
    let lower = trimmed.to_lowercase();
    PASSWORD_WORDS.iter().any(|word| lower.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(v: u32) -> Option<Rgb> {
        Some(Rgb::hex(v))
    }

    #[test]
    fn nvim_status_bar_and_painted_rows_reach_the_card_edge() {
        // 6×4：首行编辑区，第 2 行 cursorline，末行状态栏（NORMAL 蓝 | 中间深 | 时钟蓝）
        let rows = [
            [c(0x222436); 6],
            [c(0x2f334d); 6],
            [c(0x222436); 6],
            [c(0x82aaff), c(0x82aaff), c(0x1e2030), c(0x1e2030), c(0x82aaff), c(0x82aaff)],
        ];
        let ext = edge_extension(6, 4, |x, y| rows[y][x]);
        assert_eq!(ext.top, Some(rows[0].iter().map(|v| v.unwrap()).collect()));
        assert_eq!(ext.bottom, Some(rows[3].iter().map(|v| v.unwrap()).collect()));
        assert_eq!(ext.left, vec![c(0x222436), c(0x2f334d), c(0x222436), c(0x82aaff)]);
        assert_eq!(ext.right, vec![c(0x222436), c(0x2f334d), c(0x222436), c(0x82aaff)]);
    }

    #[test]
    fn partly_colored_shell_rows_are_not_extended() {
        // 提示符 / ls 着色只占半行：不往内边距里抹
        let rows = [
            [None, None, None, None],
            [c(0x00ff74), c(0x00ff74), None, None],
            [None, None, c(0xee5396), c(0xee5396)],
            [c(0x78a9ff), c(0x78a9ff), None, None],
        ];
        let ext = edge_extension(4, 4, |x, y| rows[y][x]);
        assert_eq!(ext.top, None);
        assert_eq!(ext.bottom, None);
        assert_eq!(ext.left, vec![None; 4]);
        assert_eq!(ext.right, vec![None; 4]);
    }

    #[test]
    fn vim_with_cursor_on_first_line_keeps_its_own_background() {
        // 150×48：首行是 cursorline，末行是状态栏，其余是 vim 的根背景
        let color = pick_edge_bg(150, 48, |x, y| {
            if y == 0 {
                c(0x2f334d)
            } else if y == 47 {
                if x < 10 { c(0x82aaff) } else { c(0x1e2030) }
            } else {
                c(0x222436)
            }
        });
        assert_eq!(color, c(0x222436));
    }

    #[test]
    fn plain_shell_stays_on_default_background() {
        assert_eq!(pick_edge_bg(120, 40, |_, _| c(0x1d1d1d)), c(0x1d1d1d));
    }

    #[test]
    fn centered_dialog_does_not_move_the_padding() {
        let color = pick_edge_bg(100, 30, |x, y| {
            if x > 20 && x < 80 && y > 8 && y < 22 { c(0xc0c0c0) } else { c(0x5f5fd7) }
        });
        assert_eq!(color, c(0x5f5fd7));
    }

    #[test]
    fn four_edges_decide_when_side_columns_disagree() {
        // 左侧文件树一种底色、右侧编辑区另一种：两侧打平，四边里编辑区占多数
        assert_eq!(pick_edge_bg(100, 30, |x, _| if x < 30 { c(0x111111) } else { c(0x222222) }), c(0x222222));
    }

    #[test]
    fn editor_in_the_middle_decides_when_edges_disagree() {
        // 文件树 + 标签栏 + 状态栏：两列打平，四边谁也不过半，中央的编辑区定
        let color = pick_edge_bg(120, 40, |x, y| {
            if y == 0 {
                c(0x3b4261)
            } else if y == 39 {
                c(0x82aaff)
            } else if x < 30 {
                c(0x1a1b26)
            } else {
                c(0x222436)
            }
        });
        assert_eq!(color, c(0x222436));
    }

    #[test]
    fn no_majority_anywhere_gives_no_opinion() {
        let palette = [c(0x111111), c(0x222222), c(0x333333)];
        assert_eq!(pick_edge_bg(40, 40, |x, y| palette[(x + y) % 3]), None);
        assert_eq!(pick_edge_bg(0, 0, |_, _| c(0)), None);
    }

    #[test]
    fn password_prompts_are_recognized() {
        assert!(is_password_prompt("[sudo] password for alice: "));
        assert!(is_password_prompt("Password:"));
        assert!(is_password_prompt("alice@host's password: "));
        assert!(is_password_prompt("Mot de passe :"));
        assert!(is_password_prompt("密码 Password："));
        assert!(!is_password_prompt("alice@host:~$ "));
        assert!(!is_password_prompt("echo password"));
        assert!(!is_password_prompt("password changed successfully."));
    }

    fn spans(start: Option<(u16, u32)>, end: Option<(u16, u32)>, selected: bool) -> Vec<(u16, u16, u16, bool)> {
        let mut out = Vec::new();
        match_spans(start, end, 10, 4, selected, &mut out);
        out.into_iter().map(|s| (s.y, s.start, s.end, s.selected)).collect()
    }

    #[test]
    fn a_match_becomes_one_highlight_span_per_row() {
        // 一行之内：首尾格子都算
        assert_eq!(spans(Some((2, 1)), Some((4, 1)), false), [(1, 2, 5, false)]);
        // 折行的匹配：第一行到行尾，下一行从行首
        assert_eq!(spans(Some((8, 0)), Some((1, 1)), true), [(0, 8, 10, true), (1, 0, 2, true)]);
        // 端点反着给也一样
        assert_eq!(spans(Some((1, 1)), Some((8, 0)), false), [(0, 8, 10, false), (1, 0, 2, false)]);
    }

    #[test]
    fn matches_partly_outside_the_viewport_are_clipped() {
        // 头在视口上面：从第一行行首画到尾
        assert_eq!(spans(None, Some((2, 0)), false), [(0, 0, 3, false)]);
        // 尾在视口下面：画到最后一行行尾
        assert_eq!(spans(Some((7, 3)), None, false), [(3, 7, 10, false)]);
        // 两头都不在视口里、或整个在视口下面：不画
        assert_eq!(spans(None, None, false), []);
        assert_eq!(spans(Some((0, 9)), Some((3, 9)), false), []);
    }

    #[test]
    fn the_newest_item_is_found_in_a_rotated_newest_first_list() {
        let find = |list: &[u32]| newest_index(list.len(), |i| Some(list[i]));
        assert_eq!(find(&[]), 0);
        assert_eq!(find(&[7]), 0);
        assert_eq!(find(&[9, 7, 5, 3]), 0, "没转");
        assert_eq!(find(&[5, 3, 9, 7]), 2);
        assert_eq!(find(&[3, 9, 7, 5]), 1);
        assert_eq!(find(&[7, 5, 3, 9]), 3);
        // libghostty 实际给出的顺序：回滚历史（211 … 11）在前，屏幕上的（301、300）在后
        assert_eq!(find(&[211, 128, 127, 126, 125, 124, 123, 122, 121, 120, 119, 111, 11, 301, 300]), 13);
    }

    fn text_rows(lines: &[(&str, bool)]) -> Vec<Arc<Row>> {
        lines
            .iter()
            .map(|(text, wrapped)| Arc::new(Row { cells: text.chars().map(|ch| Cell { ch, ..Cell::BLANK }).collect(), wrapped: *wrapped }))
            .collect()
    }

    /// 网址和它的格子（行, 起, 止）。
    type Found = (String, Vec<(u16, u16, u16)>);

    fn link(lines: &[(&str, bool)], x: usize, y: usize) -> Option<Found> {
        link_at(&text_rows(lines), x, y).map(|link| (link.url, link.spans.iter().map(|s| (s.y, s.start, s.end)).collect()))
    }

    #[test]
    fn a_url_under_the_pointer_is_found_with_its_cells() {
        let lines = [("see https://example.com/a?b=1 for more", false)];
        let url = Some(("https://example.com/a?b=1".to_string(), vec![(0, 4, 29)]));
        assert_eq!(link(&lines, 4, 0), url, "第一个字符");
        assert_eq!(link(&lines, 28, 0), url, "最后一个字符");
        assert_eq!(link(&lines, 3, 0), None, "前面的空格");
        assert_eq!(link(&lines, 29, 0), None, "后面的空格");
        assert_eq!(link(&lines, 0, 5), None, "行号越界");
    }

    #[test]
    fn trailing_punctuation_and_brackets_are_not_part_of_the_url() {
        assert_eq!(link(&[("(see http://a.io/x).", false)], 6, 0).unwrap().0, "http://a.io/x");
        assert_eq!(link(&[("wiki https://en.wikipedia.org/wiki/Rust_(language), ok", false)], 10, 0).unwrap().0, "https://en.wikipedia.org/wiki/Rust_(language)");
        assert_eq!(link(&[("\"https://a.io/x\"", false)], 3, 0).unwrap().0, "https://a.io/x");
        // 中文紧跟在后面：不算进网址
        assert_eq!(link(&[("打开https://a.io/x。然后", false)], 4, 0).unwrap().0, "https://a.io/x");
        // 不是网址
        assert_eq!(link(&[("xhttps://a.io nothttp:// https://", false)], 3, 0), None);
        assert_eq!(link(&[("xhttps://a.io nothttp:// https://", false)], 27, 0), None);
    }

    #[test]
    fn a_soft_wrapped_url_is_one_link_across_rows() {
        // 登录类命令打印的长链接折成两行（第一行软换行）
        let lines = [("$ login", false), ("open https://a.io/auth?c", true), ("ode=42 now", false), ("https://b.io", false)];
        let url = Some(("https://a.io/auth?code=42".to_string(), vec![(1, 5, 24), (2, 0, 6)]));
        assert_eq!(link(&lines, 10, 1), url);
        assert_eq!(link(&lines, 2, 2), url, "点第二行那一截也是同一个");
        assert_eq!(link(&lines, 8, 2), None);
        // 硬换行的下一行是另一个网址，不拼在一起
        assert_eq!(link(&lines, 3, 3).unwrap().0, "https://b.io");
    }

    #[test]
    fn only_web_and_mail_links_are_opened() {
        assert!(openable("https://example.com"));
        assert!(openable("HTTP://example.com"));
        assert!(openable("mailto:me@example.com"));
        assert!(openable("ftp://example.com/file"));
        assert!(!openable("file:///etc/passwd"));
        assert!(!openable("javascript:alert(1)"));
        assert!(!openable("x-apple.systempreferences:com.apple.preference"));
        assert!(!openable(""));
    }

    #[test]
    fn row_text_skips_wide_char_spacers() {
        let mut wide = Cell { ch: '中', ..Cell::BLANK };
        wide.flags = flags::WIDE;
        let spacer = Cell { flags: flags::SPACER, ..Cell::BLANK };
        let row = Row { cells: vec![wide, spacer, Cell { ch: 'a', ..Cell::BLANK }], wrapped: false };
        assert_eq!(row.text(), "中a");
    }
}
