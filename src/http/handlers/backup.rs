//! W8 · online backup of the store.

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
        let bytes = db.vacuum_into(&target)?;
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
