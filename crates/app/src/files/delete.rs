//! 删除：确认文案（旧 delete-prompt.ts）和远程删除命令（旧 remote-delete.ts）。

/// 远程删除经 ssh exec 交给远端 shell：路径必须单引号包住（引号内不做任何展开），
/// 内嵌单引号用 '\'' 闭合-转义-重开，与 core 的 shell_quote 一致。
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// RFC 4253 要求实现至少能处理 32768 字节的包；exec 命令按此分批，全选大目录时不至于被服务端拒收。
const MAX_COMMAND_LENGTH: usize = 30000;

/// 多个路径合进尽量少的 `rm -rf --` 命令：删 N 项只开一个 exec 通道，而不是 N 个。
pub fn remote_delete_commands(paths: &[String]) -> Vec<String> {
    let mut commands = Vec::new();
    let mut current = String::new();
    for path in paths {
        let arg = format!(" {}", shell_quote(path));
        if !current.is_empty() && current.len() + arg.len() > MAX_COMMAND_LENGTH {
            commands.push(std::mem::take(&mut current));
        }
        if current.is_empty() {
            current.push_str("rm -rf --");
        }
        current.push_str(&arg);
    }
    if !current.is_empty() {
        commands.push(current);
    }
    commands
}

/// 多选（⌘A 一次可能上百项）只列前几个名字，放在单行 detail 里（超出省略），数量写在标题上。
const PREVIEW_COUNT: usize = 10;

#[derive(Debug, PartialEq, Eq)]
pub struct DeletePrompt {
    pub title: String,
    pub message: String,
    pub detail: Option<String>,
    pub confirm_label: String,
}

fn name_of(path: &str) -> &str {
    path.split('/').rfind(|s| !s.is_empty()).unwrap_or(path)
}

/// `items`：(路径, 是否目录)。
pub fn delete_prompt(items: &[(String, bool)]) -> DeletePrompt {
    if let [(path, is_directory)] = items {
        return DeletePrompt {
            title: if *is_directory { "删除目录" } else { "删除文件" }.into(),
            message: format!("确定要删除「{}」{}吗？\n此操作不可恢复。", name_of(path), if *is_directory { "及其所有内容" } else { "" }),
            detail: None,
            confirm_label: "删除".into(),
        };
    }
    DeletePrompt {
        title: format!("删除 {} 个项目", items.len()),
        message: "此操作不可恢复。".into(),
        detail: Some(items.iter().take(PREVIEW_COUNT).map(|(path, _)| name_of(path)).collect::<Vec<_>>().join("、")),
        confirm_label: "删除".into(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;

    /// 每次都是新目录：并行的测试同一微秒里建目录时（macOS 的时钟只到微秒）别撞到一起、互删文件。
    fn temp_dir(tag: &str) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("sshl-{tag}-{}-{n}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sh(command: &str, cwd: &Path) {
        let status = Command::new("sh").arg("-c").arg(command).current_dir(cwd).status().unwrap();
        assert!(status.success(), "{command}");
    }

    fn path(dir: &Path, name: &str) -> String {
        dir.join(name).to_string_lossy().into_owned()
    }

    #[test]
    fn shell_metacharacters_delete_exactly_that_file() {
        let dir = temp_dir("rm");
        for name in ["a.txt", "a$b.txt", "it's", "q\"uote", "tick`date`"] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        let targets: Vec<String> = ["a$b.txt", "it's", "q\"uote", "tick`date`"].iter().map(|n| path(&dir, n)).collect();
        for command in remote_delete_commands(&targets) {
            sh(&command, &dir);
        }
        for target in &targets {
            assert!(!Path::new(target).exists(), "{target} should be deleted");
        }
        // 双引号拼接时 $b 展开成空，删掉的会是 a.txt
        assert!(dir.join("a.txt").exists(), "a.txt must survive");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn command_substitution_in_a_name_is_not_executed() {
        let cwd = temp_dir("cwd");
        let dir = temp_dir("rm");
        std::fs::write(dir.join("$(touch injected)"), "x").unwrap();
        for command in remote_delete_commands(&[path(&dir, "$(touch injected)")]) {
            sh(&command, &cwd);
        }
        assert!(!cwd.join("injected").exists(), "injected command ran");
        assert!(!dir.join("$(touch injected)").exists());
        assert!(dir.exists(), "the parent directory must not be deleted");
        std::fs::remove_dir_all(&cwd).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn directories_recurse_and_dash_names_are_not_options() {
        let dir = temp_dir("rm");
        std::fs::create_dir_all(dir.join("sub/deep")).unwrap();
        std::fs::write(dir.join("sub/deep/f"), "x").unwrap();
        std::fs::write(dir.join("-rf"), "x").unwrap();
        for command in remote_delete_commands(&[path(&dir, "sub")]) {
            sh(&command, &dir);
        }
        for command in remote_delete_commands(&["-rf".to_string()]) {
            sh(&command, &dir);
        }
        assert!(!dir.join("sub").exists());
        assert!(!dir.join("-rf").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn many_paths_share_one_command_split_before_the_packet_limit() {
        assert!(remote_delete_commands(&[]).is_empty());
        assert_eq!(remote_delete_commands(&["/a".into(), "/b".into(), "/c".into()]).len(), 1);
        let paths: Vec<String> = (0..2000).map(|i| format!("/srv/data/{}-{i}", "x".repeat(40))).collect();
        let commands = remote_delete_commands(&paths);
        assert!(commands.len() > 1);
        for command in &commands {
            assert!(command.starts_with("rm -rf -- "));
            assert!(command.len() <= 30000, "command too long: {}", command.len());
        }
        let quoted: Vec<String> = commands.iter().flat_map(|c| c["rm -rf -- ".len()..].split(' ').map(String::from).collect::<Vec<_>>()).collect();
        assert_eq!(quoted, paths.iter().map(|p| format!("'{p}'")).collect::<Vec<_>>());
    }

    #[test]
    fn single_and_multiple_delete_prompts() {
        let file = delete_prompt(&[("/srv/a.txt".into(), false)]);
        assert_eq!(file.title, "删除文件");
        assert_eq!(file.message, "确定要删除「a.txt」吗？\n此操作不可恢复。");
        let dir = delete_prompt(&[("/srv/logs".into(), true)]);
        assert_eq!(dir.title, "删除目录");
        assert!(dir.message.contains("「logs」及其所有内容"));
        let items: Vec<(String, bool)> = (0..12).map(|i| (format!("/srv/f{i}"), false)).collect();
        let many = delete_prompt(&items);
        assert_eq!(many.title, "删除 12 个项目");
        assert_eq!(many.detail.as_deref(), Some("f0、f1、f2、f3、f4、f5、f6、f7、f8、f9"));
        assert_eq!(many.message, "此操作不可恢复。");
    }
}
