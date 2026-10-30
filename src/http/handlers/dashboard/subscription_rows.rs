//! The words and rows of the Subscription page: durations in plain language
//! and the policy card.

use serde_json::json;

use crate::dashboard::SubscriptionView;

/// A duration in words a person reads: `30 s`, `2 min`, `7 days`.
pub(super) fn format_duration_ms(ms: i64) -> String {
    let trim = |value: f64| {
        let text = format!("{value:.1}");
        text.strip_suffix(".0").map_or(text.clone(), str::to_string)
    };
    match ms {
        i64::MIN..=-1 => "0 s".to_string(),
        0..=999 => format!("{ms} ms"),
        1_000..=59_999 => format!("{} s", trim(ms as f64 / 1_000.0)),
        60_000..=3_599_999 => format!("{} min", trim(ms as f64 / 60_000.0)),
        3_600_000..=86_399_999 => format!("{} h", trim(ms as f64 / 3_600_000.0)),
        _ => {
            let days = trim(ms as f64 / 86_400_000.0);
            format!("{days} {}", if days == "1" { "day" } else { "days" })
        }
    }
}

/// The policy card's rows: label, value, and whether kyu chose the value
/// itself (the page prints a quiet "(default)" beside those).
pub(super) fn subscription_policy_rows(
    view: &SubscriptionView,
    created_at: i64,
) -> Vec<serde_json::Value> {
    let wait = if view.backoff_ms == 0 {
        "No wait".to_string()
    } else {
        format!(
            "{}, times the attempt number",
            format_duration_ms(view.backoff_ms)
        )
    };
    vec![
        json!({ "label": "Lease", "value": format_duration_ms(view.lease_ms),
                "is_default": view.explicit.lease_ms.is_none() }),
        json!({ "label": "Attempts before a message is dead", "value": view.max_attempts.to_string(),
                "is_default": view.explicit.max_attempts.is_none() }),
        json!({ "label": "Wait between attempts", "value": wait,
                "is_default": view.explicit.backoff_ms.is_none() }),
        json!({ "label": "Time to live",
                "value": view.ttl_ms.map_or("No limit".to_string(), format_duration_ms),
                "is_default": view.explicit.ttl_ms.is_none() }),
        json!({ "label": "Created", "created_ms": created_at, "value": "", "is_default": false }),
    ]
}
