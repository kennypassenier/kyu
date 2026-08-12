//! Delivery semantics (AR1, AR9): publish, claim, ack, nack,
//! redelivery, dead-lettering, TTL, retention, idle lifecycle.
//!
//! This module is deliberately free of tokio, HTTP and any ambient wall
//! clock: time arrives through a [`clock::Clock`] and storage through
//! [`crate::store`], which is what makes the mocked-clock suites
//! (K5, K7, K9, K11) possible at all.
//!
//! L1 lands the primitives the semantics will need — injected time and
//! message identifiers. The transitions themselves arrive in L2 (verbs),
//! L4 (reliability) and L6 (lifecycle).

pub mod clock;
pub mod ids;
