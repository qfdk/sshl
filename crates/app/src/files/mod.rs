//! 文件管理（SFTP）：左本地、右远程两栏，互拖即上传 / 下载，Finder 拖进远程即上传。

mod completion;
mod delete;
mod drag;
mod identity;
mod model;
mod progress;
mod sudo;
mod view;

pub use view::{FileManager, FileManagerEvent, FileSession};
