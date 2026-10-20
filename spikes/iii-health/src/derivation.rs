//! Golden test for the commit-reveal derivation formula.
//!
//! The formula is the trust promise of the product: any third party must be
//! able to recompute the winning number from four public values. If this
//! formula changes, this file changes, and the docs change with it.

use sha2::{Digest, Sha256};

/// Derive a winner index from a seed, a raffle id and a draw index.
///
/// winner_index = u64_be(HMAC-SHA256(seed, "suertudo/v1|raffle:{id}|draw:{i}")[0..8]) % pool_size
pub fn derive_winner_index(seed: &[u8], raffle_id: &str, draw_index: u32, pool_size: u64) -> u64 {
    use hmac::{Mac, SimpleHmac};

    let mut mac = SimpleHmac::<Sha256>::new_from_slice(seed).expect("HMAC accepts any key length");
    mac.update(format!("suertudo/v1|raffle:{raffle_id}|draw:{draw_index}").as_bytes());

    let digest = mac.finalize().into_bytes();
    let mut head = [0u8; 8];
    head.copy_from_slice(&digest[0..8]);

    u64::from_be_bytes(head) % pool_size
}

pub fn commitment(seed: &[u8]) -> String {
    let digest = Sha256::digest(seed);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn commitment_is_sha256_of_the_seed() {
    let seed = [0xABu8; 32];
    let c = commitment(&seed);
    assert_eq!(c.len(), 64);
    // Independently computed with Python's hmac/hashlib.
    assert_eq!(c, "9a2db2e23f1504cd056606553ac049c5e718e8f9ce9233876df1a7a1821af885");
}

#[test]
fn derivation_is_deterministic() {
    let seed = [0x11u8; 32];
    let a = derive_winner_index(&seed, "3f8a1c22-0b6e-4a91-8d3f-2c7e5a9b1d04", 0, 1000);
    let b = derive_winner_index(&seed, "3f8a1c22-0b6e-4a91-8d3f-2c7e5a9b1d04", 0, 1000);
    assert_eq!(a, b);
}

#[test]
fn derivation_matches_the_published_golden_value() {
    // Golden value computed independently with Python's hmac/hashlib, then
    // frozen. Changing the formula breaks this test on purpose, which forces
    // the PRD and data-model docs to be updated with it.
    let seed: Vec<u8> = (0..32).map(|i| i as u8).collect();
    let index = derive_winner_index(
        &seed,
        "3f8a1c22-0b6e-4a91-8d3f-2c7e5a9b1d04",
        0,
        1000,
    );
    assert_eq!(index, 144, "the golden value changed: update PRD and data-model docs");
}

#[test]
fn redraw_sequence_is_reproducible() {
    // The full redraw sequence a third party would recompute after a
    // no-winner policy redraw.
    let seed: Vec<u8> = (0..32).map(|i| i as u8).collect();
    let id = "3f8a1c22-0b6e-4a91-8d3f-2c7e5a9b1d04";
    let sequence: Vec<u64> = (0..4)
        .map(|i| derive_winner_index(&seed, id, i, 1000))
        .collect();
    assert_eq!(sequence, vec![144, 820, 74, 885]);
}

#[test]
fn draw_index_changes_the_result() {
    let seed = [0x22u8; 32];
    let id = "3f8a1c22-0b6e-4a91-8d3f-2c7e5a9b1d04";
    let first = derive_winner_index(&seed, id, 0, 1000);
    let second = derive_winner_index(&seed, id, 1, 1000);
    assert_ne!(first, second, "a redraw must be able to produce a different number");
}

#[test]
fn raffle_id_changes_the_result() {
    let seed = [0x33u8; 32];
    let a = derive_winner_index(&seed, "raffle-a", 0, 1000);
    let b = derive_winner_index(&seed, "raffle-b", 0, 1000);
    assert_ne!(a, b, "two raffles must not share a winning number sequence");
}

#[test]
fn result_is_always_inside_the_pool() {
    let seed = [0x44u8; 32];
    for pool in [10u64, 100, 1000, 1_000_000] {
        for i in 0..50 {
            let idx = derive_winner_index(&seed, "raffle", i, pool);
            assert!(idx < pool, "index {idx} out of range for pool {pool}");
        }
    }
}
