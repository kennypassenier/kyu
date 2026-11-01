//! What `/healthz` says about the store and the sweeper (W6).

use std::sync::Arc;

use chassis::{Subsystem, SubsystemStatus};

use crate::engine::Engine;
use crate::sweeper::Heartbeat;

/// `store`: the write probe plus the last failed write. Two questions,
/// because they fail differently: the probe takes the write lock, which
/// catches a read-only store; the failure record catches what a probe
/// cannot — a disk that is full but writable only refuses at commit time.
/// The failure record is asked first and the probe skipped while it stands:
/// a successful probe clears the kit's record (the lock is taken without a
/// commit, so it proves less than a write), which would end the report on
/// the second poll with the disk still full. Both questions are the kit's
/// `Db` (`recent_write_failure`, `probe_writable`); the words are the hub's.
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
        if let Some(why) = db.recent_write_failure() {
            return SubsystemStatus::failing(format!(
                "a write failed within the last minute ({why}); the store may be full. Check \
                 free space on the data volume first, then that it is still mounted and \
                 writable by this user; the hub refuses publishes it cannot store and \
                 recovers by itself once writes succeed again"
            ));
        }
        if let Err(error) = db.probe_writable() {
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
