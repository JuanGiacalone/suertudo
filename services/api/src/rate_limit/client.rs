//! The address of the client.
//!
//! The payload of an HTTP trigger of iii carries the headers, the path, the
//! method and the body —**and not the address of the peer** (`HttpCallRequest`
//! of the SDK 0.24.6). The only source of the address is therefore a header,
//! and a header can be sent by the client.
//!
//! The trust is **structural** and not a configuration: `ADR-012` says that
//! the engine of iii is not exposed to the internet and that the reverse
//! proxy is the only way in. The edge **appends** the address it saw to
//! `X-Forwarded-For`, so the last entry is the one of the edge and
//! everything before it comes from the client. Taking the first entry
//! would be taking what the client sent.
//!
//! When there is no edge in front —development, smoke test— there is no
//! header, there is no address, and the dimensions by address do not apply.

use std::collections::HashMap;

/// The address of the client, as the edge that fronts the API saw it.
pub fn client_ip(headers: &HashMap<String, String>) -> Option<String> {
    let forwarded = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-forwarded-for"))?
        .1;

    // The last entry is the one of the edge: everything before it was sent
    // by the client.
    let last = forwarded
        .split(',')
        .map(str::trim)
        .rev()
        .find(|part| !part.is_empty())?;

    if last.is_empty() {
        None
    } else {
        Some(last.to_string())
    }
}
