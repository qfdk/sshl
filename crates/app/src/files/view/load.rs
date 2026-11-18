//! 列目录：两栏的列表（带缓存）、刷新、主目录、进目录。

use super::*;

impl FileManager {
    pub(super) fn load_local_initial(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.local.path.is_empty() {
            self.load(PaneKind::Local, None, false, window, cx);
            return;
        }
        let home = backend::spawn(local_fs::file_get_home_dir());
        cx.spawn_in(window, async move |this, cx| {
            if let Some(Ok(home)) = home.await {
                let _ = this.update_in(cx, |this, window, cx| this.load(PaneKind::Local, Some(home), false, window, cx));
            }
        })
        .detach();
    }

    pub(super) fn load_remote_initial(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session_id) = self.session_id() else { return };
        if !self.remote.path.is_empty() {
            self.load(PaneKind::Remote, None, false, window, cx);
            return;
        }
        // 这个会话第一次打开文件管理：从终端所在目录开始，读不到就从远程 home 开始。期间切了会话，这次结果作废。
        self.remote.request += 1;
        let ticket = self.remote.request;
        self.remote.loading = true;
        let start = backend::spawn(async move {
            let state = &backend::get().state;
            let (cwd, home) = futures::join!(sftp::file_terminal_cwd(state, session_id.clone()), sftp::file_remote_home(state, session_id));
            (cwd.ok().flatten(), home.ok())
        });
        cx.spawn_in(window, async move |this, cx| {
            let (cwd, home) = start.await.unwrap_or_default();
            let home = remote_home_from(home.as_deref());
            let _ = this.update_in(cx, |this, window, cx| {
                if this.remote.request != ticket {
                    return;
                }
                // 第一次打开（或刚换了身份）：终端在哪就从哪开始，不管上次跟没跟过
                if let (Some(cwd), Some(record)) = (&cwd, this.session_id().and_then(|sid| this.records.get_mut(&sid))) {
                    record.terminal_cwd = Some(cwd.clone());
                }
                let path = cwd.unwrap_or(if home.is_empty() { "/".to_string() } else { home });
                this.load(PaneKind::Remote, Some(path), false, window, cx);
            });
        })
        .detach();
    }

    pub(super) fn load(&mut self, kind: PaneKind, requested: Option<String>, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mut path = requested.filter(|p| !p.is_empty()).unwrap_or_else(|| self.pane(kind).path.clone());
        let session_id = self.session_id();
        if kind == PaneKind::Remote {
            let Some(session_id) = &session_id else {
                self.remote.files.clear();
                self.remote_empty = Some(RemoteEmpty::NotConnected);
                self.set_path(PaneKind::Remote, String::new(), window, cx);
                cx.notify();
                return;
            };
            if path.is_empty() {
                path = self.records.get(session_id).map(|r| r.remote_path.clone()).unwrap_or_default();
            }
            path = normalize_remote_path(if path.is_empty() { "/" } else { &path });
            if let Some(record) = self.records.get_mut(session_id) {
                record.remote_path = path.clone();
            }
            self.remote_empty = None;
        } else if path.is_empty() {
            return;
        }
        self.set_path(kind, path.clone(), window, cx);

        let key = self.cache_key(kind, &path);
        let cached = if force {
            None
        } else {
            match kind {
                PaneKind::Local => self.local_cache.get(&key),
                PaneKind::Remote => self.remote_cache.get(&key),
            }
        };
        let show_hidden = crate::settings::get(cx).show_hidden_files;
        let pane = self.pane_mut(kind);
        pane.request += 1;
        let request = pane.request;
        if let Some(cached) = &cached {
            (pane.files, pane.hidden) = visible_files(cached.clone(), show_hidden);
        }
        pane.loading = cached.is_none();
        pane.error = None;
        self.after_files_changed(kind);
        cx.notify();

        let listing = list_directory(kind, session_id.clone(), path.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = listing.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.pane(kind).request != request {
                    return;
                }
                match result {
                    Ok(files) => {
                        this.pane_mut(kind).loading = false;
                        match kind {
                            PaneKind::Local => this.local_cache.set(key, files.clone()),
                            PaneKind::Remote => this.remote_cache.set(key, files.clone()),
                        }
                        let current = session_id.as_ref().and_then(|id| this.records.get(id)).map(|r| r.remote_path.as_str());
                        if kind == PaneKind::Remote && current != Some(path.as_str()) {
                            return;
                        }
                        let show_hidden = crate::settings::get(cx).show_hidden_files;
                        let pane = this.pane_mut(kind);
                        (pane.files, pane.hidden) = visible_files(files, show_hidden);
                        pane.loading = false;
                        this.after_files_changed(kind);
                    }
                    Err(error) => {
                        this.pane_mut(kind).error = Some(error);
                        this.pane_mut(kind).loading = false;
                        // 远程目录打不开（没权限、被删了）就退到上一级
                        if kind == PaneKind::Remote && path != "/" {
                            let parent = parent_path(&path, kind);
                            if parent != path {
                                this.load(kind, Some(parent), force, window, cx);
                            }
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 文件列表换了：进入新目录时回到顶部、清掉两侧选择（同旧版按 path 清选择）。
    pub(super) fn after_files_changed(&mut self, kind: PaneKind) {
        let pane = self.pane_mut(kind);
        if pane.listed_path != pane.path {
            pane.listed_path = pane.path.clone();
            pane.rows.reset();
            self.local.selected.clear();
            self.remote.selected.clear();
            self.last_selected = None;
        }
    }

    pub(super) fn refresh(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) {
        match kind {
            PaneKind::Local => self.load(PaneKind::Local, None, true, window, cx),
            PaneKind::Remote => {
                if self.session.is_none() {
                    return;
                }
                let path = if self.remote.path.is_empty() { "/".to_string() } else { self.remote.path.clone() };
                self.load(PaneKind::Remote, Some(path), true, window, cx);
                self.sync_identity(false, window, cx);
            }
        }
    }

    pub(super) fn go_home(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) {
        match kind {
            PaneKind::Local => {
                let home = backend::spawn(local_fs::file_get_home_dir());
                cx.spawn_in(window, async move |this, cx| {
                    if let Some(Ok(home)) = home.await {
                        let _ = this.update_in(cx, |this, window, cx| this.load(PaneKind::Local, Some(home), true, window, cx));
                    }
                })
                .detach();
            }
            PaneKind::Remote => {
                let Some(session_id) = self.session_id() else { return };
                let home = backend::spawn(sftp::file_remote_home(&backend::get().state, session_id));
                cx.spawn_in(window, async move |this, cx| {
                    let home = remote_home_from(home.await.and_then(Result::ok).as_deref());
                    if !home.is_empty() {
                        let _ = this.update_in(cx, |this, window, cx| this.load(PaneKind::Remote, Some(home), true, window, cx));
                    }
                })
                .detach();
            }
        }
    }

    pub(super) fn browse_local(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.first()
            {
                let path = path.to_string_lossy().into_owned();
                let _ = this.update_in(cx, |this, window, cx| this.load(PaneKind::Local, Some(path), true, window, cx));
            }
        })
        .detach();
    }

    pub(super) fn open_entry(&mut self, kind: PaneKind, entry: &FileEntry, window: &mut Window, cx: &mut Context<Self>) {
        let path = if entry.is_parent() { parent_path(&self.pane(kind).path, kind) } else { entry.path.clone() };
        self.load(kind, Some(path), true, window, cx);
    }

    /// 只重新列当前目录（不同步身份）。
    pub(super) fn refresh_list(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.pane(kind).path.clone();
        let path = if kind == PaneKind::Remote && path.is_empty() { "/".to_string() } else { path };
        self.load(kind, Some(path), true, window, cx);
    }
}

/// 列目录（不改任何状态）：远程走 SFTP，本地走 std::fs。
pub(super) fn list_directory(kind: PaneKind, session_id: Option<String>, path: String) -> impl std::future::Future<Output = Result<Vec<FileEntry>, String>> {
    let listing = backend::spawn(async move {
        match kind {
            PaneKind::Remote => {
                let Some(session_id) = session_id else { return Err(tr!("请先连接到服务器").to_string()) };
                sftp::file_list(&backend::get().state, session_id, path)
                    .await
                    .map(|files| files.into_iter().map(FileEntry::from).collect())
                    .map_err(|e| e.to_string())
            }
            PaneKind::Local => local_fs::file_list_local(path)
                .await
                .map(|files| files.into_iter().map(FileEntry::from).collect())
                .map_err(|e| e.to_string()),
        }
    });
    async move { listing.await.unwrap_or_else(|| Err(tr!("获取文件列表失败").into())) }
}
