//! 后端主动通知界面的事件。回调在后端线程（tokio 工作线程）上调用，界面自己转回 UI 线程。

use std::sync::Arc;

use crate::sftp::{DownloadProgress, UploadProgress};

#[derive(Debug, Clone)]
pub enum CoreEvent {
    UploadProgress(UploadProgress),
    DownloadProgress(DownloadProgress),
    /// 连接列表或分组变了（保存、删除、拖动排序）。
    ConnectionsUpdated,
}

#[derive(Clone)]
pub struct Events(Arc<dyn Fn(CoreEvent) + Send + Sync>);

impl Events {
    pub fn new(handler: impl Fn(CoreEvent) + Send + Sync + 'static) -> Self {
        Self(Arc::new(handler))
    }

    /// 不关心事件的调用方（测试）。
    pub fn none() -> Self {
        Self::new(|_| {})
    }

    pub fn emit(&self, event: CoreEvent) {
        (self.0)(event);
    }
}
