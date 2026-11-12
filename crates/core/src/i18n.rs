//! 多语言。代码里的界面文字就写中文原文，`tr!` 按当前语言查表换成译文，表里没有就用原文：
//!
//! ```text
//! tr!("保存")                          // &'static str
//! tr!("确定要删除「{}」吗？", name)     // String；译文里可以用 {0} {1} 调换顺序
//! trc!("标签页", "端口")               // 同一个词在不同地方译法不同时带上场合，表里的键是 "端口|标签页"
//! ```
//!
//! 译文表按语言分开：后端的在 `i18n/en.rs`，界面的由应用启动时 `register_en` 登记。
//! 译文模板比 `format!` 多一种写法 `{0?file|files}`：第 0 个参数是 1 用前一个词，否则用后一个（英文单复数）。

use std::collections::HashMap;
use std::fmt::{Display, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

mod en;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

pub type Table = &'static [(&'static str, &'static str)];
type Index = HashMap<&'static str, &'static str>;

static CURRENT: AtomicU8 = AtomicU8::new(Lang::Zh as u8);
static APP_EN: OnceLock<Index> = OnceLock::new();

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == Lang::En as u8 { Lang::En } else { Lang::Zh }
}

/// 换当前语言；返回是不是真的变了。
pub fn set(lang: Lang) -> bool {
    CURRENT.swap(lang as u8, Ordering::Relaxed) != lang as u8
}

/// 登记界面的英文译文表（只认第一次）。
pub fn register_en(table: Table) {
    APP_EN.get_or_init(|| table.iter().copied().collect());
}

fn core_en() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| en::TABLE.iter().copied().collect())
}

/// 后端自己的英文译文表（测试核对用）。
pub fn core_en_table() -> Table {
    en::TABLE
}

/// 原文在这个语言里的译文；中文、或表里没有时是 None。
pub fn lookup(lang: Lang, zh: &str) -> Option<&'static str> {
    match lang {
        Lang::Zh => None,
        Lang::En => core_en().get(zh).or_else(|| APP_EN.get()?.get(zh)).copied(),
    }
}

pub fn translate(lang: Lang, zh: &'static str) -> &'static str {
    lookup(lang, zh).unwrap_or(zh)
}

/// 带场合的译文：先找「原文|场合」，没有再找原文。
pub fn translate_in(lang: Lang, context: &str, zh: &'static str) -> &'static str {
    if lang == Lang::Zh {
        return zh;
    }
    lookup(lang, &format!("{zh}|{context}")).unwrap_or_else(|| translate(lang, zh))
}

/// 运行时才知道的文字（存在状态里的提示）：表里有就换成译文，没有原样返回。
pub fn translate_text(text: &str) -> String {
    lookup(current(), text).unwrap_or(text).to_string()
}

/// 把参数填进模板：`{}` 依次取，`{0}` 按序号取，`{0?one|other}` 按第 0 个参数是不是 1 选词，`{{` `}}` 是花括号本身。
pub fn format(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0;
    let mut rest = template;
    while let Some(at) = rest.find(['{', '}']) {
        out.push_str(&rest[..at]);
        let brace = rest.as_bytes()[at];
        rest = &rest[at + 1..];
        if rest.as_bytes().first() == Some(&brace) {
            out.push(brace as char);
            rest = &rest[1..];
            continue;
        }
        if brace == b'}' {
            out.push('}');
            continue;
        }
        let Some(end) = rest.find('}') else {
            out.push('{');
            break;
        };
        let spec = &rest[..end];
        rest = &rest[end + 1..];
        let (index, choice) = match spec.split_once('?') {
            Some((index, choice)) => (index, Some(choice)),
            None => (spec, None),
        };
        let index = if index.is_empty() {
            next += 1;
            next - 1
        } else {
            index.parse().unwrap_or(usize::MAX)
        };
        let Some(arg) = args.get(index) else { continue };
        match choice {
            None => {
                let _ = write!(out, "{arg}");
            }
            Some(choice) => {
                let (one, other) = choice.split_once('|').unwrap_or((choice, choice));
                out.push_str(if arg.to_string() == "1" { one } else { other });
            }
        }
    }
    out.push_str(rest);
    out
}

/// 模板要几个参数（用到的最大序号 + 1）。测试用它核对原文和译文对得上。
pub fn arity(template: &str) -> usize {
    let mut count = 0;
    let mut next = 0;
    let mut rest = template;
    while let Some(at) = rest.find('{') {
        rest = &rest[at + 1..];
        if let Some(stripped) = rest.strip_prefix('{') {
            rest = stripped;
            continue;
        }
        let Some(end) = rest.find('}') else { break };
        let index = rest[..end].split('?').next().unwrap_or("");
        rest = &rest[end + 1..];
        let index = if index.is_empty() {
            next += 1;
            next - 1
        } else {
            index.parse().unwrap_or(0)
        };
        count = count.max(index + 1);
    }
    count
}

/// 系统「语言与地区」里排在最前、我们又有的那个语言；一个都没有用英文。
pub fn system() -> Lang {
    static SYSTEM: OnceLock<Lang> = OnceLock::new();
    *SYSTEM.get_or_init(|| pick(&preferred_languages()))
}

fn pick(preferred: &[String]) -> Lang {
    preferred
        .iter()
        .find_map(|tag| match tag.split(['-', '_']).next()?.to_ascii_lowercase().as_str() {
            "zh" => Some(Lang::Zh),
            "en" => Some(Lang::En),
            _ => None,
        })
        .unwrap_or(Lang::En)
}

#[cfg(target_os = "macos")]
fn preferred_languages() -> Vec<String> {
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFLocaleCopyPreferredLanguages() -> CFArrayRef;
    }
    unsafe {
        let languages = CFLocaleCopyPreferredLanguages();
        if languages.is_null() {
            return Vec::new();
        }
        CFArray::<CFString>::wrap_under_create_rule(languages).iter().map(|tag| tag.to_string()).collect()
    }
}

#[cfg(not(target_os = "macos"))]
fn preferred_languages() -> Vec<String> {
    std::env::var("LANG").into_iter().collect()
}

#[macro_export]
macro_rules! tr {
    ($zh:literal) => {
        $crate::i18n::translate($crate::i18n::current(), $zh)
    };
    ($zh:literal, $($arg:expr),+ $(,)?) => {
        $crate::i18n::format($crate::i18n::translate($crate::i18n::current(), $zh), &[$(&$arg as &dyn ::std::fmt::Display),+])
    };
}

#[macro_export]
macro_rules! trc {
    ($context:literal, $zh:literal) => {
        $crate::i18n::translate_in($crate::i18n::current(), $context, $zh)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_fills_in_order_by_index_and_picks_plurals() {
        assert_eq!(format("删除 {} 个项目", &[&3]), "删除 3 个项目");
        assert_eq!(format("{} 已上传到 {}", &[&"a.zip", &"/srv"]), "a.zip 已上传到 /srv");
        assert_eq!(format("{1} needs {0}'s password", &[&"root", &"sudo"]), "sudo needs root's password", "译文可以调换顺序");
        assert_eq!(format("{0} hidden {0?file|files}", &[&1]), "1 hidden file");
        assert_eq!(format("{0} hidden {0?file|files}", &[&12]), "12 hidden files");
        assert_eq!(format("{{}} {} }", &[&"x"]), "{} x }", "双写的花括号是它本身");
        assert_eq!(format("{} 和 {}", &[&"a"]), "a 和 ", "参数不够的留空，不崩");
        assert_eq!(format("没有参数", &[]), "没有参数");
    }

    #[test]
    fn arity_counts_the_highest_argument_used() {
        assert_eq!(arity("保存"), 0);
        assert_eq!(arity("{} 和 {}"), 2);
        assert_eq!(arity("{1} then {0}"), 2);
        assert_eq!(arity("{0} hidden {0?file|files}"), 1);
        assert_eq!(arity("{{}}"), 0);
    }

    #[test]
    fn system_language_is_the_first_one_we_have() {
        let tags = |list: &[&str]| list.iter().map(|tag| tag.to_string()).collect::<Vec<_>>();
        assert_eq!(pick(&tags(&["zh-Hans-CN", "en-CN"])), Lang::Zh);
        assert_eq!(pick(&tags(&["fr-FR", "en-GB", "zh-Hans"])), Lang::En, "法文没有，往下找到英文");
        assert_eq!(pick(&tags(&["zh_TW"])), Lang::Zh);
        assert_eq!(pick(&tags(&["ja-JP"])), Lang::En, "一个都没有：英文");
        assert_eq!(pick(&[]), Lang::En);
    }

    #[test]
    fn untranslated_text_falls_back_to_the_original() {
        assert_eq!(translate(Lang::Zh, "已停止"), "已停止");
        assert_eq!(translate(Lang::En, "已停止"), "Stopped");
        assert_eq!(translate(Lang::En, "表里没有的话"), "表里没有的话");
        assert_eq!(translate_in(Lang::En, "没有这个场合", "已停止"), "Stopped", "没有带场合的译文就用普通的");
        assert_eq!(translate_in(Lang::Zh, "标签页", "端口"), "端口");
    }

    #[test]
    fn every_translation_takes_the_same_arguments() {
        for (zh, en) in en::TABLE {
            assert_eq!(arity(zh), arity(en), "{zh} → {en}");
        }
    }
}
