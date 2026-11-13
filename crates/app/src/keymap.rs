//! 快捷键。设置里能改的登记在 `COMMANDS`（默认值就是原来写死的那些）；面板里的上下 / 回车 / Esc、输入框里的这类
//! 照旧写死在各模块的 `key_bindings`。整张键位表由 `apply` 一次建好：gpui-base 的 → 能改的 → 写死的
//! （同一层上后绑的优先，写死的面板按键压过 gpui-base 输入框自己的）；设置里改了、或者系统换了键盘布局，清空重建。
//!
//! 键盘布局：默认快捷键按当前布局换算（gpui 的 key equivalents，同 macOS 原生应用：法语 AZERTY 上 ⌘1 是 ⌘&，
//! 不用再按 ⇧）。用户录的快捷键存的就是按下时 gpui 报出来的那个键，原样绑定，不再换算。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use gpui::{App, Global, KeyBinding, Keystroke, OperatingSystem, SharedString};

use crate::files::{SelectAllFiles, ToggleBookmark, ToggleHiddenFiles};
use crate::settings;
use crate::terminal::view::{self as term, Copy, Find, FindNext, FindPrev, Paste, SelectAll, ZoomIn, ZoomOut, ZoomReset};
use crate::workspace::{
    CloseTab, Minimize, NewConnection, NewLocalTerminal, NextTab, OpenQuickConnect, OpenSettings, PrevTab, SelectTab, ShowFavorites, ShowFiles,
    ShowSnippets, ShowSystem, ShowTerminal, ToggleSidebar,
};
use crate::{Hide, HideOthers, Quit};

/// 设置里快捷键列表的分组，按这个顺序排。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Tabs,
    Views,
    Terminal,
    Files,
    App,
}

impl Group {
    pub const ALL: [Group; 5] = [Group::Tabs, Group::Views, Group::Terminal, Group::Files, Group::App];

    pub fn label(self) -> &'static str {
        match self {
            Group::Tabs => tr!("连接与标签"),
            Group::Views => tr!("视图与面板"),
            Group::Terminal => tr!("终端"),
            Group::Files => tr!("文件管理"),
            Group::App => trc!("快捷键分组", "应用"),
        }
    }
}

pub struct Command {
    /// 存在设置里的名字，不能改。
    pub id: &'static str,
    pub group: Group,
    label: fn() -> String,
    /// 默认快捷键（gpui 的写法，美式键盘）。
    defaults: &'static [&'static str],
    /// 绑在哪些上下文上（None = 哪儿都管用）。
    contexts: &'static [Option<&'static str>],
    bind: fn(&'static str, Option<&'static str>) -> KeyBinding,
}

impl Command {
    pub fn label(&self) -> String {
        (self.label)()
    }
}

const WORKSPACE: Option<&str> = Some("Workspace");
const TERMINAL: Option<&str> = Some(term::CONTEXT);
const SEARCH: Option<&str> = Some(term::SEARCH_CONTEXT);
const FILES: Option<&str> = Some("FileManager");

macro_rules! command {
    ($id:literal, $group:ident, $label:expr, [$($key:literal),+], [$($context:expr),+], $action:expr) => {
        Command {
            id: $id,
            group: Group::$group,
            label: $label,
            defaults: &[$($key),+],
            contexts: &[$($context),+],
            bind: |keys, context| KeyBinding::new(keys, $action, context),
        }
    };
}

/// 设置里能改的快捷键，按设置里列出来的顺序。
pub static COMMANDS: &[Command] = &[
    command!("new-connection", Tabs, || tr!("新建连接").into(), ["cmd-t"], [WORKSPACE], NewConnection),
    // 本机终端是 ⌘N（用户开终端的第一反应是它），新建连接让到 ⌘T
    command!("new-local-terminal", Tabs, || tr!("本机终端").into(), ["cmd-n"], [WORKSPACE], NewLocalTerminal),
    command!("quick-connect", Tabs, || tr!("快速连接").into(), ["cmd-k"], [WORKSPACE], OpenQuickConnect),
    command!("close-tab", Tabs, || tr!("关闭标签").into(), ["cmd-w"], [None], CloseTab),
    // 标签切换同 Ghostty / Terminal / 浏览器
    command!("next-tab", Tabs, || tr!("下一个标签").into(), ["cmd-}", "ctrl-tab"], [WORKSPACE], NextTab),
    command!("prev-tab", Tabs, || tr!("上一个标签").into(), ["cmd-{", "ctrl-shift-tab"], [WORKSPACE], PrevTab),
    command!("select-tab-1", Tabs, || tr!("切到第 {} 个标签", 1), ["cmd-1"], [WORKSPACE], SelectTab(1)),
    command!("select-tab-2", Tabs, || tr!("切到第 {} 个标签", 2), ["cmd-2"], [WORKSPACE], SelectTab(2)),
    command!("select-tab-3", Tabs, || tr!("切到第 {} 个标签", 3), ["cmd-3"], [WORKSPACE], SelectTab(3)),
    command!("select-tab-4", Tabs, || tr!("切到第 {} 个标签", 4), ["cmd-4"], [WORKSPACE], SelectTab(4)),
    command!("select-tab-5", Tabs, || tr!("切到第 {} 个标签", 5), ["cmd-5"], [WORKSPACE], SelectTab(5)),
    command!("select-tab-6", Tabs, || tr!("切到第 {} 个标签", 6), ["cmd-6"], [WORKSPACE], SelectTab(6)),
    command!("select-tab-7", Tabs, || tr!("切到第 {} 个标签", 7), ["cmd-7"], [WORKSPACE], SelectTab(7)),
    command!("select-tab-8", Tabs, || tr!("切到第 {} 个标签", 8), ["cmd-8"], [WORKSPACE], SelectTab(8)),
    command!("select-tab-9", Tabs, || tr!("切到第 {} 个标签", 9), ["cmd-9"], [WORKSPACE], SelectTab(9)),
    // 视图：顺序同顶栏（系统不常用，放最后）
    command!("show-terminal", Views, || tr!("终端").into(), ["cmd-i"], [WORKSPACE], ShowTerminal),
    command!("show-files", Views, || tr!("文件管理").into(), ["cmd-o"], [WORKSPACE], ShowFiles),
    command!("show-system", Views, || tr!("系统").into(), ["cmd-p"], [WORKSPACE], ShowSystem),
    command!("show-snippets", Views, || tr!("命令片段").into(), ["cmd-shift-p"], [WORKSPACE], ShowSnippets),
    command!("show-favorites", Views, || tr!("目录收藏").into(), ["cmd-shift-d"], [WORKSPACE], ShowFavorites),
    command!("toggle-sidebar", Views, || tr!("收起 / 展开侧边栏").into(), ["cmd-b"], [WORKSPACE], ToggleSidebar),
    command!("open-settings", Views, || tr!("设置").into(), ["cmd-,"], [WORKSPACE], OpenSettings),
    command!("terminal-copy", Terminal, || tr!("拷贝").into(), ["cmd-c"], [TERMINAL], Copy),
    command!("terminal-paste", Terminal, || tr!("粘贴").into(), ["cmd-v"], [TERMINAL], Paste),
    command!("terminal-select-all", Terminal, || tr!("全选").into(), ["cmd-a"], [TERMINAL], SelectAll),
    // 查找的三个在搜索条里也要管用（它和终端是并排的兄弟，不在终端的上下文里）
    command!("terminal-find", Terminal, || tr!("查找").into(), ["cmd-f"], [TERMINAL, SEARCH], Find),
    command!("terminal-find-next", Terminal, || tr!("查找下一个").into(), ["cmd-g"], [TERMINAL, SEARCH], FindNext),
    command!("terminal-find-prev", Terminal, || tr!("查找上一个").into(), ["cmd-shift-g"], [TERMINAL, SEARCH], FindPrev),
    // 美式键盘上 ⌘⇧= 报成 cmd-+，不按 Shift 的 ⌘= 也算放大（同浏览器）
    command!("zoom-in", Terminal, || tr!("放大字号").into(), ["cmd-=", "cmd-+"], [TERMINAL], ZoomIn),
    command!("zoom-out", Terminal, || tr!("缩小字号").into(), ["cmd--"], [TERMINAL], ZoomOut),
    command!("zoom-reset", Terminal, || tr!("默认字号").into(), ["cmd-0"], [TERMINAL], ZoomReset),
    command!("files-select-all", Files, || tr!("全选").into(), ["cmd-a"], [FILES], SelectAllFiles),
    // ⌘⇧.：macOS 把 Shift 并进字符送来「⌘>」，写 cmd-shift-. 匹配不上
    command!("files-toggle-hidden", Files, || tr!("显示 / 隐藏隐藏文件").into(), ["cmd->"], [FILES], ToggleHiddenFiles),
    command!("files-toggle-bookmark", Files, || tr!("收藏 / 取消收藏当前目录").into(), ["cmd-d"], [FILES], ToggleBookmark),
    command!("quit", App, || tr!("退出 SSHL").into(), ["cmd-q"], [None], Quit),
    command!("hide", App, || tr!("隐藏 SSHL").into(), ["cmd-h"], [None], Hide),
    command!("hide-others", App, || tr!("隐藏其他").into(), ["alt-cmd-h"], [None], HideOthers),
    command!("minimize", App, || tr!("最小化").into(), ["cmd-m"], [None], Minimize),
];

pub fn find(id: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.id == id)
}

/// 设置里改过的快捷键：命令 id → 快捷键（空 = 不要快捷键）。没改过的不在里面。
pub type Overrides = BTreeMap<String, Vec<String>>;

/// 每条命令现在的快捷键（gpui 的写法）。
pub type Active = HashMap<&'static str, Vec<String>>;

/// 规整成 gpui 的标准写法（修饰键顺序固定、小写）；不认识的返回 None。
pub fn normalize(keys: &str) -> Option<String> {
    let keys = keys.trim();
    if keys.is_empty() || keys.contains(char::is_whitespace) {
        return None;
    }
    let stroke = Keystroke::parse_with_os(keys, OperatingSystem::Mac).ok()?;
    // 「cmd-shift」这种只有修饰键的 gpui 也认（当成按住修饰键再松开），快捷键不收
    (!stroke.is_modifier_key()).then(|| stroke.unparse(OperatingSystem::Mac))
}

/// 默认快捷键怎么按键盘布局换算：运行时是 gpui 按当前布局给的（`layout`），美式键盘原样不动（`us`）。
pub type Layout<'a> = &'a dyn Fn(Keystroke) -> Keystroke;

#[cfg(test)]
pub fn us(stroke: Keystroke) -> Keystroke {
    stroke
}

/// 当前键盘布局的换算（同 macOS 原生应用的做法）。
fn with_layout<R>(cx: &App, f: impl FnOnce(Layout) -> R) -> R {
    let mapper = cx.keyboard_mapper().clone();
    f(&move |stroke| mapper.map_key_equivalent(stroke, true).inner().clone())
}

/// 默认快捷键按键盘布局换算过的样子。
fn localized_defaults(command: &Command, layout: Layout) -> Vec<String> {
    let mut keys = Vec::new();
    for default in command.defaults {
        let Ok(stroke) = Keystroke::parse_with_os(default, OperatingSystem::Mac) else { continue };
        let mapped = layout(stroke).unparse(OperatingSystem::Mac);
        if !keys.contains(&mapped) {
            keys.push(mapped);
        }
    }
    keys
}

/// 一条命令现在的快捷键：设置里改过的用改的（认不出来的那几个丢掉，一个都认不出来就当没改），没改的用默认值。
pub fn keys_for(command: &Command, overrides: &Overrides, layout: Layout) -> Vec<String> {
    if let Some(set) = overrides.get(command.id) {
        if set.is_empty() {
            return Vec::new();
        }
        let keys: Vec<String> = set.iter().filter_map(|keys| normalize(keys)).collect();
        if !keys.is_empty() {
            return keys;
        }
    }
    localized_defaults(command, layout)
}

pub fn resolve(overrides: &Overrides, layout: Layout) -> Active {
    COMMANDS.iter().map(|command| (command.id, keys_for(command, overrides, layout))).collect()
}

/// 把 `keys` 定成 `id` 的快捷键（空 = 不要快捷键）。和默认一样就把这条从设置里删掉，设置文件里只留改过的。
pub fn assign(overrides: &mut Overrides, id: &str, keys: Vec<String>, layout: Layout) {
    let Some(command) = find(id) else { return };
    if keys == localized_defaults(command, layout) {
        overrides.remove(id);
    } else {
        overrides.insert(id.to_string(), keys);
    }
}

/// 快捷键在哪儿管用：终端和文件管理是并排的两页，同一个键可以各用各的；别的都会撞上。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Everywhere,
    Terminal,
    Files,
}

fn scope(group: Group) -> Scope {
    match group {
        Group::Terminal => Scope::Terminal,
        Group::Files => Scope::Files,
        Group::Tabs | Group::Views | Group::App => Scope::Everywhere,
    }
}

/// 把 `keys` 给 `id` 的话会和哪条命令撞上：同一个键，管用的地方有重叠。
pub fn conflict(id: &str, keys: &str, active: &Active) -> Option<&'static Command> {
    let command = find(id)?;
    let mine = scope(command.group);
    COMMANDS.iter().filter(|other| other.id != id).find(|other| {
        let theirs = scope(other.group);
        let overlap = mine == Scope::Everywhere || theirs == Scope::Everywhere || mine == theirs;
        overlap && active.get(other.id).is_some_and(|set| set.iter().any(|k| k == keys))
    })
}

/// 录快捷键时按下的一个键怎么算。
#[derive(Debug, PartialEq, Eq)]
pub enum Recorded {
    /// 单按一个修饰键（松开时 gpui 也会报一次）：接着等。
    Ignore,
    /// Esc：不录了。
    Cancel,
    /// ⌫ / ⌦：去掉这条快捷键。
    Clear,
    /// 不带 ⌘ / ⌃ / ⌥ 的普通键（字母、⇧ + 字母、回车）：会把打字吃掉，不收。
    NeedsModifier,
    Set(String),
}

pub fn record(stroke: &Keystroke) -> Recorded {
    if stroke.is_modifier_key() {
        return Recorded::Ignore;
    }
    let m = stroke.modifiers;
    let function_key = is_function_key(&stroke.key);
    if !(m.platform || m.control || m.alt || m.shift) {
        match stroke.key.as_str() {
            "escape" => return Recorded::Cancel,
            "backspace" | "delete" => return Recorded::Clear,
            _ => {}
        }
    }
    if !(m.platform || m.control || m.alt || function_key) {
        return Recorded::NeedsModifier;
    }
    let stroke = Keystroke { key_char: None, ..stroke.clone() };
    normalize(&stroke.unparse(OperatingSystem::Mac)).map_or(Recorded::NeedsModifier, Recorded::Set)
}

fn is_function_key(key: &str) -> bool {
    key.strip_prefix('f').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// 显示成 macOS 菜单里的样子：⌃⌥⇧⌘ + 键。
pub fn display(keys: &str) -> String {
    let Ok(stroke) = Keystroke::parse_with_os(keys, OperatingSystem::Mac) else { return keys.to_string() };
    let m = stroke.modifiers;
    let mut out = String::new();
    for (on, symbol) in [(m.control, '⌃'), (m.alt, '⌥'), (m.shift, '⇧'), (m.platform, '⌘')] {
        if on {
            out.push(symbol);
        }
    }
    // F 键自带 fn，不另外写
    if m.function && !is_function_key(&stroke.key) {
        out.insert_str(0, "fn ");
    }
    out.push_str(&key_label(&stroke.key));
    out
}

fn key_label(key: &str) -> String {
    let symbol = match key {
        "space" => return tr!("空格").into(),
        "enter" => "↵",
        "tab" => "⇥",
        "escape" => "⎋",
        "backspace" => "⌫",
        "delete" => "⌦",
        "up" => "↑",
        "down" => "↓",
        "left" => "←",
        "right" => "→",
        "pageup" => "⇞",
        "pagedown" => "⇟",
        "home" => "↖",
        "end" => "↘",
        key if is_function_key(key) => return key.to_uppercase(),
        key => return key.to_uppercase(),
    };
    symbol.into()
}

/// 按住 ⌘ 时浮出的角标：⌘ 不写（正按着），别的修饰键照写；不带 ⌘ 的整个写出来。
pub fn hint_text(keys: &str) -> String {
    display(keys).replacen('⌘', "", 1)
}

/// gpui 的 `KeyBinding::new` 要 'static 的按键串：同一个串只留一份（能录进来的键有限，不会越积越多）。
fn intern(keys: &str) -> &'static str {
    static POOL: LazyLock<Mutex<HashSet<&'static str>>> = LazyLock::new(Default::default);
    let mut pool = POOL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(found) = pool.get(keys) {
        return found;
    }
    let leaked: &'static str = Box::leak(keys.to_string().into_boxed_str());
    pool.insert(leaked);
    leaked
}

/// 能改的这些按现在的设置建出来的绑定。
fn command_bindings(active: &Active) -> Vec<KeyBinding> {
    let mut bindings = Vec::new();
    for command in COMMANDS {
        for keys in active.get(command.id).into_iter().flatten() {
            let keys = intern(keys);
            for context in command.contexts {
                bindings.push((command.bind)(keys, *context));
            }
        }
    }
    bindings
}

/// 写死的（面板里的上下 / 回车 / Esc 这类）：要晚于 gpui-base 绑（同一节点上后绑的优先，压过输入框自己的）。
fn fixed_bindings() -> Vec<KeyBinding> {
    let mut bindings = crate::terminal::view::key_bindings();
    bindings.extend(crate::snippets::key_bindings());
    bindings.extend(crate::favorites::key_bindings());
    bindings.extend(crate::quick_connect::key_bindings());
    bindings.extend(crate::topbar::key_bindings());
    bindings.extend(crate::connection_dialog::key_bindings());
    bindings.extend(crate::files::key_bindings());
    bindings
}

/// 应用自己的全部绑定（不含 gpui-base 的）。
pub fn bindings(active: &Active) -> Vec<KeyBinding> {
    let mut bindings = command_bindings(active);
    bindings.extend(fixed_bindings());
    bindings
}

/// 测试用：美式键盘、什么都没改时的全部绑定。
#[cfg(test)]
pub fn default_bindings() -> Vec<KeyBinding> {
    bindings(&resolve(&Overrides::new(), &us))
}

/// gpui-base 在 `init` 里绑的那些：重建键位表时先放回去。
struct BaseBindings(Vec<KeyBinding>);

impl Global for BaseBindings {}

/// 现在生效的快捷键（显示用：按钮提示、⌘ 角标、设置里的列表）。
struct ActiveKeys(Active);

impl Global for ActiveKeys {}

/// 启动时（gpui-base 绑完之后）调一次。
pub fn init(cx: &mut App) {
    let base = cx.key_bindings().borrow().bindings().cloned().collect();
    cx.set_global(BaseBindings(base));
    apply(cx);
    // 切了输入法 / 键盘布局：默认快捷键按新布局重新换算，菜单栏上的也跟着变
    cx.on_keyboard_layout_change(|cx| {
        apply(cx);
        crate::set_menus(cx);
    })
    .detach();
}

/// 按现在的设置重建整张键位表。菜单栏上的快捷键是建菜单时定的，调用方自己重建菜单。
pub fn apply(cx: &mut App) {
    let active = with_layout(cx, |layout| resolve(&settings::get(cx).key_bindings, layout));
    let base = cx.global::<BaseBindings>().0.clone();
    cx.clear_key_bindings();
    cx.bind_keys(base);
    cx.bind_keys(bindings(&active));
    cx.set_global(ActiveKeys(active));
}

/// 按当前键盘布局把 `keys` 定成 `id` 的快捷键（见 `assign`）。
pub fn assign_in(cx: &App, overrides: &mut Overrides, id: &str, keys: Vec<String>) {
    with_layout(cx, |layout| assign(overrides, id, keys, layout));
}

pub fn active(cx: &App) -> &Active {
    &cx.global::<ActiveKeys>().0
}

/// 这条命令现在的第一个快捷键（显示用）；没有快捷键是 None。
pub fn shortcut(cx: &App, id: &str) -> Option<String> {
    active(cx).get(id)?.first().map(|keys| display(keys))
}

/// ⌘ 角标上的字；没有快捷键是 None。
pub fn hint(cx: &App, id: &str) -> Option<String> {
    active(cx).get(id)?.first().map(|keys| hint_text(keys))
}

/// 按钮的气泡提示：「名称  ⌘T」，没有快捷键就只有名称。
pub fn with_shortcut(cx: &App, label: &str, id: &str) -> SharedString {
    match shortcut(cx, id) {
        Some(keys) => format!("{label}  {keys}").into(),
        None => label.to_string().into(),
    }
}

#[cfg(test)]
mod tests {
    use gpui::Modifiers;

    use super::*;

    /// 法语 AZERTY（gpui-pre-macos 的 key equivalents 表里的几项）：数字在 ⇧ 上，⌘1 在 macOS 原生应用里是按 ⌘&（同一个位置）。
    fn french(mut stroke: Keystroke) -> Keystroke {
        let mapped = match stroke.key.as_str() {
            "1" => Some("&"),
            "2" => Some("é"),
            "}" => Some("*"),
            _ => None,
        };
        if let Some(key) = mapped {
            stroke.key = key.into();
        }
        stroke
    }

    fn stroke(keys: &str) -> Keystroke {
        Keystroke::parse_with_os(keys, OperatingSystem::Mac).unwrap()
    }

    #[test]
    fn ids_are_unique_and_defaults_are_valid() {
        let mut seen = HashSet::new();
        for command in COMMANDS {
            assert!(seen.insert(command.id), "重复的 id：{}", command.id);
            assert!(!command.defaults.is_empty());
            for keys in command.defaults {
                assert_eq!(normalize(keys).as_deref(), Some(*keys), "默认值要写成规整的样子：{keys}");
            }
            assert!(!command.label().is_empty());
        }
    }

    #[test]
    fn defaults_follow_the_keyboard_layout() {
        let tab_one = find("select-tab-1").unwrap();
        assert_eq!(keys_for(tab_one, &Overrides::new(), &us), ["cmd-1"]);
        assert_eq!(keys_for(tab_one, &Overrides::new(), &french), ["cmd-&"], "AZERTY 上 ⌘1 换成同一位置的 ⌘&");
        let next = find("next-tab").unwrap();
        assert_eq!(keys_for(next, &Overrides::new(), &french), ["cmd-*", "ctrl-tab"]);
        // 自己录的不换算：录的时候已经是这个布局报出来的键
        let mut overrides = Overrides::new();
        overrides.insert("select-tab-1".into(), vec!["cmd-1".into()]);
        assert_eq!(keys_for(tab_one, &overrides, &french), ["cmd-1"]);
    }

    #[test]
    fn overrides_replace_clear_or_fall_back() {
        let quick = find("quick-connect").unwrap();
        let mut overrides = Overrides::new();
        overrides.insert("quick-connect".into(), vec!["ctrl-alt-k".into()]);
        assert_eq!(keys_for(quick, &overrides, &us), ["ctrl-alt-k"]);
        overrides.insert("quick-connect".into(), Vec::new());
        assert!(keys_for(quick, &overrides, &us).is_empty(), "空 = 不要快捷键");
        overrides.insert("quick-connect".into(), vec!["cmd-shift".into(), "ctrl- x".into()]);
        assert_eq!(keys_for(quick, &overrides, &us), ["cmd-k"], "手改坏了的：照默认");
    }

    #[test]
    fn assigning_the_default_removes_the_override() {
        let mut overrides = Overrides::new();
        assign(&mut overrides, "new-connection", vec!["cmd-shift-n".into()], &us);
        assert_eq!(overrides.get("new-connection"), Some(&vec!["cmd-shift-n".to_string()]));
        assign(&mut overrides, "new-connection", vec!["cmd-t".into()], &us);
        assert!(overrides.is_empty(), "改回默认：设置里不留这一条");
        assign(&mut overrides, "next-tab", vec!["cmd-}".into()], &us);
        assert_eq!(overrides.get("next-tab"), Some(&vec!["cmd-}".to_string()]), "默认有两个键，只剩一个也算改过");
        assign(&mut overrides, "no-such-command", vec!["cmd-x".into()], &us);
        assert!(!overrides.contains_key("no-such-command"));
    }

    #[test]
    fn conflicts_only_where_both_shortcuts_work() {
        let active = resolve(&Overrides::new(), &us);
        assert_eq!(conflict("new-connection", "cmd-k", &active).map(|c| c.id), Some("quick-connect"));
        assert_eq!(conflict("new-connection", "cmd-c", &active).map(|c| c.id), Some("terminal-copy"), "工作区的键在终端里也管用");
        assert_eq!(conflict("terminal-copy", "cmd-a", &active).map(|c| c.id), Some("terminal-select-all"));
        assert_eq!(conflict("terminal-copy", "cmd-d", &active).map(|c| c.id), None, "终端和文件管理是两页，各用各的");
        assert_eq!(conflict("files-toggle-bookmark", "cmd-a", &active).map(|c| c.id), Some("files-select-all"));
        assert_eq!(conflict("files-toggle-bookmark", "cmd-c", &active).map(|c| c.id), None);
        assert_eq!(conflict("quick-connect", "cmd-k", &active).map(|c| c.id), None, "自己不算");
        assert_eq!(conflict("quick-connect", "ctrl-alt-k", &active).map(|c| c.id), None);
        assert_eq!(conflict("new-connection", "ctrl-tab", &active).map(|c| c.id), Some("next-tab"), "第二个默认键也算");
    }

    #[test]
    fn recording_needs_a_modifier_and_esc_or_delete_mean_cancel_or_clear() {
        assert_eq!(record(&stroke("cmd-shift-p")), Recorded::Set("cmd-shift-p".into()));
        assert_eq!(record(&stroke("ctrl-alt-k")), Recorded::Set("ctrl-alt-k".into()));
        assert_eq!(record(&stroke("f5")), Recorded::Set("f5".into()), "F 键可以单按");
        assert_eq!(record(&stroke("escape")), Recorded::Cancel);
        assert_eq!(record(&stroke("backspace")), Recorded::Clear);
        assert_eq!(record(&stroke("delete")), Recorded::Clear);
        assert_eq!(record(&stroke("cmd-backspace")), Recorded::Set("cmd-backspace".into()), "带修饰键的 ⌫ 照常录");
        assert_eq!(record(&stroke("cmd-escape")), Recorded::Set("cmd-escape".into()));
        assert_eq!(record(&stroke("a")), Recorded::NeedsModifier);
        assert_eq!(record(&stroke("shift-a")), Recorded::NeedsModifier, "⇧ + 字母是打大写字母");
        assert_eq!(record(&stroke("enter")), Recorded::NeedsModifier);
        assert_eq!(record(&stroke("shift-escape")), Recorded::NeedsModifier);
        // 松开单按的 ⌘ 时 gpui 也报一次（键是 platform）
        assert_eq!(record(&Keystroke { modifiers: Modifiers::none(), key: "platform".into(), key_char: None }), Recorded::Ignore);
        // 法语键盘的 ⌘é：录下来的就是 gpui 报的那个字
        let french = Keystroke { modifiers: Modifiers::command(), key: "é".into(), key_char: None };
        assert_eq!(record(&french), Recorded::Set("cmd-é".into()));
        // 带 key_char 的（⌥ 组合会打出字符）不影响录下来的键
        let alt = Keystroke { modifiers: Modifiers::alt(), key: "k".into(), key_char: Some("˚".into()) };
        assert_eq!(record(&alt), Recorded::Set("alt-k".into()));
    }

    #[test]
    fn recorded_keys_survive_a_round_trip() {
        for keys in ["cmd--", "cmd-+", "ctrl-shift-tab", "cmd->", "cmd-é", "cmd-Ö", "alt-space", "fn-f5", "cmd-,"] {
            assert_eq!(normalize(keys).as_deref(), Some(keys), "{keys}");
            let parsed = stroke(keys);
            assert_eq!(parsed.unparse(OperatingSystem::Mac), keys);
        }
        assert_eq!(normalize("shift-cmd-P").as_deref(), Some("cmd-shift-p"), "修饰键的顺序、大小写都规整");
        assert_eq!(normalize(""), None);
        assert_eq!(normalize("cmd-shift"), None, "只有修饰键的不算");
        assert_eq!(normalize("cmd-k cmd-s"), None, "不收连按两下的");
    }

    #[test]
    fn shortcuts_are_shown_like_macos_menus() {
        assert_eq!(display("cmd-shift-p"), "⇧⌘P");
        assert_eq!(display("cmd-1"), "⌘1");
        assert_eq!(display("ctrl-shift-tab"), "⌃⇧⇥");
        assert_eq!(display("alt-cmd-h"), "⌥⌘H");
        assert_eq!(display("cmd-,"), "⌘,");
        assert_eq!(display("cmd--"), "⌘-");
        assert_eq!(display("cmd-é"), "⌘É");
        assert_eq!(display("f5"), "F5");
        assert_eq!(display("fn-f5"), "F5");
        assert_eq!(display("cmd-enter"), "⌘↵");
        assert_eq!(hint_text("cmd-i"), "I");
        assert_eq!(hint_text("cmd-shift-p"), "⇧P");
        assert_eq!(hint_text("ctrl-1"), "⌃1", "不带 ⌘ 的整个写出来");
    }

    #[test]
    fn every_command_binds_in_each_of_its_contexts() {
        let active = resolve(&Overrides::new(), &us);
        let bindings = command_bindings(&active);
        let expected: usize = COMMANDS.iter().map(|c| c.defaults.len() * c.contexts.len()).sum();
        assert_eq!(bindings.len(), expected);
        let mut active = active;
        active.insert("quick-connect", Vec::new());
        assert_eq!(command_bindings(&active).len(), expected - 1, "去掉的不绑");
    }

    #[test]
    fn interned_keys_are_shared() {
        let a = intern("cmd-shift-k");
        let b = intern(&String::from("cmd-shift-k"));
        assert!(std::ptr::eq(a, b));
    }
}
