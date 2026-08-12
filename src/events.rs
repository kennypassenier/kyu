//! The hub's own events (W11), published as ordinary messages onto
//! `mailbox.*` topics so that consuming them needs no special
//! integration — an HA automation subscribes the same way anything else
//! does.
//!
//! One rule is load-bearing (AR1): events *about* `mailbox.*` topics are
//! logged, never republished. Without it a broken consumer of
//! `mailbox.events` dead-letters, which emits a dead-letter event onto
//! the same topic, which dead-letters — a self-sustaining message
//! generator.
//!
//! Built in L6.
