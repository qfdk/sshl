//! 后端文字的英文译文（原文 → 译文）。原文一个字都不能差：界面那边的测试会扫源码核对每条 `tr!` 都有译文。

pub const TABLE: super::Table = &[
    ("已停止", "Stopped"),
    ("程序", "A program"),
    ("、", ", "),
    ("和", " or "),
    ("SSH Agent 里的 {} 把密钥", "{0} {0?key|keys} from the SSH Agent"),
    (
        "免密登录没有可用的密钥：SSH Agent 里没有密钥，也没找到可用的私钥文件。",
        "No key to log in with: the SSH Agent has no keys and no usable private key file was found.",
    ),
    ("免密登录没有成功：服务器没有接受{}。", "Key login failed: the server did not accept {}."),
    (
        "{} 设了密码，免密用不了：可以先 ssh-add 加进 SSH Agent，或改用「私钥」方式并填写私钥密码。",
        " {} is protected by a passphrase and can't be used automatically: add it to the SSH Agent with ssh-add, or choose a key file and enter its passphrase.",
    ),
    ("命令不能为空", "The command can't be empty"),
    ("账号名不能为空", "The account name can't be empty"),
    ("需要 {} 的密码", "The password for {} is required"),
    ("需要密码", "A password is required"),
    ("切换到 {} 超时 {}", "Timed out switching to {} {}"),
    (
        "服务器上找不到 sftp-server，无法以该用户身份管理文件",
        "sftp-server was not found on the server, so files can't be managed as that user",
    ),
    ("当前连接未保存，没有已保存的密码", "This connection isn't saved, so it has no saved password"),
    ("当前连接未保存，无密码可填充", "This connection isn't saved, so there is no password to fill in"),
    ("未找到对应的已保存密码", "The saved password was not found"),
    ("创建暂存目录失败: {}", "Couldn't create the staging directory: {}"),
    ("无效的上传路径: {}", "Invalid upload path: {}"),
    ("无效的下载路径: {}", "Invalid download path: {}"),
    ("SFTP 通道已关闭", "The SFTP channel was closed"),
    ("SFTP 通道出错: {}", "SFTP channel error: {}"),
    ("SFTP 通道 {} 秒没有收到任何数据", "No data received on the SFTP channel for {} seconds"),
];
