//! The HTTP handlers, one module per area (AR2).
//!
//! Handlers translate HTTP into engine calls and back. No delivery logic
//! lives here — only the two things that are genuinely HTTP's business:
//! how a message is shaped on the wire, and how a long poll waits.
//!
//! Message API: [`publish`], [`receive`], [`settle`] (ack/nack). Management
//! API: [`policy`], [`dead`], [`retention`], [`backup`]. [`dashboard`] holds
//! the pages and the JSON they fetch; [`assets`] the embedded browser files.
//! Shared plumbing: [`wire`] (response shapes) and [`engine_call`].

pub mod assets;
pub mod backup;
pub mod dashboard;
pub mod dead;
pub mod engine_call;
pub mod policy;
pub mod publish;
pub mod receive;
pub mod retention;
pub mod settle;
pub mod wire;

pub use assets::{ASSET_VERSION, kyu_asset};
pub use backup::backup;
pub use dashboard::*;
pub use dead::{DeadQuery, RequeueQuery, delete_delivery, list_dead, requeue_dead, unarchive};
pub use policy::{PolicyBody, get_policy, put_policy};
pub use publish::{PublishQuery, publish};
pub use receive::{ReceiveQuery, receive};
pub use retention::{RetentionBody, get_retention, put_retention};
pub use settle::{AckQuery, NackQuery, ack, nack};
