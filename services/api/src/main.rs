//! The API worker.
//!
//! Registers the HTTP surface with the iii engine. The engine is never
//! exposed to the internet: this worker is the only public entry point
//! (`docs/architecture.md` ADR-012).

use iii_sdk::{InitOptions, register_worker};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let url = std::env::var("III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".to_string());
    let namespace = std::env::var("III_NAMESPACE").unwrap_or_else(|_| "suertudo".to_string());

    let worker = register_worker(
        &url,
        InitOptions {
            namespace: Some(namespace),
            ..Default::default()
        },
    );

    suertudo_api::handlers::register(&worker)?;

    tracing::info!(%url, "suertudo-api registered");
    std::thread::park();
    Ok(())
}
