mod config_store;
mod crypto_store;
mod error;
mod local_fs;
mod sftp;
mod ssh;
mod state;

use std::time::Duration;

use tauri::Manager;

use config_store::{load_secret, ConfigStore};
use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            ssh::ssh_disconnect,
            ssh::ssh_send_data,
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
            config_store::config_save_connection,
            config_store::config_delete_connection,
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
    for c in conns {
        let password = if c.has_password { load_secret(&c.id, "password") } else { None };
        let passphrase = if c.has_passphrase { load_secret(&c.id, "passphrase") } else { None };
        if password.is_none() && c.private_key.is_none() {
            continue;
        }
        let key = format!("{}:{}:{}", c.host, c.port, c.username);
        if state.warm_contains(&key).await {
            continue;
        }
        let started = std::time::Instant::now();
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
                        key.clone(),
                        state::WarmConn {
                            handle,
                            created_at: std::time::Instant::now(),
                        },
                    )
                    .await;
                tracing::info!(
                    "prewarm: {key} ready in {}ms",
                    started.elapsed().as_millis()
                );
            }
            Err(e) => {
                tracing::warn!("prewarm: {key} failed: {e}");
            }
        }
    }
}
