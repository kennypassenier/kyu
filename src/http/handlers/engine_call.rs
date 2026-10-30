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

/// Like [`spawn_engine`], for calls that write the store (AR5, feat-backup-1
/// in chassis 3.1.0): holds a `chassis::shell::backup::writing_blocking()`
/// ticket for the call, so `<name> backup-pause` sees it as in flight and
/// waits for it, and a new one blocks until `backup-resume` instead of
/// racing the pause hook's WAL checkpoint.
pub(crate) async fn spawn_engine_write<T, F>(work: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, crate::engine::EngineError> + Send + 'static,
    T: Send + 'static,
{
    spawn_engine(move || {
        let _ticket = chassis::shell::backup::writing_blocking();
        work()
    })
    .await
}
