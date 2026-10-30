//! `--check`'s view of the store: open read-only, change nothing (fix-check-1).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::{STORE_FILE_NAME, Store, migrations};

/// The answer of [`Store::inspect`].
#[derive(Debug, Clone)]
pub struct Inspection {
    pub path: PathBuf,
    /// `None` when there is no store file yet.
    pub pending: Option<migrations::Pending>,
    pub quick_check: Option<String>,
}

impl Inspection {
    /// One line for `--check`'s output.
    pub fn describe(&self) -> String {
        match self.pending {
            None => format!(
                "no store yet at {}; it is created at start",
                self.path.display()
            ),
            Some(p) if p.is_behind() => format!(
                "store at {} is readable and intact: schema {}, this binary knows {} — the \
                 migration runs at start, after a snapshot; --check applies nothing",
                self.path.display(),
                p.current,
                p.target
            ),
            Some(p) => format!(
                "store OK at {} (schema {}, current)",
                self.path.display(),
                p.current
            ),
        }
    }
}

impl Store {
    /// What `--check` asks of the store, without touching it (fix-check-1):
    /// does it open, is it intact, and how far behind this binary is it. The
    /// file is opened read-only, no pragma is set, no migration runs and
    /// nothing is created — a state directory with no store yet is fine,
    /// because the store is created at start, not by a check.
    pub fn inspect(data_dir: &Path) -> Result<Inspection> {
        let path = data_dir.join(STORE_FILE_NAME);
        if !path.exists() {
            return Ok(Inspection {
                path,
                pending: None,
                quick_check: None,
            });
        }
        let conn = Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| {
            format!(
                "cannot open the store at {} read-only. Check file permissions; the \
                 service user must be able to read it.",
                path.display()
            )
        })?;
        let quick_check: String = conn
            .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
            .with_context(|| format!("cannot run quick_check on {}", path.display()))?;
        if quick_check != "ok" {
            anyhow::bail!(
                "the store at {} is damaged: quick_check says {quick_check:?}. Restore \
                 the newest backup (kyu.backup-*.db or the pre-migration snapshot) \
                 before starting.",
                path.display()
            );
        }
        let pending = migrations::pending(&conn)?;
        Ok(Inspection {
            path,
            pending: Some(pending),
            quick_check: Some(quick_check),
        })
    }
}
