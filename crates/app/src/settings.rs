//! 界面设置：~/.sshl/settings.json。首次启动时从旧版（WebView）的 localStorage 迁移字号、字体、收起的分组。

use std::path::PathBuf;

use gpui::{App, Global};
use serde::{Deserialize, Serialize};
use sshl_core::i18n::{self, Lang};

use crate::theme::{Appearance, term};

pub const DEFAULT_FONT_FAMILY: &str = "JetBrainsMono Nerd Font";
pub const DEFAULT_FONT_SIZE: f32 = 16.0;

/// 设置里的「语言」。不认识的值（以后加的语言、手改的）按跟随系统。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Zh,
    En,
    /// 跟随系统「语言与地区」里排在前面的那个。
    #[default]
    #[serde(other)]
    System,
}

impl Language {
    pub fn resolve(self) -> Lang {
        match self {
            Language::System => i18n::system(),
            Language::Zh => Lang::Zh,
            Language::En => Lang::En,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub font_size: f32,
    pub font_family: String,
    /// 侧栏里收起的分组（「未分组」也算一个名字）。
    pub collapsed_groups: Vec<String>,
    pub sidebar_collapsed: bool,
    /// 文件管理本地栏占的比例（旧版 sshl.localPaneFlex）。
    pub local_pane_flex: Option<f32>,
    /// 文件管理显示点开头的隐藏文件（⌘⇧.）。
    pub show_hidden_files: bool,
    /// 终端里拖选完自动复制。
    pub copy_on_select: bool,
    /// 终端里右键直接粘贴（程序在接收鼠标时除外）。
    pub right_click_paste: bool,
    /// 终端配色的 id（`theme::term::THEMES`）。不认识的按默认显示，存的值不动。
    pub terminal_theme: String,
    /// 界面外观：跟随系统 / 浅色 / 深色（终端里的颜色归 `terminal_theme` 管）。
    pub appearance: Appearance,
    /// 界面语言：跟随系统 / 简体中文 / English。
    pub language: Language,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            font_family: DEFAULT_FONT_FAMILY.into(),
            collapsed_groups: Vec::new(),
            sidebar_collapsed: false,
            local_pane_flex: None,
            show_hidden_files: false,
            copy_on_select: false,
            right_click_paste: false,
            terminal_theme: term::DEFAULT_ID.into(),
            appearance: Appearance::System,
            language: Language::System,
        }
    }
}

impl Settings {
    pub fn terminal_theme(&self) -> &'static term::Theme {
        term::find(&self.terminal_theme)
    }
}

impl Global for Settings {}

pub fn clamp_font_size(size: f32) -> f32 {
    if size.is_finite() && size > 0.0 { size.clamp(8.0, 40.0) } else { DEFAULT_FONT_SIZE }
}

fn path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".sshl").join("settings.json"))
}

pub fn init(cx: &mut App) {
    let settings = load();
    i18n::set(settings.language.resolve());
    cx.set_global(settings);
}

pub fn get(cx: &App) -> &Settings {
    cx.global::<Settings>()
}

/// 改设置并立即写盘。
pub fn update(cx: &mut App, change: impl FnOnce(&mut Settings)) {
    preview(cx, change);
    save(cx.global::<Settings>());
}

/// 只改内存里的设置并重绘（设置对话框的实时预览），不写盘；`replace` 可整份换回。
pub fn preview(cx: &mut App, change: impl FnOnce(&mut Settings)) {
    let mut settings = cx.global::<Settings>().clone();
    change(&mut settings);
    settings.font_size = clamp_font_size(settings.font_size);
    if settings.font_family.trim().is_empty() {
        settings.font_family = DEFAULT_FONT_FAMILY.into();
    }
    replace(cx, settings);
}

/// 先换语言再换设置：盯着设置的地方（输入框的占位字）被叫到时，`tr!` 已经是新语言了。菜单栏的文字是建的时候定的，语言变了重建。
pub fn replace(cx: &mut App, settings: Settings) {
    if i18n::set(settings.language.resolve()) {
        crate::set_menus(cx);
    }
    cx.set_global(settings);
    cx.refresh_windows();
}

pub fn persist(cx: &App) {
    save(cx.global::<Settings>());
}

fn load() -> Settings {
    let Some(path) = path() else { return Settings::default() };
    if let Ok(text) = std::fs::read_to_string(&path) {
        return serde_json::from_str(&text).unwrap_or_default();
    }
    let migrated = migrate_from_webview().unwrap_or_default();
    save(&migrated);
    migrated
}

fn save(settings: &Settings) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(settings) {
        Ok(text) => {
            if let Err(error) = std::fs::write(&path, text) {
                tracing::warn!("save settings: {error}");
            }
        }
        Err(error) => tracing::warn!("encode settings: {error}"),
    }
}

/// 旧版把设置存在 WKWebView 的 localStorage（SQLite，值是 UTF-16LE）。
fn migrate_from_webview() -> Option<Settings> {
    let root = dirs::home_dir()?.join("Library/WebKit/me.qfdk.sshl/WebsiteData/Default");
    let db = find_file(&root, "localstorage.sqlite3", 4)?;
    let conn = rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let mut stmt = conn.prepare("SELECT key, value FROM ItemTable").ok()?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)))
        .ok()?
        .filter_map(Result::ok)
        .map(|(key, value)| (key, utf16le(&value)));
    let mut settings = Settings::default();
    for (key, value) in rows {
        apply_legacy(&mut settings, &key, &value);
    }
    tracing::info!("migrated settings from the WebView version");
    Some(settings)
}

fn apply_legacy(settings: &mut Settings, key: &str, value: &str) {
    match key {
        "sshl.terminalSettings" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Legacy {
                font_size: Option<f32>,
                font_family: Option<String>,
            }
            if let Ok(legacy) = serde_json::from_str::<Legacy>(value) {
                if let Some(size) = legacy.font_size {
                    settings.font_size = clamp_font_size(size);
                }
                if let Some(family) = legacy.font_family.as_deref().and_then(first_family) {
                    settings.font_family = family;
                }
            }
        }
        "sshl.collapsedConnectionGroups" => {
            if let Ok(groups) = serde_json::from_str::<Vec<String>>(value) {
                settings.collapsed_groups = groups;
            }
        }
        "sshl.localPaneFlex" => settings.local_pane_flex = value.trim().parse().ok(),
        _ => {}
    }
}

/// CSS 字体栈取第一个家族名：`"JetBrainsMono Nerd Font Mono", Menlo, monospace` → `JetBrainsMono Nerd Font Mono`。
fn first_family(stack: &str) -> Option<String> {
    let first = stack.split(',').next()?.trim().trim_matches(|c| c == '"' || c == '\'').trim();
    (!first.is_empty() && first != "monospace").then(|| first.to_string())
}

fn utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
    String::from_utf16_lossy(&units)
}

fn find_file(dir: &std::path::Path, name: &str, depth: usize) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|n| n == name) {
            return Some(path);
        }
        if depth > 0 && path.is_dir()
            && let Some(found) = find_file(&path, name, depth - 1) {
                return Some(found);
            }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_defaults_to_system_and_survives_unknown_values() {
        let old: Settings = serde_json::from_str(r#"{"fontSize":14}"#).unwrap();
        assert_eq!(old.language, Language::System, "旧设置里没有这一项：跟随系统");
        let english: Settings = serde_json::from_str(r#"{"language":"en"}"#).unwrap();
        assert_eq!((english.language, english.language.resolve()), (Language::En, Lang::En));
        assert!(serde_json::to_string(&english).unwrap().contains(r#""language":"en""#));
        let unknown: Settings = serde_json::from_str(r#"{"language":"fr","fontSize":14}"#).unwrap();
        assert_eq!((unknown.language, unknown.font_size), (Language::System, 14.0), "不认识的语言按跟随系统，别的设置不能跟着丢");
        assert_eq!(Language::Zh.resolve(), Lang::Zh);
    }

    #[test]
    fn legacy_values_are_migrated() {
        let mut s = Settings::default();
        apply_legacy(&mut s, "sshl.terminalSettings", r#"{"fontSize":14,"fontFamily":"\"JetBrainsMono Nerd Font Mono\", \"Apple Color Emoji\", Menlo, Monaco, monospace"}"#);
        apply_legacy(&mut s, "sshl.collapsedConnectionGroups", r#"["jimi","seo","未分组","CDG"]"#);
        apply_legacy(&mut s, "sshl.localPaneFlex", "0.969591");
        assert_eq!(s.font_size, 14.0);
        assert_eq!(s.font_family, "JetBrainsMono Nerd Font Mono");
        assert_eq!(s.collapsed_groups, ["jimi", "seo", "未分组", "CDG"]);
        assert_eq!(s.local_pane_flex, Some(0.969591));
    }

    #[test]
    fn plain_monospace_keeps_the_bundled_font() {
        assert_eq!(first_family("monospace"), None);
        assert_eq!(first_family("'Menlo', monospace").as_deref(), Some("Menlo"));
    }

    #[test]
    fn font_size_is_clamped() {
        assert_eq!(clamp_font_size(4.0), 8.0);
        assert_eq!(clamp_font_size(99.0), 40.0);
        assert_eq!(clamp_font_size(f32::NAN), DEFAULT_FONT_SIZE);
    }

    #[test]
    fn old_settings_without_a_terminal_theme_use_the_default() {
        let s: Settings = serde_json::from_str(r#"{"fontSize":14,"fontFamily":"Menlo","copyOnSelect":true}"#).unwrap();
        assert_eq!(s.terminal_theme, "carbonfox");
        assert_eq!(s.terminal_theme().id, "carbonfox");
        assert_eq!((s.font_size, s.copy_on_select), (14.0, true), "其他字段照常读出来");
    }

    #[test]
    fn the_terminal_theme_round_trips() {
        let s = Settings { terminal_theme: "nord".into(), ..Settings::default() };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""terminalTheme":"nord""#), "{json}");
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.terminal_theme().name, "Nord");
    }

    /// 不认识的 id（手改的、以后删掉的主题）：照默认的显示，存的值原样留着，不报错、别的设置也不丢。
    #[test]
    fn an_unknown_terminal_theme_shows_the_default_and_is_kept_as_is() {
        let s: Settings = serde_json::from_str(r#"{"fontSize":15,"terminalTheme":"no-such-theme"}"#).unwrap();
        assert_eq!(s.font_size, 15.0);
        assert_eq!(s.terminal_theme, "no-such-theme");
        assert_eq!(s.terminal_theme().id, "carbonfox");
        assert!(serde_json::to_string(&s).unwrap().contains(r#""terminalTheme":"no-such-theme""#), "写盘时不改写");
    }

    #[test]
    fn the_appearance_defaults_to_the_system_and_round_trips() {
        let old: Settings = serde_json::from_str(r#"{"fontSize":14}"#).unwrap();
        assert_eq!(old.appearance, Appearance::System, "旧设置里没有这一项：跟随系统");
        let s = Settings { appearance: Appearance::Dark, ..Settings::default() };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""appearance":"dark""#), "{json}");
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap().appearance, Appearance::Dark);
    }

    #[test]
    fn webview_values_are_utf16le() {
        let bytes: Vec<u8> = "[\"中\"]".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(utf16le(&bytes), "[\"中\"]");
    }
}
