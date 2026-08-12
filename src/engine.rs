//! Delivery semantics (AR1, AR9): publish, claim, ack, nack,
//! redelivery, dead-lettering, TTL, retention, idle lifecycle.
//!
//! This module is deliberately free of tokio, HTTP and any ambient wall
//! clock: time arrives through a `Clock` (AR7) and storage through
//! [`crate::store`], which is what makes the mocked-clock suites
//! (K5, K7, K9, K11) possible at all.
//!
//! Built in L1 (foundation) through L6 (lifecycle).
