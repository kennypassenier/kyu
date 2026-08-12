//! Error responses (AR4). Every failure answers with
//! `{"error": …, "remedy": …}` — the remedy is not optional, because an
//! error that only says what went wrong leaves the caller exactly where
//! they were (standing rule 11).

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::engine::EngineError;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub error: String,
    pub remedy: String,
}

impl ApiError {
    pub fn new(status: StatusCode, error: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            status,
            error: error.into(),
            remedy: remedy.into(),
        }
    }

    pub fn payload_too_large(limit: usize) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("the message body is larger than the {limit} byte limit"),
            format!(
                "publish a smaller payload, or raise MAILBOX_MAX_BODY_BYTES (currently \
                 {limit}) and restart the hub. mailbox refuses an oversized message rather \
                 than storing part of it."
            ),
        )
    }

    pub fn invalid_wait(max_wait_s: u64) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            format!("wait must be between 0 and {max_wait_s} seconds"),
            format!(
                "ask for at most wait={max_wait_s}, or leave it out for the default. The \
                 value is refused rather than clamped, so a long poll never silently \
                 becomes a short one."
            ),
        )
    }

    pub fn missing_subscription() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "the as parameter is required: a consumer has to say who it is",
            "add ?as=<name>, for example ?as=ha-forwarder. The name is the \
             subscription: different names on one topic each receive every message, \
             while several processes sharing a name split the work between them.",
        )
    }

    pub fn unreadable_body(detail: &str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            format!("the request body could not be read: {detail}"),
            "retry the request; if it keeps failing, check for a proxy between the client \
             and the hub that is closing the connection early."
                .to_string(),
        )
    }
}

impl From<EngineError> for ApiError {
    fn from(error: EngineError) -> Self {
        let status = match &error {
            EngineError::InvalidName { .. } => StatusCode::BAD_REQUEST,
            EngineError::ReservedTopic { .. } => StatusCode::FORBIDDEN,
            EngineError::UnknownTopic { .. }
            | EngineError::UnknownSubscription { .. }
            | EngineError::NoSuchDelivery { .. } => StatusCode::NOT_FOUND,
            EngineError::AlreadyAcked { .. }
            | EngineError::NotClaimed { .. }
            | EngineError::NotDead { .. } => StatusCode::CONFLICT,
            EngineError::InvalidPolicy { .. } => StatusCode::BAD_REQUEST,
            EngineError::ReplayUnsupported => StatusCode::NOT_IMPLEMENTED,
            EngineError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        // Internal faults are ours, not the caller's: log the detail and
        // keep the response generic (AR4).
        if let EngineError::Internal(detail) = &error {
            tracing::error!(error = ?detail, "internal failure");
        }

        let remedy = error.remedy();
        let message = match &error {
            EngineError::Internal(_) => "mailbox hit an internal error".to_string(),
            other => other.to_string(),
        };

        Self::new(status, message, remedy)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "error": self.error, "remedy": self.remedy })),
        )
            .into_response()
    }
}
