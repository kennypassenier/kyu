//! Runs blocking engine calls off the async runtime (AR5).

use crate::http::error::ApiError;

/// Runs a blocking engine call off the async runtime (AR5): rusqlite is
/// synchronous, and holding the writer lock on a runtime thread would stall
/// every other request.
pub(crate) async fn spawn_engine<T, F>(work: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, crate::engine::EngineError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result.map_err(ApiError::from),
        Err(join_error) => Err(ApiError::from(crate::engine::EngineError::Internal(
            anyhow::anyhow!("a store task failed to run: {join_error}"),
        ))),
    }
}

/// Like [`spawn_engine`], for calls that write the store. Kept as its own
/// name so a handler says what it does; the backup ticket (feat-backup-1,
/// chassis 3.1.0) is no longer taken here: the kit's `Db` takes one for each
/// write transaction, so `<name> backup-pause` still waits for the writes in
/// flight and a new one is held until `backup-resume` (for at most the kit's
/// limit, then a retryable 503) instead of racing the pause hook's WAL
/// checkpoint. Taking a second ticket here would nest two, and a pause
/// starting between them would hold the outer one while waiting on itself.
pub(crate) async fn spawn_engine_write<T, F>(work: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, crate::engine::EngineError> + Send + 'static,
    T: Send + 'static,
{
    spawn_engine(work).await
}
