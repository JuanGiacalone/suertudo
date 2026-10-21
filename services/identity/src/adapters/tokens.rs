//! The tokens: access, refresh and magic link.
//!
//! All three are signed with HS256. The access token and the refresh token
//! use different keys, so a stolen refresh token cannot be used as an access
//! token. Every token carries its kind in the `typ` claim and the verifier
//! rejects the tokens of another kind.
//!
//! The expiry is checked against the injected clock and not against the
//! machine: the rules have to be testable without waiting fifteen minutes
//! (AGENTS.md §8).

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::domain::types::{AccessClaims, MagicLinkClaims, RefreshClaims};
use crate::ports::{Clock, CryptoError, TokenSigner};

/// The kind of an access token.
const ACCESS_TYP: &str = "access";
/// The kind of a refresh token.
const REFRESH_TYP: &str = "refresh";
/// The kind of a magic link.
const MAGIC_LINK_TYP: &str = "magic_link";

#[derive(Debug, Serialize, Deserialize)]
struct AccessPayload {
    sub: String,
    role: String,
    exp: i64,
    iat: i64,
    jti: String,
    typ: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct RefreshPayload {
    sub: String,
    fam: String,
    exp: i64,
    iat: i64,
    jti: String,
    typ: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct MagicLinkPayload {
    email: String,
    name: Option<String>,
    exp: i64,
    iat: i64,
    jti: String,
    typ: String,
}

/// The signer of the three kinds of token.
pub struct Hs256TokenSigner {
    access_encoding: EncodingKey,
    access_decoding: DecodingKey,
    refresh_encoding: EncodingKey,
    refresh_decoding: DecodingKey,
    clock: std::sync::Arc<dyn Clock>,
}

impl std::fmt::Debug for Hs256TokenSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The keys never reach the logs.
        f.debug_struct("Hs256TokenSigner").finish_non_exhaustive()
    }
}

impl Hs256TokenSigner {
    /// Builds the signer with the two keys.
    ///
    /// The access key signs the access tokens and the magic links; the
    /// refresh key signs the refresh tokens. They are never the same.
    pub fn new(
        access_secret: &[u8],
        refresh_secret: &[u8],
        clock: std::sync::Arc<dyn Clock>,
    ) -> Self {
        Self {
            access_encoding: EncodingKey::from_secret(access_secret),
            access_decoding: DecodingKey::from_secret(access_secret),
            refresh_encoding: EncodingKey::from_secret(refresh_secret),
            refresh_decoding: DecodingKey::from_secret(refresh_secret),
            clock,
        }
    }

    /// The validation shared by the three kinds.
    ///
    /// The expiry is not validated here: the clock of the domain decides,
    /// not the machine of the worker.
    fn validation() -> Validation {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = false;
        validation.validate_aud = false;
        validation.leeway = 0;
        validation
    }

    /// Signs a payload with a key.
    fn sign<T: Serialize>(&self, payload: &T, key: &EncodingKey) -> Result<String, CryptoError> {
        encode(&Header::new(Algorithm::HS256), payload, key)
            .map_err(|err| CryptoError::Internal(err.to_string()))
    }

    /// Decodes a payload with a key.
    fn decode<T: for<'de> Deserialize<'de>>(
        &self,
        token: &str,
        key: &DecodingKey,
    ) -> Result<T, CryptoError> {
        decode::<T>(token, key, &Self::validation())
            .map(|data| data.claims)
            .map_err(|_| CryptoError::Invalid)
    }

    /// Whether the token is still alive, according to the injected clock.
    fn is_alive(&self, expires_at: i64) -> Result<(), CryptoError> {
        if expires_at <= self.clock.now() {
            return Err(CryptoError::Expired);
        }
        Ok(())
    }
}

impl TokenSigner for Hs256TokenSigner {
    fn sign_access(&self, claims: &AccessClaims) -> Result<String, CryptoError> {
        let payload = AccessPayload {
            sub: claims.subject.clone(),
            role: claims.role.clone(),
            exp: claims.expires_at,
            iat: claims.issued_at,
            jti: claims.token_id.clone(),
            typ: ACCESS_TYP.to_string(),
        };
        self.sign(&payload, &self.access_encoding)
    }

    fn verify_access(&self, token: &str) -> Result<AccessClaims, CryptoError> {
        let payload: AccessPayload = self.decode(token, &self.access_decoding)?;
        if payload.typ != ACCESS_TYP {
            return Err(CryptoError::Invalid);
        }
        self.is_alive(payload.exp)?;
        Ok(AccessClaims {
            subject: payload.sub,
            role: payload.role,
            issued_at: payload.iat,
            expires_at: payload.exp,
            token_id: payload.jti,
        })
    }

    fn sign_refresh(&self, claims: &RefreshClaims) -> Result<String, CryptoError> {
        let payload = RefreshPayload {
            sub: claims.subject.clone(),
            fam: claims.family_id.clone(),
            exp: claims.expires_at,
            iat: claims.issued_at,
            jti: claims.token_id.clone(),
            typ: REFRESH_TYP.to_string(),
        };
        self.sign(&payload, &self.refresh_encoding)
    }

    fn verify_refresh(&self, token: &str) -> Result<RefreshClaims, CryptoError> {
        let payload: RefreshPayload = self.decode(token, &self.refresh_decoding)?;
        if payload.typ != REFRESH_TYP {
            return Err(CryptoError::Invalid);
        }
        self.is_alive(payload.exp)?;
        Ok(RefreshClaims {
            subject: payload.sub,
            family_id: payload.fam,
            issued_at: payload.iat,
            expires_at: payload.exp,
            token_id: payload.jti,
        })
    }

    fn sign_magic_link(&self, claims: &MagicLinkClaims) -> Result<String, CryptoError> {
        let payload = MagicLinkPayload {
            email: claims.email.clone(),
            name: claims.name.clone(),
            exp: claims.expires_at,
            iat: claims.issued_at,
            jti: claims.token_id.clone(),
            typ: MAGIC_LINK_TYP.to_string(),
        };
        self.sign(&payload, &self.access_encoding)
    }

    fn verify_magic_link(&self, token: &str) -> Result<MagicLinkClaims, CryptoError> {
        let payload: MagicLinkPayload = self.decode(token, &self.access_decoding)?;
        if payload.typ != MAGIC_LINK_TYP {
            return Err(CryptoError::Invalid);
        }
        self.is_alive(payload.exp)?;
        Ok(MagicLinkClaims {
            email: payload.email,
            name: payload.name,
            issued_at: payload.iat,
            expires_at: payload.exp,
            token_id: payload.jti,
        })
    }
}
