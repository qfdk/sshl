//! 对选中的文件 / 目录做的事：删除（确认文案和远程命令在 `files::delete`）。

use super::*;

impl FileManager {
    pub(super) fn delete_items(&mut self, kind: PaneKind, items: Vec<FileEntry>, window: &mut Window, cx: &mut Context<Self>) {
        if items.is_empty() {
            return;
        }
        let prompt = delete_prompt(&items.iter().map(|i| (i.path.clone(), i.is_directory)).collect::<Vec<_>>());
        let mut options = Options::new(prompt.title).message(prompt.message).confirm(prompt.confirm_label);
        if let Some(detail) = prompt.detail {
            options = options.detail(detail);
        }
        let confirmed = prompts::confirm(window, cx, options);
        let session_id = self.session_id();
        cx.spawn_in(window, async move |this, cx| {
            if !confirmed.await.unwrap_or(false) {
                return;
            }
            match kind {
                PaneKind::Remote => {
                    let Some(session_id) = session_id else { return };
                    let paths: Vec<String> = items.iter().map(|i| i.path.clone()).collect();
                    for command in remote_delete_commands(&paths) {
                        // 以文件管理当前身份删除（终端切成 root 后就以 root 删）
                        let sid = session_id.clone();
                        let result = backend::spawn(async move { sftp::file_shell(&backend::get().state, sid, command).await }).await;
                        if let Outcome::Failed(error) = Outcome::from_result(result) {
                            alert(cx, tr!("删除失败"), error);
                        }
                    }
                }
                PaneKind::Local => {
                    for item in items {
                        let path = item.path.clone();
                        let result = if item.is_directory {
                            backend::spawn(local_fs::file_delete_local_directory(path)).await
                        } else {
                            backend::spawn(local_fs::file_delete_local(path)).await
                        };
                        if let Outcome::Failed(error) = Outcome::from_result(result) {
                            alert(cx, tr!("删除失败"), error);
                        }
                    }
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.invalidate(kind);
                this.refresh_list(kind, window, cx);
            });
        })
        .detach();
    }
}
