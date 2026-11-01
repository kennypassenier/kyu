//! What `/healthz` says about the store and the sweeper (W6).

use std::sync::Arc;

use chassis::{Subsystem, SubsystemStatus};

use crate::engine::Engine;
use crate::sweeper::Heartbeat;

/// `store`: the last failed write plus the write probe. Two questions,
/// because they fail differently: the probe takes the write lock, which
/// catches a read-only store; the failure record catches what a probe
/// cannot, a disk that is full but writable only refuses at commit time.
/// Since chassis kit-storage-2 a passing probe keeps that record, so both
/// are asked on every poll. Both are the kit's `Db`
/// (`recent_write_failure_age`, `probe_writable`); the words are the hub's.
pub struct StoreSubsystem(pub Arc<Engine>);

impl Subsystem for StoreSubsystem {
    fn name(&self) -> &str {
        "store"
    }

    fn check(&self) -> SubsystemStatus {
        let db = match self.0.db() {
            Ok(db) => db,
            Err(error) => return SubsystemStatus::failing(format!("not open: {error}")),
        };
        let probe = db.probe_writable();
        if let Some((why, age)) = db.recent_write_failure_age() {
            return SubsystemStatus::failing(format!(
                "a write failed {} s ago ({why}); the store may be full. Check free space on \
                 the data volume first, then that it is still mounted and writable by this \
                 user; the hub refuses publishes it cannot store and recovers by itself once \
                 writes succeed again",
                age.as_secs()
            ));
        }
        if let Err(error) = probe {
            return SubsystemStatus::failing(format!(
                "unwritable: {error}. Check free space on the data volume first, then that \
                 it is still mounted, writable by this user and not locked by another \
                 process; the hub refuses publishes it cannot store and recovers by itself \
                 once writes succeed again"
            ));
        }
        SubsystemStatus::ok("writable")
    }
}

/// `sweeper`: whether the background work is still happening. While it is
/// stopped, expired leases are not returned to the queue and nothing is
/// dead-lettered, so messages appear to hang rather than to fail.
pub struct SweeperSubsystem {
    pub engine: Arc<Engine>,
    pub heartbeat: Heartbeat,
}

impl Subsystem for SweeperSubsystem {
    fn name(&self) -> &str {
        "sweeper"
    }

    fn check(&self) -> SubsystemStatus {
        let now = self.engine.now_ms();
        if self.heartbeat.is_alive(now) {
            return SubsystemStatus::ok("alive");
        }
        let behind_ms = now.saturating_sub(self.heartbeat.last_beat_ms());
        SubsystemStatus::failing(format!(
            "stalled: last ran {behind_ms} ms ago. Restart the service; until then expired \
             leases stay claimed and nothing is dead-lettered"
        ))
    }
}
