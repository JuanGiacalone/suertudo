//! Authorization middleware.
//!
//! The middleware runs before every function invocation. It resolves the
//! session, checks the role and **injects the tenant context** into the
//! handler payload. Handlers never read the tenant from the request.
//!
//! `TSK-F0-04` implements the session resolution; until then this module
//! exposes the context type and the injection helper it will use.

use serde_json::{Value, json};
use uuid::Uuid;

/// The identity of the caller, resolved by the middleware.
#[derive(Debug, Clone, Copy)]
pub struct AuthContext {
    pub user_id: Uuid,
    pub business_id: Option<Uuid>,
}

/// The key the middleware uses to inject the context into the payload.
pub const CONTEXT_KEY: &str = "__auth_context";

/// Wraps a handler payload with the resolved context.
///
/// The context is injected under a reserved key so that a handler cannot
/// mistake it for user input: the payload the caller sent never contains
/// that key, and the middleware overwrites it if it somehow does.
pub fn inject(payload: Value, ctx: AuthContext) -> Value {
    let mut payload = payload;
    if let Value::Object(ref mut map) = payload {
        map.insert(
            CONTEXT_KEY.to_string(),
            json!({
                "user_id": ctx.user_id,
                "business_id": ctx.business_id,
            }),
        );
    }
    payload
}

/// Reads the context back out of a payload.
pub fn extract(payload: &Value) -> Option<AuthContext> {
    let raw = payload.get(CONTEXT_KEY)?;
    let user_id = Uuid::parse_str(raw.get("user_id")?.as_str()?).ok()?;
    let business_id = raw
        .get("business_id")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok());
    Some(AuthContext {
        user_id,
        business_id,
    })
}
