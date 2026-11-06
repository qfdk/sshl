# vendor/russh

crates.io 上的 russh **0.45.0** 原样拷贝，再加 `russh-manual-window.patch`（`patch -p1 -d russh`）。
由 `src-tauri/Cargo.toml` 的 `[patch.crates-io]` 接入；russh-keys / russh-sftp 仍来自 crates.io。

## 为什么要补丁

russh 收到 CHANNEL_DATA 时**先补接收窗口**（`client/encrypted.rs` CHANNEL_DATA 分支 →
`session.rs` `adjust_window_size`：剩余低于 target/2 就补满），再把数据塞进每通道的
unbounded 队列。窗口与消费者是否读走无关，所以终端刷屏（`cat` 大文件、`yes`）时，
数据要么堆在 WebView / xterm（超 50MB 直接丢），要么堆在 russh 队列里，^C 后积压还得放完。
在 `Handler::data` 里 await 能挡住，但挡的是整条连接的读循环：同连接的 SFTP、开通道、
keepalive 全停（实测渲染端卡住时开通道 5s 超时）。上游 0.65 也一样，只是改成有界队列满了
挡整条 TCP。要只让终端通道停下，只能按通道控制窗口。

## 补丁内容（只对显式开启的通道生效，其余通道行为不变）

- `Handle::channel_open_session_manual_window(window_size)`：开 session 通道，初始接收窗口
  用这里给的值（不用 `Config::window_size`），且数据到达时不再自动补窗口。
- `Channel::grant_window(amount)`：应用消费完数据后发 CHANNEL_WINDOW_ADJUST。
  只对手动窗口、已确认的通道生效；累计不超过 u32::MAX。
- 实现：`ChannelParams.manual_window`；`Encrypted::adjust_window_size` 对其只扣不补；
  `Encrypted::{set_manual_window, grant_window}`；`ChannelMsg::GrantWindow`；
  `client::Msg::ChannelOpenSessionManualWindow`；`channel_open_generic` 增加 `window_size` 参数。

## 升级 russh 时

重新拉对应版本源码，按 `russh-manual-window.patch` 的意图移植（0.65 的对应位置：
`session.rs` `adjust_window_size`、`client/encrypted.rs` CHANNEL_DATA 分支、`client/mod.rs`
`handle_msg`），再跑 `src-tauri/src/ssh.rs` 里的 `terminal_window_against_real_sshd` 集成测试。
vendor 之后拿不到 0.45.x 的补丁版本，安全公告需自行关注。
