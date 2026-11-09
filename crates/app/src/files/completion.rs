//! 地址栏补全（旧 path-completion.ts）：只补目录，Tab 补全到下一层。

use super::model::{FileEntry, PaneKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionQuery {
    /// 输入框里最后一个分隔符及之前的部分，补全结果拼在它后面。
    pub base: String,
    /// 要列出的目录，与文件列表加载时的路径写法一致。
    pub directory: String,
    /// 正在输入的名字。
    pub prefix: String,
}

/// /usr/lib 这类目录有上千个子目录，下拉框只放前 100 个，继续输入会自然缩小范围。
const SUGGESTION_LIMIT: usize = 100;

fn is_drive(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn collapse_slashes(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch == '/' && out.ends_with('/') {
            continue;
        }
        out.push(ch);
    }
    out
}

/// 没有分隔符的相对路径无从列目录，不补全。
pub fn split_completion_path(value: &str, kind: PaneKind) -> Option<CompletionQuery> {
    let index = match kind {
        PaneKind::Remote => value.rfind('/'),
        PaneKind::Local => value.rfind(['/', '\\']),
    }?;
    let base = value[..=index].to_string();
    let prefix = value[index + 1..].to_string();
    if kind == PaneKind::Remote {
        let mut directory = collapse_slashes(&format!("/{base}"));
        if directory.len() > 1 && directory.ends_with('/') {
            directory.pop();
        }
        return Some(CompletionQuery { base, directory, prefix });
    }
    let trimmed = base.trim_end_matches(['/', '\\']);
    // 根目录保留分隔符（"/"、"C:\"），与 is_root_path 的写法一致
    let directory = if trimmed.is_empty() || is_drive(trimmed) { base[..trimmed.len() + 1].to_string() } else { trimmed.to_string() };
    Some(CompletionQuery { base, directory, prefix })
}

/// 只列目录：地址栏是用来跳转的。隐藏目录和 shell 一样，只在输入了 "." 之后才出现。
pub fn match_directories(entries: &[FileEntry], prefix: &str) -> Vec<FileEntry> {
    let needle = prefix.to_lowercase();
    let mut matches: Vec<FileEntry> = entries
        .iter()
        .filter(|entry| {
            entry.is_directory
                && (needle.starts_with('.') || !entry.name.starts_with('.'))
                && entry.name.to_lowercase().starts_with(&needle)
        })
        .cloned()
        .collect();
    matches.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| b.name.cmp(&a.name)));
    matches.truncate(SUGGESTION_LIMIT);
    matches
}

fn common_name_prefix(names: &[&str]) -> String {
    let first: Vec<char> = names[0].chars().collect();
    let mut end = first.len();
    for name in &names[1..] {
        let chars: Vec<char> = name.chars().collect();
        let mut index = 0;
        while index < end && index < chars.len() && chars[index].to_lowercase().eq(first[index].to_lowercase()) {
            index += 1;
        }
        end = index;
    }
    first[..end].iter().collect()
}

/// Tab：高亮项或唯一匹配补全成目录并带上分隔符，接着就能列下一层；多个匹配时补到公共前缀。
pub fn tab_completion(query: &CompletionQuery, matches: &[FileEntry], highlighted: Option<usize>) -> Option<String> {
    let pick = highlighted.and_then(|i| matches.get(i)).or(if matches.len() == 1 { matches.first() } else { None });
    let separator = query.base.chars().last().unwrap_or('/');
    if let Some(pick) = pick {
        return Some(format!("{}{}{}", query.base, pick.name, separator));
    }
    if matches.is_empty() {
        return None;
    }
    let names: Vec<&str> = matches.iter().map(|entry| entry.name.as_str()).collect();
    let common = common_name_prefix(&names);
    (common.chars().count() > query.prefix.chars().count()).then(|| format!("{}{}", query.base, common))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> FileEntry {
        FileEntry { name: name.into(), path: format!("/x/{name}"), is_directory: true, size: 0, modified: None, permissions: None, owner: None }
    }

    fn file(name: &str) -> FileEntry {
        FileEntry { is_directory: false, ..dir(name) }
    }

    fn q(base: &str, directory: &str, prefix: &str) -> CompletionQuery {
        CompletionQuery { base: base.into(), directory: directory.into(), prefix: prefix.into() }
    }

    fn names(entries: Vec<FileEntry>) -> Vec<String> {
        entries.into_iter().map(|e| e.name).collect()
    }

    #[test]
    fn splits_the_typed_path_into_directory_and_name() {
        assert_eq!(split_completion_path("/Users/qfdk/Doc", PaneKind::Local), Some(q("/Users/qfdk/", "/Users/qfdk", "Doc")));
        assert_eq!(split_completion_path("/Users/", PaneKind::Local), Some(q("/Users/", "/Users", "")));
        assert_eq!(split_completion_path("/", PaneKind::Local), Some(q("/", "/", "")));
        assert_eq!(split_completion_path("/us", PaneKind::Remote), Some(q("/", "/", "us")));
        assert_eq!(split_completion_path("C:\\Users\\qf", PaneKind::Local), Some(q("C:\\Users\\", "C:\\Users", "qf")));
        assert_eq!(split_completion_path("C:\\", PaneKind::Local), Some(q("C:\\", "C:\\", "")));
    }

    #[test]
    fn remote_directories_are_normalized_like_the_file_list() {
        assert_eq!(split_completion_path("//var//lo", PaneKind::Remote).unwrap().directory, "/var");
        assert_eq!(split_completion_path("var/lo", PaneKind::Remote).unwrap().directory, "/var");
        // 远程路径里的反斜杠是普通字符，不是分隔符
        assert_eq!(split_completion_path("/srv/a\\b", PaneKind::Remote).unwrap().prefix, "a\\b");
    }

    #[test]
    fn a_path_without_separator_has_nothing_to_list() {
        assert_eq!(split_completion_path("", PaneKind::Local), None);
        assert_eq!(split_completion_path("Users", PaneKind::Remote), None);
    }

    #[test]
    fn suggests_only_directories_with_the_prefix_ignoring_case() {
        let entries = [dir("Downloads"), file("doc.txt"), dir("Documents"), dir("Desktop"), dir("src")];
        assert_eq!(names(match_directories(&entries, "do")), ["Documents", "Downloads"]);
        assert_eq!(names(match_directories(&entries, "")), ["Desktop", "Documents", "Downloads", "src"]);
    }

    #[test]
    fn hidden_directories_need_a_leading_dot() {
        let entries = [dir(".config"), dir(".cache"), dir("code")];
        assert_eq!(names(match_directories(&entries, "")), ["code"]);
        assert_eq!(names(match_directories(&entries, "c")), ["code"]);
        assert_eq!(names(match_directories(&entries, ".c")), [".cache", ".config"]);
    }

    #[test]
    fn tab_completes_the_highlighted_or_only_match() {
        let query = q("/Users/qfdk/", "/Users/qfdk", "Do");
        let matches = [dir("Documents"), dir("Downloads")];
        assert_eq!(tab_completion(&query, &matches, Some(1)).as_deref(), Some("/Users/qfdk/Downloads/"));
        assert_eq!(tab_completion(&query, &[dir("Documents")], None).as_deref(), Some("/Users/qfdk/Documents/"));
        let windows = q("C:\\Users\\", "C:\\Users", "q");
        assert_eq!(tab_completion(&windows, &[dir("qfdk")], None).as_deref(), Some("C:\\Users\\qfdk\\"));
    }

    #[test]
    fn tab_extends_to_the_common_prefix_or_does_nothing() {
        let query = q("/srv/", "/srv", "a");
        assert_eq!(tab_completion(&query, &[dir("app-api"), dir("app-web")], None).as_deref(), Some("/srv/app-"));
        let typed = q("/srv/", "/srv", "app-");
        assert_eq!(tab_completion(&typed, &[dir("app-api"), dir("app-web")], None), None);
        assert_eq!(tab_completion(&query, &[], None), None);
    }
}
