//! The rules of the identity, expressed against the ports.
//!
//! Every rule of `docs/api-contract.md` §3 lives here: the magic link that
//! works once, the refresh token that rotates and takes its family down when
//! it comes back, the profile, the devices and the consents.
//!
//! The service holds the outside world as traits, so it can be exercised
//! without a database, without a network and without an engine.

use std::sync::Arc;

use suertudo_api::rate_limit::RateGroup;

use crate::domain::errors::IdentityError;
use crate::domain::profile::{validate_business_id, validate_device, validate_patch};
use crate::domain::session::{
    ACCESS_TOKEN_TTL, DEVICE_CODE_DIGITS, DEVICE_VERIFICATION_MAX_ATTEMPTS,
    DEVICE_VERIFICATION_TTL, MAGIC_LINK_TTL, MagicLinkDecision, REFRESH_TOKEN_TTL, RefreshDecision,
    decide_magic_link, decide_refresh, default_display_name, normalise_email,
};
use crate::domain::types::{
    AccessClaims, AccountStatus, ConsentScope, ConsentState, Device, DeviceRegistration,
    DeviceRequest, MagicLinkClaims, MagicLinkIssued, ProfilePatch, RefreshClaims, Session,
    TokenKind, User,
};
use crate::ports::{
    Clock, ConsentRepository, DeviceRepository, Digest, SmsError, SmsMessage, SmsSender,
    StorageError, TokenRepository, TokenSigner, TokenSource, UserRepository,
};

/// How many times a request for a magic link retries the storage of the
/// link when another request holds the single pending slot of the address.
///
/// The endpoint answers `202` either way: the retry only makes the newest
/// link the live one, it does not change what the caller sees.
const MAGIC_LINK_ATTEMPTS: usize = 5;

/// The identity service: the rules of the participant.
pub struct IdentityService {
    pub users: Arc<dyn UserRepository>,
    pub tokens: Arc<dyn TokenRepository>,
    pub devices: Arc<dyn DeviceRepository>,
    pub consents: Arc<dyn ConsentRepository>,
    pub clock: Arc<dyn Clock>,
    pub token_source: Arc<dyn TokenSource>,
    pub digest: Arc<dyn Digest>,
    pub signer: Arc<dyn TokenSigner>,
    /// The SMS channel. The provider is not decided yet: the flow is built
    /// against the port.
    pub sms: Arc<dyn SmsSender>,
}

impl std::fmt::Debug for IdentityService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityService").finish_non_exhaustive()
    }
}

impl IdentityService {
    /// Asks for a magic link.
    ///
    /// The answer is always the same, whether the address exists or not: the
    /// endpoint must not tell the caller whether an account exists
    /// (`docs/api-contract.md` §3.1).
    pub async fn request_magic_link(
        &self,
        email: &str,
        name: Option<&str>,
    ) -> Result<MagicLinkIssued, IdentityError> {
        let email = normalise_email(email)?;
        let now = self.clock.now();

        // The cooldown of the group `auth:link`.
        //
        // The counter of Redis is the limit of the contract: five links in
        // fifteen minutes. This record is what stops the five of them from
        // going out in the same second, and it is what survives a flush of
        // the counter.
        //
        // Every attempt renews the wait, including a refused one: that is
        // what keeps an abuser locked out, and it is what a login form does
        // when it says "wait since your last attempt".
        if let Some(spacing) = RateGroup::AuthLink.min_spacing() {
            let previous = self.users.touch_magic_link(&email, now).await?;
            if let Some(previous) = previous {
                let elapsed = now - previous;
                let spacing = spacing.as_secs() as i64;
                if elapsed < spacing {
                    return Err(IdentityError::RateLimited {
                        retry_after: (spacing - elapsed).max(1) as u64,
                        remaining: 0,
                    });
                }
            }
        }

        let name = name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let claims = MagicLinkClaims {
            email: email.clone(),
            name,
            issued_at: now,
            expires_at: now + MAGIC_LINK_TTL,
            token_id: self.token_source.next_id(),
        };
        let token = self.signer.sign_magic_link(&claims)?;
        let token_hash = self.digest.sha256_hex(&token);

        // Only one pending link per address: the schema enforces it with a
        // partial unique index, and the previous link has to be cancelled
        // before the new one is stored.
        //
        // A race between two requests is not an error: both answer `202`,
        // and the caller cannot tell a race from a first request
        // (`docs/api-contract.md` §3.1).
        for _ in 0..MAGIC_LINK_ATTEMPTS {
            self.tokens.revoke_pending_magic_links(&email).await?;
            if self
                .tokens
                .insert_magic_link(&email, &token_hash, claims.expires_at)
                .await?
            {
                break;
            }
        }

        Ok(MagicLinkIssued {
            token,
            expires_in: MAGIC_LINK_TTL,
        })
    }

    /// Consumes a magic link and returns the session.
    ///
    /// The participant is created here, and not when the link was asked for:
    /// the link is the proof that the address belongs to whoever holds it.
    pub async fn verify_magic_link(&self, token: &str) -> Result<Session, IdentityError> {
        let now = self.clock.now();
        let token_hash = self.digest.sha256_hex(token);

        let stored = self
            .tokens
            .find_by_hash(&token_hash)
            .await?
            .ok_or_else(invalid_link)?;
        if stored.kind != TokenKind::MagicLink {
            return Err(invalid_link());
        }

        match decide_magic_link(&stored, now) {
            MagicLinkDecision::Consume => {}
            MagicLinkDecision::Consumed | MagicLinkDecision::Revoked => {
                return Err(IdentityError::LinkConsumed);
            }
            MagicLinkDecision::Expired => {
                return Err(IdentityError::invalid(
                    "token",
                    "El enlace venció. Pedí uno nuevo.",
                ));
            }
            MagicLinkDecision::Unknown => return Err(invalid_link()),
        }

        // The link is consumed before the account exists: the account is
        // created for a link that was won, and a race cannot leave an
        // account behind that nobody can use.
        if !self.tokens.consume_magic_link(&stored.id).await? {
            return Err(IdentityError::LinkConsumed);
        }

        let user = match self.users.find_by_email(&stored.email).await? {
            Some(user) => user,
            None => {
                let display_name = self
                    .signer
                    .verify_magic_link(token)
                    .ok()
                    .and_then(|claims| claims.name)
                    .unwrap_or_else(|| default_display_name(&stored.email));
                match self.users.create(&stored.email, &display_name).await {
                    Ok(user) => user,
                    // The account appeared while the link was consumed: it is
                    // the same participant.
                    Err(StorageError::Conflict(_)) => self
                        .users
                        .find_by_email(&stored.email)
                        .await?
                        .ok_or(IdentityError::Conflict)?,
                    Err(err) => return Err(err.into()),
                }
            }
        };

        if user.status != AccountStatus::Active {
            return Err(IdentityError::Forbidden);
        }

        // The link now belongs to the participant.
        self.tokens.bind_token_user(&stored.id, &user.id).await?;

        self.issue_session(&user, now, None).await
    }

    /// Rotates a refresh token.
    ///
    /// A token that comes back was stolen: the whole family is cancelled and
    /// the participant has to authenticate again (`FR-ID-04`).
    pub async fn refresh(&self, refresh_token: &str) -> Result<Session, IdentityError> {
        let now = self.clock.now();
        let token_hash = self.digest.sha256_hex(refresh_token);

        let stored = self
            .tokens
            .find_by_hash(&token_hash)
            .await?
            .ok_or(IdentityError::AuthRequired)?;
        if stored.kind != TokenKind::Refresh {
            return Err(IdentityError::AuthRequired);
        }

        // The owner of the token travels with every write: the policies of
        // the schema admit them only with the context of the participant.
        let user_id = stored.user_id.clone().ok_or(IdentityError::AuthRequired)?;

        match decide_refresh(&stored, now) {
            RefreshDecision::Rotate => {}
            RefreshDecision::Reuse => {
                if let Some(family_id) = &stored.family_id {
                    self.tokens.revoke_family(family_id, &user_id).await?;
                }
                return Err(IdentityError::AuthRequired);
            }
            RefreshDecision::Expired | RefreshDecision::Unknown => {
                return Err(IdentityError::AuthRequired);
            }
        }

        let family_id = stored
            .family_id
            .clone()
            .ok_or(IdentityError::AuthRequired)?;

        // The presented token dies here and the successor is born in the same
        // family, so a reuse can still take the family down.
        //
        // If the token was already revoked, somebody else rotated it while
        // this request was in flight. That is a reuse, and `FR-ID-04` asks
        // for the family to go down with it. Without this check two
        // concurrent refreshes would both mint a valid successor.
        if !self.tokens.revoke_token(&stored.id, &user_id).await? {
            self.tokens.revoke_family(&family_id, &user_id).await?;
            return Err(IdentityError::AuthRequired);
        }

        let user = self
            .users
            .find_by_id(&user_id)
            .await?
            .ok_or(IdentityError::AuthRequired)?;
        if user.status != AccountStatus::Active {
            return Err(IdentityError::Forbidden);
        }

        self.issue_session(&user, now, Some(family_id)).await
    }

    /// Cancels the family of a refresh token.
    ///
    /// The logout is idempotent: a token that is already gone is not an
    /// error, the session is over either way.
    pub async fn logout(&self, refresh_token: &str) -> Result<(), IdentityError> {
        let token_hash = self.digest.sha256_hex(refresh_token);
        let Some(stored) = self.tokens.find_by_hash(&token_hash).await? else {
            return Ok(());
        };

        // Only a refresh token is a session: the logout closes the family of
        // the refresh token and nothing else.
        if stored.kind != TokenKind::Refresh {
            return Ok(());
        }

        // The owner of the token travels with the write: the policies of the
        // schema admit it only with the context of the participant.
        let Some(user_id) = stored.user_id.clone() else {
            return Ok(());
        };

        match &stored.family_id {
            Some(family_id) => self.tokens.revoke_family(family_id, &user_id).await?,
            // The token has no family: revoking it is all there is to do.
            None => {
                self.tokens.revoke_token(&stored.id, &user_id).await?;
            }
        }

        // A device belongs to a session, not to a person: the logout hands
        // the token back, so that the same phone can open another account
        // without losing the push (`TSK-F0-10`).
        self.devices.release(&user_id).await.map_err(storage)?;
        Ok(())
    }

    /// The profile of a participant.
    pub async fn profile(&self, user_id: &str) -> Result<User, IdentityError> {
        self.users
            .find_by_id(user_id)
            .await?
            .ok_or(IdentityError::NotFound)
    }

    /// Applies a patch to the profile of a participant.
    pub async fn update_profile(
        &self,
        user_id: &str,
        patch: ProfilePatch,
    ) -> Result<User, IdentityError> {
        validate_patch(&patch)?;
        self.users
            .update(user_id, patch)
            .await?
            .ok_or(IdentityError::NotFound)
    }

    /// Anonymises the personal data of a participant.
    ///
    /// The tickets survive with a null `user_id` and the prizes keep their
    /// owner (`FR-ID-05`).
    pub async fn delete_account(&self, user_id: &str) -> Result<(), IdentityError> {
        let user = self
            .users
            .find_by_id(user_id)
            .await?
            .ok_or(IdentityError::NotFound)?;
        self.users.anonymise(&user.id).await?;
        self.tokens.revoke_all_for_user(&user.id).await?;
        Ok(())
    }

    /// Registers a push device.
    ///
    /// The token of a device is unique in the whole platform: the same
    /// phone can open a second account, and the token of the first one is
    /// not stolen, it is repeated. The repeated token opens a verification
    /// and the SMS goes to the participant that **currently owns it**: only
    /// whoever holds that phone can authorise the transfer (`TSK-F0-10`).
    pub async fn register_device(
        &self,
        user_id: &str,
        platform: &str,
        token: &str,
        app_version: Option<&str>,
    ) -> Result<DeviceRegistration, IdentityError> {
        validate_device(platform, token)?;

        // The code of the verification: six digits, ten minutes. It travels
        // as its hash: the plaintext is in the SMS and in the request of the
        // participant that received it, and nowhere else.
        let code = self.token_source.next_code(DEVICE_CODE_DIGITS);
        let code_hash = self.digest.sha256_hex(&code);
        let expires_at = self.clock.now() + DEVICE_VERIFICATION_TTL;

        let request = self
            .devices
            .request_takeover(
                user_id,
                platform,
                token,
                app_version,
                &code_hash,
                expires_at,
            )
            .await
            .map_err(storage)?;

        match request {
            DeviceRequest::Registered(device) => Ok(DeviceRegistration::Registered(device)),
            DeviceRequest::Pending { verification_id } => {
                // The SMS goes to the participant that currently owns the
                // token. If it went to the claimant, whoever stole the
                // string of the token would receive their own code and the
                // transfer would go through.
                let phone = self
                    .devices
                    .takeover_phone(&verification_id, user_id)
                    .await
                    .map_err(storage)?;

                let Some(phone) = phone else {
                    // The owner has no phone on file: the takeover cannot be
                    // verified, so the token stays where it is.
                    return Err(IdentityError::DeviceOwnerUnverifiable);
                };

                self.sms
                    .send(&phone, &SmsMessage::new(device_takeover_message(&code)))
                    .await
                    .map_err(sms_failure)?;

                Ok(DeviceRegistration::Pending {
                    verification_id,
                    expires_at,
                })
            }
        }
    }

    /// Confirms the takeover of a device with the code that the owner of
    /// the token received by SMS.
    pub async fn confirm_device_takeover(
        &self,
        user_id: &str,
        verification_id: &str,
        code: &str,
    ) -> Result<Device, IdentityError> {
        let code = code.trim();
        if code.len() != DEVICE_CODE_DIGITS || !code.chars().all(|c| c.is_ascii_digit()) {
            return Err(IdentityError::DeviceCodeInvalid);
        }

        // Five attempts: the sixth one does not reach the database.
        if let Some(verification) = self
            .devices
            .verification(verification_id, user_id)
            .await
            .map_err(storage)?
            && verification.attempts >= DEVICE_VERIFICATION_MAX_ATTEMPTS
        {
            return Err(IdentityError::DeviceCodeInvalid);
        }

        // The database counts the attempt and commits it: the refusal is a
        // result, not an error of the transaction.
        let device = self
            .devices
            .confirm_takeover(verification_id, user_id, code)
            .await
            .map_err(storage)?;

        device.ok_or(IdentityError::DeviceCodeInvalid)
    }

    /// Deactivates a push device of the participant.
    pub async fn remove_device(&self, user_id: &str, device_id: &str) -> Result<(), IdentityError> {
        if self.devices.deactivate(user_id, device_id).await? {
            Ok(())
        } else {
            Err(IdentityError::NotFound)
        }
    }

    /// The consents of the participant, one entry per business and scope.
    pub async fn list_consents(&self, user_id: &str) -> Result<Vec<ConsentState>, IdentityError> {
        self.consents.list(user_id).await.map_err(storage)
    }

    /// Records the consents of the participant for one business.
    ///
    /// The agent of the request travels with the record: the consent has to
    /// be auditable (`FR-PR-02`).
    pub async fn set_consent(
        &self,
        user_id: &str,
        business_id: &str,
        share_marketing: bool,
        share_contact: bool,
        user_agent: Option<&str>,
    ) -> Result<(), IdentityError> {
        validate_business_id(business_id)?;
        self.consents
            .record(
                user_id,
                business_id,
                ConsentScope::ShareMarketing,
                share_marketing,
                user_agent,
            )
            .await
            .map_err(storage)?;
        self.consents
            .record(
                user_id,
                business_id,
                ConsentScope::ShareContact,
                share_contact,
                user_agent,
            )
            .await
            .map_err(storage)?;
        Ok(())
    }

    /// Issues an access token and a refresh token for a participant.
    ///
    /// With a family, the refresh token joins an existing one: that is what
    /// makes a rotation detectable. Without one, the family is created
    /// first: the foreign key of the schema rejects a token of a family that
    /// does not exist, and the death of the family is what takes its tokens
    /// down (`FR-ID-04`).
    async fn issue_session(
        &self,
        user: &User,
        now: i64,
        family_id: Option<String>,
    ) -> Result<Session, IdentityError> {
        let access_claims = AccessClaims {
            subject: user.id.clone(),
            role: user.role.as_str().to_string(),
            issued_at: now,
            expires_at: now + ACCESS_TOKEN_TTL,
            token_id: self.token_source.next_id(),
        };
        let access_token = self.signer.sign_access(&access_claims)?;

        let family_id = match family_id {
            Some(family_id) => family_id,
            None => self.tokens.create_family(&user.id).await?,
        };
        let refresh_claims = RefreshClaims {
            subject: user.id.clone(),
            family_id: family_id.clone(),
            issued_at: now,
            expires_at: now + REFRESH_TOKEN_TTL,
            token_id: self.token_source.next_id(),
        };
        let refresh_token = self.signer.sign_refresh(&refresh_claims)?;
        let token_hash = self.digest.sha256_hex(&refresh_token);
        match self
            .tokens
            .insert_refresh(&user.id, &family_id, &token_hash, refresh_claims.expires_at)
            .await
        {
            Ok(()) => {}
            // The family died while the successor was being minted: the
            // reuse won the race and the session does not exist.
            Err(StorageError::FamilyRevoked(_)) => return Err(IdentityError::AuthRequired),
            Err(err) => return Err(err.into()),
        }

        Ok(Session {
            access_token,
            refresh_token,
            expires_in: ACCESS_TOKEN_TTL,
            user: user.clone(),
        })
    }
}

/// The message of the SMS of a device takeover.
///
/// It goes to the participant that currently owns the token, so it has to
/// make sense for somebody who does not know that they have an account open
/// on another phone.
fn device_takeover_message(code: &str) -> String {
    format!(
        "Suertudo: alguien está intentando registrar en su cuenta el dispositivo que ya tenés \
         registrado en otra. Para autorizar el traslado, ingresá el código {code} en la app. \
         Vence en 10 minutos. Si no reconocés el dispositivo, no hagas nada: el registro no se \
         mueve sin tu autorización."
    )
}

/// The failure of the SMS channel.
fn sms_failure(err: SmsError) -> IdentityError {
    tracing::error!(%err, "the SMS of the device takeover did not go out");
    IdentityError::Internal
}

/// The failure of a magic link that is not usable.
///
/// An unknown link and an expired one fail the same way: the answer does not
/// tell whether the address ever existed.
fn invalid_link() -> IdentityError {
    IdentityError::invalid("token", "El enlace no es válido.")
}

/// Turns a storage failure into the error the client sees.
fn storage(err: StorageError) -> IdentityError {
    match err {
        StorageError::Conflict(_) => IdentityError::Conflict,
        StorageError::Missing(_) => IdentityError::NotFound,
        // The family is dead: the caller has to authenticate again.
        StorageError::FamilyRevoked(_) => IdentityError::AuthRequired,
        // The database refused with a code of the catalogue: the answer
        // carries it, so that no refusal collapses into a `500`.
        StorageError::Refused { refusal, message } => IdentityError::Refused {
            refusal,
            message: refusal.message().to_string(),
            detail: message,
        },
        StorageError::Backend(_) => IdentityError::Internal,
    }
}

impl From<StorageError> for IdentityError {
    fn from(err: StorageError) -> Self {
        storage(err)
    }
}

impl From<crate::ports::CryptoError> for IdentityError {
    fn from(_err: crate::ports::CryptoError) -> Self {
        IdentityError::Internal
    }
}
