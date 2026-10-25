//! The HTTP surface of the identity: how a request is read and how an answer
//! is written.
//!
//! The shape of the answers is the one of `docs/api-contract.md` §1.3: a
//! success carries `data`, a failure carries `error` with a code of the
//! catalogue.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::domain::errors::IdentityError;

/// The request as the handlers see it.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// The body of the request, already parsed.
    pub body: Value,
    pub headers: HashMap<String, String>,
    pub path_params: HashMap<String, String>,
    pub query_params: HashMap<String, String>,
    pub method: String,
}

impl Request {
    /// Reads a request out of the payload of a function.
    ///
    /// The payload of an HTTP trigger carries the body, the headers and the
    /// path parameters. A function called by another worker receives the
    /// body directly, and the two are told apart by the presence of the
    /// transport fields.
    pub fn parse(payload: &Value) -> Self {
        let is_http = payload.get("headers").is_some()
            || payload.get("method").is_some()
            || payload.get("path").is_some();

        let body = if is_http {
            payload.get("body").cloned().unwrap_or(Value::Null)
        } else {
            payload.clone()
        };

        Self {
            body,
            headers: string_map(payload.get("headers")),
            path_params: string_map(payload.get("path_params")),
            query_params: string_map(payload.get("query_params")),
            method: payload
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }
    }

    /// The value of a field of the body.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.body.get(name).and_then(Value::as_str)
    }

    /// The value of a path parameter.
    pub fn path_param(&self, name: &str) -> Option<&str> {
        self.path_params.get(name).map(String::as_str)
    }

    /// The bearer token of the `Authorization` header.
    pub fn bearer_token(&self) -> Option<&str> {
        let header = self
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| value.as_str())?;
        let token = header.trim();
        let token = token
            .strip_prefix("Bearer ")
            .or_else(|| token.strip_prefix("bearer "))
            .unwrap_or(token);
        if token.is_empty() { None } else { Some(token) }
    }
}

/// Reads a JSON object of strings.
fn string_map(value: Option<&Value>) -> HashMap<String, String> {
    let Some(Value::Object(map)) = value else {
        return HashMap::new();
    };
    map.iter()
        .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_string())))
        .collect()
}

/// A successful answer.
pub fn ok(data: Value) -> Value {
    respond(200, default_headers(), json!({ "data": data }))
}

/// The answer of a resource that was created.
pub fn created(data: Value) -> Value {
    respond(201, default_headers(), json!({ "data": data }))
}

/// The answer of a request that was accepted but is not done yet.
pub fn accepted(data: Value) -> Value {
    respond(202, default_headers(), json!({ "data": data }))
}

/// The answer of a failure.
pub fn failure(error: &IdentityError) -> Value {
    let mut details = serde_json::Map::new();
    let fields = error.fields();
    if !fields.is_empty() {
        details.insert("fields".to_string(), json!(fields));
    }
    // The message of the database goes to the details: the client switches
    // on the code and the message is for whoever reads the logs.
    if let Some(detail) = error.detail() {
        details.insert("reason".to_string(), json!(detail));
    }

    // A rate limit carries two more headers: how long the caller has to
    // wait, and how many hits are left in the window
    // (`docs/api-contract.md` §1.6).
    let mut headers = default_headers();
    if let IdentityError::RateLimited {
        retry_after,
        remaining,
    } = error
    {
        headers.insert("retry-after".to_string(), (*retry_after).max(1).to_string());
        headers.insert(
            "x-ratelimit-remaining".to_string(),
            (*remaining).to_string(),
        );
    }

    respond(
        error.status(),
        headers,
        json!({
            "error": {
                "code": error.code(),
                "message": error.to_string(),
                "details": Value::Object(details),
                "trace_id": trace_id(),
            }
        }),
    )
}

/// The answer of a request that could not be checked against its rate
/// limit because the store was not reachable.
///
/// It is **not** a `429`: the limit was not reached, the limit could not
/// be consulted. The catalogue says `INTERNAL_ERROR` for "algo falló de
/// nuestro lado", and that is what happened. The client retries, and the
/// worker does not serve traffic without a limit (`docs/api-contract.md`
/// §1.6).
pub fn limiter_unavailable(reason: &str) -> Value {
    let error = IdentityError::Internal;
    let mut details = serde_json::Map::new();
    details.insert("reason".to_string(), json!(reason));

    respond(
        error.status(),
        default_headers(),
        json!({
            "error": {
                "code": error.code(),
                "message": error.to_string(),
                "details": Value::Object(details),
                "trace_id": trace_id(),
            }
        }),
    )
}

/// The headers every answer carries.
fn default_headers() -> HashMap<String, String> {
    let mut headers = HashMap::new();
    headers.insert(
        "content-type".to_string(),
        "application/json; charset=utf-8".to_string(),
    );
    headers
}

/// Wraps a body with the status and the headers of the answer.
fn respond(status_code: u16, headers: HashMap<String, String>, body: Value) -> Value {
    json!({
        "status_code": status_code,
        "headers": headers,
        "body": body,
    })
}

/// The identifier that ties an answer to the logs.
fn trace_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..16].to_string()
}
