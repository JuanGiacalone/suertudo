//! The iii surface of the API worker.
//!
//! Every function this worker exposes to the engine is registered here, with
//! its HTTP trigger. The handler code itself lives in the module that owns
//! the behaviour: `health` for the health endpoint, `middleware` for the
//! authorization context.
//!
//! The engine is never exposed to the internet: this worker is the only
//! public entry point (`docs/architecture.md` ADR-012).

use iii_sdk::Error;

/// Registers every function and trigger of the API worker.
pub fn register(worker: &iii_sdk::IIIClient) -> Result<(), Error> {
    crate::health::register(worker)?;
    Ok(())
}
