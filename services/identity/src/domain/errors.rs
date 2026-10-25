//! The failures the identity domain can produce.
//!
//! Every variant maps to exactly one code of the error catalogue of
//! `docs/api-contract.md` §1.4. The catalogue is the contract: a client
//! switches on the code, not on the message.

use thiserror::Error;

/// A failure with an HTTP meaning.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdentityError {
    /// A field is missing or malformed. The HTTP status is `400`.
    #[error("{message}")]
    Validation {
        message: String,
        /// The fields that failed, listed in `details`.
        fields: Vec<String>,
    },

    /// The magic link was already used. The HTTP status is `400`.
    #[error("Este enlace ya se usó. Pedí uno nuevo.")]
    LinkConsumed,

    /// There is no session, or the session is not valid anymore.
    /// The HTTP status is `401`.
    #[error("Necesitás iniciar sesión para hacer esto.")]
    AuthRequired,

    /// The access token is valid but expired. The HTTP status is `401`.
    #[error("Tu sesión expiró. Renová la sesión e intentá de nuevo.")]
    AuthExpired,

    /// The caller is authenticated but not allowed. The HTTP status is `403`.
    #[error("No tenés permisos para esta operación.")]
    Forbidden,

    /// The resource does not exist, or is not visible to the caller.
    /// The HTTP status is `404`.
    #[error("No encontramos lo que buscabas.")]
    NotFound,

    /// The request conflicts with the current state. The HTTP status is `409`.
    #[error("El estado actual no permite esta operación.")]
    Conflict,

    /// Something failed that the caller cannot fix. The HTTP status is `500`.
    #[error("Algo falló de nuestro lado. Probá de nuevo en un rato.")]
    Internal,

    /// The code of the verification of a device is wrong.
    ///
    /// The participant is doing the right thing: the code that reached the
    /// phone is not the one that was typed. It is not a permission error
    /// (`FR-ID-05`, `TSK-F0-10`).
    #[error("El código no es correcto. Pedí uno nuevo.")]
    DeviceCodeInvalid,

    /// The current owner of the device has no phone on file: the takeover
    /// cannot be verified, so the token stays where it is.
    #[error(
        "No podemos verificar el dispositivo: el dueño actual no tiene un teléfono en su ficha."
    )]
    DeviceOwnerUnverifiable,

    /// The request exhausted the rate limit of its group. The HTTP status
    /// is `429`, and the answer carries `Retry-After` and
    /// `X-RateLimit-Remaining` (`docs/api-contract.md` §1.6).
    #[error("Demasiados intentos. Probá de nuevo en un rato.")]
    RateLimited {
        /// The seconds the caller has to wait.
        retry_after: u64,
        /// The hits that are left in the window.
        remaining: u32,
    },

    /// A refusal of the database that the catalogue of the contract names.
    ///
    /// The database refuses with a SQLSTATE and a message; the worker
    /// answers with the code of the catalogue (`docs/api-contract.md` §1.4)
    /// and the message of the database goes to the details.
    #[error("{message}")]
    Refused {
        refusal: Refusal,
        message: String,
        detail: String,
    },
}

/// A refusal of the database that the error catalogue of the contract
/// names (`docs/api-contract.md` §1.4).
///
/// The database refuses with a SQLSTATE; the catalogue names it with a code
/// and an HTTP status. The two are tied here so that no refusal collapses
/// into a `500`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The participant reached the limit of tickets of the raffle.
    TicketLimitReached,
    /// The raffle is not open for reservations yet.
    RaffleNotOpen,
    /// The raffle no longer accepts reservations.
    RaffleClosed,
    /// The number is already reserved.
    NumberTaken,
    /// The number of numbers is out of the range of the raffle.
    TicketSizeOutOfRange,
    /// The caller is not the owner of the row it tried to write.
    RbacForbidden,
    /// The verification of the takeover of a device expired.
    DeviceVerificationExpired,
}

impl Refusal {
    /// The code of the catalogue (`docs/api-contract.md` §1.4).
    pub fn code(self) -> &'static str {
        match self {
            Refusal::TicketLimitReached => "TICKET_LIMIT_REACHED",
            Refusal::RaffleNotOpen => "RAFFLE_NOT_OPEN",
            Refusal::RaffleClosed => "RAFFLE_CLOSED",
            Refusal::NumberTaken => "NUMBER_TAKEN",
            Refusal::TicketSizeOutOfRange => "TICKET_SIZE_OUT_OF_RANGE",
            Refusal::RbacForbidden => "RBAC_FORBIDDEN",
            Refusal::DeviceVerificationExpired => "DEVICE_VERIFICATION_EXPIRED",
        }
    }

    /// The HTTP status of the code.
    pub fn status(self) -> u16 {
        match self {
            Refusal::TicketLimitReached
            | Refusal::RaffleNotOpen
            | Refusal::RaffleClosed
            | Refusal::NumberTaken => 409,
            Refusal::DeviceVerificationExpired => 409,
            Refusal::TicketSizeOutOfRange => 422,
            Refusal::RbacForbidden => 403,
        }
    }

    /// The message of the catalogue: human, in Spanish.
    pub fn message(self) -> &'static str {
        match self {
            Refusal::TicketLimitReached => "Alcanzaste el máximo de tickets de este sorteo.",
            Refusal::RaffleNotOpen => "El sorteo todavía no abrió.",
            Refusal::RaffleClosed => "El sorteo ya no acepta reservas.",
            Refusal::NumberTaken => "Alguno de los números ya está reservado.",
            Refusal::TicketSizeOutOfRange => "La cantidad de números no es la que pide el sorteo.",
            Refusal::RbacForbidden => "Tu rol no alcanza para esta operación.",
            Refusal::DeviceVerificationExpired => {
                "La verificación del dispositivo venció. Pedí una nueva."
            }
        }
    }
}

impl IdentityError {
    /// A validation error with a single field.
    pub fn invalid(field: &str, message: &str) -> Self {
        IdentityError::Validation {
            message: message.to_string(),
            fields: vec![field.to_string()],
        }
    }

    /// The code of the error catalogue (`docs/api-contract.md` §1.4).
    pub fn code(&self) -> &'static str {
        match self {
            IdentityError::Validation { .. } => "VALIDATION_ERROR",
            IdentityError::LinkConsumed => "AUTH_LINK_CONSUMED",
            IdentityError::AuthRequired => "AUTH_REQUIRED",
            IdentityError::AuthExpired => "AUTH_EXPIRED",
            IdentityError::Forbidden => "FORBIDDEN",
            IdentityError::NotFound => "NOT_FOUND",
            IdentityError::Conflict => "CONFLICT",
            IdentityError::DeviceCodeInvalid => "DEVICE_CODE_INVALID",
            IdentityError::DeviceOwnerUnverifiable => "DEVICE_OWNER_UNVERIFIABLE",
            IdentityError::RateLimited { .. } => "RATE_LIMITED",
            IdentityError::Internal => "INTERNAL_ERROR",
            IdentityError::Refused { refusal, .. } => refusal.code(),
        }
    }

    /// The HTTP status the error is returned with.
    pub fn status(&self) -> u16 {
        match self {
            IdentityError::Validation { .. } | IdentityError::LinkConsumed => 400,
            IdentityError::AuthRequired | IdentityError::AuthExpired => 401,
            IdentityError::Forbidden => 403,
            IdentityError::NotFound => 404,
            IdentityError::Conflict => 409,
            IdentityError::DeviceCodeInvalid => 400,
            IdentityError::DeviceOwnerUnverifiable => 409,
            IdentityError::RateLimited { .. } => 429,
            IdentityError::Internal => 500,
            IdentityError::Refused { refusal, .. } => refusal.status(),
        }
    }

    /// The fields listed in `details`, if the error carries any.
    pub fn fields(&self) -> &[String] {
        match self {
            IdentityError::Validation { fields, .. } => fields,
            _ => &[],
        }
    }

    /// The message of the database, if the error carries any.
    ///
    /// It goes to the details of the answer: the client switches on the
    /// code, and the message of the database is for whoever reads the logs.
    pub fn detail(&self) -> Option<&str> {
        match self {
            IdentityError::Refused { detail, .. } => Some(detail),
            _ => None,
        }
    }
}
