//! Placeholder for the identity handlers.
//!
//! `TSK-F0-04` replaces this module with the real registration of the
//! identity functions and their HTTP triggers.

use iii_sdk::Error;

pub fn register(_worker: &iii_sdk::IIIClient) -> Result<(), Error> {
    Ok(())
}
