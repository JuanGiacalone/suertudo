//! The configuration of the worker, read from the environment.
//!
//! The configuration is read once, at startup, and a mistake is a loud
//! failure: a worker that starts with the wrong secrets is worse than a
//! worker that does not start at all.

use thiserror::Error;

/// The minimum length of a signing key.
///
/// HS256 takes the key as is: a short key is a short signature. Thirty two
/// bytes is the size of the hash the algorithm is built on.
pub const MIN_SECRET_LENGTH: usize = 32;

/// Where the worker runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Development,
    Staging,
    Production,
}

impl Environment {
    /// The name of the environment, as it appears in `ENV`.
    pub fn as_str(self) -> &'static str {
        match self {
            Environment::Development => "development",
            Environment::Staging => "staging",
            Environment::Production => "production",
        }
    }
}

/// A configuration that cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfigError {
    /// A variable the worker needs is not set.
    #[error("{0} is required to run the identity worker")]
    Missing(&'static str),
    /// A signing key is too short to be safe.
    #[error("{0} needs at least {MIN_SECRET_LENGTH} characters")]
    WeakSecret(&'static str),
    /// The two signing keys are the same.
    #[error("JWT_SECRET and JWT_REFRESH_SECRET have to be different keys")]
    SameSecret,
    /// `ENV` is not one of the known environments.
    #[error("ENV is '{0}' and it is not development, staging or production")]
    UnknownEnvironment(String),
    /// The magic link is returned to the client outside development.
    ///
    /// Returning the token of a magic link outside development hands the
    /// session to whoever asks for the link: the worker refuses to start.
    #[error(
        "the magic link is returned to the client in the response and that is only allowed in development"
    )]
    DevMagicLinkOutsideDevelopment,
}

/// The configuration of the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub environment: Environment,
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_refresh_secret: String,
    /// Whether the token of a magic link travels in the response.
    pub dev_magic_link: bool,
    pub pool_max_connections: u32,
}

impl Config {
    /// Reads the configuration from the environment of the process.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(|key| std::env::var(key).ok())
    }

    /// Reads the configuration from a source of variables.
    ///
    /// The source is a function so that the rules can be tested without
    /// touching the environment of the process, which is shared with the
    /// other tests of the binary.
    pub fn from_vars<F>(vars: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let environment = match vars("ENV").as_deref() {
            None | Some("") => Environment::Production,
            Some("development") => Environment::Development,
            Some("staging") => Environment::Staging,
            Some("production") => Environment::Production,
            Some(other) => return Err(ConfigError::UnknownEnvironment(other.to_string())),
        };

        let database_url =
            non_empty(vars("DATABASE_URL")).ok_or(ConfigError::Missing("DATABASE_URL"))?;

        let jwt_secret = non_empty(vars("JWT_SECRET")).ok_or(ConfigError::Missing("JWT_SECRET"))?;
        if jwt_secret.len() < MIN_SECRET_LENGTH {
            return Err(ConfigError::WeakSecret("JWT_SECRET"));
        }

        let jwt_refresh_secret = non_empty(vars("JWT_REFRESH_SECRET"))
            .ok_or(ConfigError::Missing("JWT_REFRESH_SECRET"))?;
        if jwt_refresh_secret.len() < MIN_SECRET_LENGTH {
            return Err(ConfigError::WeakSecret("JWT_REFRESH_SECRET"));
        }
        if jwt_secret == jwt_refresh_secret {
            return Err(ConfigError::SameSecret);
        }

        // The magic link travels in the response when the environment is the
        // development one, or when the variable asks for it explicitly. The
        // second case is only allowed in development: outside it, the worker
        // refuses to start.
        let asked_for = is_truthy(vars("DEV_MAGIC_LINK").as_deref());
        let dev_magic_link = asked_for || environment == Environment::Development;
        if dev_magic_link && environment != Environment::Development {
            return Err(ConfigError::DevMagicLinkOutsideDevelopment);
        }

        let pool_max_connections = vars("DATABASE_MAX_CONNECTIONS")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(10);

        Ok(Self {
            environment,
            database_url,
            jwt_secret,
            jwt_refresh_secret,
            dev_magic_link,
            pool_max_connections,
        })
    }
}

/// The value of a variable, without the spaces around it.
fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Whether a variable asks for something to be on.
fn is_truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_lowercase).as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}
