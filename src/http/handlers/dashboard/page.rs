//! Shared page rendering for the dashboard: the embedded templates and the
//! one function that renders them inside the kit's layout, plus the error
//! mapping for dashboard faults.

use axum::http::StatusCode;
use axum::response::Html;
use chassis::Dashboard;

use crate::http::error::ApiError;

/// The hub's own page templates, rendered inside the kit's layout (K2-2,
/// 2026-09-06): the kit brings nav, login, theme picker and CSP; these
/// fill `content` and add the two hub assets in `head`.
pub(crate) const TOPICS_HTML: &str = include_str!("../../../../templates/topics.html");
pub(crate) const TOPIC_HTML: &str = include_str!("../../../../templates/topic.html");
pub(crate) const SUBSCRIPTION_HTML: &str = include_str!("../../../../templates/subscription.html");
/// The cross-page shell (command palette, keys, help, tour, anchors, uses-map).
/// The kit renders a page from one source string, so an
/// `{% include "_kp.html" %}` line is expanded here, before rendering.
const KP_HTML: &str = include_str!("../../../../templates/_kp.html");
const KP_INCLUDE: &str = "{% include \"_kp.html\" %}";

pub(crate) fn page<C: serde::Serialize>(
    dash: &Dashboard,
    nav: &str,
    source: &str,
    ctx: C,
) -> Result<Html<String>, ApiError> {
    let expanded = source.replace(KP_INCLUDE, KP_HTML);
    dash.render_project(nav, &expanded, ctx)
        .map_err(|error| internal(format!("cannot render the page: {error}")))
}

pub(crate) fn internal(message: String) -> ApiError {
    tracing::error!(error = %message, "dashboard failure");
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "the dashboard could not be rendered",
        "check the hub's logs for the matching error line; the message API is \
         unaffected by a dashboard fault.",
    )
}
