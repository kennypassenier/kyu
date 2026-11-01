//! The Topics section on the kit's status page (K2-2).

use std::sync::Arc;

use crate::dashboard::TopicView;
use crate::engine::Engine;

/// The Topics section on the kit's status page (K2-2): the counts at a
/// glance, the full list one click away on /topics.
pub struct TopicsSection(pub Arc<Engine>);

impl chassis::StatusSection for TopicsSection {
    fn render(&self) -> chassis::Section {
        let now = self.0.now_ms();
        let topics: Vec<TopicView> = self
            .0
            .store()
            .read(crate::store::queries::topic_summaries)
            .map(|summaries| {
                summaries
                    .into_iter()
                    .map(|summary| TopicView::at(summary, now))
                    .collect()
            })
            .unwrap_or_default();
        let backlog: i64 = topics.iter().map(|topic| topic.backlog).sum();
        let dead: i64 = topics.iter().map(|topic| topic.dead).sum();
        chassis::Section {
            title: "Topics".into(),
            explain: "What this hub is holding right now: every topic's unacknowledged \
                      messages add up to the backlog; dead letters gave up and wait for you."
                .into(),
            rows: vec![
                ("Topics".into(), topics.len().to_string()),
                ("Backlog".into(), backlog.to_string()),
                ("Dead letters".into(), dead.to_string()),
            ],
            html: Some(section_html(dead)),
        }
    }
}

/// The section's links and its one destructive action. [K-actions] "Prune
/// every dead letter" sweeps every topic and subscription at once, after the
/// storm that produced them is diagnosed. Kenny, 2026-11-01: a destructive
/// action keeps its own label and puts the consequence in the confirmation
/// dialog, so the button is drawn here with the dialog naming the count,
/// rather than as a kit `SectionAction`, whose dialog has one fixed text.
/// kp-themes 12.1 reads `data-kp-confirm-label` only in a typed confirm, so
/// the accept button still says "Yes, do it" until kp-themes reads it for
/// every confirm (queued in docs/KP_THEMES_11_IMPLEMENTATION.md).
fn section_html(dead: i64) -> String {
    let open = "<a class=\"kp-button\" href=\"/topics\">Open the topics</a>";
    if dead <= 0 {
        return format!("<p>{open}</p>");
    }
    let noun = if dead == 1 {
        "dead letter"
    } else {
        "dead letters"
    };
    format!(
        "<p>{open} <button type=\"button\" class=\"kp-button kp-button--destructive\" \
         data-post=\"/dashboard/dead-letters/prune\" data-busy-label=\"Pruning…\" \
         data-kp-destructive \
         data-kp-confirm=\"Delete all {dead} {noun} across every topic and subscription?\" \
         data-kp-confirm-label=\"Delete {dead} {noun}\">Prune every dead letter</button></p>"
    )
}
