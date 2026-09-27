# SSHL

> Connect. Type. Transfer.

轻量级桌面 SSH 客户端：终端 + SFTP 文件管理放在同一个窗口。Tauri 打包，Rust 后端（russh + russh-sftp），React + xterm.js 前端。

## 功能

**终端**
- xterm.js 渲染，内置 JetBrainsMono Nerd Font，图标与 Powerline 字形开箱可用
- Unicode 11 宽度表，emoji 与中文不错位
- 字体、字号可调，可选系统字体或自定义 `fontFamily`
- 终端出现 sudo / su 密码提示时一键填入已保存的密码，同一台机器可存多个账号

**连接管理**
- 密码 / 私钥（含私钥密码）认证
- 分组管理，侧边栏拖拽排序、跨组移动；侧边栏可收起
- 后台预热连接池 + `TCP_NODELAY`，首次连接快，首屏输出不丢不乱序

**文件管理**
- 本地 / 远程双栏，拖动调整面板宽度
- 文件与目录上传下载，带进度
- 新建远程目录，修改权限（常用权限一键选）与属主
- SFTP 复用已建立的 SSH 会话，不额外建连

## 快速开始

### 下载安装

从 [Releases](https://github.com/qfdk/sshl/releases) 下载 macOS 版本，解压后拖入「应用程序」。

应用未签名、未公证，首次使用前在终端执行一次（清除隔离标记并本地签名）：

```bash
xattr -cr /Applications/SSHL.app
codesign --force --deep -s - /Applications/SSHL.app
```

连接局域网主机时 macOS 会请求「本地网络」权限，请允许；若报 `No route to host`，到 系统设置 → 隐私与安全性 → 本地网络 打开 SSHL。每次升级后重新执行上面两行。

### 手动构建

需求：[Bun](https://bun.sh)、Rust 工具链

```bash
git clone https://github.com/qfdk/sshl
cd sshl
bun install
bun run dev      # 开发模式
bun run build    # 构建 SSHL.app
bun test         # 测试
```

产物在 `src-tauri/target/release/bundle/macos/SSHL.app`。

## 数据存储

- 连接、分组与加密后的凭据存于 SQLite（`~/.sshl/sshl.db`）
- 凭据加密密钥单独存放在 `~/.sshl/master.key`（权限 0600）
- 主机公钥记录在 `~/.ssh/known_hosts`，与 OpenSSH 共用

## 安全说明

- 密码与私钥密码用 AES-256-GCM 加密；密钥与数据库分离，数据库被拷走或同步也解不出密码
- 主机密钥按 TOFU 校验：首次连接信任并写入 `known_hosts`，之后密钥变更直接拒绝连接（防中间人）
- 自动填密码全程在后端完成，明文不经过渲染层；只有终端出现密码提示时才显示填充按钮

## 许可证

[GNU General Public License v3.0](LICENSE)（GPL-3.0）——衍生作品须以相同许可证开源。

## 赞助

<a href="https://voilapro.app/?ref=github-sshl"><img src="https://voilapro.app/images/icon.png" alt="Voilà Pro" width="160"/></a>

本项目由 [Voilà Pro](https://voilapro.app/?ref=github-sshl) 赞助支持 —— macOS 语音输入工具。按住快捷键说话，文字直接落到光标处，中英法混说也能识别。
