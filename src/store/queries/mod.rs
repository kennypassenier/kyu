//! Every SQL statement in the project lives under this module (AR1, AR3).
//!
//! Functions take a [`rusqlite::Transaction`] rather than opening one, so the
//! engine decides what is atomic: publishing a message and creating its
//! delivery rows must be one transaction, or a crash between them would
//! leave a message nobody is subscribed to (AR3).
//!
//! All statements are parameterized (AR11) — there is a commit gate that
//! refuses string-built SQL.
//!
//! One file per table or concern; everything is re-exported here so callers
//! keep using `queries::name`.

mod apps;
mod attempts;
mod dead_letters;
mod deliveries;
mod delivery_admin;
mod expiry;
mod lifecycle;
mod messages;
mod overdue;
mod rows;
mod stats;
mod subscriptions;
mod summaries;
mod topics;

pub use apps::*;
pub use attempts::*;
pub use dead_letters::*;
pub use deliveries::*;
pub use delivery_admin::*;
pub use expiry::*;
pub use lifecycle::*;
pub use messages::*;
pub use overdue::*;
pub use stats::*;
pub use subscriptions::*;
pub use summaries::*;
pub use topics::*;
