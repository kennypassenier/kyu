//! W8 · online backup of the store.

use std::path::Path;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::dashboard::page::internal;

/// W8 · `POST /api/backup`
///
/// Writes a consistent copy of the store while the hub keeps running.
pub async fn backup(State(state): State<AppState>) -> Result<Response, ApiError> {
    let engine = state.engine.clone();
    let keep = state.backup_keep;

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<(String, u64)> {
        let db = engine.db()?;
        let Some(path) = db.path() else {
            anyhow::bail!("this hub runs on an in-memory store, which has nothing to back up");
        };
        let directory = path.parent().unwrap_or(std::path::Path::new("."));
        // Named for the moment it was taken, so several backups coexist and
        // the newest is obvious from a directory listing.
        let target = directory.join(format!("kyu.backup-{}.db", engine.now_ms()));
        // `VACUUM INTO` writes a complete database, not a snapshot of bytes
        // mid-write, so a backup taken under load is restorable; it refuses
        // to overwrite a file that exists, and the kit opens what it wrote
        // and runs `integrity_check` on it before reporting success. Copying
        // the file by hand is the thing this exists to stop anyone doing.
        let bytes = match db.vacuum_into(&target) {
            Ok(bytes) => bytes,
            Err(error) => {
                // A failed `VACUUM INTO` can leave a partial file behind;
                // it would count as a backup and crowd out a good one.
                remove_failed(&target);
                return Err(error.into());
            }
        };
        prune(directory, &target, keep);
        Ok((target.display().to_string(), bytes))
    })
    .await
    .map_err(|error| internal(format!("the backup task failed: {error}")))?;

    match result {
        Ok((path, bytes)) => {
            tracing::info!(path = %path, bytes, "backup written");
            Ok((
                StatusCode::OK,
                axum::Json(json!({
                    "backup": path,
                    "bytes": bytes,
                    "restore": "stop the hub, replace kyu.db with this file (removing any \
                                kyu.db-wal and kyu.db-shm beside it), then start it \
                                again. The backup is a complete database, not a partial copy.",
                })),
            )
                .into_response())
        }
        Err(error) => Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the backup failed: {error:#}"),
            "check free space and that the data directory is writable. kyu keeps \
             serving either way; a failed backup does not affect delivery.",
        )),
    }
}

/// Removes the file a failed backup left, saying so if even that fails.
fn remove_failed(target: &Path) {
    match std::fs::remove_file(target) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(path = %target.display(), %error, "could not remove a failed backup")
        }
    }
}

/// Keeps the newest `keep` backups in `directory` (the one just written
/// always among them) and deletes the rest. The names embed the creation
/// time in milliseconds, so the newest is found by that number, not by file
/// metadata. Failures are logged and never fail the backup that succeeded.
fn prune(directory: &Path, written: &Path, keep: usize) {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(dir = %directory.display(), %error, "could not list backups to prune");
            return;
        }
    };
    let mut backups: Vec<(u64, std::path::PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stamp = name.strip_prefix("kyu.backup-")?.strip_suffix(".db")?;
            Some((stamp.parse().ok()?, entry.path()))
        })
        .collect();
    backups.sort_by_key(|backup| std::cmp::Reverse(backup.0));
    for (_, old) in backups.into_iter().skip(keep.max(1)) {
        if old == written {
            continue;
        }
        match std::fs::remove_file(&old) {
            Ok(()) => tracing::info!(path = %old.display(), "old backup pruned"),
            Err(error) => tracing::warn!(path = %old.display(), %error, "could not prune a backup"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_keeps_only_the_newest_backups_and_ignores_other_files() {
        let scratch = tempfile::tempdir().expect("a scratch dir");
        let dir = scratch.path().to_path_buf();
        for stamp in [100, 200, 300, 400, 5] {
            std::fs::write(dir.join(format!("kyu.backup-{stamp}.db")), b"x").expect("write");
        }
        std::fs::write(dir.join("kyu.db"), b"x").expect("write");
        prune(&dir, &dir.join("kyu.backup-400.db"), 2);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .expect("list")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["kyu.backup-300.db", "kyu.backup-400.db", "kyu.db"]);
    }
}
