//! SSHL 后端：SSH 会话、SFTP 文件操作、连接与凭据存储。不依赖界面框架：
//! 终端字节走 [`ssh::TerminalSink`]，传输进度与连接列表变化走 [`events::Events`]。

pub mod auto_auth;
pub mod config_store;
pub mod crypto_store;
pub mod db;
pub mod error;
pub mod events;
pub mod i18n;
pub mod known_hosts;
pub mod local_fs;
pub mod local_shell;
pub mod server_info;
pub mod services;
pub mod sftp;
pub mod sftp_stall;
pub mod ssh;
pub mod ssh_config;
pub mod state;
pub mod system_fonts;
pub mod terminal_user;
pub mod warm;
