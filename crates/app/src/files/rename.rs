//! 重命名的规则：输入框里先选中哪一段、敲的名字换算成新路径（原地改名的输入框在 `view/rename.rs`）。

use super::model::{PaneKind, basename, join_path, parent_path};

/// 开始改名时先选中的部分：从头到这个字节位置。文件只选扩展名前面那段（同 Finder：`a.tar.gz` 选 `a.tar`），
/// 直接敲就是换名字、扩展名留着；文件夹、点开头又没有别的点的（`.bashrc`）、没有扩展名的，全选。
pub fn stem_end(name: &str, is_directory: bool) -> usize {
    if is_directory {
        return name.len();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => dot,
        _ => name.len(),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RenameError {
    /// 名字里有 `/`：会变成移到别的目录。
    Slash,
    /// `.` 和 `..` 是目录本身和上一级。
    Dots,
}

impl RenameError {
    pub fn message(&self) -> &'static str {
        match self {
            RenameError::Slash => tr!("名称不能包含斜杠。"),
            RenameError::Dots => tr!("名称不能是「.」或「..」。"),
        }
    }
}

/// 输入框里的名字 → 改名后的完整路径。没改（原样、空的、只多了首尾空白）是 `Ok(None)`：失焦、回车都当没改过。
/// 首尾空白去掉（多半是手滑）；但原名本身带首尾空白、又一个字没动的，不能因为去空白就被改掉。
pub fn rename_target(path: &str, kind: PaneKind, input: &str) -> Result<Option<String>, RenameError> {
    let old = basename(path);
    if input == old {
        return Ok(None);
    }
    let name = input.trim();
    if name.is_empty() || name == old {
        return Ok(None);
    }
    if name.contains('/') || name.contains('\0') {
        return Err(RenameError::Slash);
    }
    if name == "." || name == ".." {
        return Err(RenameError::Dots);
    }
    Ok(Some(join_path(&parent_path(path, kind), name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_part_before_the_extension_is_selected_at_first() {
        fn selected(name: &str, dir: bool) -> &str {
            &name[..stem_end(name, dir)]
        }
        assert_eq!(selected("a.txt", false), "a");
        assert_eq!(selected("archive.tar.gz", false), "archive.tar", "同 Finder：只去掉最后一个扩展名");
        assert_eq!(selected("报告 v2.pdf", false), "报告 v2", "按字节算，中文不切坏");
        assert_eq!(selected(".bashrc", false), ".bashrc", "点开头的隐藏文件整个是名字");
        assert_eq!(selected(".env.local", false), ".env");
        assert_eq!(selected("Makefile", false), "Makefile");
        assert_eq!(selected("release.d", true), "release.d", "文件夹全选");
    }

    #[test]
    fn a_new_name_becomes_a_path_next_to_the_old_one() {
        assert_eq!(rename_target("/srv/app/a.txt", PaneKind::Remote, "b.txt"), Ok(Some("/srv/app/b.txt".into())));
        assert_eq!(rename_target("/a.txt", PaneKind::Remote, "b.txt"), Ok(Some("/b.txt".into())), "根目录下的");
        assert_eq!(rename_target("/Users/me/下载/报告.pdf", PaneKind::Local, "报告 终版.pdf"), Ok(Some("/Users/me/下载/报告 终版.pdf".into())));
        assert_eq!(rename_target("/srv/it's", PaneKind::Remote, "  it is  "), Ok(Some("/srv/it is".into())), "去掉首尾空白");
        assert_eq!(rename_target("/srv/a.txt", PaneKind::Remote, "A.txt"), Ok(Some("/srv/A.txt".into())), "只改大小写也算改了");
    }

    #[test]
    fn leaving_the_name_alone_changes_nothing() {
        assert_eq!(rename_target("/srv/a.txt", PaneKind::Remote, "a.txt"), Ok(None));
        assert_eq!(rename_target("/srv/a.txt", PaneKind::Remote, ""), Ok(None), "清空 = 不改");
        assert_eq!(rename_target("/srv/a.txt", PaneKind::Remote, "   "), Ok(None));
        assert_eq!(rename_target("/srv/a.txt", PaneKind::Remote, " a.txt "), Ok(None));
        // 原名带结尾空格、一个字没动：不能被「去空白」改成别的名字
        assert_eq!(rename_target("/srv/notes ", PaneKind::Remote, "notes "), Ok(None));
    }

    #[test]
    fn names_that_would_leave_the_directory_are_refused() {
        assert_eq!(rename_target("/srv/a.txt", PaneKind::Remote, "../a.txt"), Err(RenameError::Slash));
        assert_eq!(rename_target("/srv/a.txt", PaneKind::Remote, "logs/a.txt"), Err(RenameError::Slash));
        assert_eq!(rename_target("/srv/a.txt", PaneKind::Local, "/tmp/a.txt"), Err(RenameError::Slash));
        assert_eq!(rename_target("/srv/a.txt", PaneKind::Remote, ".."), Err(RenameError::Dots));
        assert_eq!(rename_target("/srv/a.txt", PaneKind::Remote, "."), Err(RenameError::Dots));
        assert_eq!(rename_target("/srv/a.txt", PaneKind::Remote, "...txt"), Ok(Some("/srv/...txt".into())), "点开头的普通名字可以");
    }
}
