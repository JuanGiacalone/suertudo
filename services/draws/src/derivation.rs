//! The commit–reveal derivation, and the golden test that freezes it.
//!
//! The formula is documented in `docs/PRD.md` §7.2 and in
//! `docs/data-model.md`. This file is the executable version of it, and the
//! third implementation of it: the first was the document, the second an
//! independent Python script, and this one the code that actually runs.
//!
//! **The golden values were computed with the Python implementation, not
//! with this code.** That is the point: two independent implementations
//! agreeing is what makes the formula trustworthy. A value written by hand
//! is not a golden value.

use hmac::{Mac, SimpleHmac};
use sha2::{Digest, Sha256};

/// The message that is signed. It is public: a third party has to be able to
/// type it into their own implementation.
///
/// The format is frozen with the formula.
pub const MESSAGE_TEMPLATE: &str = "suertudo/v1|raffle:{raffle_id}|draw:{draw_index}";

/// How the winner is derived, in words a third party can follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerivationSpec {
    pub algorithm: &'static str,
    pub message_template: &'static str,
    pub hash: &'static str,
    pub encoding: &'static str,
    pub modulus: &'static str,
}

/// The description of the derivation, published with every raffle.
pub const SPEC: DerivationSpec = DerivationSpec {
    algorithm: "hmac-sha256/v1",
    message_template: MESSAGE_TEMPLATE,
    hash: "HMAC-SHA256",
    encoding: "u64_be(first 8 bytes of the digest)",
    modulus: "pool_size",
};

/// The specification of the derivation, as it is published.
pub fn public_derivation_spec() -> DerivationSpec {
    SPEC
}

/// The message that is signed for a raffle and a draw attempt.
pub fn derivation_message(raffle_id: &str, draw_index: u32) -> String {
    format!("suertudo/v1|raffle:{raffle_id}|draw:{draw_index}")
}

/// Derives the index of the winning number.
///
/// ```text
/// winner_index = u64_be(HMAC-SHA256(seed, message)[0..8]) % pool_size
/// ```
///
/// `draw_index` is the attempt number. It starts at zero and increases when
/// the raffle has no winner policy that redraws: the same seed with a
/// different index produces a different number, and the sequence is
/// reproducible.
// The HMAC key of any length is valid, so the construction cannot fail.
#[allow(clippy::expect_used)]
pub fn derive_winner_index(seed: &[u8], raffle_id: &str, draw_index: u32, pool_size: u64) -> u64 {
    assert!(pool_size > 0, "the pool of a raffle is never empty");

    let mut mac = SimpleHmac::<Sha256>::new_from_slice(seed).expect("HMAC accepts any key length");
    mac.update(derivation_message(raffle_id, draw_index).as_bytes());

    let digest = mac.finalize().into_bytes();
    let mut head = [0u8; 8];
    head.copy_from_slice(&digest[0..8]);

    u64::from_be_bytes(head) % pool_size
}

/// The commitment that is published when the raffle opens.
///
/// It is the hash of the seed: the seed itself stays sealed until the draw,
/// and the commitment proves afterwards that it was the one that was chosen
/// when the raffle opened.
pub fn commitment(seed: &[u8]) -> String {
    let digest = Sha256::digest(seed);
    hex::encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seed the golden values were computed with: the bytes 0 to 31.
    fn golden_seed() -> Vec<u8> {
        (0..32).map(|i| i as u8).collect()
    }

    /// The raffle the golden values were computed with.
    const GOLDEN_RAFFLE: &str = "3f8a1c22-0b6e-4a91-8d3f-2c7e5a9b1d04";

    #[test]
    fn the_commitment_is_the_sha256_of_the_seed() {
        // Independently computed with Python's hashlib.
        assert_eq!(
            commitment(&[0xAB; 32]),
            "9a2db2e23f1504cd056606553ac049c5e718e8f9ce9233876df1a7a1821af885"
        );
    }

    #[test]
    fn the_derivation_is_deterministic() {
        let seed = golden_seed();
        let first = derive_winner_index(&seed, GOLDEN_RAFFLE, 0, 1000);
        let second = derive_winner_index(&seed, GOLDEN_RAFFLE, 0, 1000);
        assert_eq!(first, second);
    }

    #[test]
    fn the_golden_value_is_the_one_of_the_documentation() {
        // Independently computed with Python's hmac and hashlib. If this
        // fails, the formula changed: the PRD, the data model and this
        // comment have to change with it.
        assert_eq!(
            derive_winner_index(&golden_seed(), GOLDEN_RAFFLE, 0, 1000),
            144,
            "the golden value changed: update docs/PRD.md §7.2 and docs/data-model.md"
        );
    }

    #[test]
    fn the_redraw_sequence_is_reproducible() {
        // What a third party computes when the raffle has no winner and the
        // policy redraws.
        let seed = golden_seed();
        let sequence: Vec<u64> = (0..4)
            .map(|i| derive_winner_index(&seed, GOLDEN_RAFFLE, i, 1000))
            .collect();
        assert_eq!(sequence, vec![144, 820, 74, 885]);
    }

    #[test]
    fn the_draw_index_changes_the_result() {
        let seed = [0x11u8; 32];
        let first = derive_winner_index(&seed, GOLDEN_RAFFLE, 0, 1000);
        let second = derive_winner_index(&seed, GOLDEN_RAFFLE, 1, 1000);
        assert_ne!(
            first, second,
            "a redraw has to be able to produce another number"
        );
    }

    #[test]
    fn the_raffle_changes_the_result() {
        let seed = [0x33u8; 32];
        let first = derive_winner_index(&seed, "raffle-a", 0, 1000);
        let second = derive_winner_index(&seed, "raffle-b", 0, 1000);
        assert_ne!(
            first, second,
            "two raffles must not share a winning sequence"
        );
    }

    #[test]
    fn the_result_is_always_inside_the_pool() {
        let seed = [0x44u8; 32];
        for pool in [10u64, 100, 1000, 1_000_000] {
            for index in 0..50 {
                let winner = derive_winner_index(&seed, "raffle", index, pool);
                assert!(
                    winner < pool,
                    "the winner {winner} is outside the pool {pool}"
                );
            }
        }
    }

    #[test]
    fn the_published_message_is_the_one_that_is_signed() {
        assert_eq!(
            derivation_message(GOLDEN_RAFFLE, 0),
            format!("suertudo/v1|raffle:{GOLDEN_RAFFLE}|draw:0")
        );
        assert_eq!(SPEC.message_template, MESSAGE_TEMPLATE);
    }
}
