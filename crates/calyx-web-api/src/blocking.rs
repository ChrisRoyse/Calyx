use crate::{ApiError, ErrorCode};

pub(crate) async fn run_blocking<T, F>(label: &'static str, work: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(%label, error = ?error, "CALYX_WEB_API_BLOCKING_TASK_FAILED");
            Err(ApiError::of(ErrorCode::Internal))
        }
    }
}
