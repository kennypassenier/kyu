//! mailbox — a durable message hub with a three-verb HTTP API.
//!
//! Module boundaries are frozen in `docs/ARCHITECTURE_DECISIONS.md`
//! (AR1). The split exists so that delivery semantics can be tested
//! without a runtime, a socket or a real clock:
//!
//! - [`engine`] — all delivery semantics as pure logic
//! - [`store`] — all SQL, the schema and its migrations
//! - [`http`] — HTTP to engine translation, nothing more
//! - [`dashboard`] — rendering (K10)
//! - [`events`] — the hub's own events onto `mailbox.*` topics (W11)
//!
//! [`config`] and [`sweeper`] sit outside that list on purpose: they are
//! shell concerns — configuration read once before anything can start, and
//! the timer that drives the engine's background transitions.

pub mod cli;
pub mod config;
pub mod crypto;
pub mod dashboard;
pub mod engine;
pub mod events;
pub mod http;
pub mod store;
pub mod sweeper;
