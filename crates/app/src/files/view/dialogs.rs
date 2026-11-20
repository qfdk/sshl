//! 新建文件夹、修改权限两个对话框。

use gpui::KeyDownEvent;
use gpui_base::FocusTrapElement;

use super::*;

pub(super) struct PermissionDialog {
    /// 整个框：Tab 出不去的范围。
    focus: FocusHandle,
    path: String,
    value: String,
    checks: [bool; 9],
    input: Entity<InputState>,
    _subscription: Subscription,
}

pub(super) struct NewFolderDialog {
    focus: FocusHandle,
    input: Entity<InputState>,
    _subscription: Subscription,
}

impl FileManager {
    pub(super) fn open_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(tr!("请输入文件夹名称")));
        let focus = input.read(cx).focus_handle(cx);
        let subscription = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.create_folder(window, cx);
            }
        });
        self.new_folder = Some(NewFolderDialog { focus: cx.focus_handle(), input, _subscription: subscription });
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(super) fn create_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.new_folder.take() else { return };
        let name = dialog.input.read(cx).value().trim().to_string();
        window.focus(&self.focus, cx);
        cx.notify();
        if name.is_empty() {
            return;
        }
        if name.contains('/') || name.contains('\\') {
            prompts::alert_later(window, cx, Options::new(tr!("无法新建文件夹")).message(tr!("文件夹名称不能包含斜杠。")));
            return;
        }
        let Some(session_id) = self.session_id() else { return };
        let full = if self.remote.path == "/" { format!("/{name}") } else { format!("{}/{name}", self.remote.path) };
        let created = backend::spawn(async move { sftp::file_create_remote_directory(&backend::get().state, session_id, full).await });
        cx.spawn_in(window, async move |this, cx| match Outcome::from_result(created.await) {
            Outcome::Failed(error) => alert(cx, tr!("新建文件夹失败"), error),
            _ => {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.invalidate(PaneKind::Remote);
                    this.refresh_list(PaneKind::Remote, window, cx);
                });
            }
        })
        .detach();
    }

    pub(super) fn open_permissions(&mut self, entry: &FileEntry, window: &mut Window, cx: &mut Context<Self>) {
        let value = format_permissions(entry.permissions);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value.clone()));
        let subscription = cx.subscribe_in(&input, window, |this, input, event: &InputEvent, _, cx| {
            if let (InputEvent::Change, Some(dialog)) = (event, this.permission.as_mut()) {
                dialog.value = input.read(cx).value().trim().chars().take(3).collect();
                dialog.checks = permissions_to_checks(&dialog.value);
                cx.notify();
            }
        });
        // 焦点留在后面的文件列表上的话，回车会给选中的那个文件改名。全选：直接敲新的权限就换掉
        input.update(cx, |input, cx| {
            input.select_all(window, cx);
            input.focus(window, cx);
        });
        self.permission = Some(PermissionDialog {
            focus: cx.focus_handle(),
            path: entry.path.clone(),
            checks: permissions_to_checks(&value),
            value,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn close_permissions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.permission = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn close_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_folder = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn set_permission_value(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.permission.as_mut() else { return };
        dialog.checks = permissions_to_checks(&value);
        dialog.value = value.clone();
        dialog.input.update(cx, |input, cx| input.set_value(value, window, cx));
        cx.notify();
    }

    pub(super) fn toggle_permission(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.permission.as_mut() else { return };
        let mut checks = dialog.checks;
        checks[index] = !checks[index];
        let value = checks_to_permissions(&checks);
        self.set_permission_value(value, window, cx);
    }

    pub(super) fn apply_permissions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = &self.permission else { return };
        let Some(session_id) = self.session_id() else { return };
        if !valid_octal(&dialog.value) {
            return;
        }
        let mode = u32::from_str_radix(&dialog.value, 8).unwrap_or(0o755);
        let path = dialog.path.clone();
        let changed = backend::spawn(async move { sftp::file_change_permissions(&backend::get().state, session_id, path, mode).await });
        cx.spawn_in(window, async move |this, cx| match Outcome::from_result(changed.await) {
            Outcome::Failed(error) => alert(cx, tr!("修改权限失败"), error),
            _ => {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.close_permissions(window, cx);
                    this.invalidate(PaneKind::Remote);
                    this.refresh_list(PaneKind::Remote, window, cx);
                });
            }
        })
        .detach();
    }

    pub(super) fn render_permission_dialog(&self, dialog: &PermissionDialog, cx: &mut Context<Self>) -> AnyElement {
        let valid = valid_octal(&dialog.value);
        let groups = [tr!("所有者"), tr!("组"), tr!("其他用户")];
        let bits = [tr!("读取"), tr!("写入"), tr!("执行")];
        let body = div()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_permissions(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .min_w_0()
                            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(tr!("修改文件权限")))
                            .child(div().text_size(px(14.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child(dialog.path.clone())),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(tr!("八进制权限")))
                            .child(div().w(px(80.)).font_family(MONO_FONT).child(kit::text_field(&dialog.input, false)))
                            .child(div().font_family(MONO_FONT).text_size(px(14.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child(format!("= {}", readable_permissions(&dialog.value)))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(div().mr_2().text_size(px(14.)).text_color(theme::color(ui::MUTED_FOREGROUND)).child(tr!("常用权限:")))
                            .children(["755", "644", "777", "600"].into_iter().enumerate().map(|(index, value)| {
                                kit::button(("perm-preset", index), Variant::Outline)
                                    .h(px(32.))
                                    .px_3()
                                    .aria_label(value)
                                    .child(value)
                                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.set_permission_value(value.to_string(), window, cx)))
                            })),
                    )
                    .children(groups.iter().enumerate().map(|(group, label)| {
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .pt_3()
                            .border_t_1()
                            .border_color(theme::color(ui::BORDER))
                            .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(*label))
                            .child(div().flex().gap_4().children(bits.iter().enumerate().map(|(bit, name)| {
                                let index = group * 3 + bit;
                                div()
                                    .id(("perm-bit", index))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_size(px(14.))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.toggle_permission(index, window, cx)))
                                    .child(kit::checkbox(("perm-check", index), dialog.checks[index]).aria_label(format!("{label}{name}")))
                                    .child(*name)
                            })))
                    })),
            )
            .child(
                kit::modal_footer()
                    .child(
                        kit::button("perm-cancel", Variant::Outline)
                            .aria_label(tr!("取消"))
                            .child(tr!("取消"))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.close_permissions(window, cx))),
                    )
                    .child(
                        kit::button("perm-apply", Variant::Primary)
                            .aria_label(tr!("应用"))
                            .child(tr!("应用"))
                            .when(!valid, |b| b.opacity(0.5).cursor_default())
                            .when(valid, |b| b.on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.apply_permissions(window, cx)))),
                    ),
            )
            .focus_trap("permission-trap", &dialog.focus);
        kit::modal("permission-dialog", 512., body).into_any_element()
    }

    pub(super) fn render_new_folder_dialog(&self, dialog: &NewFolderDialog, cx: &mut Context<Self>) -> AnyElement {
        let body = div()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_new_folder(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(tr!("新建文件夹")))
                    .child(kit::text_field(&dialog.input, true)),
            )
            .child(
                kit::modal_footer()
                    .child(
                        kit::button("folder-cancel", Variant::Outline)
                            .aria_label(tr!("取消"))
                            .child(tr!("取消"))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.close_new_folder(window, cx))),
                    )
                    .child(
                        kit::button("folder-create", Variant::Primary)
                            .aria_label(tr!("新建"))
                            .child(tr!("新建"))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.create_folder(window, cx))),
                    ),
            )
            .focus_trap("new-folder-trap", &dialog.focus);
        kit::modal("new-folder-dialog", 420., body).into_any_element()
    }

    pub(super) fn overlay(element: impl IntoElement, window: &Window) -> AnyElement {
        let size = window.viewport_size();
        deferred(anchored().position(Point::default()).child(div().relative().w(size.width).h(size.height).child(element))).with_priority(1).into_any_element()
    }
}
