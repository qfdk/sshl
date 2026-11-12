// SFTP 通道卡死检测。russh-sftp 的请求超时从请求发出起计时：分段下载、目录并发传输让几 MB 数据同时在途，
// 慢链路上排在后面的请求要等前面的数据先过完，按单个请求计时就会误超时。这里改看整条通道：
// 有请求在等、却 stall 时长内一个字节都没收到，才算卡死；只要还在收数据，排队多久都不算。
//
// russh-sftp 2.3 的读循环（client/mod.rs `run`）遇到流错误只记一条日志、接着读，只有 EOF 才退出；退出后
// 在等的请求也不会出错：它们的 oneshot 发送端留在 RawSftpSession 与读循环共享的请求表里，只能各等各的
// 请求超时，而写请求（write_nowait）和 File drop 发的 CLOSE 根本不计时，会一直挂着。所以卡死 / 通道关闭 /
// 出错后不把 EOF 交给它，而是替每个在等的请求合成一个 STATUS(SSH_FX_CONNECTION_LOST) 回复（还在初始化就合成
// VERSION，再由 open 报错），之后再发出的请求也立刻这样回复，直到会话关闭（russh-sftp 发 shutdown）才给 EOF。

use std::collections::HashSet;
use std::future::Future;
use std::io;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{ready, Context, Poll, Waker};
use std::time::Duration;

use bytes::Bytes;
use russh_sftp::client::{Config, SftpSession};
use russh_sftp::protocol::{Packet, StatusCode, Version};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::{Instant, Sleep};

/// SFTP 会话 + 它的通道是否已失效。失效后在等的和之后发出的请求都立刻报错，调用方应换一个新会话。
pub struct Sftp {
    session: SftpSession,
    failed: Arc<OnceLock<String>>,
}

impl Sftp {
    pub fn is_dead(&self) -> bool {
        self.failed.get().is_some()
    }
}

impl Deref for Sftp {
    type Target = SftpSession;

    fn deref(&self) -> &SftpSession {
        &self.session
    }
}

/// 在 `stream`（SFTP 子系统 / sftp-server 的通道）上建 SFTP 会话，通道卡死 `stall` 即判失效（见 StallGuard）。
/// russh-sftp 自己的单请求超时设为 `request_timeout_secs`，只作兜底。
pub async fn open<S>(stream: S, stall: Duration, request_timeout_secs: u64) -> Result<Sftp, russh_sftp::client::error::Error>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let guard = StallGuard::new(stream, stall);
    let failed = guard.failed.clone();
    let config = Config { request_timeout_secs, ..Config::default() };
    let session = SftpSession::new_with_config(guard, config).await?;
    // 初始化期间通道就失效了：拿到的是合成的 VERSION
    if let Some(reason) = failed.get() {
        return Err(russh_sftp::client::error::Error::UnexpectedBehavior(reason.clone()));
    }
    Ok(Sftp { session, failed })
}

const SSH_FXP_INIT: u8 = 1;
const SSH_FXP_VERSION: u8 = 2;

/// 在等回复的请求：请求 id；INIT（回复是 VERSION）没有 id，记为 None。
type Key = Option<u32>;

enum Edge {
    /// 读到一个包的包头（长度、类型、id）。
    Start(Key),
    /// 这个包的字节全部过去了。
    End(Key),
}

/// 按 SFTP 分帧（u32 长度 + 类型 + 请求 id + 其余）切分一个方向的字节流，只看包头，不改字节。
#[derive(Default)]
struct Frames {
    head: [u8; 9],
    filled: usize,
    /// 当前包包头之后还剩的字节数。
    left: usize,
    /// 当前包的 Key；长度不足 5 的畸形包为 None（只跳过，不记）。
    key: Option<Key>,
}

impl Frames {
    fn feed(&mut self, mut data: &[u8], mut on: impl FnMut(Edge)) {
        while !data.is_empty() {
            if self.left > 0 {
                let n = self.left.min(data.len());
                self.left -= n;
                data = &data[n..];
                if self.left == 0 {
                    if let Some(key) = self.key.take() {
                        on(Edge::End(key));
                    }
                }
                continue;
            }
            let want = if self.filled < 4 { 4 } else { 9 };
            let n = (want - self.filled).min(data.len());
            self.head[self.filled..self.filled + n].copy_from_slice(&data[..n]);
            self.filled += n;
            data = &data[n..];
            let len = u32::from_be_bytes([self.head[0], self.head[1], self.head[2], self.head[3]]) as usize;
            if self.filled == 4 && len < 5 {
                (self.filled, self.left, self.key) = (0, len, None);
            } else if self.filled == 9 {
                let key = match self.head[4] {
                    SSH_FXP_INIT | SSH_FXP_VERSION => None,
                    _ => Some(u32::from_be_bytes([self.head[5], self.head[6], self.head[7], self.head[8]])),
                };
                (self.filled, self.left) = (0, len - 5);
                on(Edge::Start(key));
                if self.left == 0 {
                    on(Edge::End(key));
                } else {
                    self.key = Some(key);
                }
            }
        }
    }

    /// 停在一个包的中间（已交出去一部分）。
    fn mid_frame(&self) -> bool {
        self.filled > 0 || self.left > 0
    }
}

/// 通道失效之后的状态。
struct Failure {
    /// 作废半个包时交给 russh-sftp 的错误类型。不能是 UnexpectedEof：它会让读循环退出，在等的请求就收不到合成回复了。
    kind: io::ErrorKind,
    reason: String,
    /// russh-sftp 读到一半的包要先作废（见 poll_read）。
    discard: bool,
    /// 还没交给 russh-sftp 的合成回复。
    replies: Vec<u8>,
}

/// 包在 SFTP 通道外面：记下在等回复的请求和最后一次收到数据的时刻，判定卡死并在失效后合成回复（见文件头）。
struct StallGuard<S> {
    inner: S,
    stall: Duration,
    sent: Frames,
    received: Frames,
    waiting: HashSet<Key>,
    /// 卡死计时起点：最后一次收到数据，或从没有请求在等变成有请求在等的时刻（空闲很久后发出的请求从发出起算）。
    since: Instant,
    timer: Pin<Box<Sleep>>,
    /// 读方向的 waker：写出请求后要叫醒它去挂计时器 / 送合成回复。
    reader: Option<Waker>,
    failure: Option<Failure>,
    /// russh-sftp 已关闭会话（shutdown）。
    closed: bool,
    /// 失效原因，与 Sftp 共享。
    failed: Arc<OnceLock<String>>,
}

impl<S> StallGuard<S> {
    fn new(inner: S, stall: Duration) -> Self {
        let now = Instant::now();
        Self {
            inner,
            stall,
            sent: Frames::default(),
            received: Frames::default(),
            waiting: HashSet::new(),
            since: now,
            timer: Box::pin(tokio::time::sleep_until(now + stall)),
            reader: None,
            failure: None,
            closed: false,
            failed: Arc::new(OnceLock::new()),
        }
    }

    fn fail(&mut self, kind: io::ErrorKind, reason: String) {
        if !self.closed {
            tracing::warn!("{reason}，在等的 {} 个 SFTP 请求报错返回", self.waiting.len());
        }
        let _ = self.failed.set(reason.clone());
        self.failure = Some(Failure { kind, reason, discard: self.received.mid_frame(), replies: Vec::new() });
        self.wake_reader();
    }

    fn wake_reader(&mut self) {
        if let Some(waker) = self.reader.take() {
            waker.wake();
        }
    }

    fn park_reader(&mut self, cx: &Context<'_>) {
        if !self.reader.as_ref().is_some_and(|w| w.will_wake(cx.waker())) {
            self.reader = Some(cx.waker().clone());
        }
    }

    /// 记下发出去的字节里的请求。
    fn track_sent(&mut self, data: &[u8]) {
        let (waiting, since) = (&mut self.waiting, &mut self.since);
        let mut wake = false;
        self.sent.feed(data, |edge| {
            if let Edge::Start(key) = edge {
                if waiting.is_empty() {
                    *since = Instant::now();
                    wake = true;
                }
                waiting.insert(key);
            }
        });
        if wake || (self.failure.is_some() && !self.waiting.is_empty()) {
            self.wake_reader();
        }
    }

    /// 失效后的读：先让 russh-sftp 作废读到一半的包，再交出在等请求的合成回复；会话关闭后给 EOF。
    fn poll_read_failed(&mut self, cx: &Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let Some(failure) = self.failure.as_mut() else { unreachable!() };
        if failure.discard {
            // russh-sftp 的读循环遇到非 EOF 的错误只记日志，下一轮从新的包头读起，读了一半的包随之丢弃
            failure.discard = false;
            return Poll::Ready(Err(io::Error::new(failure.kind, failure.reason.clone())));
        }
        for key in self.waiting.drain() {
            let reply = match key {
                Some(id) => Packet::status(id, StatusCode::ConnectionLost, &failure.reason, ""),
                // INIT 的回复只能是 VERSION（没有 id、带不了错误）：让初始化先返回，由 open 报错
                None => Packet::Version(Version::new()),
            };
            match Bytes::try_from(reply) {
                Ok(bytes) => failure.replies.extend_from_slice(&bytes),
                Err(e) => tracing::error!("encode SFTP reply: {e}"),
            }
        }
        if !failure.replies.is_empty() {
            let n = failure.replies.len().min(buf.remaining());
            buf.put_slice(&failure.replies[..n]);
            failure.replies.drain(..n);
            return Poll::Ready(Ok(()));
        }
        if self.closed {
            return Poll::Ready(Ok(()));
        }
        self.park_reader(cx);
        Poll::Pending
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for StallGuard<S> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if this.failure.is_none() {
            let before = buf.filled().len();
            match Pin::new(&mut this.inner).poll_read(cx, buf) {
                Poll::Ready(Ok(())) if buf.filled().len() > before => {
                    this.since = Instant::now();
                    let waiting = &mut this.waiting;
                    this.received.feed(&buf.filled()[before..], |edge| {
                        if let Edge::End(key) = edge {
                            waiting.remove(&key);
                        }
                    });
                    return Poll::Ready(Ok(()));
                }
                Poll::Ready(Ok(())) => this.fail(io::ErrorKind::ConnectionAborted, crate::tr!("SFTP 通道已关闭").into()),
                Poll::Ready(Err(e)) => this.fail(io::ErrorKind::ConnectionAborted, crate::tr!("SFTP 通道出错: {}", e)),
                Poll::Pending => {
                    this.park_reader(cx);
                    if this.waiting.is_empty() {
                        return Poll::Pending;
                    }
                    let deadline = this.since + this.stall;
                    if this.timer.deadline() != deadline {
                        this.timer.as_mut().reset(deadline);
                    }
                    if this.timer.as_mut().poll(cx).is_pending() {
                        return Poll::Pending;
                    }
                    let secs = this.stall.as_secs();
                    this.fail(io::ErrorKind::TimedOut, crate::tr!("SFTP 通道 {} 秒没有收到任何数据", secs));
                }
            }
        }
        this.poll_read_failed(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for StallGuard<S> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        // 失效后不再往通道里写：请求照收，由读方向合成回复
        let n = if this.failure.is_some() {
            buf.len()
        } else {
            match ready!(Pin::new(&mut this.inner).poll_write(cx, buf)) {
                Ok(n) => n,
                Err(e) => {
                    this.fail(io::ErrorKind::ConnectionAborted, crate::tr!("SFTP 通道出错: {}", e));
                    buf.len()
                }
            }
        };
        this.track_sent(&buf[..n]);
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.failure.is_some() {
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.closed = true;
        this.wake_reader();
        if this.failure.is_some() {
            // 尽量给远端发个 EOF 让 sftp-server 退出（russh 的通道 drop 时不会关），发不出去就算了：通道可能正卡着
            let _ = Pin::new(&mut this.inner).poll_shutdown(cx);
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh_sftp::protocol::{Data, Handle, Version};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

    const STALL: Duration = Duration::from_secs(60);

    /// 一个 SFTP 包：u32 长度 + 类型 + id + 包体。
    fn frame(kind: u8, id: u32, body: &[u8]) -> Vec<u8> {
        let mut out = ((5 + body.len()) as u32).to_be_bytes().to_vec();
        out.push(kind);
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(body);
        out
    }

    const READ: u8 = 5;
    const DATA: u8 = 103;

    fn guarded() -> (StallGuard<DuplexStream>, DuplexStream) {
        let (client, server) = tokio::io::duplex(1 << 20);
        (StallGuard::new(client, STALL), server)
    }

    /// 测试里等数据的上限（虚拟时钟）：等不到就失败，不挂住。
    const WAIT: Duration = Duration::from_secs(30 * 60);

    /// 从 guard 读出一个完整的包，返回包体（不含长度）。
    async fn read_frame(guard: &mut StallGuard<DuplexStream>) -> io::Result<Vec<u8>> {
        tokio::time::timeout(WAIT, async {
            let len = guard.read_u32().await?;
            let mut body = vec![0; len as usize];
            guard.read_exact(&mut body).await?;
            Ok(body)
        }).await.expect("等不到数据")
    }

    fn status_of(body: Vec<u8>) -> russh_sftp::protocol::Status {
        match Packet::try_from(&mut Bytes::from(body)).unwrap() {
            Packet::Status(s) => s,
            other => panic!("应当是 STATUS，实际 {other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn slow_but_steady_reply_never_stalls() {
        let (mut guard, mut server) = guarded();
        guard.write_all(&frame(READ, 7, &[0; 20])).await.unwrap();
        let reply = frame(DATA, 7, &vec![9; 997]);
        let expected = reply[4..].to_vec();
        tokio::spawn(async move {
            // 7 字节 / 5 秒：整个回复要 ~12 分钟，远超 STALL，但从没有一次停够 STALL
            for piece in reply.chunks(7) {
                tokio::time::sleep(Duration::from_secs(5)).await;
                server.write_all(piece).await.unwrap();
            }
            std::future::pending::<()>().await;
        });
        let start = Instant::now();
        assert_eq!(read_frame(&mut guard).await.unwrap(), expected);
        assert!(start.elapsed() > 10 * STALL);
        assert!(guard.failed.get().is_none());
        assert!(guard.waiting.is_empty(), "收全回复后不再有请求在等");
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_request_without_any_data_fails_after_stall() {
        let (mut guard, _server) = guarded();
        guard.write_all(&frame(READ, 7, &[0; 20])).await.unwrap();
        let start = Instant::now();
        let status = status_of(read_frame(&mut guard).await.unwrap());
        assert_eq!(start.elapsed(), STALL);
        assert_eq!((status.id, status.status_code), (7, StatusCode::ConnectionLost));
        assert!(status.error_message.contains("60 秒没有收到任何数据"), "{}", status.error_message);
        assert!(guard.failed.get().is_some());

        // 失效之后再发的请求立刻得到同样的回复
        let start = Instant::now();
        guard.write_all(&frame(READ, 8, &[0; 20])).await.unwrap();
        assert_eq!(status_of(read_frame(&mut guard).await.unwrap()).id, 8);
        assert_eq!(start.elapsed(), Duration::ZERO);

        // 会话关闭后给 EOF
        guard.shutdown().await.unwrap();
        let mut rest = Vec::new();
        assert_eq!(tokio::time::timeout(WAIT, guard.read_to_end(&mut rest)).await.expect("关闭后应 EOF").unwrap(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn idle_channel_never_stalls() {
        let (mut guard, mut server) = guarded();
        let mut byte = [0u8; 1];
        assert!(tokio::time::timeout(10 * STALL, guard.read(&mut byte)).await.is_err(), "没有请求在等，不该报错也不该有数据");
        assert!(guard.failed.get().is_none());

        // 空闲很久之后发的请求从发出起计时，不从上一次收到数据起算
        guard.write_all(&frame(READ, 1, &[0; 20])).await.unwrap();
        tokio::spawn(async move {
            tokio::time::sleep(STALL / 2).await;
            server.write_all(&frame(DATA, 1, b"ok")).await.unwrap();
            std::future::pending::<()>().await;
        });
        assert_eq!(read_frame(&mut guard).await.unwrap(), frame(DATA, 1, b"ok")[4..]);
        assert!(guard.failed.get().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn pipelined_requests_count_from_the_last_byte_received() {
        // 有一个回复到了、其余还在等：从这之后 STALL 内没数据就要报错，不能因为「最后一次读晚于最后一次写」就不计时
        let (mut guard, mut server) = guarded();
        for id in 1..=3 {
            guard.write_all(&frame(READ, id, &[0; 20])).await.unwrap();
        }
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            server.write_all(&frame(DATA, 2, b"two")).await.unwrap();
            std::future::pending::<()>().await;
        });
        let start = Instant::now();
        assert_eq!(read_frame(&mut guard).await.unwrap(), frame(DATA, 2, b"two")[4..]);
        let mut failed = vec![status_of(read_frame(&mut guard).await.unwrap()).id, status_of(read_frame(&mut guard).await.unwrap()).id];
        failed.sort();
        assert_eq!(failed, [1, 3], "只给还在等的请求合成回复");
        assert_eq!(start.elapsed(), Duration::from_secs(30) + STALL);
    }

    #[tokio::test(start_paused = true)]
    async fn half_received_reply_is_discarded_before_the_synthetic_one() {
        let (mut guard, mut server) = guarded();
        guard.write_all(&frame(READ, 5, &[0; 20])).await.unwrap();
        let reply = frame(DATA, 5, &[1; 100]);
        server.write_all(&reply[..50]).await.unwrap();
        let mut head = vec![0; 50];
        guard.read_exact(&mut head).await.unwrap();
        let start = Instant::now();
        // 读剩下的半个包：到点报错（russh-sftp 据此丢弃这半个包），之后从新的包头开始是合成的 STATUS
        let mut rest = vec![0; 50];
        let err = tokio::time::timeout(WAIT, guard.read_exact(&mut rest)).await.expect("等不到数据").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert_eq!(start.elapsed(), STALL);
        assert_eq!(status_of(read_frame(&mut guard).await.unwrap()).id, 5);
    }

    #[test]
    fn frames_split_anywhere_still_track_requests() {
        let mut stream = frame(READ, 1, &[0; 30]);
        stream.extend(frame(SSH_FXP_INIT, 3, &[]));
        stream.extend(frame(READ, 0xdead_beef, &[]));
        stream.extend([0, 0, 0, 2, 9, 9]); // 畸形短包：只跳过
        stream.extend(frame(READ, 4, &[1; 3]));
        for step in [1, 2, 3, 5, 8, stream.len()] {
            let (mut frames, mut edges) = (Frames::default(), Vec::new());
            for piece in stream.chunks(step) {
                frames.feed(piece, |edge| edges.push(match edge {
                    Edge::Start(key) => ('s', key),
                    Edge::End(key) => ('e', key),
                }));
            }
            assert_eq!(edges, [
                ('s', Some(1)), ('e', Some(1)), ('s', None), ('e', None),
                ('s', Some(0xdead_beef)), ('e', Some(0xdead_beef)), ('s', Some(4)), ('e', Some(4)),
            ], "step {step}");
            assert!(!frames.mid_frame());
        }
    }

    fn encode(packet: Packet) -> Vec<u8> {
        Bytes::try_from(packet).unwrap().to_vec()
    }

    /// 假 sftp-server：INIT 回 VERSION、OPEN 回句柄，其余请求的回复由 `reply` 给（可以是半个包或不回）；
    /// `reply` 返回 None 时关掉通道。
    async fn fake_server(mut io: DuplexStream, mut reply: impl FnMut(Packet) -> Option<Vec<u8>> + Send) {
        loop {
            let Ok(len) = io.read_u32().await else { return };
            let mut body = vec![0; len as usize];
            if io.read_exact(&mut body).await.is_err() {
                return;
            }
            let out = match Packet::try_from(&mut Bytes::from(body)).unwrap() {
                Packet::Init(_) => encode(Packet::Version(Version::new())),
                Packet::Open(open) => encode(Packet::Handle(Handle { id: open.id, handle: format!("h{}", open.id) })),
                other => match reply(other) {
                    Some(out) => out,
                    None => return,
                },
            };
            io.write_all(&out).await.unwrap();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn every_waiting_request_fails_once_the_channel_stalls() {
        let (client, server) = tokio::io::duplex(1 << 20);
        tokio::spawn(fake_server(server, |packet| Some(match packet {
            // READ 只回半个 DATA 包，之后再没有数据；别的请求都不回
            Packet::Read(read) => {
                let full = encode(Packet::Data(Data { id: read.id, data: vec![7; 1000] }));
                full[..full.len() / 2].to_vec()
            }
            _ => Vec::new(),
        })));
        let sftp = open(client, STALL, 3600).await.unwrap();
        let mut reader = sftp.open("/r").await.unwrap();
        let mut writer = sftp.create("/w").await.unwrap();
        assert!(!sftp.is_dead());

        let start = Instant::now();
        let (stat, read, write) = tokio::time::timeout(2 * STALL, async {
            tokio::join!(
                sftp.metadata("/x"),
                async { reader.read(&mut [0u8; 1000]).await },
                // 写请求 russh-sftp 不计时：没有合成回复的话 shutdown 等写确认会永远挂着
                async {
                    writer.write_all(&[1; 3000]).await?;
                    writer.shutdown().await
                },
            )
        }).await.expect("在等的请求必须出错返回，不能挂住");
        assert_eq!(start.elapsed(), STALL);
        let reason = "SFTP 通道 60 秒没有收到任何数据";
        assert!(stat.unwrap_err().to_string().contains(reason));
        assert!(read.unwrap_err().to_string().contains(reason));
        assert!(write.unwrap_err().to_string().contains(reason));
        assert!(sftp.is_dead());

        // 之后的请求立刻失败，不等兜底超时
        let start = Instant::now();
        assert!(sftp.metadata("/y").await.unwrap_err().to_string().contains(reason));
        assert_eq!(start.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn init_fails_with_the_channel_instead_of_waiting_for_the_request_timeout() {
        // 通道一开就断了：立刻失败
        let (client, server) = tokio::io::duplex(1 << 20);
        drop(server);
        let start = Instant::now();
        let err = open(client, STALL, 3600).await.err().expect("通道断了，初始化应失败");
        assert!(err.to_string().contains("SFTP 通道"), "{err}");
        assert_eq!(start.elapsed(), Duration::ZERO);

        // 服务端一直不回 VERSION：卡死判定到点就失败
        let (client, _server) = tokio::io::duplex(1 << 20);
        let start = Instant::now();
        let err = tokio::time::timeout(2 * STALL, open(client, STALL, 3600)).await
            .expect("初始化不能等到兜底超时")
            .err()
            .expect("没有 VERSION，初始化应失败");
        assert!(err.to_string().contains("60 秒没有收到任何数据"), "{err}");
        assert_eq!(start.elapsed(), STALL);
    }

    #[tokio::test(start_paused = true)]
    async fn closed_channel_fails_waiting_requests_at_once() {
        let (client, server) = tokio::io::duplex(1 << 20);
        let mut stats = 0;
        tokio::spawn(fake_server(server, move |packet| match packet {
            // 第一个 STAT 不回，第二个到了就关通道（sftp-server 退出、连接断开）
            Packet::Stat(_) => {
                stats += 1;
                (stats == 1).then(Vec::new)
            }
            _ => Some(Vec::new()),
        }));
        let sftp = open(client, STALL, 3600).await.unwrap();
        let start = Instant::now();
        let (first, second) = tokio::join!(sftp.metadata("/a"), sftp.metadata("/b"));
        assert_eq!(start.elapsed(), Duration::ZERO, "通道关了，在等的请求不能等到卡死判定或兜底超时");
        assert!(first.unwrap_err().to_string().contains("SFTP 通道已关闭"));
        assert!(second.is_err());
        assert!(sftp.is_dead());
        assert!(sftp.metadata("/c").await.is_err());
        assert_eq!(start.elapsed(), Duration::ZERO);
    }
}
