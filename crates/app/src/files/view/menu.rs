//! 右键菜单：点在文件上（单个 / 多个）和点在空白处各是一套。

use super::*;

impl FileManager {
    pub(super) fn open_menu(&mut self, kind: PaneKind, target: Option<FileEntry>, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = &target {
            self.ensure_context_selection(kind, entry);
        }
        self.menu = Some(MenuState { pane: kind, position, target });
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn menu_entries(&self, menu: &MenuState, cx: &mut Context<Self>) -> Vec<MenuEntry> {
        let kind = menu.pane;
        let this = cx.entity().downgrade();
        let on = move |f: fn(&mut FileManager, &mut Window, &mut Context<FileManager>)| {
            let this = this.clone();
            move |window: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |fm, cx| f(fm, window, cx));
            }
        };
        let this = cx.entity().downgrade();
        let with_items = move |items: Vec<FileEntry>, f: fn(&mut FileManager, Vec<FileEntry>, &mut Window, &mut Context<FileManager>)| {
            let this = this.clone();
            move |window: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |fm, cx| f(fm, items.clone(), window, cx));
            }
        };
        let refresh = MenuItem::new(tr!("刷新"), {
            let f: fn(&mut FileManager, &mut Window, &mut Context<FileManager>) = match kind {
                PaneKind::Local => |fm, w, cx| fm.refresh(PaneKind::Local, w, cx),
                PaneKind::Remote => |fm, w, cx| fm.refresh(PaneKind::Remote, w, cx),
            };
            on(f)
        })
        .icon(Icon::RefreshCw);
        let connected = self.session.is_some();
        let Some(target) = &menu.target else {
            let first = match kind {
                PaneKind::Remote => MenuItem::new(tr!("新建文件夹"), on(|fm, w, cx| fm.open_new_folder(w, cx))).icon(Icon::FolderPlus).disabled(!connected),
                PaneKind::Local => MenuItem::new(tr!("选择文件夹上传"), on(|fm, w, cx| fm.select_folder_to_upload(w, cx))).icon(Icon::Upload).disabled(!connected),
            };
            let hidden_label = if crate::settings::get(cx).show_hidden_files { tr!("不显示隐藏文件") } else { tr!("显示隐藏文件") };
            let hidden = MenuItem::new(hidden_label, on(|fm, w, cx| fm.toggle_hidden_files(w, cx)));
            return vec![MenuEntry::Item(first), MenuEntry::Item(refresh), MenuEntry::Separator, MenuEntry::Item(hidden)];
        };
        let items = self.selected_items(kind);
        let selected = if items.iter().any(|i| i.path == target.path) { items } else { vec![target.clone()] };
        let delete: fn(&mut FileManager, Vec<FileEntry>, &mut Window, &mut Context<FileManager>) = match kind {
            PaneKind::Local => |fm, items, w, cx| fm.delete_items(PaneKind::Local, items, w, cx),
            PaneKind::Remote => |fm, items, w, cx| fm.delete_items(PaneKind::Remote, items, w, cx),
        };
        let copy: fn(&mut FileManager, Vec<FileEntry>, &mut Window, &mut Context<FileManager>) = |fm, items, _, cx| fm.copy_paths(items, cx);
        let transfer: fn(&mut FileManager, Vec<FileEntry>, &mut Window, &mut Context<FileManager>) = match kind {
            PaneKind::Local => |fm, items, w, cx| fm.upload_items(items, None, w, cx),
            PaneKind::Remote => |fm, items, w, cx| fm.download_items(items, None, w, cx),
        };
        if selected.len() > 1 {
            let count = selected.len();
            let (label, glyph) = if kind == PaneKind::Remote { (tr!("下载 {} 个项目", count), Icon::Download) } else { (tr!("上传 {} 个项目", count), Icon::Upload) };
            return vec![
                MenuEntry::Item(MenuItem::new(label, with_items(selected.clone(), transfer)).icon(glyph)),
                MenuEntry::Item(MenuItem::new(tr!("复制 {} 个路径", count), with_items(selected.clone(), copy)).icon(Icon::Copy)),
                MenuEntry::Item(MenuItem::new(tr!("删除 {} 个项目", count), with_items(selected, delete)).icon(Icon::Trash)),
                MenuEntry::Item(refresh),
            ];
        }
        let item = selected[0].clone();
        let label = match (kind, item.is_directory) {
            (PaneKind::Remote, true) => (tr!("下载文件夹"), Icon::Download),
            (PaneKind::Remote, false) => (tr!("下载文件"), Icon::Download),
            (PaneKind::Local, true) => (tr!("上传文件夹"), Icon::Upload),
            (PaneKind::Local, false) => (tr!("上传文件"), Icon::Upload),
        };
        let delete_label = if item.is_directory { tr!("删除目录") } else { tr!("删除文件") };
        let rename: fn(&mut FileManager, Vec<FileEntry>, &mut Window, &mut Context<FileManager>) = match kind {
            PaneKind::Local => |fm, items, w, cx| fm.start_rename(PaneKind::Local, &items[0], w, cx),
            PaneKind::Remote => |fm, items, w, cx| fm.start_rename(PaneKind::Remote, &items[0], w, cx),
        };
        let open: fn(&mut FileManager, Vec<FileEntry>, &mut Window, &mut Context<FileManager>) = match kind {
            PaneKind::Local => |fm, items, w, cx| fm.open_file(PaneKind::Local, &items[0], w, cx),
            PaneKind::Remote => |fm, items, w, cx| fm.open_file(PaneKind::Remote, &items[0], w, cx),
        };
        let mut entries = Vec::new();
        if !item.is_directory {
            entries.push(MenuEntry::Item(MenuItem::new(tr!("打开"), with_items(vec![item.clone()], open)).icon(Icon::FileText)));
        }
        entries.extend([
            MenuEntry::Item(MenuItem::new(label.0, with_items(vec![item.clone()], transfer)).icon(label.1)),
            MenuEntry::Item(MenuItem::new(tr!("重命名"), with_items(vec![item.clone()], rename)).icon(Icon::Pencil)),
            MenuEntry::Item(MenuItem::new(tr!("复制路径"), with_items(vec![item.clone()], copy)).icon(Icon::Copy)),
            MenuEntry::Item(refresh),
            MenuEntry::Item(MenuItem::new(delete_label, with_items(vec![item], delete)).icon(Icon::Trash)),
        ]);
        entries
    }
}
