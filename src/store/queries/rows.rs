//! Shared row-to-struct mapping helpers for the query modules.
//!
//! The six policy columns are selected in the same order by several
//! statements; mapping them in one place keeps those statements in step.

use super::StoredPolicy;

/// Reads the six policy columns (lease, max attempts, backoff, ttl, idle
/// flag, idle archive) starting at column `first`.
pub(super) fn policy_from_row(
    row: &rusqlite::Row<'_>,
    first: usize,
) -> rusqlite::Result<StoredPolicy> {
    Ok(StoredPolicy {
        lease_ms: row.get(first)?,
        max_attempts: row.get(first + 1)?,
        backoff_ms: row.get(first + 2)?,
        ttl_ms: row.get(first + 3)?,
        idle_flag_ms: row.get(first + 4)?,
        idle_archive_ms: row.get(first + 5)?,
    })
}
