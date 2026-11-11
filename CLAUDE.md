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
cargo clippy --workspace --all-targets   # 自己的 crate 保持零警告（vendor/russh 那条不改，补丁保持最小）
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
  窗口选项 `app_owns_titlebar_drag: true`：AppKit 不再在标题栏高度内自己拖窗口（标签就在那一条里，不设的话拖标签排序时
  整个窗口跟着走，GPUI 层面拦不住；devhook 的合成事件也碰不到 AppKit 这层，测不出来）。所以能拖窗口的地方都要是 drag_region
  （顶栏的上边距算在顶栏里）。拖动区里的标签、按钮外面包 `workspace::controls`（按下不冒泡给拖动区）。
  没有底部状态栏。终端页（和没会话时的空页面）任何时候都贴满内容区，左贴侧栏 / 窗口、右下贴窗口，直角无阴影（同 Ghostty）；
  文件管理 / 系统页在侧栏展开时右下留 6px（`CONTENT_EDGE`），收起时也贴满；顶栏两边留 6px。设置按钮在侧栏标题行（+ 和收起按钮之间）。
  侧栏收起（⌘B，`settings.sidebar_collapsed`）= 整个不画，不留图标栏：顶栏和红绿灯同一行，左边让出 `TRAFFIC_LIGHTS_WIDTH`、
  只放展开按钮（新建连接不重复放，标签栏末尾有 +）。收起时焦点在搜索框里就交回内容区——直接问焦点句柄，
  不看 `search_focused`：窗口不在前台时 GPUI 不发 Focus / Blur 事件。
  关窗口（红点 / 没有标签时 ⌘W）只隐藏应用、进程常驻（会话、预热连接都留着），点 Dock 走 `on_reopen` 显示；⌘Q 退出。
- **终端线程模型**：libghostty-vt 的类型都是 `!Send`，每个会话一个终端线程独占它（`terminal/engine.rs`）。
  SSH 输出、键鼠、改尺寸都以 `Input` 消息进来；界面只拿 `Frame` 快照来画，行是 `Arc<Row>`，没变的行沿用。
  每轮先处理控制消息（按键、^C 不排在积压输出后面），输出按 64KB 切片解析，刷屏时最多 16ms 出一帧。
  同步输出（DEC 2026，Claude Code / Ink、neovim 重画时用；libghostty 对 DECRQM 2026 答「支持」，Claude Code 据此启用）：
  `on_render_hold` 回调在 hold 开始时（vt_write 中途）就把当时完整的一帧拼好（`Painter`），hold 期间只显示这一帧，
  结束后按最新的出帧，最长 1s 超时。不认的话擦掉重写的中间状态会被画出来，整块闪。
  键盘 / 鼠标 / 粘贴 / 焦点的编码都交给 libghostty（应用光标键、Kitty 键盘协议、括号粘贴）；
  普通字符走 GPUI 的输入法通道（`EntityInputHandler`），中文输入法组字画在光标处。
- **终端里查找**（⌘F，`terminal/view.rs` 的搜索条 + `engine.rs` 的 `SearchDriver`）：用 libghostty 的 `Search`，搜回滚历史和屏幕（ASCII 字母不分大小写）。
  终端线程每轮最多搜 4ms（`SEARCH_SLICE`），结果（总数、第几个、屏幕上看得见的匹配按行切好的 `MatchSpan`）放进帧的 `search`；
  界面画淡黄底黑字、当前选中的橙底。一敲词就选中最新（最靠下）的那个，↵ / ⌘G 往更早的、⇧↵ / ⌘⇧G 往回，Esc 关（屏幕停在找到的地方）。
  搜索条是终端的兄弟元素、不在 `Terminal` 上下文里：放进去的话在里面敲的字会被终端的 key_down 发给远端。
  选中项由我们自己管（`SearchDriver::select`：记它第一格在整个屏幕里的位置和「第几个」），不用 libghostty 的 select_next / select_prev：
  它的匹配列表按文档是从新到旧，实际上屏幕跨几个内部页时这几页的先后是反的（`[旧页][新页][更早的历史]`，每段内部从新到旧；
  根因在它的 `search/active.zig`，页是倒着加进正向窗口的），照它的顺序走会跳来跳去，而且它只能一步步走、每步要重扫活动区。
  `frame::newest_first_segments` 只扫屏幕附近那一段找出页的交界、把列表切成真正从新到旧的几段，跳转一步到位；
  滚过去时把匹配放在离顶三分之一处（头两行被搜索条盖着，算看不见）。改尺寸后 libghostty 从头重搜，选中项按位置对回去。
  匹配很多时 libghostty 的 `feed` 不便宜（回滚历史里搜一个常见字母有二十多万个匹配，release 实测 feed 一次 7ms）：每次刷新只 feed 一回
  （tick 之后用一份快照取高亮），超过 `NEWEST_EXACT_MAX` 个匹配时不再每次取整张列表核对选中项，
  并且按上次刷新的耗时节流（`search_next`，搜索最多占终端线程约两成），刷屏时高亮晚一点跟上、按键和 ^C 不受拖累。
- **终端里的链接和字号**：按住 ⌘ 时鼠标下的网址画下划线、光标变手形，⌘ 点击打开（这次按下、拖动、松开都不交给选区，也不上报给程序；
  松开时才打开，按下后拖开了不算——按下就开的话浏览器抢走前台，松开收不到）。
  纯文本网址在界面线程按帧识别（`frame::link_at`：沿软换行 `Row::wrapped` 拼出整条逻辑行再找，折成几行的长链接是一个；
  结尾的标点和没配对的右括号不算）；OSC 8 超链接不在帧里，只在点击时问终端线程（`Input::LinkAt`，查那一格的 `hyperlink_uri`），
  所以悬停时只给纯文本网址画下划线。点击时 OSC 8 优先（程序明说的目标；显示的字可能是截断的或另一个网址），没有才用纯文本的。只开 http / https / ftp / mailto（`frame::openable`），file: / javascript: 之类一律不开。
  ⌘+（⌘=）/ ⌘- / ⌘0 放大 / 缩小 / 回默认字号：改的是全局字号并存盘（`settings::update`，所有终端一起变）。
- **终端流控**：shell 通道用 vendored russh 的手动窗口（2MB）。终端线程解析完一片就 `ShellHandle::ack`，
  core 攒够 64KB 补窗口；刷屏时积压与 ^C 后的尾巴都以 2MB 为界（release 实测刷屏中 ^C 到 `yes` 退出 71ms）。其他通道（SFTP / exec / 探活）用自动窗口
  （`CONNECTION_WINDOW` 8MB）。不要绕过 ack：不确认的话终端在 2MB 输出后停住。
- **终端外观**：铺满内容区，四边 4px 内边距。排不下一整行的余量全放在上面（`grid_offset`）：最后一行（nvim 状态栏）下面正好只有 4px。
  底边这 4px 不铺最后一行的颜色，用内边距色（nvim 的背景）。这是试了三种之后用户定的：余量放底下 = 状态栏下一条时宽时窄的深色边；
  4px 铺状态栏的颜色 = 状态栏比别的行高出一截（「多出 4px」）；底边不留 = 贴着窗口底边太挤（「很奇怪」）；
  只在 vim 这类全屏程序里不留（按备用屏判断）也试过，用户看了还是觉得贴得太紧——所以任何时候都留 4px，别再改。左右余量平分。
  边缘整行的底色左右铺到边，上边只铺 4px（再往上是余量，用内边距色）；其余内边距取边缘格子的主色
  并把整行有底色的边缘行（除了最后一行往下）延伸进去（`frame::pick_edge_bg` / `edge_extension`）。默认光标闪烁竖线，DECSCUSR 0 回到它。光标闪不闪只听 DECSCUSR，
  不认 DEC 12（vim / htop / less 退出时的 cnorm 带 `\e[?12l`，认了之后光标就一直不闪）：`engine.rs` 的 `BlinkPolicy` 每片输出后把模式位设回去。
  粗体 + 基本 8 色画成亮色（同旧版 xterm）。PTY 尺寸变化节流到 ~30 次/秒。
- **内置字体**：JetBrainsMono Nerd Font（非 Mono 版，图标是正常大小；Mono 版把图标压成一格，用户嫌不好看）两个字重 `include_bytes` 进二进制并在启动时注册；不要换成系统回退。
- **图标**：`ui::icon` 画 Lucide SVG，不传颜色时用继承的文字颜色（currentColor）。gpui 自带 `svg()` 只认自身的
  text_color、不继承，别直接用。图标在 `assets.rs` 的 `icons!` 表里登记。
  位图（`img()`）是双线性采样、没有 mipmap，大图缩小会发毛：侧栏标志用按 2 倍屏缩好的 `assets/sidebar-logo.png`（60px 画 30pt）。
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
- **新建 / 编辑连接里的分组**（`connection_dialog.rs` 的 `group_field`，逻辑在 `groups::group_choices` / `resolve_group`）：可以敲字的下拉。
  聚焦 / 点进去列出已有分组（顺序同侧栏，空分组也在），敲字按包含过滤，↑↓ 选、↵ 选中（没选中某行时 ↵ 照旧提交表单）、Esc 先收下拉再关对话框；
  敲的名字还没有就多一行「新建分组」，保存连接时分组自然建出来；只差大小写的归到已有分组。按在下拉上要 `prevent_default`，
  否则对话框拿走焦点、输入框一失焦下拉就收了，点击落空。
- **终端里的 Tab**：gpui-base 的 Root 把 tab / shift-tab 绑成切换焦点，终端上下文里用 `NoAction` 压掉（`terminal::view::key_bindings`），
  否则补全键会把焦点跳到搜索框。
- **IPC 已不存在**：界面直接调 `sshl_core`。后端跑在独立 tokio 运行时（`backend.rs`），界面用 `backend::spawn` 在 GPUI 执行器里
  await 结果；后端事件（传输进度、连接列表变化）经 `sshl_core::events::Events` 回调转回界面线程。
- **预热池**归后端所有（`sshl_core::warm::run`），界面不发预热调用。
- **SFTP 超时**：没有单请求超时；`sftp_stall.rs` 包住通道，整整 `SFTP_STALL`（60s）一个字节都没收到才让所有等待中的请求失败，
  `sftp_for` 再以同一身份重开。下载写 `.<name>.<rand>.sshl-part`，成功后改名，失败不留半截、不覆盖原文件。
  上传同理：先写远端同目录的 `.<name>.<rand>.sshl-part`，传完删旧文件再改名（SFTP rename 不覆盖），沿用原文件权限位；
  目标是符号链接、不归自己所有、或建不了临时文件时照旧直接写目标（换名会改属主 / 把链接换成文件）。
  文件管理的传输走一个队列（`files/view.rs` 的 `enqueue`）：一次传一个，后加的排在后面；进度显示速度、剩余时间、还有几个，
  底栏和终端右下角的进度卡片都有停止（清空队列 + 停下正在传的）。目标已有同名的先问覆盖还是跳过（文件夹是合并），
  还有排队的时可勾「后面再遇到同名的也这样处理」（`prompts::confirm_with_check`），这一轮里都照办。
  传输可停止：`sftp::cancellable(flag, fut)` 用任务局部变量传停止开关，读写循环每块检查，停止走出错路径（`AppError::Cancelled`），临时文件照样清掉。
- **SFTP 通道复用**：每个 SSH 会话缓存一个 SFTP 会话共用。
- **文件管理跟随终端用户**：终端里 `su` / `sudo -i` 之后，文件管理以那个用户运行（`files/identity.rs`、
  `crates/core/src/terminal_user.rs`）。`file_switch_user` 用 su/sudo 起的 `sftp-server` 换掉缓存的 SFTP 会话；
  远程删除 / chmod / chown 必须走 `exec_as_file_user` / `file_shell`，不能用普通 exec。只自动用所需用户自己保存的密码，
  不去试别的已保存密码。SSH 登录用户是 root 时 su 到谁都不要密码（`su_needs_password`），直接切、不弹框。
  拖文件到终端上传前先同步一次身份（`after_sync` 队列，身份对上了才传），root `su - 某人` 后传上去的文件归那个人。SFTP 起来之前任何失败都要 EOF + 关通道（russh drop 不关通道，挂着的 sudo 会占住时间戳锁）。
  目录也跟随：同一进程链上最深的 shell 的 cwd（Linux 读 `/proc/<pid>/cwd`，否则 lsof），以文件管理当前身份读（`file_terminal_cwd`）。
  第一次打开从终端目录开始；之后只在终端目录变了时跟过去，用户在文件管理里自己点到别处不会被拽回。不依赖服务器配 OSC 7。
- **从 `~/.ssh/config` 导入**（新建连接对话框左下角，`import_dialog.rs` + `crates/core/src/ssh_config.rs`）：列出不带通配的 Host，
  逐个 `ssh -G -F ~/.ssh/config 别名` 解析主机 / 用户 / 端口 / 私钥；写了 IdentityFile 的导入为私钥方式，没写的导入为自动；
  同一 用户@主机:端口 已有就不重复导入；导入到分组「ssh config」。
- **「系统」页**（⌘P，顶栏顺序 终端（⌘I）/ 文件管理（⌘O）/ 系统，系统不常用放最后，`system_panel.rs` + core 的 `server_info.rs` / `services.rs`）：负载 / 内存 / 磁盘 / 运行时间，
  下面三个列表：进程（`ps -eo`）、监听端口（有 ss 用 ss、否则 Linux netstat -p、再否则 lsof）、systemd 服务（list-units + list-unit-files，
  没被载入的已停服务也列出）。可结束进程、启停 / 重启服务、开关开机启动（停止、重启先确认）。容器管理做过又删了：用户用不到。
  命令走 `run_as_file_user`，切过来时先同步文件管理身份，终端 sudo -i 之后就以 root 跑。没权限（`services::needs_root`）时不只报错，
  而是提议「在终端里用 sudo 执行」，把 sudo 命令发到终端（`SystemPanelEvent::RunInTerminal`），密码在终端里输或点填充密码。
  看服务日志（journalctl -f）也发到终端。只在这页显示时每 3 秒刷新。
  Linux 的集成测试可以用 lima 虚拟机：`limactl start --name=sshl-test template://docker-rootful`，
  `SSHL_SYSTEMD_IT=127.0.0.1:<SSH 端口> SSHL_SYSTEMD_IT_USER=$USER SSHL_SYSTEMD_IT_KEY=~/.lima/_config/user`。
- **本机终端**（⌘T，`crates/core/src/local_shell.rs` + `TerminalView::local`）：openpty + 用户的登录 shell（argv[0] 带 `-`，读 .zprofile），
  接口同 SSH 终端（`ShellHandle` 进、`TerminalSink` 出）。读线程按 ack 记账，积压 2MB 就停（同 SSH 终端窗口，刷屏中 ^C 约 40ms 回到提示符）；
  键盘输入单独一个写线程，程序不读输入时卡住的写不挡确认 / 改尺寸 / 关闭。`setsid` + `TIOCSCTTY` 做控制终端，开 IUTF8；
  TERM 同 SSH（xterm-256color），没有 LANG / LC_* 时按系统地区补 UTF-8 的 LANG。吞吐在 macOS PTY 本身的上限上
  （`seq` 这类逐行 write 的和 `script -q /dev/null` 一样慢，是内核开销，不是终端）。
  工作区里本机标签 id 是 `local:<n>`（`Session.local`）：只有终端（不显示 终端 / 文件管理 / 系统、填充密码，⌘O / ⌘P 不切），
  shell 退出标签直接关；⌘W 前台只有 shell 时直接关，有程序在跑（`tcgetpgrp` ≠ shell）才问；新标签从当前本机标签前台进程的目录开始
  （`proc_pidinfo`）；从 Finder 拖进来是把转义后的路径粘贴到命令行，不是上传。命令片段的「本机」、本机的目录收藏都归属 `local`（`workspace::LOCAL_SCOPE`），所有本机标签共用。
  标签切换 ⌘⇧] / ⌘⇧[、⌃Tab / ⌃⇧Tab（`groups::cycle_tab`，两头绕回）；⌘1 … ⌘9 切到第几个标签（`groups::nth_tab`，本机标签也算，
  没那么多就不动）。点标签、⌘ 数字、⌘⇧] 都走 `Workspace::select_tab`。按住 ⌘ 时前 9 个标签角上浮出序号，视图按钮浮出 I / O / P。
- **快速连接**（⌘K，`quick_connect.rs`）：左右居中、中间偏上的浮层（顶边在窗口高度 30% 处，矮窗口往上收，`top_offset`；用户嫌离顶 72px 太靠上，要窗口中间略偏上），只搜保存的连接（空格分开的每个词都要出现在名称 / 主机 / 用户 / 分组里，
  名称以第一个词开头的排前面），↑↓ 选、↵ 连、Esc / 点外面关；已开着的带绿点、选了就切过去（`connect_to_saved`）。
  顺序同侧栏。侧栏收起时空页面提示 ⌘K。有对话框 / 提示框 / 正在连接时不弹。
- **右键菜单**（`sidebar.rs` 的 `connection_menu_overlay`，`ConnectionMenu.on_tab` 区分）：侧栏连接项是 连接 / 编辑 / 断开 / 删除（连着时编辑、删除置灰）；
  菜单背后有一层铺满窗口的遮罩，按下就关菜单；菜单面板外面要包 `occlude()`，否则按在菜单项上也会传到遮罩、菜单先没了，松开时点击落空。
  标签上只显示名字（连接名 / 改的名字），不带主机地址（用户不想让 IP 露在界面上）；鼠标悬停时气泡提示主机地址，
  名字本身就是地址的不重复提示（`groups::distinct_host`）。
  标签是 重命名 / 重新连接 / 复制主机地址 / 关闭其他标签 / 关闭（本机标签只有 重命名 / 关闭其他标签 / 关闭）。连接项上不放悬停按钮（和右键菜单、标签 × 重复）。
  标签的「重新连接」对还连着的会话先在后端断开旧会话（`reconnect_tab`；终端的 reconnect 只换 shell 通道，不关旧连接）。
- **标签重命名**（`topbar.rs` 的 `start_rename` / `finish_rename`）：双击标签或右键「重命名」，名字原地换成输入框（预填、全选），
  ↵ 确认、Esc 取消、点到别处 / 失焦也算确认，之后焦点回内容区。名字存在 `Session.custom_title`，只跟着这个打开的标签：
  不写库、不改保存的连接名、重启不留；原地重连、拖动排序都还在。去掉首尾空白、最多 40 个字（`groups::tab_custom_title`），
  清空或改回默认名 = 恢复默认（连接名 / 「本机」）。窗口不在前台时没有失焦事件，所以每次画顶栏还看一眼输入框丢没丢焦点（`settle_rename`）。
- **命令片段**（`snippets.rs`，库里 `snippets` 表，`config_store::snippets`）：顶栏按钮 / ⌘⇧P 打开面板，搜索、↑↓ 选、↵ 执行、⇧↵ 只填入。
  按粘贴发到终端（多行命令走括号粘贴，不会边粘边执行），执行时再补回车；不在终端页会先切过去。片段可以所有服务器通用，
  也可以只属于某个连接（删连接时一起删）；新建 / 编辑里的两个选项是「所有服务器」和连接名（本机标签是「本机」），不带「仅」字。面板里的上下 / 回车 / Esc 绑在 `SnippetPanel > Input`，压过 gpui-base 输入框自己的绑定。
- **目录收藏**：按连接存在 `sshl.db` 的 `bookmarks` 表（删连接时一起删；本机标签共用 `local` 这一份）。图标一律是星星。两个入口、同一份数据：
  顶栏星星按钮 / ⌘⇧D（`favorites.rs`，在命令片段按钮右边，本机标签也有）打开面板：搜索、↑↓ 选、↵ 进入、悬停 × 删除，搜索行右端的星星收藏 / 取消当前目录。
  在终端页（和本机标签）选中 = 往终端发 `cd`（`favorites::cd_command`，文件管理跟着终端目录走），在文件管理页选中 = 文件管理直接跳过去。
  「当前目录」：本机问前台进程，文件管理页是远程栏的目录，远程终端页后台读终端目录（`file_terminal_cwd`），读到了星星才出来。
  顶栏的两个面板（命令片段、目录收藏）互斥，状态是 `Workspace::top_panel`，按钮和弹层在 `topbar.rs` 的 `panel_button`。
  文件管理里同浏览器地址栏：远程地址栏右端的星星
  一点就收藏 / 取消当前目录（实心蓝 = 已收藏，⌘D 同）；点进地址栏，下拉列出收藏（目录名 + 灰色上级路径，当前目录标蓝），
  点了过去、悬停的 × 直接删（不用先进那个目录）、↑↓ ↵ 可选，一敲字换成子目录补全。文件管理里不另放收藏按钮 / 菜单（顶栏那个星星是全局入口）。
  地址栏的两个下拉都包在 `deferred` 里画，否则会被后面的表头和文件行盖住。
- **拖放**：应用内拖动用 GPUI 的 `on_drag` / `on_drop`（连接换组排序、标签排序、文件互拖）；Finder 拖入用 `ExternalPaths`。
- **设置**：`~/.sshl/settings.json`（字号、字体、收起的分组、侧栏收起、本地栏宽度、是否显示隐藏文件、选中即复制、右键粘贴）。首次启动从旧版 WebView 的
  localStorage 迁移。设置对话框里的改动先预览（`settings::preview`），点保存才写盘，关掉不保存会还原。
- **存储**：连接与加密凭据在 `~/.sshl/sshl.db`，密钥在 `~/.sshl/master.key`（0600）；密码明文不经过界面
  （填充密码由后端解密直接写通道）。
- **无障碍**：可点击的元素给 `role` + `aria_label`（也是无头测试按名字点击的依据）。连接项鼠标双击才连，
  键盘 / 辅助功能「按下」（`on_a11y_action(Click)`）直接连。
- **devhook 测试口**：debug 构建、或带 `--features devhook` 的构建里才编进去（`#[cfg(any(debug_assertions, feature = "devhook"))]`），
  且要设了 `SSHL_DEV_SOCKET=/tmp/xxx.sock` 才启用（`devhook.rs`）。经 Unix 套接字注入按键（`keys ctrl-c enter` /
  `type echo hi`；`mods cmd` / `mods none` 按住 / 松开 ⌘，测快捷键角标，之后的 `click` / `move` 也带着 ⌘，测 ⌘ 悬停 / 点击链接；
  测试口开着时点链接只记日志 `open link: …`、不真的开浏览器；`press X Y` / `release X Y` 把左键按下、松开分两条发，中间会重绘一帧，
  和真实点击一样——`click` 是连着发的，「按下时就把被点的东西收掉」这类毛病它测不出来，菜单、弹层的点击要用 press / release 测），走 GPUI 完整的按键分发 + 输入法通道；无头测试终端输入用，不抢用户焦点。测 release 性能时用
  `cargo build -p sshl --release --features devhook`；`scripts/bundle.sh` 的正式构建不带这个 feature。macOS 的套接字路径上限 104 字节。
- **Rust 后端规则**：core 不依赖界面；新增界面需求优先在 app 里做，确实要改 core 的行为（SFTP、sudo、known_hosts）
  要带测试。core 的集成测试用 `tests/fixtures-sudo.Dockerfile` 起的容器。
