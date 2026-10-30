//! The hub's dashboard (K10, W9, AR11): the pages, the form actions behind
//! their buttons, and the JSON their scripts fetch.

pub mod actions;
pub mod bulk;
pub mod form;
pub mod history;
pub mod page;
pub mod stats;
pub mod subscription;
pub mod subscription_rows;
pub mod topic;
pub mod topic_data;
pub mod topics;

pub use actions::{
    dashboard_archive_subscription, dashboard_delete_delivery, dashboard_delete_subscription,
    dashboard_prune_dead_letters, dashboard_publish, dashboard_requeue,
};
pub use bulk::{dashboard_bulk_delete, dashboard_bulk_requeue, dashboard_replay};
pub use history::{AttemptsQuery, DaysQuery, dashboard_attempts, dashboard_daily};
pub use stats::{HoursQuery, dashboard_retention, dashboard_stats_hourly, dashboard_stats_totals};
pub use subscription::dashboard_subscription;
pub use topic::dashboard_topic;
pub use topics::{apps_redirect, topics_page};
