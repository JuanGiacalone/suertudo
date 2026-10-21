//! The secrets: the randomness, the digest and the hashing of passwords.

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher as _, PasswordVerifier, SaltString};
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest as _, Sha256};

use crate::ports::{CryptoError, Digest, PasswordHasher, TokenSource};

/// The randomness of the operating system.
///
/// The tokens are credentials: they come from the CSPRNG of the kernel, never
/// from a predictable generator.
#[derive(Debug)]
pub struct RandomTokenSource;

impl TokenSource for RandomTokenSource {
    fn next_secret(&self) -> String {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        hex::encode(bytes)
    }

    fn next_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

/// The SHA-256 of a token.
///
/// Only the digest of a token is stored: if the database leaks, the tokens it
/// holds are not usable.
#[derive(Debug)]
pub struct Sha256Digest;

impl Digest for Sha256Digest {
    fn sha256_hex(&self, input: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        hex::encode(hasher.finalize())
    }
}

/// The hashing of passwords, with Argon2.
///
/// No endpoint uses it yet: the participant does not have a password and the
/// business login (`FR-ID-02`) reuses this adapter.
#[derive(Debug)]
pub struct Argon2PasswordHasher;

impl PasswordHasher for Argon2PasswordHasher {
    fn hash(&self, password: &str) -> Result<String, CryptoError> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|err| CryptoError::Internal(err.to_string()))
    }

    fn verify(&self, password: &str, hash: &str) -> Result<bool, CryptoError> {
        let parsed = PasswordHash::new(hash).map_err(|_| CryptoError::Invalid)?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    }
}
