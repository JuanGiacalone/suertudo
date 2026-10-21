//! The system clock.
//!
//! The domain never reads it directly: the expiry rules are only testable if
//! the clock is injected (AGENTS.md §8).

use crate::ports::Clock;

/// The wall clock of the machine.
#[derive(Debug)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        chrono::Utc::now().timestamp()
    }
}
