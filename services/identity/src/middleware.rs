//! The authorization middleware.
//!
//! The middleware resolves the session **before** the handler runs and
//! injects the identity of the caller into the payload of the handler. The
//! handler never reads the tenant from the body or from the path: if the
//! context is not there, the request has no caller and it is rejected
//! (AGENTS.md §5.2).
//!
//! The resolution is the same in the two shapes the transport uses:
//!
//! - The RBAC port of the engine hands over the payload of the function. The
//!   middleware returns the payload with the context injected.
//! - The HTTP worker hands over the request. Its middleware cannot change
//!   the payload of the handler, so it answers `continue` when the session is
//!   good and `respond` with a `401` when it is not. The handler then
//!   resolves the very same session, with the very same code, and finds the
//!   context that the middleware would have injected.

use std::sync::Arc;

use serde_json::{Value, json};
use suertudo_api::middleware::{AuthContext, extract, inject};
use uuid::Uuid;

use crate::domain::errors::IdentityError;
use crate::http::{Request, failure};
use crate::ports::{CryptoError, TokenSigner};

/// Resolves the session of a caller.
pub struct SessionResolver {
    signer: Arc<dyn TokenSigner>,
}

impl std::fmt::Debug for SessionResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionResolver").finish_non_exhaustive()
    }
}

impl SessionResolver {
    /// Builds the resolver over the signer of the access tokens.
    pub fn new(signer: Arc<dyn TokenSigner>) -> Self {
        Self { signer }
    }

    /// The identity behind an access token.
    pub fn context_from_token(&self, token: &str) -> Result<AuthContext, IdentityError> {
        let claims = self.signer.verify_access(token).map_err(|err| match err {
            // An expired token is not an invalid one: the client has to
            // refresh the session, and the answer says so.
            CryptoError::Expired => IdentityError::AuthExpired,
            _ => IdentityError::AuthRequired,
        })?;

        let user_id = Uuid::parse_str(&claims.subject).map_err(|_| IdentityError::AuthRequired)?;
        Ok(AuthContext {
            user_id,
            // The participant has no business of its own: the business
            // context comes with the session of the commerce, which is
            // another task (`FR-ID-02`).
            business_id: None,
        })
    }

    /// The identity of the caller of a request.
    ///
    /// The injected context wins: when the middleware already resolved the
    /// session, the handler does not resolve it again.
    pub fn context_from_payload(&self, payload: &Value) -> Result<AuthContext, IdentityError> {
        if let Some(context) = extract(payload) {
            return Ok(context);
        }

        let request = Request::parse(payload);
        self.context_from_headers(&request)
    }

    /// The identity of the caller of a request that carries the headers.
    fn context_from_headers(&self, request: &Request) -> Result<AuthContext, IdentityError> {
        let token = request.bearer_token().ok_or(IdentityError::AuthRequired)?;
        self.context_from_token(token)
    }
}

/// The middleware of the authorization.
///
/// It answers with the payload of the handler, with the context of the caller
/// injected, or with the failure that stops the request.
pub fn authorize(resolver: &SessionResolver, input: Value) -> Result<Value, iii_sdk::Error> {
    // The HTTP worker middleware: the request, and an answer that either
    // lets the request through or answers it right away.
    if input.get("request").is_some() {
        let request = Request::parse(&input["request"]);
        return match resolver.context_from_headers(&request) {
            Ok(_) => Ok(json!({ "action": "continue" })),
            Err(error) => Ok(json!({
                "action": "respond",
                "response": {
                    "status_code": error.status(),
                    "headers": { "content-type": "application/json; charset=utf-8" },
                    "body": failure(&error)["body"].clone(),
                }
            })),
        };
    }

    // The RBAC port of the engine: the payload of the function, that comes
    // back with the context of the caller injected.
    let payload = input.get("payload").cloned().unwrap_or(input);
    match resolver.context_from_payload(&payload) {
        Ok(context) => Ok(inject(payload, context)),
        Err(error) => Err(iii_sdk::Error::Remote {
            code: error.code().to_string(),
            message: error.to_string(),
            stacktrace: None,
        }),
    }
}
