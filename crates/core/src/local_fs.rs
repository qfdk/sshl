use serde::Serialize;
use std::path::{Path, PathBuf};

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

pub async fn file_get_home_dir() -> AppResult<String> {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| AppError::Other("home dir not found".into()))
}

/// 整个目录在一个阻塞任务里用 std::fs 列完：tokio::fs 的每条 `metadata()` 都各起一次 spawn_blocking，
/// 5000 项的目录要 ~40ms，这样 ~12ms。条目元数据不跟随符号链接（lstat）。
pub async fn file_list_local(directory: String) -> AppResult<Vec<LocalEntry>> {
    tokio::task::spawn_blocking(move || list_local(&PathBuf::from(directory)))
        .await
        .map_err(|e| AppError::Other(format!("list local directory: {e}")))?
}

fn list_local(dir: &Path) -> AppResult<Vec<LocalEntry>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
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

pub async fn file_delete_local(file_path: String) -> AppResult<()> {
    tokio::fs::remove_file(&file_path).await?;
    Ok(())
}

pub async fn file_delete_local_directory(dir_path: String) -> AppResult<()> {
    tokio::fs::remove_dir_all(&dir_path).await?;
    Ok(())
}

/// 本地路径各是不是目录（从 Finder 拖进来的只有路径）。读不到的按文件处理，上传时再报错。
pub async fn file_local_kinds(paths: Vec<String>) -> Vec<bool> {
    let mut kinds = Vec::with_capacity(paths.len());
    for path in paths {
        kinds.push(tokio::fs::metadata(&path).await.map(|m| m.is_dir()).unwrap_or(false));
    }
    kinds
}

/// 本地路径是否已存在：None = 不存在，Some(是不是目录)。跟随符号链接（指向目录的算目录），坏链接算已存在的文件。
pub fn local_exists(path: &str) -> Option<bool> {
    match std::fs::metadata(path) {
        Ok(meta) => Some(meta.is_dir()),
        Err(_) => std::fs::symlink_metadata(path).ok().map(|_| false),
    }
}

#[cfg(test)]
mod tests {
    use super::{file_list_local, local_exists};

    #[test]
    fn existence_follows_links_and_counts_broken_ones() {
        let root = std::env::temp_dir().join(format!("sshl-exists-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("dir")).unwrap();
        std::fs::write(root.join("f.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(root.join("dir"), root.join("to-dir")).unwrap();
        std::os::unix::fs::symlink(root.join("gone"), root.join("broken")).unwrap();
        let at = |name: &str| local_exists(&root.join(name).to_string_lossy());
        assert_eq!((at("dir"), at("f.txt"), at("to-dir"), at("broken"), at("missing")), (Some(true), Some(false), Some(true), Some(false), None));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn lists_entries_without_following_symlinks() {
        let root = std::env::temp_dir().join(format!("sshl-local-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.txt"), b"abc").unwrap();
        std::os::unix::fs::symlink(root.join("sub"), root.join("link-dir")).unwrap();
        std::os::unix::fs::symlink("a.txt", root.join("link-file")).unwrap();

        let listed = file_list_local(root.to_string_lossy().into_owned()).await;
        std::fs::remove_dir_all(&root).unwrap();
        let mut listed = listed.unwrap();
        listed.sort_by(|a, b| a.name.cmp(&b.name));

        let names: Vec<&str> = listed.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.txt", "link-dir", "link-file", "sub"]);
        for e in &listed {
            assert_eq!(e.path, root.join(&e.name).to_string_lossy());
            assert!(e.modify_time.is_some(), "{} 缺修改时间", e.name);
        }
        let kind = |i: usize| (listed[i].is_directory, listed[i].size);
        assert_eq!(kind(0), (false, 3));
        assert!(listed[3].is_directory);
        // 符号链接取链接本身（lstat）：指向目录的不算目录，大小是链接内容（目标路径）的长度。
        assert!(!listed[1].is_directory);
        assert_eq!(listed[1].size, root.join("sub").to_string_lossy().len() as u64);
        assert_eq!(kind(2), (false, "a.txt".len() as u64));

        assert!(file_list_local(root.to_string_lossy().into_owned()).await.is_err(), "目录不存在要报错");
    }
}
