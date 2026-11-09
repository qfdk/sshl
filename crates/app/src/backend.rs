//! 后端（sshl-core）跑在独立的 tokio 运行时上；界面在 GPUI 的执行器里 await 它的结果。

use std::future::Future;
use std::sync::OnceLock;

use futures::channel::oneshot;
use sshl_core::events::{CoreEvent, Events};
use sshl_core::state::AppState;

pub struct Backend {
    pub runtime: tokio::runtime::Runtime,
    pub state: AppState,
    pub events: Events,
}

static BACKEND: OnceLock<Backend> = OnceLock::new();

/// 启动后端：tokio 运行时 + 预热连接池。`on_event` 在后端线程上调用。
pub fn init(on_event: impl Fn(CoreEvent) + Send + Sync + 'static) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name("sshl-backend")
        .enable_all()
        .build()
        .expect("start tokio runtime");
    let backend = Backend { runtime, state: AppState::new(), events: Events::new(on_event) };
    if BACKEND.set(backend).is_err() {
        panic!("backend initialized twice");
    }
    let backend = get();
    backend.runtime.spawn(sshl_core::warm::run(&backend.state));
}

pub fn get() -> &'static Backend {
    BACKEND.get().expect("backend not initialized")
}

/// 在后端运行时上跑一个任务，返回可以在界面执行器里 await 的结果。
pub fn spawn<F>(future: F) -> impl Future<Output = Option<F::Output>>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    get().runtime.spawn(async move {
        let _ = tx.send(future.await);
    });
    async move { rx.await.ok() }
}
