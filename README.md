# SSHL

基于 Tauri 的桌面 SSH 客户端：Rust 后端（russh + russh-sftp），React + xterm.js 前端，自带 SFTP 文件管理。

## 功能特点

- **终端** - xterm.js 渲染，内置 JetBrainsMono Nerd Font，Unicode 11 宽度表，emoji 与中文不错位
- **连接管理** - 分组、拖拽排序，支持密码与私钥（含 passphrase）认证
- **凭据加密** - AES-256-GCM 加密存储，密钥独立保存在 `~/.sshl/master.key`，数据库被拷走也解不出密码
- **主机校验** - 基于 `~/.ssh/known_hosts` 的 TOFU 校验
- **文件管理** - 本地 / 远程双栏，文件与目录上传下载、新建目录、修改权限和属主；SFTP 复用已建立的 SSH 会话
- **连接快** - 后台预热连接池，首屏输出不丢不乱序

## 安装

从 [Releases](https://github.com/qfdk/sshl/releases) 下载 macOS 版本，解压后拖入「应用程序」。

应用未签名、未公证，首次使用前在终端执行一次（清除隔离标记并本地签名）：

```bash
xattr -cr /Applications/SSHL.app
codesign --force --deep -s - /Applications/SSHL.app
```

连接局域网主机时 macOS 会请求「本地网络」权限，请允许。若报 `No route to host`，到 系统设置 → 隐私与安全性 → 本地网络 打开 SSHL。每次升级后重新执行上面两行。

## 从源码构建

需要 [Bun](https://bun.sh) 和 Rust 工具链。

```bash
git clone https://github.com/qfdk/sshl.git
cd sshl
bun install
bun run dev      # 开发模式
bun run build    # 构建 SSHL.app
bun test         # 测试
```

构建产物在 `src-tauri/target/release/bundle/macos/SSHL.app`。

## 数据位置

| 路径 | 内容 |
| --- | --- |
| `~/.sshl/sshl.db` | 连接、分组与加密后的凭据 |
| `~/.sshl/master.key` | 凭据加密密钥（权限 0600） |

## 赞助

<a href="https://voilapro.app/"><img src="docs/voilapro.png" width="64" alt="VoilaPro" align="left"></a>

本项目由 **[VoilaPro](https://voilapro.app/)** 赞助支持。
<br clear="left">

