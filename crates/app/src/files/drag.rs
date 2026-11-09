//! 文件管理里的拖拽（旧 file-drag.ts 的纯逻辑）：本地 ⇄ 远程两侧互拖即上传 / 下载，从 Finder 拖进远程一侧即上传。
//! 原生下两侧互拖用 GPUI 的 on_drag / on_drop，Finder 拖入用 ExternalPaths。

use super::model::{FileEntry, PaneKind};

/// 落点：哪一侧、是否落在某个文件夹行上。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropTarget {
    pub pane: PaneKind,
    pub directory: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropKind {
    Upload,
    Download,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropAction {
    pub kind: DropKind,
    pub directory: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragSource {
    Pane(PaneKind),
    Finder,
}

/// 拖到哪、做什么：本地 → 远程 = 上传，远程 → 本地 = 下载，Finder 只能拖进远程。目标目录缺省为那一侧的当前目录。
pub fn plan_drop(source: DragSource, target: Option<&DropTarget>, local_dir: &str, remote_dir: &str) -> Option<DropAction> {
    let target = target?;
    let allowed = match source {
        DragSource::Finder => target.pane == PaneKind::Remote,
        DragSource::Pane(pane) => pane != target.pane,
    };
    if !allowed {
        return None;
    }
    let current = if target.pane == PaneKind::Remote { remote_dir } else { local_dir };
    let directory = target.directory.clone().filter(|d| !d.is_empty()).unwrap_or_else(|| current.to_string());
    if directory.is_empty() {
        return None;
    }
    let kind = if target.pane == PaneKind::Remote { DropKind::Upload } else { DropKind::Download };
    Some(DropAction { kind, directory })
}

/// 按住的行在选中项里就拖整组选中，否则只拖这一行。「..」不能拖。
pub fn drag_items(entry: &FileEntry, selected: &std::collections::HashSet<String>, files: &[FileEntry]) -> Vec<FileEntry> {
    if entry.is_parent() {
        return Vec::new();
    }
    if selected.contains(&entry.path) {
        files.iter().filter(|file| !file.is_parent() && selected.contains(&file.path)).cloned().collect()
    } else {
        vec![entry.clone()]
    }
}

pub fn drag_label(items: &[FileEntry]) -> String {
    match items {
        [only] => only.name.clone(),
        _ => format!("{} 个项目", items.len()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    const LOCAL: &str = "/Users/me/Downloads";
    const REMOTE: &str = "/home/bob/uploads";

    fn target(pane: PaneKind, directory: Option<&str>) -> DropTarget {
        DropTarget { pane, directory: directory.map(String::from) }
    }

    fn action(kind: DropKind, directory: &str) -> Option<DropAction> {
        Some(DropAction { kind, directory: directory.into() })
    }

    fn entry(name: &str, path: &str) -> FileEntry {
        FileEntry { name: name.into(), path: path.into(), is_directory: false, size: 0, modified: None, permissions: None, owner: None }
    }

    #[test]
    fn local_to_remote_uploads_and_remote_to_local_downloads() {
        let local = DragSource::Pane(PaneKind::Local);
        let remote = DragSource::Pane(PaneKind::Remote);
        assert_eq!(plan_drop(local, Some(&target(PaneKind::Remote, None)), LOCAL, REMOTE), action(DropKind::Upload, REMOTE));
        let folder = "/home/bob/uploads/existing";
        assert_eq!(plan_drop(local, Some(&target(PaneKind::Remote, Some(folder))), LOCAL, REMOTE), action(DropKind::Upload, folder));
        assert_eq!(plan_drop(remote, Some(&target(PaneKind::Local, None)), LOCAL, REMOTE), action(DropKind::Download, LOCAL));
        assert_eq!(plan_drop(local, Some(&target(PaneKind::Local, None)), LOCAL, REMOTE), None, "同一侧内不做移动");
        assert_eq!(plan_drop(remote, None, LOCAL, REMOTE), None);
        assert_eq!(plan_drop(local, Some(&target(PaneKind::Remote, None)), LOCAL, ""), None, "远程还没打开目录");
    }

    #[test]
    fn finder_drops_only_land_on_the_remote_side() {
        assert_eq!(plan_drop(DragSource::Finder, Some(&target(PaneKind::Remote, None)), LOCAL, REMOTE), action(DropKind::Upload, REMOTE));
        assert_eq!(plan_drop(DragSource::Finder, Some(&target(PaneKind::Local, None)), LOCAL, REMOTE), None);
    }

    #[test]
    fn dragging_a_selected_row_drags_the_whole_selection() {
        let files = [entry("..", "/home"), entry("a", "/x/a"), entry("b", "/x/b"), entry("c", "/x/c")];
        let selected: HashSet<String> = ["/x/a".to_string(), "/x/c".to_string()].into();
        let names = |items: Vec<FileEntry>| items.into_iter().map(|f| f.name).collect::<Vec<_>>();
        assert_eq!(names(drag_items(&files[1], &selected, &files)), ["a", "c"]);
        assert_eq!(names(drag_items(&files[2], &selected, &files)), ["b"]);
        assert!(drag_items(&files[0], &HashSet::new(), &files).is_empty());
        assert_eq!(drag_label(&files[1..2]), "a");
        assert_eq!(drag_label(&files[1..]), "3 个项目");
    }
}
