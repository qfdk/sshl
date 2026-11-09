//! 界面设置：~/.sshl/settings.json。首次启动时从旧版（WebView）的 localStorage 迁移字号、字体、收起的分组。

use std::path::PathBuf;

use gpui::{App, Global};
use serde::{Deserialize, Serialize};

pub const DEFAULT_FONT_FAMILY: &str = "JetBrainsMono Nerd Font Mono";
pub const DEFAULT_FONT_SIZE: f32 = 16.0;

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
        }
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
    cx.set_global(settings);
    cx.refresh_windows();
}

pub fn replace(cx: &mut App, settings: Settings) {
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
    fn webview_values_are_utf16le() {
        let bytes: Vec<u8> = "[\"中\"]".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(utf16le(&bytes), "[\"中\"]");
    }
}
