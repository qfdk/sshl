//! 界面文字的英文译文（原文 → 译文），按文件分段。原文和代码里 `tr!` 的一个字都不能差：
//! 本文件底下的测试扫全部源码，核对每条 `tr!` / `trc!` 都有译文、参数个数一致、表里没有用不到的。
//! 同一个词在不同地方译法不同的，键写成「原文|场合」（代码里是 `trc!("场合", "原文")`）。
//! 写法：菜单栏按 macOS 习惯每个词首字母大写，应用里的按钮、标题只大写第一个词。

pub const TABLE: sshl_core::i18n::Table = &[
    // connection_dialog.rs
    ("不填就用主机地址", "Defaults to the host"),
    ("选择或输入新分组", "Pick or type a new group"),
    ("example.com 或 192.168.1.1", "example.com or 192.168.1.1"),
    ("密码", "Password"),
    ("私钥密码（可选）", "Passphrase (optional)"),
    ("请填写主机", "Enter a host"),
    ("请填写用户名", "Enter a username"),
    ("端口应为 1–65535", "Port must be 1–65535"),
    ("请选择私钥文件", "Choose a key file"),
    ("自动", "Auto"),
    ("自动选择私钥（免密）", "Pick a key automatically"),
    ("指定文件", "Key file"),
    ("指定私钥文件", "Use a specific key file"),
    ("新建分组「{}」", "New group “{}”"),
    ("分组", "Group"),
    ("（可选）", "(optional)"),
    ("编辑连接", "Edit connection"),
    ("新建连接", "New connection"),
    ("名称", "Name"),
    ("主机", "Host"),
    ("端口", "Port"),
    ("用户名", "Username"),
    ("认证方式", "Authentication"),
    ("私钥", "Private key"),
    ("依次尝试 SSH Agent 和 ~/.ssh 里的私钥，遵循 ~/.ssh/config", "Tries the SSH Agent, then the keys in ~/.ssh, following ~/.ssh/config"),
    ("浏览", "Browse"),
    ("保存私钥密码", "Save passphrase"),
    ("保存密码", "Save password"),
    ("从 ~/.ssh/config 导入", "Import from ~/.ssh/config"),
    ("取消", "Cancel"),
    ("保存", "Save"),
    ("连接", "Connect"),
    // favorites.rs
    ("搜索收藏的目录…", "Search favorite folders…"),
    ("改名失败", "Couldn't rename"),
    ("收藏失败", "Couldn't add the favorite"),
    ("取消收藏 {}", "Remove {} from favorites"),
    ("收藏当前目录 {}", "Add the current folder {} to favorites"),
    ("改名", "Rename"),
    ("删除这个收藏", "Remove this favorite"),
    ("还没有收藏的目录。进到常用的目录，点右上角的星星收藏。", "No favorite folders yet. Go to a folder you use often and click the star at the top right."),
    ("没有匹配的目录", "No matching folders"),
    ("目录收藏", "Favorite folders"),
    // files/delete.rs
    ("删除目录", "Delete folder"),
    ("删除文件", "Delete file"),
    ("确定要删除「{}」及其所有内容吗？\n此操作不可恢复。", "Delete “{}” and everything in it?\nThis can't be undone."),
    ("确定要删除「{}」吗？\n此操作不可恢复。", "Delete “{}”?\nThis can't be undone."),
    ("删除", "Delete"),
    ("删除 {} 个项目", "Delete {} items"),
    ("此操作不可恢复。", "This can't be undone."),
    // files/drag.rs
    ("{} 个项目", "{} items"),
    // files/progress.rs
    ("剩 {} 秒", "{} s left"),
    ("剩 {} 分钟", "{} min left"),
    ("剩 {} 小时 {} 分", "{} h {} min left"),
    ("正在上传 {} {}%", "Uploading {} {}%"),
    ("正在下载 {} {}%", "Downloading {} {}%"),
    ("{}/{} 个文件", "{}/{} files"),
    ("还有 {} 个", "{} more"),
    // files/sudo.rs
    ("后台任务异常退出", "The background task exited unexpectedly"),
    ("当前登录用户没有 sudo 权限。\n{}", "The login user isn't allowed to use sudo.\n{}"),
    ("sudo 密码不正确。\n{}", "Wrong sudo password.\n{}"),
    ("服务器的 sudo 要求终端（requiretty），无法在后台执行。\n{}", "sudo on this server needs a terminal (requiretty), so it can't run in the background.\n{}"),
    ("未知错误", "Unknown error"),
    ("连接密码", "Login password"),
    // files/view.rs
    ("上传到 {}", "Upload to {}"),
    ("下载到 {}", "Download to {}"),
    ("请先连接到服务器", "Connect to a server first"),
    ("连接已断开，请重新连接到服务器", "Disconnected. Reconnect to the server."),
    ("删除失败", "Couldn't delete"),
    ("请输入文件夹名称", "Folder name"),
    ("无法新建文件夹", "Can't create the folder"),
    ("文件夹名称不能包含斜杠。", "A folder name can't contain a slash."),
    ("新建文件夹失败", "Couldn't create the folder"),
    ("修改权限失败", "Couldn't change permissions"),
    ("上传失败", "Upload failed"),
    ("读不到终端当前目录，也拿不到远程主目录。", "Couldn't read the terminal's current folder or the remote home folder."),
    ("取消收藏当前目录", "Remove the current folder from favorites"),
    ("取消收藏", "Remove from favorites"),
    ("收藏当前目录", "Add the current folder to favorites"),
    ("收藏的目录", "Favorite folders"),
    ("{} 的密码不正确", "Wrong password for {}"),
    ("以 {} 身份管理文件", "Manage files as {}"),
    (
        "终端已切换到 {0}，文件管理也以 {0} 身份浏览、上传和下载，需要 {0} 的密码。",
        "The terminal switched to {0}. To browse, upload and download as {0} too, the file manager needs {0}'s password.",
    ),
    (
        "终端用 sudo 切换到了 {0}，文件管理也以 {0} 身份浏览、上传和下载，需要 {1} 的 sudo 密码。",
        "The terminal switched to {0} with sudo. To browse, upload and download as {0} too, the file manager needs {1}'s sudo password.",
    ),
    ("切换", "Switch"),
    ("无法以 {} 身份管理文件", "Can't manage files as {}"),
    ("刷新", "Refresh"),
    ("新建文件夹", "New folder"),
    ("选择文件夹上传", "Upload a folder…"),
    ("不显示隐藏文件", "Hide hidden files"),
    ("显示隐藏文件", "Show hidden files"),
    ("下载 {} 个项目", "Download {} items"),
    ("上传 {} 个项目", "Upload {} items"),
    ("下载文件夹", "Download folder"),
    ("下载文件", "Download file"),
    ("上传文件夹", "Upload folder"),
    ("上传文件", "Upload file"),
    ("修改权限", "Change permissions"),
    ("点击修改权限", "Click to change permissions"),
    ("文件管理以 {} 身份运行（跟随终端）", "The file manager runs as {} (following the terminal)"),
    ("终端当前是 {}，点击让文件管理也切过去", "The terminal is now {}. Click to switch the file manager too"),
    ("切到 {}", "Switch to {}"),
    ("重新连接", "Reconnect"),
    ("目录为空", "This folder is empty"),
    ("远程文件", "Remote"),
    ("本地文件", "Local"),
    ("子目录", "Subfolders"),
    ("转到路径", "Go to path"),
    ("选择目录", "Choose folder"),
    ("主目录", "Home"),
    ("大小", "Size"),
    ("修改日期", "Modified"),
    ("所有者", "Owner"),
    ("权限", "Mode"),
    ("已隐藏 {} 个隐藏文件", "{0} hidden {0?file|files} not shown"),
    ("显示", "Show"),
    ("停止传输", "Stop transfers"),
    ("组", "Group"),
    ("其他用户", "Others"),
    ("读取", "Read"),
    ("写入", "Write"),
    ("执行", "Execute"),
    ("修改文件权限", "Change permissions"),
    ("八进制权限", "Octal mode"),
    ("常用权限:", "Common:"),
    ("应用", "Apply"),
    ("新建", "Create"),
    ("拖动调整面板宽度", "Drag to resize the panes"),
    ("上传", "Upload"),
    ("下载", "Download"),
    ("上传|进行中", "Uploading"),
    ("上传文件夹|进行中", "Uploading folder"),
    ("下载|进行中", "Downloading"),
    ("下载文件夹|进行中", "Downloading folder"),
    ("覆盖会合并进原来的文件夹，同名的文件被替换。", "Replacing merges into the existing folder; files with the same name are replaced."),
    ("覆盖会替换原来的文件。", "Replacing overwrites the existing file."),
    ("{} 已存在", "{} already exists"),
    ("后面再遇到同名的也这样处理", "Do the same for the rest"),
    ("{} 个已上传到 {}", "{} items uploaded to {}"),
    ("{} 已上传到 {}", "{} uploaded to {}"),
    ("{} 个已下载到 {}", "{} items downloaded to {}"),
    ("{} 已下载到 {}", "{} downloaded to {}"),
    (" · 还有 {} 个", " · {} more"),
    ("正在{} {}{}", "{} {}{}"),
    ("覆盖", "Replace"),
    ("跳过", "Skip"),
    ("已跳过 {}", "Skipped {}"),
    ("已取消{}", "{} cancelled"),
    ("已停止{}", "{} stopped"),
    ("{}失败", "{} failed"),
    ("没有写入权限", "No write permission"),
    (
        "当前登录用户不能写入这个目录。要用 sudo 上传吗？文件会归目录的所有者。",
        "The login user can't write to this folder. Upload with sudo? The files will belong to the folder's owner.",
    ),
    ("用 sudo 上传", "Upload with sudo"),
    ("sudo 密码不正确", "Wrong sudo password"),
    ("需要 sudo 密码", "sudo password required"),
    ("sudo 要的是当前登录用户的密码。", "sudo asks for the login user's password."),
    ("sudo 要的是当前登录用户 {} 的密码。", "sudo asks for the password of the login user, {}."),
    ("输入密码", "Enter password"),
    ("或输入密码", "Or enter a password"),
    ("获取文件列表失败", "Couldn't list the files"),
    // groups.rs
    ("本机", "Local"),
    ("本机 {}", "Local {}"),
    // import_dialog.rs
    ("导入 {} 失败：{}", "Couldn't import {}: {}"),
    ("已存在", "Already added"),
    ("正在读取 ~/.ssh/config…", "Reading ~/.ssh/config…"),
    ("~/.ssh/config 里没有找到主机。", "No hosts found in ~/.ssh/config."),
    ("导入 {} 个", "Import {}"),
    ("导入", "Import"),
    // main.rs（菜单栏）
    ("隐藏 SSHL", "Hide SSHL"),
    ("隐藏其他", "Hide Others"),
    ("全部显示", "Show All"),
    ("退出 SSHL", "Quit SSHL"),
    ("编辑", "Edit"),
    ("拷贝", "Copy"),
    ("粘贴", "Paste"),
    ("全选", "Select All"),
    ("窗口", "Window"),
    ("最小化", "Minimize"),
    // prompts.rs
    ("确定", "OK"),
    ("知道了", "OK"),
    ("已保存的密码", "Saved passwords"),
    // quick_connect.rs
    ("搜索连接…", "Search connections…"),
    ("没有保存的连接", "No saved connections"),
    ("没有匹配的连接", "No matching connections"),
    ("快速连接", "Quick connect"),
    // settings_dialog.rs
    ("JetBrainsMono Nerd Font (内置·含图标)", "JetBrainsMono Nerd Font (built in, with icons)"),
    ("{} (含图标)", "{} (with icons)"),
    ("新分组名称", "New group name"),
    ("无法保存分组", "Can't save the group"),
    ("分组名称已存在。", "A group with this name already exists."),
    ("保存分组失败", "Couldn't save the groups"),
    ("设置", "Settings"),
    ("关闭", "Close"),
    ("终端", "Terminal"),
    ("外观", "Appearance"),
    ("分组管理", "Groups"),
    ("自定义…", "Custom…"),
    ("图标字体（Nerd Font）", "Icon fonts (Nerd Font)"),
    ("等宽字体", "Monospaced fonts"),
    ("其他", "Other"),
    ("字号", "Size"),
    ("减小", "Smaller"),
    ("增大", "Larger"),
    ("字体", "Font"),
    ("自定义字体", "Custom font"),
    ("Nerd Font 图标:", "Nerd Font icons:"),
    ("配色", "Color scheme"),
    ("选中即复制", "Copy on select"),
    ("拖选文字后自动复制到剪贴板", "Selected text is copied to the clipboard automatically"),
    ("右键粘贴", "Right-click to paste"),
    ("在终端里点右键直接粘贴（程序接收鼠标时除外）", "Pastes on right click, unless the program uses the mouse"),
    ("还原默认", "Reset to defaults"),
    ("跟随系统", "System"),
    ("浅色", "Light"),
    ("深色", "Dark"),
    ("界面外观", "Appearance"),
    ("终端里的颜色不跟着变，在「终端」页的配色里选", "Terminal colors don't follow this; choose them under Terminal → Color scheme"),
    ("语言", "Language"),
    ("未分组", "Ungrouped"),
    ("添加分组", "Add group"),
    ("分组与机器", "Groups and hosts"),
    ("拖动机器归组，拖动把手排序分组", "Drag hosts into groups; drag a handle to reorder groups"),
    ("有未保存的改动", "Unsaved changes"),
    ("删除分组（机器移到未分组）", "Delete group (its hosts become ungrouped)"),
    ("快捷键", "Shortcuts"),
    ("{} 已经是「{}」的快捷键", "{} is already the shortcut for “{}”"),
    ("改给「{}」后，「{}」就不再用这个快捷键。", "If you give it to “{}”, “{}” will no longer use it."),
    ("改用", "Reassign"),
    ("点快捷键后按新的组合键：Esc 取消，⌫ 去掉", "Click a shortcut, then press the new keys. Esc cancels, ⌫ removes it."),
    ("要带 ⌘、⌃ 或 ⌥", "Add ⌘, ⌃ or ⌥"),
    ("按下新的快捷键…", "Press new shortcut…"),
    ("无", "None"),
    ("恢复默认", "Reset"),
    ("{} 的快捷键", "Shortcut for {}"),
    // keymap.rs
    ("连接与标签", "Connections and tabs"),
    ("视图与面板", "Views and panels"),
    ("应用|快捷键分组", "App"),
    ("关闭标签", "Close tab"),
    ("下一个标签", "Next tab"),
    ("上一个标签", "Previous tab"),
    ("切到第 {} 个标签", "Go to tab {}"),
    ("收起 / 展开侧边栏", "Hide / show sidebar"),
    ("查找下一个", "Find next"),
    ("查找上一个", "Find previous"),
    ("放大字号", "Bigger text"),
    ("缩小字号", "Smaller text"),
    ("默认字号", "Default text size"),
    ("显示 / 隐藏隐藏文件", "Show / hide hidden files"),
    ("收藏 / 取消收藏当前目录", "Add / remove current folder in favorites"),
    ("空格", "Space"),
    // sidebar.rs
    ("展开侧边栏", "Show sidebar"),
    ("收起侧边栏", "Hide sidebar"),
    ("拖到此处移出分组", "Drop here to remove from the group"),
    ("重命名", "Rename"),
    ("关闭其他标签", "Close other tabs"),
    ("复制主机地址", "Copy host address"),
    ("断开连接", "Disconnect"),
    // snippets.rs
    ("搜索命令片段…", "Search snippets…"),
    ("删除命令片段", "Delete snippet"),
    ("确定要删除「{}」吗？", "Delete “{}”?"),
    ("还没有命令片段。点 ＋ 把常用命令存起来。", "No snippets yet. Click + to save a command you use often."),
    ("没有匹配的命令片段", "No matching snippets"),
    ("命令片段", "Snippets"),
    ("新建命令片段", "New snippet"),
    ("↵ 执行　⇧↵ 只填入", "↵ Run   ⇧↵ Insert only"),
    ("命令，可以多行", "Command (can be several lines)"),
    ("名称（可选）", "Name (optional)"),
    ("编辑命令片段", "Edit snippet"),
    ("所有服务器", "All servers"),
    // system_panel.rs
    ("{} 分钟", "{} min"),
    ("{} 小时 {} 分", "{} h {} min"),
    ("{} 天 {} 小时", "{} d {} h"),
    (
        "没有权限结束这个进程。在终端里 sudo -i（或 su）切到 root 后再试，这里会跟着用 root 身份。",
        "Not permitted to end this process. Switch to root in the terminal with sudo -i (or su) and try again; this page follows the terminal's user.",
    ),
    ("进程已经不在了。", "The process is already gone."),
    ("失败", "Failed"),
    ("已生效", "Active"),
    ("运行中", "Running"),
    ("启动中", "Starting"),
    ("停止中", "Stopping"),
    ("重载中", "Reloading"),
    ("没有权限：在终端里 sudo -i 后再看。", "Not permitted: run sudo -i in the terminal, then look again."),
    ("读取失败：{}", "Couldn't read: {}"),
    ("启动", "Start"),
    ("停止", "Stop"),
    ("重启", "Restart"),
    ("设为开机启动", "Enable at boot"),
    ("取消开机启动", "Disable at boot"),
    ("搜索…", "Search…"),
    ("强制结束 {}？", "Force kill {}?"),
    (
        "PID {} · kill -9：进程来不及收尾（不保存、不清理），只在普通结束不管用时用。",
        "PID {} · kill -9: the process gets no chance to finish up (nothing saved or cleaned). Use only when ending it normally doesn't work.",
    ),
    ("结束 {}？", "End {}?"),
    ("PID {} · 发送 TERM，进程可以先收尾再退出。", "PID {} · Sends TERM, so the process can finish up before it exits."),
    ("强制结束", "Force kill"),
    ("结束进程", "End process"),
    ("结束 {} 需要 root 权限", "Ending {} requires root"),
    ("没能结束 {}", "Couldn't end {}"),
    ("在终端里用 sudo 执行", "Run with sudo in the terminal"),
    ("{} {} 需要 root 权限", "{} {} requires root"),
    ("没能{}{}", "{} {} failed"),
    ("{}{}？", "{} {}?"),
    ("{} 核", "{0} {0?core|cores}"),
    ("共 {}", "of {}"),
    ("总量", "Total"),
    ("共 {} · /", "of {} · /"),
    ("负载（1 / 5 / 15 分钟）", "Load (1 / 5 / 15 min)"),
    ("内存已用", "Memory used"),
    ("磁盘已用", "Disk used"),
    ("已运行", "Uptime"),
    ("按{}排序", "Sort by {}"),
    ("结束 {}", "End {}"),
    ("结束", "End"),
    ("所有地址", "All addresses"),
    ("本机|监听地址", "Loopback"),
    ("看不到进程（需要 root）", "Process hidden (needs root)"),
    ("处理中…", "Working…"),
    ("日志", "Logs"),
    ("{} 的日志", "Logs of {}"),
    ("已屏蔽", "Masked"),
    ("进程|标签页", "Processes"),
    ("端口|标签页", "Ports"),
    ("服务|标签页", "Services"),
    ("用户", "User"),
    ("内存", "Memory"),
    ("命令", "Command"),
    ("协议", "Protocol"),
    ("地址", "Address"),
    ("进程", "Process"),
    ("状态", "Status"),
    ("服务", "Service"),
    ("说明", "Description"),
    ("开机启动", "At boot"),
    ("连接服务器后显示它的进程、端口和服务", "Connect to a server to see its processes, ports and services"),
    ("正在读取…", "Loading…"),
    ("这台服务器没有 systemd", "This server doesn't have systemd"),
    ("没有内容", "Nothing here"),
    ("没有匹配的结果", "No matching results"),
    ("强制结束（kill -9）", "Force kill (kill -9)"),
    ("复制 PID", "Copy PID"),
    ("复制命令", "Copy command"),
    ("在终端里看日志", "View logs in the terminal"),
    ("复制名称", "Copy name"),
    // terminal/view.rs
    ("松开上传到终端当前目录", "Drop to upload to the terminal's current folder"),
    ("{} 密钥指纹\n{}", "{} key fingerprint\n{}"),
    ("{} 的主机密钥变了", "The host key of {} has changed"),
    (
        "和上次记录的不一样。可能是服务器重装或换了密钥，也可能有人在冒充它（中间人攻击）。不确定就取消，先问问服务器管理员。\n\n{}",
        "It differs from the one recorded last time. The server may have been reinstalled or given a new key, or someone may be impersonating it (a man-in-the-middle attack). If you're not sure, cancel and ask the server's administrator.\n\n{}",
    ),
    ("信任新密钥", "Trust the new key"),
    ("第一次连接 {}", "First connection to {}"),
    ("请核对服务器的密钥指纹，确认无误再连接。\n\n{}", "Check the server's key fingerprint and connect only if it matches.\n\n{}"),
    ("信任并连接", "Trust and connect"),
    ("连接已断开 · 按 Enter 重新连接", "Disconnected · Press Enter to reconnect"),
    ("查找", "Find"),
    ("未信任主机", "Host not trusted"),
    ("没有信任主机密钥 · 按 Enter 重试", "Host key not trusted · Press Enter to retry"),
    ("连接失败：{} · 按 Enter 重试", "Connection failed: {} · Press Enter to retry"),
    ("正在重新连接…", "Reconnecting…"),
    ("在终端里查找", "Find in terminal"),
    ("上一个（↵）", "Previous (↵)"),
    ("下一个（⇧↵）", "Next (⇧↵)"),
    ("关闭（Esc）", "Close (Esc)"),
    ("查找中…", "Searching…"),
    ("无结果", "No results"),
    ("{} 个", "{0} {0?match|matches}"),
    // topbar.rs
    ("文件管理", "Files"),
    ("系统", "System"),
    ("填充发送完毕", "Password sent"),
    ("填充密码", "Fill password"),
    ("选择账号密码", "Choose an account"),
    ("新增账号密码", "Add an account"),
    ("断开", "Disconnect"),
    ("已打开的会话", "Open sessions"),
    ("重命名标签", "Rename tab"),
    ("超时", "Timeout"),
    // workspace.rs
    ("搜索连接...", "Search connections..."),
    ("打开本机终端失败", "Couldn't open a local terminal"),
    ("确定要关闭其他 {} 个标签吗？连着的会话会断开。", "Close the {0} other {0?tab|tabs}? Connected sessions will be disconnected."),
    ("连接 {}", "Connect to {}"),
    ("输入 {} 的密码。", "Enter the password for {}."),
    ("保存连接失败", "Couldn't save the connection"),
    ("连接失败", "Connection failed"),
    ("「{}」还在运行，关闭标签会结束它。", "“{}” is still running. Closing the tab will end it."),
    ("关闭「{}」", "Close “{}”"),
    ("这个会话", "this session"),
    ("正在进行的文件传输会中断。", "File transfers in progress will be interrupted."),
    ("文件管理会一并关闭。", "The file manager will close too."),
    ("确定要断开「{}」吗？", "Disconnect “{}”?"),
    ("删除连接", "Delete connection"),
    ("确定要删除这个连接吗？", "Delete this connection?"),
    ("编辑连接失败", "Couldn't edit the connection"),
    ("账号", "Account"),
    ("填充密码失败", "Couldn't fill the password"),
    ("保存失败", "Couldn't save"),
    ("按 {} 快速连接，或新建连接开始使用", "Press {} to quick connect, or create a new connection to get started"),
    ("新建连接开始使用", "Create a new connection to get started"),
    ("点击左侧连接或创建新连接开始使用", "Click a connection on the left, or create a new one to get started"),
    ("本机终端", "Local terminal"),
    ("正在连接服务器...", "Connecting to the server..."),
    ("退出 SSHL？", "Quit SSHL?"),
    ("退出", "Quit"),
    ("开着的 {} 个标签会关闭，连着的会话会断开。", "The {0} open {0?tab|tabs} will be closed and connected sessions disconnected."),
];

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::path::Path;

    use sshl_core::i18n::{self, Lang};

    use super::TABLE;

    /// 源码里一个字符串字面量的内容（从开头的引号后面读起），和它后面剩下的源码。
    fn literal(source: &str) -> Option<(String, &str)> {
        let mut out = String::new();
        let mut chars = source.strip_prefix('"')?.char_indices();
        while let Some((at, c)) = chars.next() {
            match c {
                '"' => return Some((out, &source[at + 2..])),
                '\\' => match chars.next()?.1 {
                    'n' => out.push('\n'),
                    other => out.push(other),
                },
                c => out.push(c),
            }
        }
        None
    }

    /// 一个文件里所有 `tr!("…")` / `trc!("场合", "…")` 的键。
    fn keys_in(source: &str, keys: &mut HashSet<String>) {
        for (at, _) in source.match_indices("tr!(").chain(source.match_indices("trc!(")) {
            if source[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                continue;
            }
            let with_context = source[at..].starts_with("trc!(");
            let rest = source[at + if with_context { 5 } else { 4 }..].trim_start();
            let Some((first, rest)) = literal(rest) else { continue };
            if !with_context {
                keys.insert(first);
            } else if let Some((zh, _)) = rest.trim_start().strip_prefix(',').and_then(|rest| literal(rest.trim_start())) {
                keys.insert(format!("{zh}|{first}"));
            }
        }
    }

    fn sources(dir: &Path, keys: &mut HashSet<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                sources(&path, keys);
            } else if path.extension().is_some_and(|ext| ext == "rs") && !path.to_string_lossy().contains("i18n") {
                keys_in(&std::fs::read_to_string(&path).unwrap(), keys);
            }
        }
    }

    fn used_keys() -> HashSet<String> {
        let app = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut keys = HashSet::new();
        sources(&app.join("src"), &mut keys);
        sources(&app.join("../core/src"), &mut keys);
        keys
    }

    #[test]
    fn scanner_reads_keys_contexts_and_escapes() {
        let mut keys = HashSet::new();
        keys_in(r#"a(tr!("保存"), tr!( "删除「{}」\n吗", name), trc!("标签页", "端口"), attr!("不算"))"#, &mut keys);
        let mut keys: Vec<_> = keys.into_iter().collect();
        keys.sort();
        assert_eq!(keys, ["保存", "删除「{}」\n吗", "端口|标签页"]);
    }

    #[test]
    fn every_text_in_the_source_has_an_english_translation() {
        let english: HashMap<&str, &str> = TABLE.iter().chain(i18n::core_en_table()).copied().collect();
        let used = used_keys();
        assert!(used.len() > 300, "只扫到 {} 条，扫描坏了", used.len());
        let mut missing: Vec<&String> = used.iter().filter(|key| !english.contains_key(key.as_str())).collect();
        missing.sort();
        assert!(missing.is_empty(), "这些文字没有英文译文：{missing:#?}");
        let mut unused: Vec<&str> = english.keys().copied().filter(|key| !used.contains(*key)).collect();
        unused.sort();
        assert!(unused.is_empty(), "译文表里这些用不到了：{unused:#?}");
    }

    #[test]
    fn translations_keep_the_arguments_and_are_listed_once() {
        let mut seen = HashSet::new();
        for (zh, en) in TABLE.iter().chain(i18n::core_en_table()) {
            assert!(seen.insert(*zh), "重复的原文：{zh}");
            let original = zh.rsplit_once('|').map_or(*zh, |(text, _)| text);
            assert_eq!(i18n::arity(original), i18n::arity(en), "参数个数对不上：{zh} → {en}");
            assert!(!en.trim().is_empty(), "空译文：{zh}");
            assert!(!en.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)), "译文里还有中文：{zh} → {en}");
        }
    }

    #[test]
    fn english_lookup_goes_through_both_tables_and_contexts() {
        i18n::register_en(TABLE);
        assert_eq!(i18n::translate(Lang::En, "新建连接"), "New connection");
        assert_eq!(i18n::translate(Lang::En, "已停止"), "Stopped", "后端表里的也查得到");
        assert_eq!(i18n::translate_in(Lang::En, "标签页", "端口"), "Ports");
        assert_eq!(i18n::translate(Lang::En, "端口"), "Port");
        assert_eq!(i18n::format(i18n::translate(Lang::En, "已隐藏 {} 个隐藏文件"), &[&1]), "1 hidden file not shown");
        assert_eq!(i18n::format(i18n::translate(Lang::En, "已隐藏 {} 个隐藏文件"), &[&3]), "3 hidden files not shown");
        assert_eq!(i18n::translate(Lang::Zh, "新建连接"), "新建连接");
    }
}
