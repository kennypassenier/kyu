//! W8 · a consistent copy of the live store.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::Store;

impl Store {
    /// W8 · writes a consistent copy of the store while the hub keeps
    /// running.
    ///
    /// `VACUUM INTO` produces a complete database file, not a snapshot of
    /// bytes mid-write, so a backup taken under load is restorable. Copying
    /// the file by hand is the thing this exists to stop anyone doing.
    pub fn backup_to(&self, path: &Path) -> Result<u64> {
        if path.exists() {
            anyhow::bail!(
                "{} already exists. VACUUM INTO refuses to overwrite, so choose \
                 another name or remove the old backup first.",
                path.display()
            );
        }
        let conn = self
            .writer
            .lock()
            .expect("the writer lock is never poisoned");
        conn.execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
            .with_context(|| {
                format!(
                    "cannot write the backup to {}. Check free space and that the \
                     directory is writable by this user.",
                    path.display()
                )
            })?;
        let bytes = std::fs::metadata(path)
            .with_context(|| format!("cannot stat the backup at {}", path.display()))?
            .len();

        // Open what was just written and ask SQLite whether it is intact. A
        // backup is only a backup if it restores; reporting success for a
        // file that merely exists is how a truncated copy gets discovered
        // on the day it is needed.
        let verify = Connection::open(path)
            .with_context(|| format!("the backup at {} cannot be opened", path.display()))?;
        let integrity: String = verify
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .with_context(|| format!("the backup at {} cannot be checked", path.display()))?;
        anyhow::ensure!(
            integrity == "ok",
            "the backup at {} is not a sound database ({integrity}). It has been left \
             in place for inspection; take another one, and check free space first.",
            path.display()
        );

        Ok(bytes)
    }
}
