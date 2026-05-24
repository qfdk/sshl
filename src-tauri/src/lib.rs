mod config_store;
mod error;
mod local_fs;
mod sftp;
mod ssh;
mod state;

use config_store::ConfigStore;
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
