//! The ES modules the four dashboard page scripts are split into.
//!
//! `topic.js`, `topics.js`, `subscription.js` and `common.js` (in the asset table of
//! [`handlers`](super::handlers)) are entry files; each imports its parts by a relative
//! URL such as `./topic-state.js`, which the browser resolves to `/assets/topic-state.js`
//! without the `?v=` fingerprint the entry carries. So these parts are served by the same
//! `/assets/{file}` route but never as `immutable`: they revalidate by `ETag` (the
//! fingerprint of all assets), and a changed binary can never be paired with stale parts.

/// One embedded module: its file name under `/assets/` and its source.
macro_rules! module {
    ($name:literal) => {
        ($name, include_str!(concat!("../../static/", $name)))
    };
}

/// Every part module, by file name. A new `static/<page>-<part>.js` is registered here.
pub const PAGE_MODULES: &[(&str, &str)] = &[
    module!("common-anchors.js"),
    module!("common-context.js"),
    module!("common-help.js"),
    module!("common-keys.js"),
    module!("common-kit.js"),
    module!("common-palette.js"),
    module!("common-shell.js"),
    module!("common-uses-graph.js"),
    module!("common-uses.js"),
    module!("subscription-attempts.js"),
    module!("subscription-backlog.js"),
    module!("subscription-gauges.js"),
    module!("subscription-header.js"),
    module!("subscription-page.js"),
    module!("subscription-state.js"),
    module!("subscription-uptime.js"),
    module!("subscription-util.js"),
    module!("topic-dead-actions.js"),
    module!("topic-dead-core.js"),
    module!("topic-dead.js"),
    module!("topic-detail.js"),
    module!("topic-header.js"),
    module!("topic-problems.js"),
    module!("topic-publish.js"),
    module!("topic-readers.js"),
    module!("topic-recent.js"),
    module!("topic-reconcile.js"),
    module!("topic-refresh.js"),
    module!("topic-send.js"),
    module!("topic-stage.js"),
    module!("topic-state.js"),
    module!("topic-subs-menu.js"),
    module!("topic-subs-rows.js"),
    module!("topic-subs.js"),
    module!("topic-util.js"),
    module!("topic-viewer.js"),
    module!("topics-attention.js"),
    module!("topics-filters.js"),
    module!("topics-heat.js"),
    module!("topics-kpis.js"),
    module!("topics-paint.js"),
    module!("topics-row.js"),
    module!("topics-state.js"),
    module!("topics-swap.js"),
    module!("topics-table.js"),
    module!("topics-test.js"),
    module!("topics-trends.js"),
    module!("topics-util.js"),
];

/// The content type of every part module.
pub const MODULE_CONTENT_TYPE: &str = "text/javascript; charset=utf-8";
