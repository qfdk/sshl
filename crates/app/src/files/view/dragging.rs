//! 应用内拖放（两栏互拖）和从 Finder 拖进远程栏；拖动时跟着指针的提示。

use super::*;

/// 拖动的那一行（实际拖哪些在开始拖时按选择算）。
#[derive(Clone)]
pub(super) struct DragFiles {
    pub(super) pane: PaneKind,
    pub(super) entry: FileEntry,
}

pub(super) struct DragState {
    pub(super) source: PaneKind,
    pub(super) items: Vec<FileEntry>,
    pub(super) preview: Option<Entity<DragPreview>>,
}

/// 拖拽中跟着指针的提示：拖了什么、松手会传到哪。
pub(super) struct DragPreview {
    pub(super) label: SharedString,
    pub(super) folder: bool,
    pub(super) source: PaneKind,
    pub(super) destination: Option<String>,
    pub(super) offset: Point<Pixels>,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let upload = self.source == PaneKind::Local;
        div().pl(self.offset.x + px(14.)).pt(self.offset.y + px(14.)).child(
            div()
                .max_w(px(320.))
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(8.))
                .border_1()
                .border_color(theme::color(ui::BORDER))
                .bg(theme::color(ui::CARD))
                .shadow_lg()
                .text_size(px(12.))
                .text_color(theme::color(ui::FOREGROUND))
                .child(if self.folder {
                    kit::icon_sized(Icon::Folder, 14.).text_color(theme::color(ui::PRIMARY))
                } else {
                    kit::icon_sized(Icon::FileText, 14.).text_color(theme::color(ui::MUTED_FOREGROUND))
                })
                .child(div().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(self.label.clone()))
                .when_some(self.destination.clone(), |d, destination| {
                    let name = basename(&destination);
                    let name = if name.is_empty() || destination == "/" { "/".to_string() } else { name };
                    d.child(div().flex_shrink_0().text_color(theme::color(ui::MUTED_FOREGROUND)).child(if upload { tr!("上传到 {}", name) } else { tr!("下载到 {}", name) }))
                }),
        )
    }
}

impl FileManager {
    pub(super) fn begin_drag(&mut self, value: &DragFiles) -> Vec<FileEntry> {
        let pane = self.pane(value.pane);
        let items = drag_items(&value.entry, &pane.selected, &pane.files);
        self.dragging = Some(DragState { source: value.pane, items: items.clone(), preview: None });
        self.hover_pane = None;
        self.hover_row = None;
        items
    }

    pub(super) fn drag_target(&self) -> Option<DropTarget> {
        let pane = self.hover_pane?;
        let directory = self.hover_row.as_ref().filter(|(p, _)| *p == pane).map(|(_, dir)| dir.clone());
        Some(DropTarget { pane, directory })
    }

    pub(super) fn update_drag_preview(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = &self.dragging else { return };
        let destination = plan_drop(DragSource::Pane(drag.source), self.drag_target().as_ref(), &self.local.path, &self.remote.path).map(|a| a.directory);
        if let Some(preview) = drag.preview.clone() {
            preview.update(cx, |preview, cx| {
                if preview.destination != destination {
                    preview.destination = destination;
                    cx.notify();
                }
            });
        }
    }

    pub(super) fn drag_over_pane(&mut self, kind: PaneKind, inside: bool, cx: &mut Context<Self>) {
        let next = if inside { Some(kind) } else if self.hover_pane == Some(kind) { None } else { self.hover_pane };
        if next != self.hover_pane {
            self.hover_pane = next;
            self.update_drag_preview(cx);
        }
    }

    pub(super) fn drag_over_row(&mut self, kind: PaneKind, directory: &str, inside: bool, cx: &mut Context<Self>) {
        let current = self.hover_row.as_ref().is_some_and(|(p, d)| *p == kind && d == directory);
        if inside && !current {
            self.hover_row = Some((kind, directory.to_string()));
            self.update_drag_preview(cx);
        } else if !inside && current {
            self.hover_row = None;
            self.update_drag_preview(cx);
        }
    }

    pub(super) fn drop_files(&mut self, target: DropTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.hover_pane = None;
        self.hover_row = None;
        let Some(drag) = self.dragging.take() else { return };
        let Some(action) = plan_drop(DragSource::Pane(drag.source), Some(&target), &self.local.path, &self.remote.path) else { return };
        match action.kind {
            DropKind::Upload => self.upload_items(drag.items, Some(action.directory), window, cx),
            DropKind::Download => self.download_items(drag.items, Some(action.directory), window, cx),
        }
    }

    pub(super) fn drop_external(&mut self, paths: &ExternalPaths, directory: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        let target = DropTarget { pane: PaneKind::Remote, directory };
        let Some(action) = plan_drop(DragSource::Finder, Some(&target), &self.local.path, &self.remote.path) else { return };
        let paths: Vec<String> = paths.paths().iter().map(|p| p.to_string_lossy().into_owned()).collect();
        if !paths.is_empty() {
            self.upload_paths(paths, action.directory, window, cx);
        }
    }
}
