use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::ssh::SshSession;

#[derive(Default)]
pub struct AppState {
    pub sessions: Arc<Mutex<HashMap<String, Arc<SshSession>>>>,
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
}
