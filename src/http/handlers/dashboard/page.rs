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
/// The partials a page may include: the cross-page shell (command palette,
/// keys, help, tour, anchors, uses-map) and the Topic page's sections, one
/// file each. The kit renders a page from one source string, so every
/// `{% include "<name>" %}` line is expanded here, before rendering.
const PARTIALS: &[(&str, &str)] = &[
    ("_kp.html", include_str!("../../../../templates/_kp.html")),
    (
        "_topic_send.html",
        include_str!("../../../../templates/_topic_send.html"),
    ),
    (
        "_topic_subs.html",
        include_str!("../../../../templates/_topic_subs.html"),
    ),
    (
        "_topic_dead.html",
        include_str!("../../../../templates/_topic_dead.html"),
    ),
    (
        "_topic_recent.html",
        include_str!("../../../../templates/_topic_recent.html"),
    ),
    (
        "_topic_publish.html",
        include_str!("../../../../templates/_topic_publish.html"),
    ),
];

pub(crate) fn page<C: serde::Serialize>(
    dash: &Dashboard,
    nav: &str,
    source: &str,
    ctx: C,
) -> Result<Html<String>, ApiError> {
    let expanded = PARTIALS
        .iter()
        .fold(source.to_string(), |page, (name, body)| {
            page.replace(
                &format!("{{% include \"{name}\" %}}"),
                body.trim_end_matches('\n'),
            )
        });
    dash.render_project(nav, &expanded, ctx)
        .map_err(|error| internal(format!("cannot render the page: {error}")))
}

/// A view as JSON. A serialisation failure is logged and becomes `null`
/// rather than vanishing: a missing row in a page is then traceable.
pub(crate) fn to_json<T: serde::Serialize>(view: &T) -> serde_json::Value {
    serde_json::to_value(view).unwrap_or_else(|error| {
        tracing::error!(%error, "a dashboard view could not be serialised");
        serde_json::Value::Null
    })
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
