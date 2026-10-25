//! The identity worker.
//!
//! Registers the identity functions and their HTTP triggers with the iii
//! engine. See `docs/api-contract.md` §3 for the endpoint contracts and
//! `docs/architecture.md` ADR-002 for why the engine is behind an HTTP
//! contract rather than in front of it.
//!
//! The configuration is read before anything else: a worker that starts with
//! the wrong secrets is worse than a worker that does not start at all.

use std::sync::Arc;

use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{InitOptions, RegisterFunction, register_worker};
use serde_json::{Value, json};
use suertudo_db::pool;

use suertudo_identity::adapters::{
    Hs256TokenSigner, LoggingSmsSender, PostgresConsentRepository, PostgresDeviceRepository,
    PostgresTokenRepository, PostgresUserRepository, RandomTokenSource, Sha256Digest, SystemClock,
};
use suertudo_identity::config::Config;
use suertudo_identity::domain::service::IdentityService;
use suertudo_identity::handlers::Deps;
use suertudo_identity::middleware::SessionResolver;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // The startup check happens here: outside development the worker refuses
    // to start when the magic link travels in the response.
    let config =
        Config::from_env().map_err(|err| anyhow::anyhow!("invalid configuration: {err}"))?;

    let pool = Arc::new(pool::connect(&config.database_url, config.pool_max_connections).await?);

    let clock = Arc::new(SystemClock);
    let signer = Arc::new(Hs256TokenSigner::new(
        config.jwt_secret.as_bytes(),
        config.jwt_refresh_secret.as_bytes(),
        clock.clone(),
    ));

    // The provider of the SMS is not decided yet: the only implementation
    // that exists writes the message to the log. The configuration refuses
    // to start the worker with it in production, and in staging it needs
    // the explicit consent of DEV_SMS_SENDER.
    let sms: Arc<dyn suertudo_identity::ports::SmsSender> = match config.sms_provider {
        suertudo_identity::config::SmsProvider::Logging => {
            // Loud, on purpose: the codes of the verifications end up in the
            // log of the worker, and whoever reads the log can use them.
            tracing::warn!(
                environment = %config.environment.as_str(),
                "the SMS sender of development is in use: the codes of the device verifications go to the log of the worker"
            );
            Arc::new(LoggingSmsSender)
        }
    };

    let service = Arc::new(IdentityService {
        users: Arc::new(PostgresUserRepository::new(pool.clone())),
        tokens: Arc::new(PostgresTokenRepository::new(pool.clone())),
        devices: Arc::new(PostgresDeviceRepository::new(pool.clone())),
        consents: Arc::new(PostgresConsentRepository::new(pool.clone())),
        clock: clock.clone(),
        token_source: Arc::new(RandomTokenSource),
        digest: Arc::new(Sha256Digest),
        signer: signer.clone(),
        sms,
    });

    // The store of the counters of the rate limits. The connection is
    // eager: **a worker that cannot reach the store does not start** — a
    // rate limit that cannot be consulted is not a rate limit, and a
    // worker that serves traffic without it is the open door the limit
    // was there to close.
    let limiter = suertudo_api::adapters::redis::RedisRateLimiter::connect(&config.redis_url)
        .await
        .map_err(|error| -> anyhow::Error {
            anyhow::anyhow!("REDIS_URL: the store of the rate limits is not reachable: {error}")
        })?;
    tracing::info!(url = %config.redis_url, "the store of the rate limits is up");

    let deps = Arc::new(Deps {
        service,
        resolver: SessionResolver::new(signer),
        dev_magic_link: config.dev_magic_link,
        limiter: Arc::new(limiter),
    });

    let url = std::env::var("III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".to_string());
    let namespace = std::env::var("III_NAMESPACE").unwrap_or_else(|_| "suertudo".to_string());

    let worker = register_worker(
        &url,
        InitOptions {
            namespace: Some(namespace),
            ..Default::default()
        },
    );

    suertudo_identity::handlers::register(&worker, &deps)?;

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

    tracing::info!(%url, environment = config.environment.as_str(), "suertudo-identity registered");
    // A worker that returns from main dies: the threads of the runtime do
    // not keep the process alive (AGENTS.md §5.8).
    std::thread::park();
    Ok(())
}
