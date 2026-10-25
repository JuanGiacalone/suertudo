//! The adapters of the API worker.
//!
//! Every trait of `crate::ports` has its implementation here, and the
//! domain only ever sees the traits.

pub mod redis;
