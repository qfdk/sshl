use futures::future::AbortHandle;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::ssh::{ClientHandler, SshSession};

pub struct WarmConn {
    pub handle: Arc<Mutex<russh::client::Handle<ClientHandler>>>,
}

#[derive(Default)]
pub struct AppState {
    pub sessions: Arc<Mutex<HashMap<String, Arc<SshSession>>>>,
    pub warm_pool: Arc<Mutex<HashMap<String, WarmConn>>>,
    pending_connections: Arc<Mutex<HashMap<String, AbortHandle>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(&self, id: String, session: Arc<SshSession>) {
        self.sessions.lock().await.insert(id, session);
    }

    pub async fn get(&self, id: &str) -> Option<Arc<SshSession>> {
        self.sessions.lock().await.get(id).cloned()
    }

    pub async fn remove(&self, id: &str) -> Option<Arc<SshSession>> {
        self.sessions.lock().await.remove(id)
    }

    pub async fn pending_insert(&self, id: String, handle: AbortHandle) {
        self.pending_connections.lock().await.insert(id, handle);
    }

    pub async fn pending_remove(&self, id: &str) {
        self.pending_connections.lock().await.remove(id);
    }

    pub async fn pending_abort(&self, id: &str) -> bool {
        if let Some(handle) = self.pending_connections.lock().await.remove(id) {
            handle.abort();
            return true;
        }
        false
    }

    pub async fn warm_take(&self, key: &str) -> Option<WarmConn> {
        self.warm_pool.lock().await.remove(key)
    }

    pub async fn warm_insert(&self, key: String, conn: WarmConn) {
        self.warm_pool.lock().await.insert(key, conn);
    }

    pub async fn warm_contains(&self, key: &str) -> bool {
        self.warm_pool.lock().await.contains_key(key)
    }

    pub async fn warm_keys(&self) -> HashSet<String> {
        self.warm_pool.lock().await.keys().cloned().collect()
    }

    /// 已开会话占用的 host:port:user —— 预热池不给它们另开闲置连接。
    pub async fn active_warm_keys(&self) -> HashSet<String> {
        self.sessions.lock().await.values().map(|s| s.warm_key.clone()).collect()
    }

    /// 移出已断开的（keepalive 无响应时 russh 会关闭）和已不在保存列表里的预热连接，交给调用方断开。
    pub async fn warm_evict_unwanted(&self, saved: &HashSet<String>) -> Vec<WarmConn> {
        let mut pool = self.warm_pool.lock().await;
        let mut unwanted = Vec::new();
        for (key, conn) in pool.iter() {
            if !saved.contains(key) || conn.handle.lock().await.is_closed() {
                unwanted.push(key.clone());
            }
        }
        unwanted.into_iter().filter_map(|k| pool.remove(&k)).collect()
    }
}
