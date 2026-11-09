//! 预热连接池：启动即为全部已保存连接握手、认证好放着，连接时直接开终端通道，省掉 TCP + 握手 + 认证的往返。
//! 连接池归后端所有，界面不发任何预热调用。

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use futures::StreamExt;

use crate::config_store::{self, load_secret};
use crate::known_hosts::HostKeyCheck;
use crate::ssh;
use crate::state::{self, AppState};

/// 后台维护预热池：启动即预热全部已保存连接，之后每 WARM_POOL_TICK 一轮。不返回。
pub async fn run(state: &AppState) {
    let mut failed_at = HashMap::new();
    loop {
        maintain_warm_pool(state, &mut failed_at).await;
        tokio::time::sleep(WARM_POOL_TICK).await;
    }
}

/// 每轮预热的并发握手数。串行时一台不可达主机（connect_tcp 4 次重试 ≈ 22s）
/// 会拖住排在它后面的所有主机。
const PREWARM_CONCURRENCY: usize = 8;

/// 预热池维护周期。
const WARM_POOL_TICK: Duration = Duration::from_secs(60);

/// 预热握手失败（主机不可达、密码已改）后的冷却期：不可达主机每轮都要白等 ~22s 的 TCP 重试，
/// 错误密码被反复提交还会触发 fail2ban。
const WARM_RETRY_BACKOFF: Duration = Duration::from_secs(600);

/// 本轮要补预热的连接（保持原顺序）：同一 host:port:user 只握手一次；池里已有的、正被会话占用的
/// （会话结束后下一轮再补）、冷却期内失败过的都跳过。
fn plan_prewarm<T>(
    candidates: impl IntoIterator<Item = (String, T)>,
    pooled: &HashSet<String>,
    active: &HashSet<String>,
    failed_at: &HashMap<String, Instant>,
    now: Instant,
) -> Vec<(String, T)> {
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|(key, _)| {
            seen.insert(key.clone())
                && !pooled.contains(key)
                && !active.contains(key)
                && failed_at
                    .get(key)
                    .map_or(true, |at| now.duration_since(*at) >= WARM_RETRY_BACKOFF)
        })
        .collect()
}

/// 预热连接常驻、用掉就补。预热句柄有 10s keepalive，服务端无响应时 russh 自行关闭，所以不设 TTL：
/// 每轮清掉已关闭的和已删除连接的，再给缺的补握手。以前固定 10 分钟过期且不补，
/// app 开着超过 10 分钟后每次连接都要完整握手。
async fn maintain_warm_pool(state: &AppState, failed_at: &mut HashMap<String, Instant>) {
    let conns = match config_store::connections() {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("prewarm: failed to read connections: {e}");
            return;
        }
    };
    let saved: HashSet<String> = conns
        .iter()
        .map(|c| ssh::warm_key(&c.host, c.port, &c.username))
        .collect();
    let evicted = state.warm_evict_unwanted(&saved).await;
    if !evicted.is_empty() {
        tracing::info!("warm pool: dropped {} closed or deleted connection(s)", evicted.len());
    }
    for warm in evicted {
        let _ = warm
            .handle
            .disconnect(russh::Disconnect::ByApplication, "warm pool evicted", "en")
            .await;
    }

    let now = Instant::now();
    failed_at.retain(|key, at| saved.contains(key) && now.duration_since(*at) < WARM_RETRY_BACKOFF);
    let targets = plan_prewarm(
        conns
            .into_iter()
            .map(|c| (ssh::warm_key(&c.host, c.port, &c.username), c)),
        &state.warm_keys().await,
        &state.active_warm_keys().await,
        failed_at,
        now,
    );

    let failures: Vec<String> = futures::stream::iter(targets)
        .map(|(key, c)| async move {
            // 不依赖 has_password / has_passphrase 标志（重连 re-save 曾把它误置为 false）；
            // secrets 库才是凭据真相来源，无条件按 id 读取。
            let password = load_secret(&c.id, "password");
            let passphrase = load_secret(&c.id, "passphrase");
            if password.is_none() && c.private_key.is_none() {
                return None;
            }
            match ssh::do_handshake(
                &c.host,
                c.port,
                &c.username,
                password.as_deref(),
                c.private_key.as_deref(),
                passphrase.as_deref(),
                // 用户不在场：没确认过的主机先不连，等他点连接时再问
                HostKeyCheck::user(None),
            )
            .await
            {
                Ok(handle) => {
                    tracing::info!("prewarm: {key} ready");
                    state.warm_insert(key, state::WarmConn { handle }).await;
                    None
                }
                Err(e) => {
                    tracing::warn!("prewarm: {key} failed: {e}");
                    Some(key)
                }
            }
        })
        .buffer_unordered(PREWARM_CONCURRENCY)
        .filter_map(|failed| async move { failed })
        .collect()
        .await;
    let failed_now = Instant::now();
    for key in failures {
        failed_at.insert(key, failed_now);
    }
}

#[cfg(test)]
mod tests {
    use super::{plan_prewarm, WARM_RETRY_BACKOFF};
    use std::collections::{HashMap, HashSet};
    use std::time::{Duration, Instant};

    fn keys(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn planned(plan: Vec<(String, u32)>) -> Vec<u32> {
        plan.into_iter().map(|(_, id)| id).collect()
    }

    #[test]
    fn refills_keys_that_left_the_pool() {
        let candidates = vec![("a".to_string(), 1), ("b".to_string(), 2), ("c".to_string(), 3)];
        // b 还在池里；a 被会话用掉、会话也已结束，必须再补，否则之后连 a 又要完整握手。
        let plan = plan_prewarm(candidates, &keys(&["b"]), &keys(&[]), &HashMap::new(), Instant::now());
        assert_eq!(planned(plan), [1, 3]);
    }

    #[test]
    fn skips_hosts_with_an_open_session_and_duplicate_saves() {
        let candidates = vec![
            ("a".to_string(), 1),
            ("b".to_string(), 2),
            ("a".to_string(), 3),
            ("c".to_string(), 4),
        ];
        // a 正被会话占用：不另开一条闲置连接，会话结束后下一轮再补。同一 host:port:user 只握手一次。
        let plan = plan_prewarm(candidates, &keys(&[]), &keys(&["a"]), &HashMap::new(), Instant::now());
        assert_eq!(planned(plan), [2, 4]);
    }

    #[test]
    fn failed_hosts_wait_out_the_backoff() {
        let now = Instant::now() + 2 * WARM_RETRY_BACKOFF;
        let failed_at = HashMap::from([
            ("recent".to_string(), now - Duration::from_secs(30)),
            ("old".to_string(), now - WARM_RETRY_BACKOFF),
        ]);
        let candidates = vec![("recent".to_string(), 1), ("old".to_string(), 2)];
        // 不可达主机每轮都要白等 TCP 重试；密码错了反复提交还会触发 fail2ban。
        let plan = plan_prewarm(candidates, &keys(&[]), &keys(&[]), &failed_at, now);
        assert_eq!(planned(plan), [2]);
    }
}
