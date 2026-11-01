//! K7 · changing one field of a subscription's policy from its dashboard page.
//!
//! The API's `PUT …/policy` replaces the whole policy; a page that edits one
//! field in place would have to send every other field back, including the
//! idle thresholds it does not show. This merges one field into what is
//! stored instead, so a field nobody touched keeps its explicit value.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::http::handlers::engine_call::spawn_engine_write;
use crate::http::handlers::policy::policy_json;
use crate::store::queries::StoredPolicy;

/// One field and its new value; `null` returns the field to its default.
#[derive(Debug, Deserialize)]
pub struct PolicyFieldBody {
    pub field: String,
    pub value: Option<i64>,
}

/// `POST /t/{topic}/dashboard/subs/{subscription}/policy`
pub async fn dashboard_set_policy_field(
    State(state): State<AppState>,
    Path((topic, subscription)): Path<(String, String)>,
    axum::Json(body): axum::Json<PolicyFieldBody>,
) -> Result<Response, ApiError> {
    let field = Field::parse(&body.field)?;
    let engine = state.engine.clone();
    let defaults = engine.defaults();
    let (effective, stored) = spawn_engine_write(move || {
        let (_, mut stored) = engine.policy(&topic, &subscription)?;
        *field.slot(&mut stored) = body.value;
        let effective = engine.set_policy(&topic, &subscription, stored)?;
        Ok((effective, stored))
    })
    .await?;
    let idle = (
        stored.idle_flag_ms.unwrap_or(defaults.idle_flag_ms),
        stored.idle_archive_ms.unwrap_or(defaults.idle_archive_ms),
    );
    Ok((
        StatusCode::OK,
        axum::Json(policy_json(effective, stored, idle)),
    )
        .into_response())
}

/// The policy fields the page edits.
#[derive(Debug, Clone, Copy)]
enum Field {
    Lease,
    MaxAttempts,
    Backoff,
    Ttl,
}

impl Field {
    /// An unknown name is refused rather than ignored.
    fn parse(name: &str) -> Result<Self, ApiError> {
        match name {
            "lease_ms" => Ok(Self::Lease),
            "max_attempts" => Ok(Self::MaxAttempts),
            "backoff_ms" => Ok(Self::Backoff),
            "ttl_ms" => Ok(Self::Ttl),
            other => Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("there is no policy field called {other:?}"),
                "send one of lease_ms, max_attempts, backoff_ms or ttl_ms",
            )),
        }
    }

    fn slot(self, stored: &mut StoredPolicy) -> &mut Option<i64> {
        match self {
            Self::Lease => &mut stored.lease_ms,
            Self::MaxAttempts => &mut stored.max_attempts,
            Self::Backoff => &mut stored.backoff_ms,
            Self::Ttl => &mut stored.ttl_ms,
        }
    }
}
