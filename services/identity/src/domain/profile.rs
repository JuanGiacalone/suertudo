//! The rules of the participant profile, the devices and the consents.
//!
//! The checks live in the domain so that every entry point applies the same
//! ones: an invalid phone number is invalid whether it arrives from the app
//! or from a script.

use crate::domain::errors::IdentityError;
use crate::domain::session::is_valid_platform;
use crate::domain::types::ProfilePatch;

/// The longest display name the profile accepts.
pub const MAX_DISPLAY_NAME: usize = 120;

/// The longest locale the profile accepts.
pub const MAX_LOCALE: usize = 35;

/// The longest timezone the profile accepts.
pub const MAX_TIMEZONE: usize = 64;

/// Validates a profile patch, listing every field that failed.
pub fn validate_patch(patch: &ProfilePatch) -> Result<(), IdentityError> {
    let mut fields: Vec<String> = Vec::new();

    if let Some(display_name) = &patch.display_name {
        let trimmed = display_name.trim();
        if trimmed.is_empty() || trimmed.chars().count() > MAX_DISPLAY_NAME {
            fields.push("display_name".to_string());
        }
    }

    if let Some(phone) = &patch.phone_e164
        && let Some(phone) = phone
        && !is_valid_phone(phone)
    {
        fields.push("phone_e164".to_string());
    }

    if let Some(locale) = &patch.locale
        && (locale.trim().is_empty() || locale.chars().count() > MAX_LOCALE)
    {
        fields.push("locale".to_string());
    }

    if let Some(timezone) = &patch.timezone
        && (timezone.trim().is_empty() || timezone.chars().count() > MAX_TIMEZONE)
    {
        fields.push("timezone".to_string());
    }

    if fields.is_empty() {
        Ok(())
    } else {
        Err(IdentityError::Validation {
            message: "Revisá los campos del perfil.".to_string(),
            fields,
        })
    }
}

/// Whether a phone number is in the E.164 form the profile accepts.
///
/// The form is a `+` followed by the digits of the country code and the
/// number, with at most fifteen digits in total.
pub fn is_valid_phone(phone: &str) -> bool {
    let Some(digits) = phone.strip_prefix('+') else {
        return false;
    };
    digits.len() >= 8 && digits.len() <= 15 && digits.chars().all(|c| c.is_ascii_digit())
}

/// Validates the registration of a push device.
pub fn validate_device(platform: &str, token: &str) -> Result<(), IdentityError> {
    let mut fields: Vec<String> = Vec::new();

    if !is_valid_platform(platform) {
        fields.push("platform".to_string());
    }
    if token.trim().is_empty() {
        fields.push("token".to_string());
    }

    if fields.is_empty() {
        Ok(())
    } else {
        Err(IdentityError::Validation {
            message: "Revisá los datos del dispositivo.".to_string(),
            fields,
        })
    }
}

/// Validates a business identifier used to scope a consent.
pub fn validate_business_id(business_id: &str) -> Result<(), IdentityError> {
    if business_id.trim().is_empty() {
        return Err(IdentityError::invalid(
            "business_id",
            "El negocio es obligatorio para el consentimiento.",
        ));
    }
    Ok(())
}
