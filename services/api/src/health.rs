//! The health endpoint.
//!
//! `GET /v1/health` answers `200` while the worker is alive. It never
//! reveals the state of a dependency: that belongs to the admin-only
//! endpoint (`docs/api-contract.md` §6).

use iii_sdk::RegisterFunction;
use serde_json::{Value, json};

/// Builds the health handler.
pub fn handler() -> impl Fn(Value) -> Result<Value, iii_sdk::Error> + Send + Sync + 'static {
    move |_payload: Value| {
        Ok(json!({
            "status_code": 200,
            "headers": { "content-type": "application/json" },
            "body": {
                "status": "ok",
                "service": "suertudo-api"
            }
        }))
    }
}

/// Registers the health function and its HTTP trigger.
pub fn register(worker: &iii_sdk::IIIClient) -> Result<(), iii_sdk::Error> {
    worker.register_function("health::check", RegisterFunction::new(handler()));

    worker.register_trigger(iii_sdk::protocol::RegisterTriggerInput {
        trigger_type: "http".into(),
        function_id: "health::check".into(),
        config: json!({ "api_path": "/v1/health", "http_method": "GET" }),
        metadata: None,
        namespace: None,
        trigger_namespace: None,
    })?;

    Ok(())
}
