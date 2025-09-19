use codex_gui::{bootstrap, default_service_handle};
use eyre::Result;
use std::env;

fn main() -> Result<()> {
    setup_tracing();
    if env::args().any(|arg| arg == "--dry-run-ui") {
        let handle = default_service_handle()?;
        // Быстрая проверка, что сервис поднимается без GUI.
        let sessions = handle.list_sessions()?;
        println!("dry-run sessions: {}", sessions.len());
        return Ok(());
    }
    let handle = default_service_handle()?;
    bootstrap(handle)
}

fn setup_tracing() {
    if tracing::dispatcher::has_been_set() {
        return;
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}
