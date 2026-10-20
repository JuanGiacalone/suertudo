use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{InitOptions, RegisterFunction, register_worker};
use serde_json::{json, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".to_string());
    let worker = register_worker(
        &url,
        InitOptions {
            namespace: Some("spike".into()),
            ..Default::default()
        },
    );

    worker.register_function(
        "health::check",
        RegisterFunction::new(|_payload: Value| {
            Ok(json!({
                "status_code": 200,
                "headers": { "content-type": "application/json" },
                "body": {
                    "status": "ok",
                    "spike": "iii-health",
                    "isolation": "process"
                }
            }))
        }),
    );

    worker.register_trigger(RegisterTriggerInput {
        trigger_type: "http".into(),
        function_id: "health::check".into(),
        config: json!({
            "api_path": "/v1/health",
            "http_method": "GET"
        }),
        metadata: None,
        namespace: None,
        trigger_namespace: None,
    })?;

    eprintln!("iii-health-spike registered health::check on {url}");
    std::thread::park();
    Ok(())
}
