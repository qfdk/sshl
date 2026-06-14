// 枚举系统字体家族名。
// 主路径：Core Text 拿到 macOS 注册的 CSS-friendly family name。
// 兜底路径：扫 ~/Library/Fonts 等目录的文件名做家族名启发式推断。

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::error::AppResult;

#[cfg(target_os = "macos")]
fn enumerate_via_core_text() -> Vec<String> {
    use core_text::font_collection;
    let collection = font_collection::create_for_all_families();
    let Some(descriptors) = collection.get_descriptors() else { return Vec::new(); };
    let mut out = Vec::with_capacity(descriptors.len() as usize);
    for i in 0..descriptors.len() {
        if let Some(d) = descriptors.get(i) {
            let name = d.family_name();
            if !name.is_empty() {
                out.push(name);
            }
        }
    }
    out
}

#[cfg(not(target_os = "macos"))]
fn enumerate_via_core_text() -> Vec<String> { Vec::new() }

fn font_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(home) = dirs::home_dir() {
        v.push(home.join("Library/Fonts"));
    }
    v.push(PathBuf::from("/Library/Fonts"));
    v.push(PathBuf::from("/System/Library/Fonts"));
    v
}

fn family_from_filename(stem: &str) -> Vec<String> {
    // 文件名常见: "JetBrainsMonoNerdFont-Regular.ttf"
    let base = stem.split('-').next().unwrap_or(stem).trim();
    if base.is_empty() { return Vec::new(); }

    // 去除已知 weight/style 词尾
    const STYLES: &[&str] = &[
        "Regular", "Bold", "Italic", "Oblique", "Light", "Medium",
        "Heavy", "Black", "Thin", "ExtraLight", "ExtraBold", "Semibold",
        "SemiBold", "DemiBold", "Book", "Roman",
    ];
    let mut parts: Vec<&str> = base.split_whitespace().collect();
    while let Some(last) = parts.last() {
        if STYLES.iter().any(|s| s.eq_ignore_ascii_case(last)) {
            parts.pop();
        } else {
            break;
        }
    }
    let cleaned = parts.join(" ").trim().to_string();
    if cleaned.is_empty() { return Vec::new(); }

    // 候选：原始字符串 + Nerd Font 规范命名（"JetBrainsMonoNerdFont" → "JetBrainsMono Nerd Font"）。
    // 不做 camel_to_spaced 全拆分——它会把 "JetBrainsMono" 拆成 "Jet Brains Mono"，产生系统不存在的
    // 无效 family 名，污染字体选择器（前端虽有 canvas 验证兜底，但不该在源头制造垃圾）。
    let mut out = vec![cleaned.clone()];
    if let Some(coalesced) = coalesce_nerd_naming(&cleaned) {
        if !out.contains(&coalesced) { out.push(coalesced); }
    }
    out
}

/// 把 "JetBrainsMonoNerdFont" 这种形式转成 macOS 注册的 family 名 "JetBrainsMono Nerd Font"：
/// 在每个独立大写单词（Nerd, Font, Mono, Propo）前插空格。
fn coalesce_nerd_naming(s: &str) -> Option<String> {
    const MARKERS: &[&str] = &["Nerd", "Font", "Mono", "Propo", "Sans", "Serif"];
    let mut result = s.to_string();
    let mut changed = false;
    for &m in MARKERS {
        let needle = m;
        // 在 needle 前插空格（如果前面不是空格）
        let mut i = 0;
        while let Some(pos) = result[i..].find(needle).map(|p| p + i) {
            if pos > 0 && !result[..pos].ends_with(' ') {
                let before = &result[..pos];
                let after = &result[pos..];
                result = format!("{} {}", before, after);
                changed = true;
                i = pos + needle.len() + 1;
            } else {
                i = pos + needle.len();
            }
            if i >= result.len() { break; }
        }
    }
    if changed { Some(result) } else { None }
}

fn enumerate_via_filesystem() -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for dir in font_dirs() {
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue; };
            for entry in entries.flatten() {
                let path = entry.path();
                if let Ok(ft) = entry.file_type() {
                    if ft.is_dir() {
                        stack.push(path);
                        continue;
                    }
                }
                let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue; };
                let ext = ext.to_ascii_lowercase();
                if ext != "ttf" && ext != "otf" && ext != "ttc" { continue; }
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue; };
                for f in family_from_filename(stem) {
                    out.insert(f);
                }
            }
        }
    }
    out.into_iter().collect()
}

#[tauri::command]
pub async fn list_system_fonts() -> AppResult<Vec<String>> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for n in enumerate_via_core_text() { set.insert(n); }
    for n in enumerate_via_filesystem() { set.insert(n); }
    Ok(set.into_iter().collect())
}
