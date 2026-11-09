# CLAUDE.md

Guidance for Claude Code when working in this repo.

## Project Overview

原生 macOS SSH 客户端：终端 + SFTP 文件管理。界面是 GPUI（Zed 的 GPU 界面框架），终端模拟是
libghostty-vt（Ghostty 内核），后端是 Rust（russh + russh-sftp）。不用 WebView / Tauri / JS。

## Layout

```
Cargo.toml            workspace（版本号在 [workspace.package]）
rust-toolchain.toml   Rust 1.98.1（只在本仓库固定）
crates/core/          sshl-core：SSH / SFTP / 存储后端，不依赖界面框架
crates/app/           sshl：GPUI 应用（工作区、侧栏、终端、文件管理、对话框）
  src/terminal/       终端：engine.rs（终端线程）、frame.rs（帧快照）、view.rs（绘制 + 输入法）、keys.rs
  src/files/          文件管理（本地 / 远程两栏、传输、跟随终端用户）
assets/               内置字体、Lucide 图标（线宽 2.5）、应用图标、Info.plist 模板
vendor/russh/         russh 0.45 + 手动接收窗口补丁（见 vendor/README.md）
scripts/              bundle.sh（构建 SSHL.app）、deploy.sh（本地安装）
tests/                fixtures-sudo.Dockerfile：sshl-core 集成测试用的 sshd 容器
```

## Build & Run

```bash
brew install zig                   # Zig 0.17：libghostty-vt 构建用
cargo run -p sshl                  # 开发版（debug；libghostty-vt 也是 Zig Debug，刷屏很慢，测性能用 release）
cargo test -p sshl-core            # 后端单元测试（集成测试 #[ignore]，要 SSHL_SFTP_IT / SSHL_SUDO_IT 指向测试容器）
cargo test -p sshl                 # 界面纯逻辑测试（分组、帧、键位、设置迁移、文件管理逻辑）
cargo clippy --workspace
scripts/bundle.sh                  # release 构建 + 组装 target/release/bundle/SSHL.app（ad-hoc 签名）
scripts/deploy.sh                  # 装到 /Applications（Developer ID 重签 + 自定义图标），先跑 bundle.sh
```

## Key invariants

- **版本钉死**：`gpui-pre` / `gpui-pre-platform` = 0.3.12、`gpui-base` = 0.9.1 必须配套（每周发版、常有破坏性改动），
  `libghostty-vt` 钉在 git rev。升级要一起升并重测启动和终端。
- **不用 gpui-component / gpui-kit 的样式层**：它的 init 枚举全部系统字体，冷启动多 ~170ms（实测 320ms → 488ms）。
  只用 `gpui-base`（输入框 / 输入法、焦点、弹层等行为），外观在 `ui.rs` 按纸页设计自己画。
- **冷启动**：从 `open` 到窗口显示中位 ~300ms（旧 Tauri 版 ~600ms）。启动路径上别加同步 IO、字体枚举、网络；
  系统字体列表只在设置对话框里后台加载。启动时不主动激活应用（Finder / Dock 打开时系统会激活）。
- **窗口**：透明标题栏，红绿灯在侧栏顶部单独一行；拖动区用 `workspace::drag_region`（按下移动窗口、双击缩放）。
  关窗口（红点 / 没有标签时 ⌘W）只隐藏应用、进程常驻（会话、预热连接都留着），点 Dock 走 `on_reopen` 显示；⌘Q 退出。
- **终端线程模型**：libghostty-vt 的类型都是 `!Send`，每个会话一个终端线程独占它（`terminal/engine.rs`）。
  SSH 输出、键鼠、改尺寸都以 `Input` 消息进来；界面只拿 `Frame` 快照来画，行是 `Arc<Row>`，没变的行沿用。
  每轮先处理控制消息（按键、^C 不排在积压输出后面），输出按 64KB 切片解析，刷屏时最多 16ms 出一帧。
  键盘 / 鼠标 / 粘贴 / 焦点的编码都交给 libghostty（应用光标键、Kitty 键盘协议、括号粘贴）；
  普通字符走 GPUI 的输入法通道（`EntityInputHandler`），中文输入法组字画在光标处。
- **终端流控**：shell 通道用 vendored russh 的手动窗口（2MB）。终端线程解析完一片就 `ShellHandle::ack`，
  core 攒够 64KB 补窗口；刷屏时积压与 ^C 后的尾巴都以 2MB 为界（release 实测刷屏中 ^C 到 `yes` 退出 71ms）。其他通道（SFTP / exec / 探活）用自动窗口
  （`CONNECTION_WINDOW` 8MB）。不要绕过 ack：不确认的话终端在 2MB 输出后停住。
- **终端外观**：卡片按整行整列贴齐（余下不足一行 / 一列留给外面的浅色背景），四边 4px 内边距；内边距取边缘格子的主色
  并把整行有底色的边缘行延伸进去（`frame::pick_edge_bg` / `edge_extension`）。默认光标闪烁竖线，DECSCUSR 0 回到它。
  粗体 + 基本 8 色画成亮色（同旧版 xterm）。PTY 尺寸变化节流到 ~30 次/秒。
- **内置字体**：JetBrainsMono Nerd Font Mono 两个字重 `include_bytes` 进二进制并在启动时注册；不要换成系统回退。
- **图标**：`ui::icon` 画 Lucide SVG，不传颜色时用继承的文字颜色（currentColor）。gpui 自带 `svg()` 只认自身的
  text_color、不继承，别直接用。图标在 `assets.rs` 的 `icons!` 表里登记。
- **圆角**：gpui 的 `overflow_hidden` 只按矩形裁，不裁圆角。卡片里贴边的有底色子元素（对话框底部按钮条、设置左栏）
  要自己加对应的 `rounded_*`。
- **提示框**：一律用 `prompts::alert / confirm / password`（应用内模态框，排队显示，回车确认、Esc 取消，关掉后焦点还给
  弹出前的地方）。不要用系统 NSAlert / `cx.prompt`：原生弹窗按钮是英文、窄、中文长句被拦腰折行，与应用风格不一。
- **连接流程**（`workspace.rs`）：一个连接只开一个会话；连上才出标签，慢于 300ms 才显示连接遮罩（Esc / 取消可中止）。
  切换到已开会话是乐观切换，探活在后台，失效且用户仍停在该会话上时原地重连；用户主动断开不算失效。
  远端关闭 / 掉线时标签和屏幕留着（灰点），终端里提示按 Enter 原地重连（`TerminalView::reconnect`：新 shell 通道经
  `Input::Rebind` 接到同一个终端引擎，回滚历史不丢）；侧栏双击、文件管理的「重新连接」也是原地重连。
  断开已连着的会话先确认，已断开的标签直接关。新建连接先连接、连上再保存。
- **主机密钥**（`crates/core/src/known_hosts.rs`）：信任记在 `~/.sshl/known_hosts`；`~/.ssh/known_hosts` 只读不写，那里认过的直接信任。
  没记录 / 密钥变了由界面弹确认框（`HostKeyCheck` 的 prompt），确认才写入；拒绝报 `AppError::HostKeyDeclined`，界面不再弹「连接失败」。
  预热池没人可问（prompt 为 None），没确认过的主机不预热。测试用 `HostKeyCheck::trust_any_for_tests()`，不碰开发机的记录。
- **认证方式**（`config_store::AuthKind`，库里 `connections.auth` 列；空 = 旧数据，按有没有私钥推断）：界面上只分密码 / 私钥，
  私钥里再选「自动」（`AuthKind::Auto`，免密）或「指定文件」（`Key`，默认 `~/.ssh/id_rsa`，路径可写 `~/`）。
  自动（`auto_auth.rs`）依次试 none、SSH Agent、私钥文件，私钥和 Agent 按 `ssh -G` 解析的 `~/.ssh/config`
  （IdentityFile / IdentityAgent / IdentitiesOnly），解析不了退到 OpenSSH 的默认私钥；失败时说明试了哪些、哪把设了密码。
  密码方式没存密码时，连接前先弹框问（只用这一次）。预热池按 `user@host:port` 复用，不区分认证方式。
- **终端里的 Tab**：gpui-base 的 Root 把 tab / shift-tab 绑成切换焦点，终端上下文里用 `NoAction` 压掉（`terminal::view::key_bindings`），
  否则补全键会把焦点跳到搜索框。
- **IPC 已不存在**：界面直接调 `sshl_core`。后端跑在独立 tokio 运行时（`backend.rs`），界面用 `backend::spawn` 在 GPUI 执行器里
  await 结果；后端事件（传输进度、连接列表变化）经 `sshl_core::events::Events` 回调转回界面线程。
- **预热池**归后端所有（`sshl_core::warm::run`），界面不发预热调用。
- **SFTP 超时**：没有单请求超时；`sftp_stall.rs` 包住通道，整整 `SFTP_STALL`（60s）一个字节都没收到才让所有等待中的请求失败，
  `sftp_for` 再以同一身份重开。下载写 `.<name>.<rand>.sshl-part`，成功后改名，失败不留半截、不覆盖原文件。
- **SFTP 通道复用**：每个 SSH 会话缓存一个 SFTP 会话共用。
- **文件管理跟随终端用户**：终端里 `su` / `sudo -i` 之后，文件管理以那个用户运行（`files/identity.rs`、
  `crates/core/src/terminal_user.rs`）。`file_switch_user` 用 su/sudo 起的 `sftp-server` 换掉缓存的 SFTP 会话；
  远程删除 / chmod / chown 必须走 `exec_as_file_user` / `file_shell`，不能用普通 exec。只自动用所需用户自己保存的密码，
  不去试别的已保存密码。SFTP 起来之前任何失败都要 EOF + 关通道（russh drop 不关通道，挂着的 sudo 会占住时间戳锁）。
  目录也跟随：同一进程链上最深的 shell 的 cwd（Linux 读 `/proc/<pid>/cwd`，否则 lsof），以文件管理当前身份读（`file_terminal_cwd`）。
  第一次打开从终端目录开始；之后只在终端目录变了时跟过去，用户在文件管理里自己点到别处不会被拽回。不依赖服务器配 OSC 7。
- **远程目录收藏**：按连接存在 `sshl.db` 的 `bookmarks` 表（删连接时一起删）；远程栏路径框旁的收藏按钮，
  当前目录已收藏时是蓝色，菜单里收藏 / 取消收藏当前目录、点收藏的目录直接过去。
- **拖放**：应用内拖动用 GPUI 的 `on_drag` / `on_drop`（连接换组排序、标签排序、文件互拖）；Finder 拖入用 `ExternalPaths`。
- **设置**：`~/.sshl/settings.json`（字号、字体、收起的分组、侧栏收起、本地栏宽度、是否显示隐藏文件、选中即复制、右键粘贴）。首次启动从旧版 WebView 的
  localStorage 迁移。设置对话框里的改动先预览（`settings::preview`），点保存才写盘，关掉不保存会还原。
- **存储**：连接与加密凭据在 `~/.sshl/sshl.db`，密钥在 `~/.sshl/master.key`（0600）；密码明文不经过界面
  （填充密码由后端解密直接写通道）。
- **无障碍**：可点击的元素给 `role` + `aria_label`（也是无头测试按名字点击的依据）。连接项鼠标双击才连，
  键盘 / 辅助功能「按下」（`on_a11y_action(Click)`）直接连。
- **devhook 测试口**：debug 构建、或带 `--features devhook` 的构建里才编进去（`#[cfg(any(debug_assertions, feature = "devhook"))]`），
  且要设了 `SSHL_DEV_SOCKET=/tmp/xxx.sock` 才启用（`devhook.rs`）。经 Unix 套接字注入按键（`keys ctrl-c enter` /
  `type echo hi`），走 GPUI 完整的按键分发 + 输入法通道；无头测试终端输入用，不抢用户焦点。测 release 性能时用
  `cargo build -p sshl --release --features devhook`；`scripts/bundle.sh` 的正式构建不带这个 feature。macOS 的套接字路径上限 104 字节。
- **Rust 后端规则**：core 不依赖界面；新增界面需求优先在 app 里做，确实要改 core 的行为（SFTP、sudo、known_hosts）
  要带测试。core 的集成测试用 `tests/fixtures-sudo.Dockerfile` 起的容器。
