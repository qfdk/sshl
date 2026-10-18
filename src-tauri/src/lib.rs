mod config_store;
mod crypto_store;
mod db;
mod error;
mod local_fs;
mod sftp;
mod ssh;
mod state;
mod system_fonts;

use std::time::Duration;

use futures::StreamExt;
use tauri::Manager;

use config_store::{load_secret, ConfigStore};
use state::AppState;

#[cfg(target_os = "macos")]
fn disable_press_and_hold() {
    use objc2_foundation::{NSString, NSUserDefaults};

    // 1) 持久化到 app domain plist —— WKWebView 的 WebContent 子进程通过 plist 继承。
    let _ = std::process::Command::new("defaults")
        .args(["write", "me.qfdk.sshl", "ApplePressAndHoldEnabled", "-bool", "false"])
        .status();

    // 2) 立即写入当前进程 standardUserDefaults
    unsafe {
        let defaults = NSUserDefaults::standardUserDefaults();
        let key = NSString::from_str("ApplePressAndHoldEnabled");
        defaults.setBool_forKey(false, &key);
    }
}

#[cfg(not(target_os = "macos"))]
fn disable_press_and_hold() {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    disable_press_and_hold();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sshl=info,warn".into()),
        )
        .try_init()
        .ok();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .manage(ConfigStore::new())
        .invoke_handler(tauri::generate_handler![
            // ssh
            ssh::ssh_connect,
            ssh::ssh_cancel_connect,
            ssh::ssh_validate_session,
            ssh::ssh_disconnect,
            ssh::ssh_send_data,
            ssh::ssh_fill_password,
            ssh::ssh_resize,
            ssh::ssh_execute,
            ssh::ssh_refresh_prompt,
            ssh::ssh_activate_session,
            ssh::ssh_get_session_buffer,
            ssh::ssh_prewarm,
            // sftp
            sftp::file_list,
            sftp::file_upload,
            sftp::file_download,
            sftp::file_create_remote_directory,
            sftp::file_upload_directory,
            sftp::file_download_directory,
            sftp::file_change_permissions,
            sftp::file_change_owner,
            // local fs
            local_fs::file_get_home_dir,
            local_fs::file_list_local,
            local_fs::file_delete_local,
            local_fs::file_delete_local_directory,
            // config
            config_store::config_get_connections,
            config_store::config_get_connection_groups,
            config_store::config_save_connection,
            config_store::config_delete_connection,
            config_store::config_apply_connection_layout,
            config_store::cred_list,
            config_store::cred_set,
            config_store::cred_delete,
            system_fonts::list_system_fonts,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            // Background: prewarm saved connections + evict idle warm entries.
            // Mirrors main.js app.whenReady() in the Electron build.
            tauri::async_runtime::spawn(async move {
                prewarm_saved_connections(&handle).await;
                let state = handle.state::<AppState>();
                loop {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    let evicted = state.warm_evict_expired(Duration::from_secs(600)).await;
                    for warm in evicted {
                        let h = warm.handle.lock().await;
                        let _ = h
                            .disconnect(russh::Disconnect::ByApplication, "warm pool expired", "en")
                            .await;
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 启动预热的并发握手数。串行时一台不可达主机（connect_tcp 4 次重试 ≈ 22s）
/// 会拖住排在它后面的所有主机。
const PREWARM_CONCURRENCY: usize = 8;

async fn prewarm_saved_connections(app: &tauri::AppHandle) {
    let store = app.state::<ConfigStore>();
    let conns = match store.read_all().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("prewarm: failed to read connections: {e}");
            return;
        }
    };
    let state = app.state::<AppState>();
    let mut seen = std::collections::HashSet::new();
    let mut targets = Vec::new();
    for c in conns {
        // 不依赖 has_password / has_passphrase 标志（重连 re-save 曾把它误置为 false）；
        // secrets 库才是凭据真相来源，无条件按 id 读取。
        let password = load_secret(&c.id, "password");
        let passphrase = load_secret(&c.id, "passphrase");
        if password.is_none() && c.private_key.is_none() {
            continue;
        }
        let key = format!("{}:{}:{}", c.host, c.port, c.username);
        // 同一 host:port:user 保存了多份时只握手一次（并发下无法再靠 warm_contains 去重）。
        if !seen.insert(key.clone()) || state.warm_contains(&key).await {
            continue;
        }
        targets.push((key, c, password, passphrase));
    }

    futures::stream::iter(targets)
        .for_each_concurrent(PREWARM_CONCURRENCY, |(key, c, password, passphrase)| {
            let state = &state;
            async move {
                match ssh::do_handshake(
                    &c.host,
                    c.port,
                    &c.username,
                    password.as_deref(),
                    c.private_key.as_deref(),
                    passphrase.as_deref(),
                )
                .await
                {
                    Ok(handle) => {
                        state
                            .warm_insert(
                                key,
                                state::WarmConn {
                                    handle,
                                    created_at: std::time::Instant::now(),
                                },
                            )
                            .await;
                    }
                    Err(e) => {
                        tracing::warn!("prewarm: {key} failed: {e}");
                    }
                }
            }
        })
        .await;
}
