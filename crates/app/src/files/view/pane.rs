//! 一栏的画法：表头（标题、地址栏、按钮）、列名、文件行；栏窄时先藏哪些列。

use super::dragging::{DragFiles, DragPreview};
use super::*;

pub(super) const ROW_HEIGHT: f32 = 34.;
/// 名称列至少留这么宽（图标 + 十来个字）。
pub(super) const NAME_MIN_WIDTH: f32 = 140.;
pub(super) const SIZE_WIDTH: f32 = 80.;
pub(super) const DATE_WIDTH: f32 = 144.;
pub(super) const OWNER_WIDTH: f32 = 96.;
pub(super) const PERMISSIONS_WIDTH: f32 = 64.;

/// 名称以外显示哪些列。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Columns {
    size: bool,
    date: bool,
    owner: bool,
    permissions: bool,
}

impl Columns {
    pub(super) fn width(&self) -> f32 {
        [(self.size, SIZE_WIDTH), (self.date, DATE_WIDTH), (self.owner, OWNER_WIDTH), (self.permissions, PERMISSIONS_WIDTH)]
            .iter()
            .filter(|(shown, _)| *shown)
            .map(|(_, width)| width)
            .sum()
    }
}

/// 栏宽 `width`（还没量到为 None）放得下哪些列：名称至少留 NAME_MIN_WIDTH，不够时先藏远程才有的所有者、权限，
/// 再藏修改日期、大小。
pub(super) fn columns_for(kind: PaneKind, width: Option<f32>) -> Columns {
    let remote = kind == PaneKind::Remote;
    let mut columns = Columns { size: true, date: true, owner: remote, permissions: remote };
    let Some(width) = width else { return columns };
    let hide: [fn(&mut Columns); 4] = [|c| c.owner = false, |c| c.permissions = false, |c| c.date = false, |c| c.size = false];
    for step in hide {
        if columns.width() + NAME_MIN_WIDTH <= width {
            break;
        }
        step(&mut columns);
    }
    columns
}

impl FileManager {
    pub(super) fn render_rows(&mut self, kind: PaneKind, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let connected = self.session.is_some();
        let shown = columns_for(kind, self.pane_width(kind));
        let pane = self.pane(kind);
        let parent_offset = pane.parent_offset();
        let current = pane.path.clone();
        let mut out = Vec::with_capacity(range.len());
        for ix in range {
            let Some(entry) = pane.row(ix) else { break };
            let parent = entry.is_parent();
            let selected = !parent && pane.selected.contains(&entry.path);
            let directory = if parent { Some(parent_path(&current, kind)) } else if entry.is_directory { Some(entry.path.clone()) } else { None };
            let file_index = ix.saturating_sub(parent_offset);
            let muted = theme::color(ui::MUTED_FOREGROUND);
            let icon = if parent {
                kit::icon(Icon::ArrowUp).text_color(theme::color(ui::PRIMARY))
            } else if entry.is_directory {
                kit::icon(Icon::Folder).text_color(theme::color(ui::PRIMARY))
            } else {
                kit::icon(Icon::FileText).text_color(muted)
            };
            let click_entry = entry.clone();
            let menu_entry = entry.clone();
            let mut row = div()
                .id(("file-row", ix))
                .role(Role::Row)
                .aria_label(SharedString::from(entry.name.clone()))
                // uniform_list 的行默认按内容收缩，不撑满就和表头的列对不齐
                .w_full()
                .h(px(ROW_HEIGHT))
                .flex()
                .items_center()
                .text_size(px(12.))
                .border_b_1()
                .border_color(theme::color(ui::BORDER))
                .cursor_pointer()
                .map(|r| {
                    if selected {
                        r.bg(theme::color(ui::ACCENT)).text_color(theme::color(ui::ACCENT_FOREGROUND))
                    } else {
                        r.hover(|s| s.bg(theme::alpha(ui::MUTED, 0.5)))
                    }
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.active_pane = Some(kind);
                        let focus = this.pane(kind).list_focus.clone();
                        window.focus(&focus, cx);
                    }),
                )
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if !click_entry.is_parent() {
                        this.select_row(kind, &click_entry, file_index, event.modifiers());
                    }
                    if event.click_count() >= 2 && click_entry.is_directory {
                        this.open_entry(kind, &click_entry, window, cx);
                    } else if event.click_count() == 2 {
                        this.open_file(kind, &click_entry, window, cx);
                    }
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.active_pane = Some(kind);
                        let target = (!menu_entry.is_parent()).then(|| menu_entry.clone());
                        this.open_menu(kind, target, event.position, window, cx);
                    }),
                );
            if let Some(directory) = directory.clone().filter(|_| connected) {
                let drop_dir = directory.clone();
                let move_dir = directory.clone();
                row = row
                    .drag_over::<DragFiles>(move |s, drag, _, _| if drag.pane != kind { s.bg(theme::alpha(ui::PRIMARY, 0.15)) } else { s })
                    .on_drop(cx.listener(move |this, _: &DragFiles, window, cx| {
                        this.drop_files(DropTarget { pane: kind, directory: Some(drop_dir.clone()) }, window, cx);
                    }))
                    .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<DragFiles>, _, cx| {
                        this.drag_over_row(kind, &move_dir, event.bounds.contains(&event.event.position), cx);
                    }));
                if kind == PaneKind::Remote {
                    let finder_dir = directory.clone();
                    row = row
                        .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(theme::alpha(ui::PRIMARY, 0.15)))
                        .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                            this.drop_external(paths, Some(finder_dir.clone()), window, cx);
                        }));
                }
            }
            if !parent && connected {
                let weak = cx.entity().downgrade();
                row = row.on_drag(DragFiles { pane: kind, entry: entry.clone() }, move |value, offset, _, cx| {
                    let items = weak.update(cx, |this, _| this.begin_drag(value)).unwrap_or_default();
                    let folder = !(items.len() == 1 && !items[0].is_directory);
                    let preview = cx.new(|_| DragPreview { label: drag_label(&items).into(), folder, source: value.pane, destination: None, offset });
                    let _ = weak.update(cx, |this, _| {
                        if let Some(drag) = &mut this.dragging {
                            drag.preview = Some(preview.clone());
                        }
                    });
                    preview
                });
            }
            let mut name = div()
                .flex_1()
                .min_w_0()
                .px(px(14.))
                .flex()
                .items_center()
                .gap_2()
                .child(icon)
                .child(match self.rename_field(kind, &entry.path, cx) {
                    Some(field) => field,
                    None => div().min_w_0().truncate().child(entry.name.clone()).into_any_element(),
                });
            if selected {
                name = name.font_weight(FontWeight::MEDIUM);
            }
            let size = if parent { String::new() } else if entry.is_directory { "-".into() } else { format_file_size(entry.size) };
            let date = if parent { String::new() } else { format_date(entry.modified) };
            row = row
                .child(name)
                .when(shown.size, |row| row.child(div().w(px(SIZE_WIDTH)).flex_shrink_0().px_3().flex().justify_end().text_color(muted).child(size)))
                .when(shown.date, |row| row.child(div().w(px(DATE_WIDTH)).flex_shrink_0().px_3().text_color(muted).child(date)));
            if shown.owner {
                let owner = if parent { String::new() } else { entry.owner.clone().unwrap_or_else(|| "-".into()) };
                row = row.child(div().w(px(OWNER_WIDTH)).flex_shrink_0().px_3().min_w_0().truncate().text_color(muted).child(owner));
            }
            if shown.permissions {
                let permissions = if parent { String::new() } else { format_permissions(entry.permissions) };
                let perm_entry = entry.clone();
                row = row.child(
                    div()
                        .id(("perm", ix))
                        .role(Role::Button)
                        .aria_label(tr!("修改权限"))
                        .w(px(PERMISSIONS_WIDTH))
                        .flex_shrink_0()
                        .px_3()
                        .when(!parent, |cell| {
                            cell.tooltip(|window, cx| kit::tooltip(tr!("点击修改权限").into(), window, cx)).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_permissions(&perm_entry, window, cx);
                            }))
                        })
                        .child(permissions),
                );
            }
            out.push(row.into_any_element());
        }
        out
    }

    pub(super) fn pane_button(&self, id: (&'static str, usize), glyph: Icon, title: &'static str, disabled: bool, on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static, cx: &mut Context<Self>) -> AnyElement {
        kit::button(id, Variant::Ghost)
            .w(px(32.))
            .h(px(32.))
            .px_0()
            .aria_label(title)
            .tooltip(move |window, cx| kit::tooltip(title.into(), window, cx))
            .child(kit::icon(glyph))
            .when(disabled, |b| b.opacity(0.5).cursor_default())
            .when(!disabled, |b| b.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| on_click(this, window, cx))))
            .into_any_element()
    }

    pub(super) fn render_pane(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let remote = kind == PaneKind::Remote;
        let disabled = remote && self.session.is_none();
        let shown = columns_for(kind, self.pane_width(kind));
        let badge = if remote { self.render_badge(cx) } else { None };
        let bookmark_toggle = (remote && !disabled && !self.remote.path.is_empty()).then(|| self.bookmark_toggle(cx));
        let bookmark_dropdown = self.showing_bookmarks(kind).then(|| self.bookmark_dropdown(cx));
        let pane = self.pane(kind);
        let suggestions = pane.suggestions();
        let highlight = pane.highlight;
        let input_focused = pane.input_focused;
        let input = pane.input.clone();
        let loading = pane.loading;
        let rows = pane.row_count();
        let hidden = if pane.loading { 0 } else { pane.hidden };
        let list_focus = pane.list_focus.clone();
        let scroll = pane.scroll.clone();
        let empty: Option<AnyElement> = if loading {
            None
        } else if remote && let (Some(connection), Some(message)) = (self.remote_empty_connection.clone(), self.remote_empty) {
            Some(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .child(message.text())
                    .child(
                        kit::button("reconnect", Variant::Primary)
                            .h(px(32.))
                            .px_3()
                            .text_size(px(12.))
                            .aria_label(tr!("重新连接"))
                            .child(tr!("重新连接"))
                            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(FileManagerEvent::Reconnect(connection.clone())))),
                    )
                    .into_any_element(),
            )
        } else {
            let message = if remote {
                self.remote_empty
                    .map(|empty| SharedString::from(empty.text()))
                    .or_else(|| if self.session.is_none() { Some(RemoteEmpty::NotConnected.text().into()) } else { pane.error.clone().map(Into::into) })
            } else {
                pane.error.clone().map(Into::into)
            };
            message.map(|m| div().child(m).into_any_element()).or_else(|| (rows == 0).then(|| div().child(tr!("目录为空")).into_any_element()))
        };

        let header = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(14.))
            .pt_3()
            .pb(px(10.))
            .child(div().mr_1().flex_shrink_0().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(if remote { tr!("远程文件") } else { tr!("本地文件") }))
            .children(badge)
            .child(
                div()
                    .id(("path-bar", kind as usize))
                    .key_context("FilePathBar")
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .on_action(cx.listener(move |this, _: &PathNext, window, cx| this.path_next(kind, true, window, cx)))
                    .on_action(cx.listener(move |this, _: &PathPrev, window, cx| this.path_next(kind, false, window, cx)))
                    .on_action(cx.listener(move |this, _: &PathComplete, window, cx| this.path_complete(kind, window, cx)))
                    .on_action(cx.listener(move |this, _: &PathDismiss, _, cx| this.path_dismiss(kind, cx)))
                    .child(
                        div()
                            .h(px(32.))
                            .px_3()
                            .flex()
                            .items_center()
                            .rounded(px(9.))
                            .border_1()
                            .border_color(theme::color(if input_focused { ui::RING } else { ui::BORDER }))
                            .bg(theme::color(ui::MUTED))
                            .font_family(MONO_FONT)
                            .text_size(px(12.))
                            .when(disabled, |d| d.opacity(0.5))
                            // 点进远程地址栏：下拉先列收藏的目录（已在敲字补全时不打断）
                            .when(remote && !disabled, |d| {
                                d.on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                        if !this.bookmarks.is_empty() && !this.remote.suggesting && !this.bookmark_list {
                                            this.bookmark_list = true;
                                            this.remote.highlight = None;
                                            cx.notify();
                                        }
                                    }),
                                )
                            })
                            .child(div().flex_1().min_w_0().child(Input::new(&input)))
                            .children(bookmark_toggle),
                    )
                    .children(bookmark_dropdown.map(deferred))
                    // 下拉放到最后画（deferred），不然后面的表头和文件行会画在它上面
                    .when(!suggestions.is_empty(), |bar| {
                        bar.child(deferred(
                            div()
                                .id(("suggestions", kind as usize))
                                .role(Role::ListBox)
                                .aria_label(tr!("子目录"))
                                .absolute()
                                .left_0()
                                .right_0()
                                .top_full()
                                .mt_1()
                                .max_h(px(256.))
                                .overflow_y_scroll()
                                .p_1()
                                .rounded(px(10.))
                                .border_1()
                                .border_color(theme::color(ui::BORDER))
                                .bg(theme::color(ui::CARD))
                                .shadow_md()
                                .occlude()
                                .children(suggestions.into_iter().enumerate().map(|(index, entry)| {
                                    let open = entry.clone();
                                    div()
                                        .id(("suggestion", index))
                                        .role(Role::ListBoxOption)
                                        .px_2()
                                        .py(px(6.))
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .rounded(px(6.))
                                        .font_family(MONO_FONT)
                                        .text_size(px(12.))
                                        .cursor_pointer()
                                        .when(highlight == Some(index), |d| d.bg(theme::color(ui::ACCENT)).text_color(theme::color(ui::ACCENT_FOREGROUND)))
                                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                            if *hovered {
                                                this.pane_mut(kind).highlight = Some(index);
                                                cx.notify();
                                            }
                                        }))
                                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.open_suggestion(kind, &open, window, cx)))
                                        .child(kit::icon_sized(Icon::Folder, 14.).text_color(theme::color(ui::PRIMARY)))
                                        .child(div().min_w_0().child(entry.name))
                                })),
                        ))
                    }),
            )
            .child(if remote {
                self.pane_button(("pane-go", 1), Icon::ChevronRight, tr!("转到路径"), disabled, move |this, w, cx| this.load(PaneKind::Remote, None, true, w, cx), cx).into_any_element()
            } else {
                self.pane_button(("pane-browse", 0), Icon::FolderOpen, tr!("选择目录"), false, |this, w, cx| this.browse_local(w, cx), cx).into_any_element()
            })
            .child(self.pane_button(("pane-home", kind as usize), Icon::House, tr!("主目录"), disabled, move |this, w, cx| this.go_home(kind, w, cx), cx))
            .child(self.pane_button(("pane-refresh", kind as usize), Icon::RefreshCw, tr!("刷新"), disabled, move |this, w, cx| this.refresh(kind, w, cx), cx));

        let columns = div()
            .h(px(32.))
            .flex()
            .flex_shrink_0()
            .items_center()
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::color(ui::MUTED_FOREGROUND))
            .border_b_1()
            .border_color(theme::color(ui::BORDER))
            .child(div().flex_1().min_w_0().px(px(14.)).child(tr!("名称")))
            .when(shown.size, |c| c.child(div().w(px(SIZE_WIDTH)).flex_shrink_0().px_3().flex().justify_end().child(tr!("大小"))))
            .when(shown.date, |c| c.child(div().w(px(DATE_WIDTH)).flex_shrink_0().px_3().child(tr!("修改日期"))))
            .when(shown.owner, |c| c.child(div().w(px(OWNER_WIDTH)).flex_shrink_0().px_3().child(tr!("所有者"))))
            .when(shown.permissions, |c| c.child(div().w(px(PERMISSIONS_WIDTH)).flex_shrink_0().px_3().child(tr!("权限"))));

        let list = div()
            .id(("file-list", kind as usize))
            .key_context(FILE_LIST_CONTEXT)
            .track_focus(&list_focus)
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(columns)
            .child(match empty {
                Some(message) => div()
                    .h(px(128.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .px_4()
                    .text_size(px(12.))
                    .text_color(theme::color(ui::MUTED_FOREGROUND))
                    .child(message)
                    .into_any_element(),
                None => uniform_list(("rows", kind as usize), rows, cx.processor(move |this, range, window, cx| this.render_rows(kind, range, window, cx)))
                    .track_scroll(&scroll)
                    .w_full()
                    .flex_1()
                    .into_any_element(),
            })
            .when(hidden > 0, |d| {
                d.child(
                    div()
                        .id(("hidden-files", kind as usize))
                        .h(px(30.))
                        .flex_shrink_0()
                        .px(px(14.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_t_1()
                        .border_color(theme::color(ui::BORDER))
                        .text_size(px(12.))
                        .text_color(theme::color(ui::MUTED_FOREGROUND))
                        .role(Role::Button)
                        .aria_label(tr!("显示隐藏文件"))
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme::color(ui::FOREGROUND)))
                        .child(tr!("已隐藏 {} 个隐藏文件", hidden))
                        .child(div().text_color(theme::color(ui::PRIMARY)).child(crate::keymap::with_shortcut(cx, tr!("显示"), "files-toggle-hidden")))
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.toggle_hidden_files(window, cx))),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    let focus = this.pane(kind).list_focus.clone();
                    window.focus(&focus, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.active_pane = Some(kind);
                    this.open_menu(kind, None, event.position, window, cx);
                }),
            )
            .when(loading, |l| {
                l.child(div().absolute().inset_0().rounded_b(px(8.)).flex().items_center().justify_center().bg(theme::alpha(ui::CARD, 0.7)).child(spinner(24.)))
            });

        let ring = |s: gpui::StyleRefinement| {
            s.border_color(theme::color(ui::PRIMARY)).shadow(vec![BoxShadow {
                color: theme::alpha(ui::PRIMARY, 0.3),
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(2.),
                inset: false,
            }])
        };
        let _ = window;
        div()
            .id(("pane", kind as usize))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .rounded(px(8.))
            .border_1()
            .border_color(theme::color(ui::BORDER))
            .bg(theme::color(ui::CARD))
            .shadow(vec![
                BoxShadow { color: theme::alpha(0x16181d, 0.06), offset: point(px(0.), px(1.)), blur_radius: px(2.), spread_radius: px(0.), inset: false },
                BoxShadow { color: theme::alpha(0x16181d, 0.06), offset: point(px(0.), px(8.)), blur_radius: px(24.), spread_radius: px(0.), inset: false },
            ])
            .when(!disabled, |p| {
                p.drag_over::<DragFiles>(move |s, drag, _, _| if drag.pane != kind { ring(s) } else { s })
                    .on_drop(cx.listener(move |this, _: &DragFiles, window, cx| this.drop_files(DropTarget { pane: kind, directory: None }, window, cx)))
                    .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<DragFiles>, _, cx| {
                        this.drag_over_pane(kind, event.bounds.contains(&event.event.position), cx);
                    }))
            })
            .when(remote && !disabled, |p| {
                p.drag_over::<ExternalPaths>(move |s, _, _, _| ring(s))
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| this.drop_external(paths, None, window, cx)))
            })
            .child(header)
            .child(list)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Columns, NAME_MIN_WIDTH, PaneKind, columns_for};

    #[test]
    fn narrow_panes_drop_columns_before_the_name_disappears() {
        let all_remote = Columns { size: true, date: true, owner: true, permissions: true };
        assert_eq!(columns_for(PaneKind::Remote, None), all_remote, "还没量到宽度时全显示");
        assert_eq!(columns_for(PaneKind::Remote, Some(900.)), all_remote);
        assert_eq!(columns_for(PaneKind::Local, Some(900.)), Columns { size: true, date: true, owner: false, permissions: false });
        // 窗口 1000pt 宽时远程栏约 378：先藏远程才有的所有者、权限
        assert_eq!(columns_for(PaneKind::Remote, Some(378.)), Columns { size: true, date: true, owner: false, permissions: false });
        assert_eq!(columns_for(PaneKind::Remote, Some(440.)), Columns { size: true, date: true, owner: false, permissions: true });
        // 本地栏最窄 220（减边框 218）：只剩名称
        assert_eq!(columns_for(PaneKind::Local, Some(218.)), Columns::default());
        for kind in [PaneKind::Local, PaneKind::Remote] {
            for width in (100..1200).step_by(7) {
                let width = width as f32;
                let columns = columns_for(kind, Some(width));
                assert!(columns == Columns::default() || columns.width() + NAME_MIN_WIDTH <= width, "{kind:?} {width}: {columns:?}");
            }
        }
    }
}
