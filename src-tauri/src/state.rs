use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::ssh::{ClientHandler, SshSession};

pub struct WarmConn {
    pub handle: Arc<Mutex<russh::client::Handle<ClientHandler>>>,
    pub created_at: Instant,
}

#[derive(Default)]
pub struct AppState {
    pub sessions: Arc<Mutex<HashMap<String, Arc<SshSession>>>>,
    pub warm_pool: Arc<Mutex<HashMap<String, WarmConn>>>,
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

    pub async fn warm_take(&self, key: &str) -> Option<WarmConn> {
        self.warm_pool.lock().await.remove(key)
    }

    pub async fn warm_insert(&self, key: String, conn: WarmConn) {
        self.warm_pool.lock().await.insert(key, conn);
    }

    pub async fn warm_contains(&self, key: &str) -> bool {
        self.warm_pool.lock().await.contains_key(key)
    }

    /// Drop warm entries older than ttl; returns evicted handles for explicit disconnect.
    pub async fn warm_evict_expired(&self, ttl: Duration) -> Vec<WarmConn> {
        let mut pool = self.warm_pool.lock().await;
        let now = Instant::now();
        let expired_keys: Vec<String> = pool
            .iter()
            .filter(|(_, c)| now.duration_since(c.created_at) > ttl)
            .map(|(k, _)| k.clone())
            .collect();
        expired_keys
            .into_iter()
            .filter_map(|k| pool.remove(&k))
            .collect()
    }
}
