//! Queue and cron probes for the iii spike.
//!
//! Verifies the async path that `draws::execute_draw` and
//! `notifications::dispatch` depend on: publish, consume, retry, DLQ,
//! redrive, and the cron trigger.

use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{Error, RegisterFunction};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};

/// Messages consumed successfully from the `spike-jobs` topic.
static CONSUMED: AtomicU64 = AtomicU64::new(0);
/// Delivery attempts against the `spike-fail` topic, successful or not.
static FAIL_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
/// Cron ticks observed.
static CRON_TICKS: AtomicU64 = AtomicU64::new(0);
/// Deliveries to the named-queue target.
static NAMED_CONSUMED: AtomicU64 = AtomicU64::new(0);

pub const TOPIC_JOBS: &str = "spike-jobs";
pub const TOPIC_FAIL: &str = "spike-fail";
pub const NAMED_QUEUE: &str = "spike-named";

/// Register the queue and cron probes on an already-connected worker.
pub fn register(worker: &iii_sdk::IIIClient) -> Result<(), Error> {
    // A subscriber that always succeeds. Proves publish -> consume.
    worker.register_function(
        "queue::consume",
        RegisterFunction::new(|payload: Value| {
            CONSUMED.fetch_add(1, Ordering::SeqCst);
            Ok(json!({ "consumed": CONSUMED.load(Ordering::SeqCst), "payload": payload }))
        }),
    );

    // A subscriber that always fails. Proves retry -> DLQ.
    worker.register_function(
        "queue::fail_always",
        RegisterFunction::new(|_payload: Value| -> Result<Value, Error> {
            let attempt = FAIL_ATTEMPTS.fetch_add(1, Ordering::SeqCst) + 1;
            Err(Error::Handler(format!("probe failure, attempt {attempt}")))
        }),
    );

    // The target of the named function queue.
    worker.register_function(
        "queue::consume_named",
        RegisterFunction::new(|payload: Value| {
            NAMED_CONSUMED.fetch_add(1, Ordering::SeqCst);
            Ok(json!({ "named_consumed": NAMED_CONSUMED.load(Ordering::SeqCst), "payload": payload }))
        }),
    );

    // Publish helpers, callable from the CLI with `iii trigger`.
    let publisher = worker.clone();
    worker.register_function(
        "queue::publish",
        RegisterFunction::new_async(move |payload: Value| {
            let publisher = publisher.clone();
            async move {
                let topic = payload
                    .get("topic")
                    .and_then(Value::as_str)
                    .unwrap_or(TOPIC_JOBS)
                    .to_string();
                let data = payload.get("data").cloned().unwrap_or(json!({ "n": 1 }));
                publisher.trigger(TriggerRequest {
                    function_id: "iii::durable::publish".into(),
                    payload: json!({ "queue": topic, "data": data }),
                    action: None,
                    timeout_ms: None,
                }).await
            }
        }),
    );

    // Proves the named function-queue path (TriggerAction::Enqueue).
    let enqueuer = worker.clone();
    worker.register_function(
        "queue::enqueue_named",
        RegisterFunction::new_async(move |payload: Value| {
            let enqueuer = enqueuer.clone();
            async move {
                let data = payload.get("data").cloned().unwrap_or(json!({ "n": 1 }));
                enqueuer.trigger(TriggerRequest {
                    function_id: "queue::consume_named".into(),
                    payload: data,
                    action: Some(iii_sdk::TriggerAction::Enqueue {
                        queue: NAMED_QUEUE.to_string(),
                    }),
                    timeout_ms: None,
                }).await
            }
        }),
    );

    // Creates the named queue, pointing at queue::consume_named.
    let definer = worker.clone();
    worker.register_function(
        "queue::define_named",
        RegisterFunction::new_async(move |_payload: Value| {
            let definer = definer.clone();
            async move {
                definer.trigger(TriggerRequest {
                    function_id: "queue::define".into(),
                    payload: json!({
                        "queue": NAMED_QUEUE,
                        "config": {
                            "type": "standard",
                            "concurrency": 4,
                            "max_retries": 3,
                            "backoff_ms": 1000
                        }
                    }),
                    action: None,
                    timeout_ms: None,
                }).await
            }
        }),
    );

    // Counters, so the shell script can assert on real numbers.
    worker.register_function(
        "queue::stats",
        RegisterFunction::new(|_payload: Value| {
            Ok(json!({
                "consumed": CONSUMED.load(Ordering::SeqCst),
                "fail_attempts": FAIL_ATTEMPTS.load(Ordering::SeqCst),
                "cron_ticks": CRON_TICKS.load(Ordering::SeqCst),
                "named_consumed": NAMED_CONSUMED.load(Ordering::SeqCst),
            }))
        }),
    );

    // Cron trigger. The 6-field expression is sec min hour day month weekday.
    worker.register_function(
        "cron::tick",
        RegisterFunction::new(|_payload: Value| {
            let ticks = CRON_TICKS.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(json!({ "tick": ticks }))
        }),
    );

    worker.register_trigger(RegisterTriggerInput {
        trigger_type: "durable:subscriber".into(),
        function_id: "queue::consume".into(),
        config: json!({ "queue": TOPIC_JOBS, "max_retries": 3, "backoff_ms": 1000 }),
        metadata: None,
        namespace: None,
        trigger_namespace: None,
    })?;

    worker.register_trigger(RegisterTriggerInput {
        trigger_type: "durable:subscriber".into(),
        function_id: "queue::fail_always".into(),
        config: json!({ "queue": TOPIC_FAIL, "max_retries": 3, "backoff_ms": 1000 }),
        metadata: None,
        namespace: None,
        trigger_namespace: None,
    })?;

    worker.register_trigger(RegisterTriggerInput {
        trigger_type: "cron".into(),
        function_id: "cron::tick".into(),
        config: json!({ "expression": "*/2 * * * * *" }),
        metadata: None,
        namespace: None,
        trigger_namespace: None,
    })?;

    Ok(())
}
