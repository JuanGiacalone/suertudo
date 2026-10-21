//! The iii surface of the identity worker.
//!
//! Every function this worker exposes is registered here, with its HTTP
//! trigger. The rules live in the domain and the queries in the adapters:
//! this module only translates between the two and the wire
//! (`docs/api-contract.md` §3).

use std::sync::Arc;

use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{Value, json};

use crate::domain::service::IdentityService;
use crate::domain::types::{NotificationPreferences, ProfilePatch, Session, User};
use crate::http::{Request, accepted, created, failure, ok};
use crate::middleware::SessionResolver;

/// What the handlers need to answer a request.
pub struct Deps {
    pub service: Arc<IdentityService>,
    pub resolver: SessionResolver,
    /// Whether the token of a magic link travels in the response.
    pub dev_magic_link: bool,
}

impl std::fmt::Debug for Deps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deps")
            .field("dev_magic_link", &self.dev_magic_link)
            .finish_non_exhaustive()
    }
}

/// Registers every function and trigger of the identity worker.
pub fn register(worker: &IIIClient, deps: &Arc<Deps>) -> Result<(), iii_sdk::Error> {
    register_handler(
        worker,
        "identity::request_magic_link",
        "Pide un enlace magico de un solo uso",
        deps,
        |deps, payload| async move { request_magic_link(&deps, payload).await },
    );
    http_trigger(
        worker,
        "identity::request_magic_link",
        "/v1/auth/magic-link",
        "POST",
    )?;

    register_handler(
        worker,
        "identity::verify_magic_link",
        "Consume un enlace magico y devuelve la sesion",
        deps,
        |deps, payload| async move { verify_magic_link(&deps, payload).await },
    );
    http_trigger(
        worker,
        "identity::verify_magic_link",
        "/v1/auth/magic-link/verify",
        "POST",
    )?;

    register_handler(
        worker,
        "identity::refresh_session",
        "Rota el refresh token de la sesion",
        deps,
        |deps, payload| async move { refresh_session(&deps, payload).await },
    );
    http_trigger(
        worker,
        "identity::refresh_session",
        "/v1/auth/refresh",
        "POST",
    )?;

    register_handler(
        worker,
        "identity::logout",
        "Revoca la familia de refresh tokens de la sesion",
        deps,
        |deps, payload| async move { logout(&deps, payload).await },
    );
    http_trigger(worker, "identity::logout", "/v1/auth/logout", "POST")?;

    register_handler(
        worker,
        "identity::get_profile",
        "Devuelve el perfil del participante de la sesion",
        deps,
        |deps, payload| async move { get_profile(&deps, payload).await },
    );
    http_trigger(worker, "identity::get_profile", "/v1/me", "GET")?;

    register_handler(
        worker,
        "identity::update_profile",
        "Actualiza el perfil del participante de la sesion",
        deps,
        |deps, payload| async move { update_profile(&deps, payload).await },
    );
    http_trigger(worker, "identity::update_profile", "/v1/me", "PATCH")?;

    register_handler(
        worker,
        "identity::delete_profile",
        "Anonimiza los datos personales del participante",
        deps,
        |deps, payload| async move { delete_profile(&deps, payload).await },
    );
    http_trigger(worker, "identity::delete_profile", "/v1/me", "DELETE")?;

    register_handler(
        worker,
        "identity::register_device",
        "Registra un dispositivo push",
        deps,
        |deps, payload| async move { register_device(&deps, payload).await },
    );
    http_trigger(
        worker,
        "identity::register_device",
        "/v1/me/devices",
        "POST",
    )?;

    register_handler(
        worker,
        "identity::delete_device",
        "Desactiva un dispositivo push",
        deps,
        |deps, payload| async move { delete_device(&deps, payload).await },
    );
    http_trigger(
        worker,
        "identity::delete_device",
        "/v1/me/devices/:device_id",
        "DELETE",
    )?;

    register_handler(
        worker,
        "identity::list_consents",
        "Devuelve los consentimientos del participante",
        deps,
        |deps, payload| async move { list_consents(&deps, payload).await },
    );
    http_trigger(worker, "identity::list_consents", "/v1/me/consents", "GET")?;

    register_handler(
        worker,
        "identity::set_consent",
        "Registra el consentimiento del participante para un negocio",
        deps,
        |deps, payload| async move { set_consent(&deps, payload).await },
    );
    http_trigger(
        worker,
        "identity::set_consent",
        "/v1/me/consents/:business_id",
        "PUT",
    )?;
    http_trigger(worker, "identity::set_consent", "/v1/me/consents", "PUT")?;

    register_handler(
        worker,
        "identity::get_user",
        "Devuelve un participante por identificador",
        deps,
        |deps, payload| async move { get_user(&deps, payload).await },
    );

    let resolver = deps.clone();
    worker.register_function(
        "identity::authorize",
        RegisterFunction::new(move |input: Value| {
            crate::middleware::authorize(&resolver.resolver, input)
        })
        .description("Resuelve la sesion e inyecta el contexto del llamador"),
    );

    Ok(())
}

/// Registers an asynchronous handler.
///
/// The handler receives the dependencies and the payload of the invocation,
/// and answers with the body of the response.
fn register_handler<F, Fut>(
    worker: &IIIClient,
    function_id: &str,
    description: &str,
    deps: &Arc<Deps>,
    handler: F,
) where
    F: Fn(Arc<Deps>, Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Value> + Send + 'static,
{
    let handler = Arc::new(handler);
    let deps = deps.clone();
    worker.register_function(
        function_id,
        RegisterFunction::new_async(move |payload: Value| {
            let handler = handler.clone();
            let deps = deps.clone();
            async move { Ok(handler(deps, payload).await) }
        })
        .description(description),
    );
}

/// Binds a function to an HTTP route.
fn http_trigger(
    worker: &IIIClient,
    function_id: &str,
    api_path: &str,
    http_method: &str,
) -> Result<(), iii_sdk::Error> {
    worker
        .register_trigger(RegisterTriggerInput {
            trigger_type: "http".into(),
            function_id: function_id.into(),
            config: json!({ "api_path": api_path, "http_method": http_method }),
            metadata: None,
            namespace: None,
            trigger_namespace: None,
        })
        .map(|_| ())
}

/// `POST /v1/auth/magic-link`.
///
/// The answer is the same whether the address exists or not: the endpoint
/// does not tell the caller whether an account exists.
pub async fn request_magic_link(deps: &Deps, payload: Value) -> Value {
    let request = Request::parse(&payload);
    let email = request.field("email").unwrap_or_default();
    let name = request.field("name");

    match deps.service.request_magic_link(email, name).await {
        Ok(issued) => {
            let mut data = json!({
                "sent": true,
                "expires_in": issued.expires_in,
                "delivery": "email",
            });
            // The magic link travels in the response only in development:
            // outside it, the worker refuses to start (see `crate::config`).
            if deps.dev_magic_link {
                data["dev_magic_link"] = Value::String(issued.token);
            }
            // The answer is `202`: the session is not there yet.
            accepted(data)
        }
        Err(error) => failure(&error),
    }
}

/// `POST /v1/auth/magic-link/verify`.
pub async fn verify_magic_link(deps: &Deps, payload: Value) -> Value {
    let request = Request::parse(&payload);
    let token = request.field("token").unwrap_or_default();

    match deps.service.verify_magic_link(token).await {
        Ok(session) => ok(session_json(&session)),
        Err(error) => failure(&error),
    }
}

/// `POST /v1/auth/refresh`.
pub async fn refresh_session(deps: &Deps, payload: Value) -> Value {
    let request = Request::parse(&payload);
    let refresh_token = request.field("refresh_token").unwrap_or_default();

    match deps.service.refresh(refresh_token).await {
        Ok(session) => ok(session_json(&session)),
        Err(error) => failure(&error),
    }
}

/// `POST /v1/auth/logout`.
pub async fn logout(deps: &Deps, payload: Value) -> Value {
    let request = Request::parse(&payload);
    let refresh_token = request.field("refresh_token").unwrap_or_default();

    match deps.service.logout(refresh_token).await {
        Ok(()) => ok(json!({ "revoked": true })),
        Err(error) => failure(&error),
    }
}

/// `GET /v1/me`.
pub async fn get_profile(deps: &Deps, payload: Value) -> Value {
    let context = match deps.resolver.context_from_payload(&payload) {
        Ok(context) => context,
        Err(error) => return failure(&error),
    };

    match deps.service.profile(&context.user_id.to_string()).await {
        Ok(user) => ok(user_json(&user)),
        Err(error) => failure(&error),
    }
}

/// `PATCH /v1/me`.
pub async fn update_profile(deps: &Deps, payload: Value) -> Value {
    let context = match deps.resolver.context_from_payload(&payload) {
        Ok(context) => context,
        Err(error) => return failure(&error),
    };
    let request = Request::parse(&payload);

    match deps
        .service
        .update_profile(&context.user_id.to_string(), patch_from(&request.body))
        .await
    {
        Ok(user) => ok(user_json(&user)),
        Err(error) => failure(&error),
    }
}

/// `DELETE /v1/me`.
///
/// The anonymisation is immediate and the deletion of the row is a job of
/// thirty days: the answer is `202` because the participant is not gone yet.
pub async fn delete_profile(deps: &Deps, payload: Value) -> Value {
    let context = match deps.resolver.context_from_payload(&payload) {
        Ok(context) => context,
        Err(error) => return failure(&error),
    };

    match deps
        .service
        .delete_account(&context.user_id.to_string())
        .await
    {
        Ok(()) => accepted(json!({ "status": "accepted" })),
        Err(error) => failure(&error),
    }
}

/// `POST /v1/me/devices`.
pub async fn register_device(deps: &Deps, payload: Value) -> Value {
    let context = match deps.resolver.context_from_payload(&payload) {
        Ok(context) => context,
        Err(error) => return failure(&error),
    };
    let request = Request::parse(&payload);

    let platform = request.field("platform").unwrap_or_default();
    let token = request.field("token").unwrap_or_default();
    let app_version = request.field("app_version");

    match deps
        .service
        .register_device(&context.user_id.to_string(), platform, token, app_version)
        .await
    {
        Ok(device) => created(json!({
            "id": device.id,
            "platform": device.platform,
            "app_version": device.app_version,
            "active": device.active,
        })),
        Err(error) => failure(&error),
    }
}

/// `DELETE /v1/me/devices/{id}`.
pub async fn delete_device(deps: &Deps, payload: Value) -> Value {
    let context = match deps.resolver.context_from_payload(&payload) {
        Ok(context) => context,
        Err(error) => return failure(&error),
    };
    let request = Request::parse(&payload);
    let device_id = request.path_param("device_id").unwrap_or_default();

    match deps
        .service
        .remove_device(&context.user_id.to_string(), device_id)
        .await
    {
        Ok(()) => ok(json!({ "deleted": true })),
        Err(error) => failure(&error),
    }
}

/// `GET /v1/me/consents`.
pub async fn list_consents(deps: &Deps, payload: Value) -> Value {
    let context = match deps.resolver.context_from_payload(&payload) {
        Ok(context) => context,
        Err(error) => return failure(&error),
    };

    match deps
        .service
        .list_consents(&context.user_id.to_string())
        .await
    {
        Ok(states) => {
            let data: Vec<Value> = states
                .iter()
                .map(|state| {
                    json!({
                        "business_id": state.business_id,
                        "scope": state.scope.as_str(),
                        "granted": state.granted,
                        "updated_at": state.updated_at,
                    })
                })
                .collect();
            ok(Value::Array(data))
        }
        Err(error) => failure(&error),
    }
}

/// `PUT /v1/me/consents/{business_id}`.
///
/// The business identifies the consent, not the tenant of the request: the
/// participant grants a consent for one business and for no other.
pub async fn set_consent(deps: &Deps, payload: Value) -> Value {
    let context = match deps.resolver.context_from_payload(&payload) {
        Ok(context) => context,
        Err(error) => return failure(&error),
    };
    let request = Request::parse(&payload);

    let business_id = request
        .path_param("business_id")
        .map(str::to_string)
        .or_else(|| request.field("business_id").map(str::to_string))
        .unwrap_or_default();
    let share_marketing = request
        .body
        .get("share_marketing")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let share_contact = request
        .body
        .get("share_contact")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // The agent travels with the consent: the record has to be auditable
    // (`FR-PR-02`).
    let user_agent = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
        .map(|(_, value)| value.as_str());

    match deps
        .service
        .set_consent(
            &context.user_id.to_string(),
            &business_id,
            share_marketing,
            share_contact,
            user_agent,
        )
        .await
    {
        Ok(()) => ok(json!({
            "business_id": business_id,
            "share_marketing": share_marketing,
            "share_contact": share_contact,
        })),
        Err(error) => failure(&error),
    }
}

/// `identity::get_user`: a participant by identifier, for the other workers.
pub async fn get_user(deps: &Deps, payload: Value) -> Value {
    let request = Request::parse(&payload);
    let user_id = request.field("user_id").unwrap_or_default();

    match deps.service.profile(user_id).await {
        Ok(user) => ok(user_json(&user)),
        Err(error) => failure(&error),
    }
}

/// The participant as it travels to the client.
fn user_json(user: &User) -> Value {
    json!({
        "id": user.id,
        "email": user.email,
        "display_name": user.display_name,
        "role": user.role.as_str(),
        "status": user.status.as_str(),
        "phone_e164": user.phone_e164,
        "locale": user.locale,
        "timezone": user.timezone,
        "marketing_opt_in": user.marketing_opt_in,
        // The three channels of `docs/api-contract.md` §3.6, as they are
        // stored.
        "notification_preferences": {
            "push": user.notification_preferences.push,
            "email": user.notification_preferences.email,
            "marketing": user.notification_preferences.marketing,
        },
    })
}

/// The session as it travels to the client.
fn session_json(session: &Session) -> Value {
    json!({
        "access_token": session.access_token,
        "refresh_token": session.refresh_token,
        "token_type": "Bearer",
        "expires_in": session.expires_in,
        "user": user_json(&session.user),
    })
}

/// The changes of a profile, as they arrive in the body.
fn patch_from(body: &Value) -> ProfilePatch {
    ProfilePatch {
        display_name: body
            .get("display_name")
            .and_then(Value::as_str)
            .map(str::to_string),
        // An absent field leaves the phone alone and an explicit null sets it
        // to null: the difference matters and the body keeps it.
        phone_e164: body
            .get("phone_e164")
            .map(|value| value.as_str().map(str::to_string)),
        locale: body
            .get("locale")
            .and_then(Value::as_str)
            .map(str::to_string),
        timezone: body
            .get("timezone")
            .and_then(Value::as_str)
            .map(str::to_string),
        notification_preferences: body.get("notification_preferences").map(|value| {
            NotificationPreferences {
                push: value.get("push").and_then(Value::as_bool).unwrap_or(true),
                email: value.get("email").and_then(Value::as_bool).unwrap_or(true),
                marketing: value
                    .get("marketing")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }
        }),
    }
}
