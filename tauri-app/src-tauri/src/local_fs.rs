use serde::Serialize;
use std::path::PathBuf;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalEntry {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub size: u64,
    #[serde(rename = "modifyTime")]
    pub modify_time: Option<u64>,
}

#[tauri::command]
pub async fn file_get_home_dir() -> AppResult<String> {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| AppError::Other("home dir not found".into()))
}

#[tauri::command]
pub async fn file_list_local(directory: String) -> AppResult<Vec<LocalEntry>> {
    let dir = PathBuf::from(&directory);
    let mut out = Vec::new();
    let mut rd = tokio::fs::read_dir(&dir).await?;
    while let Some(entry) = rd.next_entry().await? {
        let meta = entry.metadata().await?;
        let path = entry.path();
        let modify_time = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64);
        out.push(LocalEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            path: path.to_string_lossy().to_string(),
            is_directory: meta.is_dir(),
            size: meta.len(),
            modify_time,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn file_delete_local(file_path: String) -> AppResult<()> {
    tokio::fs::remove_file(&file_path).await?;
    Ok(())
}

#[tauri::command]
pub async fn file_delete_local_directory(dir_path: String) -> AppResult<()> {
    tokio::fs::remove_dir_all(&dir_path).await?;
    Ok(())
}
