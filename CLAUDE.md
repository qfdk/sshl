# CLAUDE.md

Guidance for Claude Code when working in this repo.

## Project Overview

原生 macOS SSH 客户端：终端 + SFTP 文件管理。界面是 GPUI（Zed 的 GPU 界面框架），终端模拟是
libghostty-vt（Ghostty 内核），后端是 Rust（russh + russh-sftp）。不用 WebView / Tauri / JS。

## Layout

```
Cargo.toml            workspace（版本号在 [workspace.package]）
rust-toolchain.toml   Rust 1.100.0（只在本仓库固定）
crates/core/          sshl-core：SSH / SFTP / 存储后端，不依赖界面框架
crates/app/           sshl：GPUI 应用（工作区、侧栏、终端、文件管理、对话框）
  src/terminal/       终端：engine.rs（终端线程）、frame.rs（帧快照）、view.rs（绘制 + 输入法）、keys.rs
  src/files/          文件管理（本地 / 远程两栏、传输、跟随终端用户）：view.rs 只放状态和整体布局，做法按功能分在 view/ 下
assets/               内置字体、Lucide 图标（线宽 2.5）、服务端软件、AI 工具和系统的标志（icons/brands，Simple Icons 16.36.0，Redis 用 12.0.0 的经典标志，OpenAI 来自 gilbarbara/logos；大多 CC0，个别另有许可，逐个记在 icons/brands/LICENSE；几个系统的官方彩色版在 icons/brands/color/，许可另见那里的 LICENSE）、应用图标、Info.plist 模板
vendor/russh/         russh 0.45 + 手动接收窗口补丁（见 vendor/README.md）
scripts/              bundle.sh（构建 SSHL.app）、deploy.sh（本地安装）、release.sh（签名 + 公证的发布包）
tests/                fixtures-sudo.Dockerfile：sshl-core 集成测试用的 sshd 容器
```

## Build & Run

```bash
brew install zig                   # Zig 0.17：libghostty-vt 构建用
cargo run -p sshl                  # 开发版（debug；libghostty-vt 也是 Zig Debug，刷屏很慢，测性能用 release）
cargo test -p sshl-core            # 后端单元测试（集成测试 #[ignore]，要 SSHL_SFTP_IT / SSHL_SUDO_IT 指向测试容器）
cargo test -p sshl                 # 界面纯逻辑测试（分组、帧、键位、设置迁移、文件管理逻辑）+ 提示框的键盘交互（gpui 的 test-support，只在测试里开）
python3 tests/ui/shortcuts.py     # 快捷键 × 状态矩阵（真实按键投给后台的测试实例，不抢焦点；要先 build --release --features devhook，SSHL_UI_SSH=<已存的连接> 加跑 SSH 的）
cargo clippy --workspace --all-targets   # 自己的 crate 保持零警告（vendor/russh 的 3 条不改：一条没用的 import、两条弃用的 std::u32::MAX / u8::MAX，补丁保持最小）
scripts/bundle.sh                  # release 构建 + 组装 target/release/bundle/SSHL.app（ad-hoc 签名）
scripts/deploy.sh                  # 装到 /Applications（Developer ID 重签 + 自定义图标），先跑 bundle.sh
NOTARY_PROFILE=<配置名> scripts/release.sh   # 发布包：Developer ID 签名 + 公证 + 装订，产物 target/release/SSHL-v<版本>-macos.zip
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
  ⌘Q 开着标签时先确认（`Workspace::request_quit`，用户手滑按到过，所有会话一起没了），没有标签直接退。按住 ⌘Q 会连发：确认框开着时后面几次不叠框、也不算确认。
  这个动作要由工作区的元素接（`.on_action`），不能在全局处理函数里直接 `window.update`：按键分发途中窗口正被占着，拿不到。
  没有主窗口时（⌘M 最小化了）菜单的 ⌘Q 没人接、落到 `main.rs` 的全局处理：它 `cx.defer` 等这一轮完了再进窗口，把窗口拿出来交给 `request_quit` 问，
  不能直接退（以前就是直接退，最小化后手滑按 ⌘Q 所有会话不声不响全没了）。已经有提示框开着时 ⌘Q 不再排一个「退出」在后面。
- **终端线程模型**：libghostty-vt 的类型都是 `!Send`，每个会话一个终端线程独占它（`terminal/engine.rs`）。
  SSH 输出、键鼠、改尺寸都以 `Input` 消息进来；界面只拿 `Frame` 快照来画，行是 `Arc<Row>`，没变的行沿用。
  每轮先处理控制消息（按键、^C 不排在积压输出后面），输出按 64KB 切片解析，刷屏时最多 16ms 出一帧。
  同步输出（DEC 2026，Claude Code / Ink、neovim 重画时用；libghostty 对 DECRQM 2026 答「支持」，Claude Code 据此启用）：
  `on_render_hold` 回调在 hold 开始时（vt_write 中途）就把当时完整的一帧拼好（`Painter`），hold 期间只显示这一帧，
  结束后按最新的出帧，最长 1s 超时。不认的话擦掉重写的中间状态会被画出来，整块闪。
  键盘 / 鼠标 / 粘贴 / 焦点的编码都交给 libghostty（应用光标键、Kitty 键盘协议、括号粘贴）；
  普通字符走 GPUI 的输入法通道（`EntityInputHandler`），中文输入法组字画在光标处。
  组字里没有可见字符就不画（`visible_composition`）：鼠须管把拼音显示在候选框里时，只往组字里塞一个占位（官方版全角空格、
  去下划线补丁版零宽空格），画出来只是光标边上一截下划线（零宽文字 GPUI 还会往左补半个字宽的下划线）。组字状态照旧，
  `key_down` 看到有组字就不把按键发给远端，所以占位不能当成没有组字。
  粘贴图片（⌘V / 右键粘贴，`view.rs` 的 `clipboard_paste`）：剪贴板里没有文字但有图片时发一次空粘贴——开了括号粘贴的程序收到一对空括号，
  Claude Code 在 macOS 上据此自己去读剪贴板里的图（同它的 Ctrl+V）；图片不经过终端，没开括号粘贴的程序什么都收不到。只对本机终端有用，
  SSH 那头的程序读不到这台机器的剪贴板。
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
  所以悬停时只给纯文本网址画下划线。点击时 OSC 8 优先（程序明说的目标；显示的字可能是截断的或另一个网址），没有才用纯文本的。只开 http / https / mailto（`frame::openable`），file: / javascript: 之类一律不开，ftp: 也不开（访达会去挂载对方的服务器）。
  OSC 8 的目标悬停时看不到：屏幕上显示的是一个站的网址、超链接却指向另一个站（`frame::disguised`，比主机）时先弹框给用户看真正的目标；同一个站（显示的是截断的网址）直接开。
  例外是程序把网址后面的字也括进了超链接（`frame::link_click` → `LogicalLine::swallowed_by`）：Claude Code 给回复里的网址自动挂超链接时一直括到下一个空白，
  中文句子里网址后面直接接「，」「。」，超链接就成了 `https://a.net，我先…`（以前弹「指向别处」，点「仍然打开」浏览器开出 `xn--net,,-…` 乱码域名，用户报过）。
  超链接 = 屏幕上的网址 + 屏幕上紧跟着的字、多出来的那段从标点 / 空白开头时，开屏幕上的网址（点在被括进去的字上也是）；
  多出来的从字开头的（路径里直接写中文的维基百科网址）照旧开超链接。中文字的右半边是占位格，点在上面算点在这个字上（`LogicalLine::at`）。
  SSHL 的本机终端里（`TERM_PROGRAM=SSHL`）Claude Code 2.1.326 照样给回复里的网址和状态栏的「/rc」挂超链接（实测录下的输出）。
  ⌘+（⌘=）/ ⌘- / ⌘0 放大 / 缩小 / 回默认字号：改的是全局字号并存盘（`settings::update`，所有终端一起变）。
- **GNU screen 不进备用屏**（`terminal/gnu_screen.rs`，效果同 `~/.screenrc` 里的 `termcapinfo xterm* ti@:te@`，服务器上不用配）：
  screen 每次连上（启动、`screen -r`、挂起后 fg）都先发 is2（`\e[!p\e[?3;4l\e[4l\e>`）再紧跟 smcup，认出连着的这一整串就把 smcup 吞掉，之后它的 rmcup 也吞掉。
  只认整串、不只看 DECSTR：别的程序先软复位再进备用屏会被认错，跑在主屏上冲掉回滚历史；认漏了只是照旧进备用屏。
  screen 留在主屏上画，滚出去的行进回滚历史（它在从第一行开始的滚动区底边换行，最后一行是 hardstatus 时 libghostty 也照样存），滚轮、⌘F 照常用。
  不这样的话：备用屏没有回滚历史，滚轮在备用屏里又换成方向键（less / man 靠它翻页），发给 screen 窗口里的程序——
  脚本在跑时被 tty 回显成一串 `^[[B`，还真进了它的输入（用户报过）。vim / less 的 smcup 前面没有 is2，照常进备用屏。
  被切在两片输出之间的半截序列留到下一片再判断；断线原地重连时状态清掉。副作用同 ti@:te@：screen 退出 / 断开后它的画面留在屏幕上，
  screen 里切窗口时几个窗口的内容混在回滚历史里。tmux 不管：`set -g mouse on` 之后滚轮走鼠标上报，tmux 自己翻。
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
  方块字符（U+2580–U+259F：▀ ▄ █ ▌ ▐ ▛ ▜ ░ ▒ ▓…）不用字体画，按格子自己涂矩形（`terminal/blocks.rs`，同 Ghostty 的 sprite font）：
  字体里 █ 的宽是字体自己的字宽，比按设备像素取整的格子窄一点，相邻两格之间漏一条缝——Claude Code 标志（▐▛███▜▌）上的竖线，用户报过，
  Ghostty 用同一套字体没有。░ ▒ ▓ 画成整格 25% / 50% / 75% 不透明（同 Ghostty），不是字体里的网点。画框线（─ │ ┼）还是字体画的。
- **终端配色**（设置 →「终端」页的「配色」，`settings.terminal_theme` 存主题 id，默认 `carbonfox`）：内置主题在 `theme.rs` 的 `term::THEMES`
  （`term::Theme`：底色、前景、光标、选区底 / 字、16 色；`term::find` 查不到退回默认，存的 id 不改写）。默认的 Carbonfox 就是原来写死的那套
  （用户 Ghostty 配置），颜色一个都不能变（有测试盯着）。其他主题的颜色取自官方配置，出处写在每套上面；只有两处为了看得清偏离官方：
  Solarized Light 的前景 / 光标用 base01（官方 base00 对底色只有 4.1:1）、Catppuccin Latte 的光标用 Text（官方 Rosewater 只有 2.3:1）。
  测试要求每套前景对底色 ≥ 4.5:1、光标 ≥ 3:1。搜索高亮三个颜色不属于主题，是常量。
  换配色：`TerminalView` 用 `observe_global::<Settings>` 盯着（预览、保存、关对话框还原都会触发），id 变了就给终端线程发
  `Input::Theme(&'static Theme)`，不在前台的标签也发；新终端在 `engine::spawn` 时就拿当前主题。终端线程的 `apply_theme` 交给 libghostty
  改默认前景 / 背景 / 光标和调色板前 16 色（程序用 OSC 改过的照留），libghostty 因此把整屏标脏，下一帧所有行重建：格子里存的是
  「默认色 / 第几号色」，屏幕上和回滚历史里的字都跟着变，真彩色不变。选区色不在 libghostty 里，`Painter.theme` 跟着换。
  终端没出第一帧时的底色、没会话时的空页面、设置里的字体预览也跟着配色；输入法组字用帧里的 `Frame.foreground` / `background`。
- **界面外观**（设置 →「外观」页：跟随系统 / 浅色 / 深色，`settings.appearance`，默认跟随系统）：和终端配色是两回事，只管侧栏、顶栏、文件管理、系统页、对话框。
  `theme::ui` 里每个颜色角色是一个 `Token`（亮、深两个值），`theme::color` / `alpha` 画的时候按当前外观取；写死的 `0xRRGGBB` 也能传，不跟着变
  （只用在跟着终端配色走的地方，比如空页面的提示字）。新颜色一律加成 `Token`，别在界面代码里写死十六进制。
  深色的层次是反的：窗口底（`SHELL`）最暗，卡片亮一点，浮起来的东西再亮一点——所以分段控件的选中项用 `RAISED`、描边按钮用 `CONTROL` / `CONTROL_BORDER`，
  不是 `CARD`（深色下 `CARD` 比槽还暗；描边按钮用 `CARD` 的话在深底上只剩一块暗影，用户看着比旁边的主按钮小）。气泡提示用 `TOOLTIP`。
  当前是不是深色记在 `theme` 的一个静态开关里，`workspace::sync_appearance` 在启动、设置变了（预览 / 保存 / 还原）、系统外观变了时重新定，
  变了就换掉 gpui-base 的 token（输入框的字、占位字、光标从那里取）并重画所有窗口。测试盯着两种外观下的文字对比度。
  devhook 的 `appearance dark|light|system` 只改内存里的外观，看深色界面用；注意别的操作触发 `settings::update` 时会把它一起写盘，测完检查设置文件。
- **多语言**（简体中文 / English；设置 →「外观」页的「语言」，`settings.language`，默认跟随系统「语言与地区」里排在前面、我们又有的那个，都没有用英文）：
  代码里的界面文字就写中文原文，包一层 `tr!("保存")`（带参数 `tr!("删除 {} 个项目", n)`，只认 `{}` / `{0}`，要 `{:.1}` 这类格式先自己格式化好再传）；
  当前语言不是中文时按原文查译文表，表里没有就显示原文。机制在 `sshl_core::i18n`（core 的报错文案也要译，所以放在 core；不依赖界面框架），
  译文表按语言分文件：后端的 `crates/core/src/i18n/en.rs`、界面的 `crates/app/src/i18n_en.rs`。加新文字 = 写 `tr!` + 在表里加一行；
  `i18n_en.rs` 的测试扫全部源码，缺译文、参数个数对不上、表里有用不到的都会失败。加一门语言 = 加一份表 + `Lang` 一个值 + 设置里一个选项。
  同一个词在不同地方译法不同（「端口」是列名 Port、是标签页 Ports）用 `trc!("标签页", "端口")`，表里的键是 `端口|标签页`。
  英文单复数写在译文里：`{0} hidden {0?file|files}`。别把半句话拼起来（「正在」+ 动词 + 名字）：语序各语言不同，整句写成一个模板。
  文字一律在画的时候取，别把译好的字存进状态（换语言后不跟着变）：存原因（`files/view.rs` 的 `RemoteEmpty`）或原始数据。
  建的时候就定死的两处要自己跟：输入框占位字用 `ui::input(|| tr!("…"))`（盯着设置，语言变了重设）；菜单栏在 `settings::replace` 里重建（`set_menus`）。
  `settings::replace` 先换语言再换设置，盯着设置的回调里 `tr!` 已经是新语言。语言名（简体中文 / English）用它自己的文字写，不翻译。
  测试里语言是全局的、默认中文：断言中文文案的测试照旧；测英文用 `i18n::translate(Lang::En, …)`，不要去改全局语言（测试并行）。
  系统的选文件面板等跟系统语言走（`Info.plist` 的 `CFBundleLocalizations`），不跟应用里的设置。
  英文比中文长：定宽的列头、一行里带说明的勾选项都在英文下看过一遍（文件管理的权限列头用 Mode）。devhook 的 `language zh|en|system` 只改内存里的语言，看英文界面用。
- **内置字体**：JetBrainsMono Nerd Font（非 Mono 版，图标是正常大小；Mono 版把图标压成一格，用户嫌不好看）两个字重 `include_bytes` 进二进制并在启动时注册；不要换成系统回退。
- **图标**：`ui::icon` 画 Lucide SVG，不传颜色时用继承的文字颜色（currentColor）。gpui 自带 `svg()` 只认自身的
  text_color、不继承，别直接用。图标在 `assets.rs` 的 `icons!` 表里登记。
  位图（`img()`）是双线性采样、没有 mipmap，大图缩小会发毛：侧栏标志用按 2 倍屏缩好的 `assets/sidebar-logo.png`（60px 画 30pt）。
- **字不截断**（用户 2026-11-18：「不要截断不要截断」）：界面上的字一律写全，不用 `truncate()`、不出「…」。放不下就在自己那一格里折行，行 / 卡片跟着长高
  （写死的 `.h(px(N))` 改成 `.min_h`）；按钮式的东西（标签、分区按钮）不折行，按内容变宽，所在的那一排本身会换行；面包屑放不下换到下一行。
  短值不从中间拆：数字和单位之间是不断行空格（`format_bytes` / `format_file_size` 用 U+00A0，折行时不会拆成「3.3」「GB」），
  两格并排时短的那格（地址、用户名）`flex_shrink_0` + 不换行，长的那格去折；指标卡片的数值按整段（负载的三个数各一段）换行。
  实在太长又不重要的（远程进程的完整命令行，可能几千字）整段放得下才显示、放不下就不显示，悬停看全文；小地方（矩形树图的块）先量字宽再决定，
  见磁盘页那条。表格（文件列表、进程 / 端口 / 服务、磁盘列表）因此是 gpui 的 `list`（可变行高），不是 `uniform_list`：状态用 `kit::Rows`，
  每帧 `sync(行数)`（多了少了只动末尾，滚动位置不变）、换一整张表 `reset()`；看得见的行每帧都重画重量，内容变了不用另外通知。
  系统页的行数据在 `render` 里算一次放进 `Rc`（`Table`），各行从里面取——`list` 按行调渲染函数，别在里面重新过滤排序整张表。
  气泡（`kit::tooltip`）最宽 480pt，长的折行。可编辑的输入框（地址栏）不算：内容随光标滚动。
- **圆角**：gpui 的 `overflow_hidden` 只按矩形裁，不裁圆角。卡片里贴边的有底色子元素（对话框底部按钮条、设置左栏）
  要自己加对应的 `rounded_*`。
- **提示框**：一律用 `prompts::alert / confirm / password`（应用内模态框，排队显示，回车确认、Esc 取消，关掉后焦点还给
  弹出前的地方）。不要用系统 NSAlert / `cx.prompt`：原生弹窗按钮是英文、窄、中文长句被拦腰折行，与应用风格不一。
  键盘同系统对话框：弹出时焦点就在默认按钮（关闭 / 删除 / 确定…）上；Tab / ⇧Tab 在框里的按钮（和密码框、复选框、已保存密码）之间转，
  空格或回车按下选中的那个。回车 / 空格会按下的按钮画焦点框（`kit::ring`，不管框是键盘还是鼠标点开的都画——用户：「看不出来是不是关闭」；
  gpui 的 `focus_visible` 只在上一次输入是键盘时画，鼠标一动就没了），鼠标点了框里空白（焦点到框本身）时还是默认按钮带框。
  复选框只在 Tab 选中时画（`kit::focus_ring`），空格勾它。密码框焦点在输入框里，默认按钮不画框。测试在 `prompts.rs`（GPUI 测试窗口 + 模拟按键）。
- **对话框和焦点**（用户报过「确认框键盘关不掉」：焦点不在框里，回车落在框后面）：
  - 对话框的外层用 gpui-base 的 `.focus_trap(id, &焦点句柄)`，不用 `track_focus`：Root 把 Tab 绑成切到窗口里下一个能停的地方，
    不圈住的话会切到框后面（侧栏搜索框、地址栏），框还开着、敲的字和回车却去了后面。新加对话框照这个做。
  - 浮层面板（⌘K、命令片段、目录收藏）和 ⌘F 搜索条不能套陷阱：鼠标按下带焦点句柄的元素时 GPUI 会把焦点给它，
    点一下面板空白处搜索框就失焦、上下键失灵。它们在自己的 `… > Input` 上下文里把 Tab / ⇧Tab 绑成上下选（搜索条是下一个 / 上一个），
    别绑 `NoAction`：那样 Tab 落进输入框，成了搜索词里的一个制表符（终端里绑 NoAction 是对的，Tab 要发给 shell）。
  - 对话框 / 提示框开着时（`Workspace::dialog_holds_focus`），工作区快捷键不动（`modal_open`：⌘W、⌘T、⌘,、切标签、切视图、开面板），
    后台挪焦点的地方（`refocus`、`activate`、`session_gone`）也不动——要挪焦点一律走 `refocus`，别直接 `window.focus` 到终端。
    以前按两下 ⌘W（或按住连发）会排几个一样的确认框，回车关掉第一个、关标签时 `session_gone` 又把焦点从第二个框上抢走，之后键盘全落空。
  - 焦点所在的东西没了（关掉的标签的终端）：`on_focus_lost` 交给 `refocus`，焦点回当前内容区。
  - 改了快捷键、对话框、面板、关标签 / 退出的逻辑，跑一遍 `tests/ui/shortcuts.py`（用户：「快捷键在各种状态下要好好测」），新状态 / 新快捷键往里加一行。
- **连接流程**（`workspace.rs`）：一个连接只开一个会话；连上才出标签，慢于 300ms 才显示连接遮罩（Esc / 取消可中止）。
  切换到已开会话是乐观切换，探活在后台，失效且用户仍停在该会话上时原地重连；用户主动断开不算失效。
  远端关闭 / 掉线时标签和屏幕留着（灰点），终端里提示按 Enter 原地重连（`TerminalView::reconnect`：新 shell 通道经
  `Input::Rebind` 接到同一个终端引擎，回滚历史不丢；
  同时把断线时正跑着的程序留下的模式复位——鼠标上报、Kitty 键盘协议、括号粘贴、备用屏、隐藏的光标、滚动区等，`engine.rs` 的 `reset_modes`，
  不然新 shell 把鼠标移动、方向键收成 `35;97;12M`、`1:1A` 这样的乱码）；侧栏双击、文件管理的「重新连接」也是原地重连。
  断开已连着的会话先确认，已断开的标签直接关。新建连接先连接、连上再保存。
- **主机密钥**（`crates/core/src/known_hosts.rs`）：信任记在 `~/.sshl/known_hosts`；`~/.ssh/known_hosts` 只读不写，那里认过的直接信任。
  没记录 / 密钥变了由界面弹确认框（`HostKeyCheck` 的 prompt），确认才写入；拒绝报 `AppError::HostKeyDeclined`，界面不再弹「连接失败」。
  预热池没人可问（prompt 为 None），没确认过的主机不预热。测试用 `HostKeyCheck::trust_any_for_tests()`，不碰开发机的记录。
  记录自己逐行读（不用 russh-keys 的 `check_known_hosts_path`）：这台主机有记录、但没有这次这把密钥，不管类型一不一样都算「变了」——
  只比同类型的话，中间人换一种密钥类型，警告就降成普通的「第一次连接」。配套地，握手时把记录里有的密钥类型排在前面
  （`ssh::host_key_algorithms`，同 OpenSSH），记录里只有 ecdsa / rsa 的老主机不会因为默认先要 ed25519 而误报。
- **认证方式**（`config_store::AuthKind`，库里 `connections.auth` 列；空 = 旧数据，按有没有私钥推断）：界面上只分密码 / 私钥，
  私钥里再选「自动」（`AuthKind::Auto`，免密）或「指定文件」（`Key`，默认 `~/.ssh/id_rsa`，路径可写 `~/`）。
  自动（`auto_auth.rs`）依次试 none、SSH Agent、私钥文件，私钥和 Agent 按 `ssh -G` 解析的 `~/.ssh/config`
  （IdentityFile / IdentityAgent / IdentitiesOnly），解析不了退到 OpenSSH 的默认私钥；失败时说明试了哪些、哪把设了密码。
  密码方式没存密码时，连接前先弹框问（只用这一次）。预热池按 `user@host:port` 复用，不区分认证方式。
- **新建 / 编辑连接里的分组**（`connection_dialog.rs` 的 `group_field`，逻辑在 `groups::group_choices` / `resolve_group`）：可以敲字的下拉。
  聚焦 / 点进去列出已有分组（顺序同侧栏，空分组也在），敲字按包含过滤，↑↓ 选、↵ 选中（没选中某行时 ↵ 照旧提交表单）、Esc 先收下拉再关对话框；
  敲的名字还没有就多一行「新建分组」，保存连接时分组自然建出来；只差大小写的归到已有分组。按在下拉上要 `prevent_default`，
  否则对话框拿走焦点、输入框一失焦下拉就收了，点击落空。
- **连接图标是服务器的系统**（用户 2026-11-21：「登录之后可以自动改图标，比如 Ubuntu 或者 Debian」）：每次连上（含原地重连，`Workspace::detect_os`）
  在后台开一个 exec 通道跑 `server_info::OS_COMMAND`（`uname -s` + os-release），`parse_os` 记成小写的 `内核` 或 `内核/发行版`
  （`linux/ubuntu`、`linux`、`darwin`、`freebsd/freebsd`；值是服务器给的，只收 os-release ID 允许的字符、32 字以内，输出里没有 `@@` 标记的不认），
  存进 `connections.os`，和记着的一样不写库；没认出来就留着原来的。存原始值、画的时候再查标志（`brands::of_os`）：认得的发行版画它的，
  认不出的 Linux（Oracle Linux、Amazon Linux：Simple Icons 里没有）画企鹅，不借 ID_LIKE（Oracle Linux 的是 fedora）；macOS 画苹果；
  NetBSD（标志是一行字，小了看不清）、还没认过的画服务器图标。改了主机或端口就清空（`config_store::upsert_connection_in`），下次连上再认。
  侧栏、⌘K、设置的分组树共用 `sidebar::connection_icon`（侧栏、⌘K 20pt——用户嫌 16pt「小了一点」，分组树 14pt；放在同样大的格子里，后面的字对齐）。
  怎么画由 `brands::os_logo` 定。浅色外观：官方标志是多色或带底形的（Ubuntu、Tux、CentOS、树莓派、AlmaLinux）画官方彩色版——Simple Icons 只有单色路径，
  Ubuntu 只剩三个小人、涂成橙色像一圈虚线（用户：「不像官方图标」）。彩色版在 `assets/icons/brands/color/`（`color_logos!` 表），用 `img()` 画：
  gpui 按 SVG 自己写的宽高的 2 倍栅格化，所以 SVG 的宽高写成 20（`COLOR_LOGO_SIZE`，有测试），2 倍屏上一像素对一像素。
  其余是单色路径涂品牌色，在这一处的几种底色（平常 / 选中 / 悬停）上对比都 ≥ 2:1 才用，否则这一处整个用正文色
  （`brands::color_on`，同一处的标志不随选中变色）。
  深色外观（夜间）下一律纯色（`Logo::Plain`，用户：「纯色模式，给夜间模式用」）：单色路径用 `kit::glyph` 画，跟着所在位置的文字颜色，
  同旁边的服务器图标（连着的是正文色，没连的淡一些）。用户确认过「没连是灰色、连上之后是白色」是对的，别改成一律白色。彩色版在深色底上不好看（Tux 的黑身子融进底色），白色的单色图形就是各家给深色底用的那一版。
  设置 →「外观」的「连接图标」（`settings.connection_icons`）选「通用图标」时一律服务器图标（用户：「有些人不想显示图标」），系统照样在后台认。
  新加一个发行版：从 Simple Icons 同一版本拿 SVG 放进 `assets/icons/brands/`、`brands!` 表登记，`brands::distro` 按 os-release 的 ID 加一行；
  官方标志是多色的，另找许可宽松的彩色版（gilbarbara/logos 是 CC0、devicon 是 MIT）放进 `color/`、`color_logos!` 登记，宽高改成 20，LICENSE 记来源。
- **快捷键可改**（设置 →「快捷键」页，`keymap.rs`，`settings.key_bindings` 只存改过的：命令 id → 快捷键列表，空列表 = 不要快捷键）：
  能改的登记在 `keymap::COMMANDS`（新建连接、本机终端、⌘K、关标签、切标签、⌘1…9、终端 / 文件管理 / 系统、面板、侧栏、设置、终端的拷贝粘贴查找字号、
  文件管理的全选 / 隐藏文件 / 收藏、退出 / 隐藏 / 最小化），默认值就是原来写死的那些；面板里的上下 / 回车 / Esc 这类导航键写死在各模块的 `key_bindings`，不开放。
  新加快捷键：能改的进 `COMMANDS`（带 id、分组、默认键、上下文），别再在模块里写死 ⌘ 组合。
  整张键位表由 `keymap::apply` 一次建好：gpui-base `init` 时绑的那些（启动时存一份）→ 能改的 → 写死的（同一层后绑的优先），设置的快捷键变了
  （`settings::replace` 里比较）或系统换了键盘布局（`on_keyboard_layout_change`）就清空重建，并重建菜单栏（菜单上的快捷键是建菜单时定的）。
  键盘布局：默认快捷键按当前布局换算（gpui 的 key equivalents，同 macOS 原生应用：法语 AZERTY 上 ⌘1 是 ⌘&，不用再按 ⇧）；
  用户录的存的是按下时 gpui 报出来的键，原样绑定、不再换算。录的时候用 `cx.intercept_keystrokes` 抢在快捷键之前截住（⌘Q / ⌘W 也录得进来）：
  要带 ⌘ / ⌃ / ⌥（F 键可以单按），Esc 取消（不关对话框），⌫ 去掉；撞了别的命令（终端和文件管理是并排的两页，同一个键各用各的，别的都算撞）先问，
  改用的话那条命令只去掉这一个键。和其他设置页一样先预览、点保存才写盘。按钮气泡（`keymap::with_shortcut`）、⌘ 角标（`keymap::hint`）、空页面提示都读实际绑定，
  别在文案里写死 ⌘T / ⌘B 这类。
- **终端里的 Tab**：gpui-base 的 Root 把 tab / shift-tab 绑成切换焦点，终端上下文里用 `NoAction` 压掉（`terminal::view::key_bindings`），
  否则补全键会把焦点跳到搜索框。
- **IPC 已不存在**：界面直接调 `sshl_core`。后端跑在独立 tokio 运行时（`backend.rs`），界面用 `backend::spawn` 在 GPUI 执行器里
  await 结果；后端事件（传输进度、连接列表变化）经 `sshl_core::events::Events` 回调转回界面线程。
- **预热池**归后端所有（`sshl_core::warm::run`），界面不发预热调用。
- **SFTP 超时**：没有单请求超时；`sftp_stall.rs` 包住通道，整整 `SFTP_STALL`（60s）一个字节都没收到才让所有等待中的请求失败，
  `sftp_for` 再以同一身份重开。下载写 `.<name>.<rand>.sshl-part`，成功后改名，失败不留半截、不覆盖原文件。
  上传同理：先写远端同目录的 `.<name>.<rand>.sshl-part`，传完删旧文件再改名（SFTP rename 不覆盖），沿用原文件权限位；
  目标是符号链接、不归自己所有、或建不了临时文件时照旧直接写目标（换名会改属主 / 把链接换成文件）。
  覆盖时临时文件建出来就带原文件的权限位（0600 的文件传的途中别人也读不到），传完在打开的句柄上补齐（fsetstat，不按路径改：
  以 root 传时别的用户能把临时文件换成符号链接）。目录列表里带 `/` 的名字一律不认（`sftp::plain_name`）：名字是服务器给的，
  `../../x`、绝对路径拼到本地路径上会写到下载目录外面。
  文件管理的传输走一个队列（`files/view/transfer.rs` 的 `enqueue`）：一次传一个，后加的排在后面；进度显示速度、剩余时间、还有几个，
  底栏和终端右下角的进度卡片都有停止（清空队列 + 停下正在传的）。目标已有同名的先问覆盖还是跳过（文件夹是合并），
  还有排队的时可勾「后面再遇到同名的也这样处理」（`prompts::confirm_with_check`），这一轮里都照办。
  传输可停止：`sftp::cancellable(flag, fut)` 用任务局部变量传停止开关，读写循环每块检查，停止走出错路径（`AppError::Cancelled`），临时文件照样清掉。
- **文件管理里改名**（`files/view/rename.rs`，规则在 `files/rename.rs`）：右键「重命名」或选中一项按 ↵（同 Finder，`files-rename`），
  名字原地换成输入框，文件只选中扩展名前面那段；↵ 确认、Esc 取消、失焦也算确认；空的 / 没改的不动，带 `/`、是 `.` `..` 的不收。
  远程走 SFTP rename（文件管理当前身份），本地 `local_fs::file_rename_local`；目标已有东西一律不覆盖、报「已被占用」
  （本地的 rename(2) 会悄悄覆盖，所以先比 inode：只改大小写在不分大小写的磁盘上是同一个文件，照改）。改名的是目录时收藏跟着挪
  （`config_store::move_bookmarks`，本地栏用本机标签那份 `LOCAL_SCOPE`）。
  ↵ 改名只绑在文件列表（`FILE_LIST_CONTEXT`）上，改名框里的 ↵ / Esc 也自己绑（`FileRename > Input`）：gpui-base 的单行输入框
  处理完回车还 `cx.propagate()`，绑在整个文件管理上的话，地址栏、对话框、改名框里按回车会顺带把选中的那项再改一次名。
- **文件管理里打开文件**（`files/view/open.rs`，规则在 `files/open.rs`）：双击文件或右键「打开」。本地的交给默认程序（同 Finder）。
  远程的先下载到 `~/Library/Caches/SSHL/open/<随机>/<原名>`（目录 0700，这次运行第一次打开时清掉上次留下的），再交给本机打开：
  文本（开头 8KB 没有零字节，含没扩展名的配置、日志）一律 `open -t` 进默认文本编辑器——`.command` 这类也只当文字打开，不会被执行；
  二进制只放行图片、PDF、音视频（`open` 给预览 / QuickTime），别的（`.jar`、`.pkg`、`.dmg`…交给默认程序会被执行、安装、挂载）拒绝打开。
  超过 50MB 先问。打开后每秒看一眼副本的修改时间，连着两轮一样（编辑器写完了）就传回原路径（`upload_with_sudo`：文件管理当前身份，
  没权限时问要不要 sudo）。传回前比服务器上的大小 + 修改时间（`sftp::file_remote_stamp`），和打开 / 上次传回时不一样就先问覆盖还是不传。
  传失败、取消、不传都算处理过这一版（不每秒重试、重复弹框），下次保存再传。会话没了就不再传回。测试口开着时只记日志 `open file: …`、不启动别的程序。
- **SFTP 通道复用**：每个 SSH 会话缓存一个 SFTP 会话共用。
- **数据目录权限**：`~/.sshl` 是 0700（`db::private_dir`，每次开库时保证），`master.key` 建出来就是 0600。
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
  下面四个列表：进程（`ps -eo`；Linux 的 procps 默认把用户名截到 8 个字加「+」（systemd-resolve → systemd+），先拿 `ps -o user:64= -p $$` 试，认得才用宽的——macOS 不认时只警告一句、照样输出缺了用户列的行，所以不能失败了再跑一遍）、监听端口（有 ss 用 ss、否则 Linux netstat -p、macOS 用 netstat -anvl、再否则 lsof）、systemd 服务（list-units + list-unit-files，
  没被载入的已停服务也列出）、磁盘（分区用量和目录占用，见下一条）。可结束进程、启停 / 重启服务、开关开机启动（停止、重启先确认）。容器管理做过又删了：用户用不到。
  看哪台机器是 `server_info::Machine`：远程会话（`Remote`）的命令走 `run_as_file_user`，切过来时先同步文件管理身份，终端 sudo -i 之后就以 root 跑；
  本机标签（`Local`）看这台 Mac，命令以 SSHL 自己的身份用 /bin/sh 跑，结束进程直接 kill(2)（`local_pid`：0 和负数是进程组，不发），
  有进程 / 端口 / 磁盘（`system_panel::views_for`，macOS 没有 systemd）。没权限（`services::needs_root`）时不只报错，
  而是提议「在终端里用 sudo 执行」，把 sudo 命令发到终端（`SystemPanelEvent::RunInTerminal`），密码在终端里输或点填充密码。
  看服务日志（journalctl -f）也发到终端。只在这页显示时每 3 秒刷新。
  外观是用户在 Claude Design 画布上从五个方案里挑的（2026-11-16，「上面 B 的卡片、下面 C 的表格」；嫌原来「太白」，A 的表格首字母方块 + 数据条又「太密」）：
  四项指标在一张卡片里，每项一段同色浅底 + 圆环（`paint_ring`，PathBuilder 画弧、两头补圆点；用量 ≥75% / ≥90% 整段换橙 / 红，`usage_color`）；
  表格铺满卡片宽、细线分行，标签页选中项实心蓝底带条数，排序列蓝字 + 箭头。进程表：用户列 root 橙、登录用户蓝（`set_machine` 的 `me`：本机 $USER、远程连接的用户名）、
  `_` 开头的系统账号淡色；CPU 0.5–5% 数字上色、≥5% 再加同色浅底（`CPU_TINT`；原来是实心色块白字，用户 2026-11-18 嫌「颜色过于重」）；内存 ≥100 MB 紫字。
  列的顺序同 Mole：进程 | 用户 | 内存 | CPU% | PID | 操作（用户：「调下顺序」）。远程：进程名加粗 + 完整命令行淡色。
  **本机的进程表按应用合（同 Mole / 活动监视器，`system_panel/apps.rs`）**：同一个 .app（最外层的，Chrome 的 Helper 算 Chrome）的进程合成一行，
  CPU、内存加起来，PID 和「结束」是主进程的（程序直接在 `Contents/MacOS` 下的），名字是访达里的显示名（`app_icons::display_name`：微信、日历），
  点一下展开看每个进程。自己不在 .app 里、由 launchd 替应用起的 XPC 服务（Docker 的虚拟机、WebContent）按系统私有的
  `responsibility_get_pid_responsible_for_pid`（dlsym 找，找不到退回只按路径）归到那个应用；终端里敲出来的程序父进程不是 launchd，照旧单独一行
  （不然 claude 算进 SSHL，SSHL 看着占 1 GB）。本机每行只有图标 + 名字，不要路径和参数（用户：「本地理论上不要这么长的路径吧」「这样的参数应该用不着了」），
  **远程按进程树合**（用户：「远程服务器理论上也可以考虑主进程还有子进程」，`apps::tree`，ps 多取一列 ppid）：一个进程和它下面同名的子孙合成一行——
  nginx 的 master 和 worker、postgres 和后台进程、php-fpm 和进程池、sshd 和登录会话；主进程是树最上面那个（PID、结束都是它），展开键 `pid:<主进程>`；
  不同名的子进程（sshd 下的 bash）单独一行。远程照旧显示完整命令行。SSHL 自己取数据的命令（带 ` @@` 分隔标记的 sh -c 和里面的 `ps -eo pid=…`）
  在 core 解析时就去掉（`server_info::is_own_probe`），本机、远程都不列。
  完整命令行在右键「复制命令」里。图标（`app_icons.rs`）：应用用系统给的图标（NSWorkspace，后台线程画成 2 倍像素的 PNG，22pt——16、20 用户都嫌小），
  Info.plist 没声明图标 / 声明的文件不在的（identityservicesd、WindowManager）系统给的是灰格子占位图，不用；不在 .app 里的画淡色线条图标：
  系统目录下的程序或 root / `_` 账号跑的是齿轮，别的是终端。不用系统的通用可执行文件图标（小尺寸下是一排黑方块）。
  远程服务器（用户问「远程的话能不能带出图标来」）没有应用图标：按进程名认出常见的服务端软件（`brands.rs`：nginx、redis、postgres、docker、
  node、python、php-fpm8.2、`*_exporter`…）画品牌标志（单色 SVG 按品牌色画，和卡片底色对比不到 2:1 的——深色下近黑的 Bun、浅色下浅黄的 Vault——用正文色）；
  认不出的同本机：系统目录下的、或不是登录用户自己跑的算系统服务（齿轮），其余终端。本机不在 .app 里的命令行程序（claude、codex、node）也用标志。
  AI 工具（用户要的）：claude、codex（OpenAI）、gemini、copilot、cursor-agent、opencode、qwen，跑模型的 ollama、vllm、Hugging Face 的 TGI、`nvidia-*`、jupyter…
  node / bun / python 跑的脚本是认得出的软件时，这一行的名字和图标用脚本的（`system_panel::known_script`：`node /usr/local/bin/gemini` 是 gemini）——
  Gemini CLI、Qwen Code 不改进程名（npm 包里查过，没设 process.title），不认脚本的话只是一个 node；认不出的脚本（`node server.js`）照旧是解释器的名字。
  名字表是照用户 10 台服务器上实际跑着的进程挑的（2026-11-21）：sshd、systemd、cron、v2ray、openresty 这些 Simple Icons 里没有，照旧齿轮；
  VMware（vmtoolsd）的标志是一行字，22pt 下看不清，不收。
  新加一个软件：从 Simple Icons 同一版本下 SVG 放进 `assets/icons/brands/`，`assets.rs` 的 `brands!` 表登记（带品牌色），`brands.rs` 的名字表加一行；
  Simple Icons 数据里这个标志带 license 字段的（不是 CC0），在 `assets/icons/brands/LICENSE` 记上许可和来源，MIT / BSD 的附原文。
  数字用界面字体的等宽数字（`tnum`），等宽字体只留给命令行和监听地址。「进程」列按进程名排（不按命令行路径；本机的应用按显示名）。
  窄窗口：指标两段一组（每段至少 `SEGMENT_MIN_WIDTH`），放不下一排四段就 2 × 2；进程名先放全（比整格还长就折行），命令行拿剩下的宽度，整段放得下才显示：
  它在一行高、能换行、超出就裁掉的格子里，前面垫一个零宽的元素，放不下时整段挤到第二行被裁掉——不会出现半截；藏起来时悬停那块空白看全文。
  进程名按 ps 的 comm 切（`ProcessInfo::program`）：macOS 的程序路径里常有空格（Chrome / Docker 的 Helper），只按第一个空格切会只剩「Google」。
  macOS（本机和远程的 Mac 一样）：端口用 `netstat -anvl`——不是 root 也带每个端口的 `进程名:pid`（lsof 只看得到自己的），名字是 16 字节的 p_comm，
  被截断的用 `ps -axo comm`（argv[0]，redis 这类会改写）里以它开头的补全；内存用 vm_stat，已用同活动监视器（匿名页 - 可清除 + 联动 + 压缩）；
  磁盘按 APFS 容器算（已用 = 总量 - 可用：/ 是只读的系统卷，它自己的已用只有十几 GB）。
  **netstat 只在进程的祖先里没有 ad-hoc 签名的程序时才列得出端口**（macOS 27 实测：同一个程序 ad-hoc 签名时一行都没有、不报错，
  Developer ID 签名后正常；读 pcblist 的 sysctl 本身对没有私有授权的进程只给空表）。装好的 SSHL（deploy.sh）和发版包都是 Developer ID 签名的；
  `cargo run`、`cargo test`、只跑 bundle.sh 的 ad-hoc 包里 netstat 是空的，命令自动退到 lsof（只有自己的进程）。验证 netstat 那条路要把二进制用
  Developer ID 签了再跑（单测见 `local_ports_of_every_user_when_signed` 的说明）。Homebrew 的 python / perl 也是 ad-hoc 的，从它们里面跑 netstat 同样是空的。
  Linux 的集成测试可以用 lima 虚拟机：`limactl start --name=sshl-test template://docker-rootful`，
  `SSHL_SYSTEMD_IT=127.0.0.1:<SSH 端口> SSHL_SYSTEMD_IT_USER=$USER SSHL_SYSTEMD_IT_KEY=~/.lima/_config/user`。
- **磁盘页**（系统页的「磁盘」标签，`system_panel/disk.rs` + core 的 `disk_usage.rs` / `disk_scan.rs`，用户 2026-11-18 要的「看服务器空间占用」，比自动更新重要）：
  用途是磁盘快满时找出哪个文件夹、哪个文件占得多。点开先看默认分区（/，没有就用得最多的那个），像 ncdu 按大小往下点；
  别的分区在顶上一排按钮切换（带使用率）。不要先列一张分区表（用户：「弄了多种目录就很奇怪」），也不按 lsblk 的物理盘分组（做过又撤了：
  用户要的是占用，不是盘的结构）。每一层子目录（来自扫描）和文件（打开那一层时现列：远程走 SFTP、本机按块算，`disk_usage::files_in`）
  放在一起从大到小（`disk::rows`），最多列 200 个最大的文件、其余合成「其余 N 个文件」，文件还没列出来时用「这一层的文件」总和代替；
  所有目录都能点进去（没有子目录也有文件）。
  宽的时候（≥ 760pt）列表右边是矩形树图（用户 2026-11-18 看了 Mole 的磁盘分析说「界面还是可以的」；`system_panel/treemap.rs`，squarified 排法）：
  面积 = 占用，目录蓝、文件紫、合起来的灰，越大颜色越深（靠数据上色，不按块轮换颜色）；不到这一层 0.4% 或排在第 40 名之后的合成「其余 N 项」；
  点目录块进去、右键同列表、写不下字的块悬停看名字和大小；鼠标停在块上列表那一行跟着亮，反过来也是（`DiskState::hovered`）。这时列表去掉用量条那一列；
  块里的字整段写得下才写（用户 2026-11-18 看到窄块里截成「Libr…」、大小折成「3.3 / GB」：「这是不可以的」）：画之前用真实字体量宽度（`treemap::label`），
  名字放不下一行就同访达在 `.` `-` `_` 空格 `/` 之后、中文字之间折行（`treemap::wrap_name`，「com.tencent.」「qq」，最多 3 行），每行不再让 gpui 折；
  没有断点的一段（「Library」、哈希）只在写字的地方宽过 96pt 时才在字中间断开，窄块里不写。大小只整段一行写，名字写不下时大小也不写，图标放不下就去掉图标。
  窄了只有列表。宽度要布局之后才知道：用 gpui 的 `container_query`（回调里 `entity.update` 拿到面板再画）。比缝还窄的块不画（内边距会把它撑出边界）。
  面包屑第一段是硬盘图标 +「整个磁盘」（`root_label`：/ 和 Mac 的数据卷；用户嫌单独一个「/」「很奇怪」），别的分区写挂载点；有扫描结果时右边是这个分区的用量条。
  还没有扫描结果时（没扫、扫描中、停了、失败）不是一大片空白（用户：「很空白，很难受」）：底下铺一张很淡的示意矩形树图，正中一张卡片
  （`disk::stage`：分区名和设备、粗用量条 + 已用 / 可用 / 总共、这一步的说明和按钮）。
  **扫描要用户点、离开就停**（用户：「扫描最好是要手动，避免让服务器宕机没资源」「切换界面要停下扫描」「避免出现死循环」）：打开页面、换分区只显示
  「开始扫描」，不自己扫；扫描中有「停止」；离开这一页（换标签 `set_view`、换到终端 / 文件管理 `set_active(false)`、换会话 `set_machine`、换分区）
  一律 `stop_scans`。停下 = `ScanProgress::stop`：本机的扫描线程拿下一个目录前收手；远程的扫描命令第一行先报进程号再 `exec` 成 du，
  `disk_usage::run_stoppable` 叫停后以同一身份 kill（只杀进程名还是 du / nice / ionice / taskpolicy 的，防进程号复用），杀的次数和等它退出的时间都有上限。
  回来时显示「扫描已停止」，失败了也只给按钮，不自己重来；停下的扫描晚回来的结果不收（`DiskState::finish_scan` / `finish_deeper` 比对是不是还在跑的那一次）。
  点进第 4 层往下的目录时补扫那一个（点进去就是要看），一次只补扫一个，离开那个目录就停；重新扫描后退到的那层要补扫也等用户点「扫描这个目录」。
  实测（lima 虚拟机，冷缓存 du 3.4s）：开始扫描 0.6s 后切走 / 点停止 / 换会话，du 都在 0.67s 时没了。分区列表来自 `df -kP`（只留块设备、网络盘和 /，去掉 tmpfs、snap 的 loop、/proc /sys /dev /run /etc、
  macOS 上 nobrowse 的卷和只读系统卷 /；同一设备挂了几处只留路径最短的）。
  用户要「速度快、别因为太大卡顿」：远程 `du -xk` 只列 4 层（`SCAN_DEPTH`；GNU 老版本只认 `--max-depth`，
  BusyBox / macOS 只认 `-d`，拿 /dev/null 试），du 照样走完整棵树（总数准），输出几万行以内；第 4 层的目录记成「边上的」（`frontier`），
  点进去只补扫那一个目录再 `merge` 回树上。子目录建树时排好；当前这一层的列表按（机器, 目录, 搜索词, 扫描版本）缓存，每帧只画看得见的行。
  本机不用 du，在 SSHL 里多线程并行走目录（`disk_scan.rs`，同 gdu / dua；共享队列后进先出、每个线程先记自己的账最后合、
  硬链接只在 nlink > 1 时查重），结果和 `du -xk -d 4` 逐个目录一致（有测试比对）。扫描线程降到 utility QoS（用户要「不要卡顿我的 UI」）：
  实测这台 Mac 数据卷 454 万文件 29.4s（普通优先级 25.8s，du 117s），扫描中主线程处理测试口命令的往返 p99 1.4ms、最长 1.5ms（没有超过一帧的），CPU 占用也比普通优先级低；
  列目录用 `getattrlistbulk`（同 Mole 的 Mac 版，用户说「mole 的扫描比较快」）：一次系统调用拿回一批目录项的名字、类型、设备、inode、链接数和占用，
  不再每项 read_dir + lstat 两次系统调用；实测 ~/Library 108 万文件 7.6–8.7s 对 10.2–12.5s（快约 1.4 倍，文件数一致）。原来的列法留作非 macOS 的后备，
  测试里两种列法逐项比对（`both_ways_of_listing_agree`）。Mole 另一半的「快」是把扫过的结果按目录修改时间缓存 7 天、再开直接显示：深处变了上层看不出来，没学。
  扫描中显示已扫多少文件、多大（`ScanProgress`）。远程服务器上跑不了我们的代码，还是 du（要并行得把扫描程序传到服务器上，没做，等用户定）。
  服务器上挂在 `ionice -c3 -t`（Linux）或 `taskpolicy -b`（macOS）+ `nice -n 19` 下跑。命令以文件管理当前身份跑（`server_info::exec`，du 读不了某些目录时退出码 1 但照样有输出，
  记下读不了几个，提示 sudo -i 后重扫）。扫描结果按（机器, 分区）留着，切走再回来不重扫；重新扫描时停在深处的话退到新树上最深的祖先再补扫。
  右键目录或文件：远程「在文件管理中打开」（文件打开它所在的目录，`SystemPanelEvent::OpenInFiles`），本机「在访达中显示」。本机也有这页（用户要的）：
  第一次扫描时 macOS 可能问「文稿」「桌面」等文件夹的访问权限，界面上提示了；不允许的算进「读不了」。
- **本机终端**（⌘N——用户开终端的第一反应是它；新建连接是 ⌘T，`crates/core/src/local_shell.rs` + `TerminalView::local`）：openpty + 用户的登录 shell（argv[0] 带 `-`，读 .zprofile），
  接口同 SSH 终端（`ShellHandle` 进、`TerminalSink` 出）。读线程按 ack 记账，积压 2MB 就停（同 SSH 终端窗口，刷屏中 ^C 约 40ms 回到提示符）；
  键盘输入单独一个写线程，程序不读输入时卡住的写不挡确认 / 改尺寸 / 关闭。`setsid` + `TIOCSCTTY` 做控制终端，开 IUTF8；
  TERM 同 SSH（xterm-256color），没有 LANG / LC_* 时按系统地区补 UTF-8 的 LANG。吞吐在 macOS PTY 本身的上限上
  （`seq` 这类逐行 write 的和 `script -q /dev/null` 一样慢，是内核开销，不是终端）。
  SSHL 自己的环境原样传给 shell，只去掉说的是「启动 SSHL 的那个程序」的变量（`local_shell::from_launcher`）：Ghostty / iTerm 的、开发测试口的、
  Claude Code 的会话标记（`CLAUDECODE`、`CLAUDE_CODE_*`、`CLAUDE_PID`、`AI_AGENT`…）。在 Claude Code 里 `open -a SSHL`（部署完重开）时
  整个会话的环境都跟进 SSHL；不去掉的话本机终端里再开的 claude 以为自己是那次会话的子会话，提示「Transcript saving is off」、不存对话记录，
  而且从这样的终端里再部署又会带进下一个 SSHL（用户反复遇到过）。
  工作区里本机标签 id 是 `local:<n>`（`Session.local`）：没有文件管理和填充密码，顶栏只切 终端 / 系统（`workspace::main_tabs_for`，⌘O 不切），
  shell 退出标签直接关；用户关本机标签（⌘W、×、右键「关闭」）一律先问（用户 2026-11-20：「只要是关闭标签都要有一个确认框」，以前只有 shell 时直接关），有程序在跑（`tcgetpgrp` ≠ shell）时说会结束它；新标签从当前本机标签前台进程的目录开始
  （`proc_pidinfo`）；从 Finder 拖进来是把转义后的路径粘贴到命令行，不是上传。命令片段的「本机」、本机的目录收藏都归属 `local`（`workspace::LOCAL_SCOPE`），所有本机标签共用。
  标签切换 ⌘⇧] / ⌘⇧[、⌃Tab / ⌃⇧Tab（`groups::cycle_tab`，两头绕回）；⌘1 … ⌘9 切到第几个标签（`groups::nth_tab`，本机标签也算，
  没那么多就不动）。点标签、⌘ 数字、⌘⇧] 都走 `Workspace::select_tab`。按住 ⌘ 时前 9 个标签角上浮出序号，视图按钮浮出 I / O / P
  （角标写的是实际绑定，去掉 ⌘；设置里改过的照改的显示）。
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
  收藏可以起名字（表里的 `name` 列，空 = 没起）：面板里悬停的铅笔把标题原地换成输入框（预填、全选），↵ / 失焦确认、Esc 取消，清空或和目录名一样 = 不起名
  （`favorites::bookmark_name`，规整同标签改名）。起了名的显示「名字 + 完整路径」，没起的「目录名 + 上一级路径」（`files::bookmark_lines`，面板和地址栏下拉共用），搜索也搜名字。
  搜索结果名字优先（`favorites::filter_bookmarks`；名字 = 列表里的标题，没起名就是目录名）：名字以第一个词开头的最前，其次名字里中的词多的，
  只在路径里中的最后（用户：搜「ct」时路径里有 projects 的几个排在 ctf 前面不对）。敲字后选中第一行，回车进的就是名字最像的。
  改名只在面板里做，地址栏下拉只显示。改名框也在 `FavoritePanel > Input` 上下文里，回车 / Esc 的动作先看是不是在改名。
  顶栏的两个面板（命令片段、目录收藏）互斥，状态是 `Workspace::top_panel`，按钮和弹层在 `topbar.rs` 的 `panel_button`。
  文件管理里同浏览器地址栏：远程地址栏右端的星星
  一点就收藏 / 取消当前目录（实心蓝 = 已收藏，⌘D 同）；点进地址栏，下拉列出收藏（目录名 + 灰色上级路径，当前目录标蓝），
  点了过去、悬停的 × 直接删（不用先进那个目录）、↑↓ ↵ 可选，一敲字换成子目录补全。文件管理里不另放收藏按钮 / 菜单（顶栏那个星星是全局入口）。
  地址栏的两个下拉都包在 `deferred` 里画，否则会被后面的表头和文件行盖住。
- **拖放**：应用内拖动用 GPUI 的 `on_drag` / `on_drop`（连接换组排序、标签排序、文件互拖）；Finder 拖入用 `ExternalPaths`。
- **设置**：`~/.sshl/settings.json`（字号、字体、收起的分组、侧栏收起、本地栏宽度、是否显示隐藏文件、选中即复制、右键粘贴、终端配色、界面外观、语言、连接图标）。首次启动从旧版 WebView 的
  localStorage 迁移。设置对话框里的改动先预览（`settings::preview`），点保存才写盘，关掉不保存会还原。
- **存储**：连接与加密凭据在 `~/.sshl/sshl.db`，密钥在 `~/.sshl/master.key`（0600）；密码明文不经过界面
  （填充密码由后端解密直接写通道）。
- **无障碍**：可点击的元素给 `role` + `aria_label`（也是无头测试按名字点击的依据）。连接项鼠标双击才连，
  键盘 / 辅助功能「按下」（`on_a11y_action(Click)`）直接连。
  终端那个元素的角色是 `Role::Terminal`（macOS 上报告成 AXTextArea）并带「设置值」动作（收到的字按粘贴发进终端）：有角色，系统看到的焦点才落在终端上
  而不是窗口上，值才算可写——按住鼠标说话的听写工具（Voilà）靠这两点认出这里能输入。不要给它加文本内容（text runs）：加了会开放选区范围，
  听写工具改走无障碍直接写入、写完核对不了，字就丢了。
  还要 `main.rs` 里的 `add_focus_forwarder_to_window_class("GPUIWindow")`：GPUI 的第一响应者是它自己的视图，无障碍树（accesskit）挂在外层的内容视图上，
  系统问「焦点元素」只问到窗口；不补转发的话焦点元素永远是 AXWindow（命中测试走视图层级，不受影响，所以只测命中会漏掉这个）。
  焦点节点只在窗口是键盘窗口时才报告：验证要把窗口切到前台再用探针问 kAXFocusedUIElement。
  无障碍树是懒建的（第一次有人问才激活，那次答的是空树、没有焦点元素，之后的帧才有真树），`main.rs` 的 `warm_accessibility` 开窗后自己问一次提前激活：
  不然每次启动后头几次按住说话都落空（「时灵时不灵」）。验证要用刚启动、还没被任何探针问过的进程，第一次问就要答 AXTextArea。
- **devhook 测试口**：debug 构建、或带 `--features devhook` 的构建里才编进去（`#[cfg(any(debug_assertions, feature = "devhook"))]`），
  且要设了 `SSHL_DEV_SOCKET=/tmp/xxx.sock` 才启用（`devhook.rs`）。经 Unix 套接字注入按键（`keys ctrl-c enter` /
  `type echo hi`；`mods cmd` / `mods none` 按住 / 松开 ⌘，测快捷键角标，之后的 `click` / `move` 也带着 ⌘，测 ⌘ 悬停 / 点击链接；
  测试口开着时点链接只记日志 `open link: …`、不真的开浏览器；`press X Y` / `release X Y` 把左键按下、松开分两条发，中间会重绘一帧，
  和真实点击一样——`click` 是连着发的，「按下时就把被点的东西收掉」这类毛病它测不出来，菜单、弹层的点击要用 press / release 测），走 GPUI 完整的按键分发 + 输入法通道；无头测试终端输入用，不抢用户焦点。测 release 性能时用
  `cargo build -p sshl --release --features devhook`；`scripts/bundle.sh` 的正式构建不带这个 feature。macOS 的套接字路径上限 104 字节。
  `SSHL_DEV_DISPLAY=secondary` 把窗口开在副屏；`SSHL_DEV_SIZE=1440x880` 按这个大小开在屏幕中间、不铺满（README 截图用的就是这个尺寸，
  英文图在 `docs/screenshots/`、中文图在 `docs/screenshots/zh/`，连的是 Docker 起的演示服务器，2 倍像素；
  演示服务器的镜像和重拍步骤在 `docs/screenshots/demo/`，每套五张：terminal / find / files / system / dark）。
- **Rust 后端规则**：core 不依赖界面；新增界面需求优先在 app 里做，确实要改 core 的行为（SFTP、sudo、known_hosts）
  要带测试。core 的集成测试用 `tests/fixtures-sudo.Dockerfile` 起的容器。
