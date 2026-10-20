//! The identity worker.
//!
//! Registers the identity functions and their HTTP triggers with the iii
//! engine. See `docs/api-contract.md` §3 for the endpoint contracts and
//! `docs/architecture.md` ADR-002 for why the engine is behind an HTTP
//! contract rather than in front of it.

use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{InitOptions, RegisterFunction, register_worker};
use serde_json::{Value, json};

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

    suertudo_identity::handlers::register(&worker)?;

    worker.register_function(
        "identity::ping",
        RegisterFunction::new(|_payload: Value| Ok(json!({ "status": "ok" }))),
    );

    worker.register_trigger(RegisterTriggerInput {
        trigger_type: "http".into(),
        function_id: "identity::ping".into(),
        config: json!({ "api_path": "/v1/identity/ping", "http_method": "GET" }),
        metadata: None,
        namespace: None,
        trigger_namespace: None,
    })?;

    tracing::info!(%url, "suertudo-identity registered");
    std::thread::park();
    Ok(())
}
