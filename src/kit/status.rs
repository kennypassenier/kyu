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
            html: Some("<p><a class=\"kp-button\" href=\"/topics\">Open the topics</a></p>".into()),
        }
    }

    /// [K-actions, chassis-rs 1.8.0] "Prune every dead letter" — every dead
    /// letter across every topic and subscription, in one confirmed sweep,
    /// once the storm that produced them is diagnosed. The one-at-a-time
    /// Delete on a topic page (W15) does not scale to a storm; this is that
    /// button's hub-wide sibling, not a replacement for it.
    fn actions(&self) -> Vec<chassis::SectionAction> {
        vec![chassis::SectionAction {
            label: "Prune every dead letter".into(),
            route: "/dashboard/dead-letters/prune".into(),
            method: "POST".into(),
            destructive: true,
            confirm: Some(
                "Delete every dead letter across every topic and subscription? \
                 This cannot be undone."
                    .into(),
            ),
            busy_label: Some("Pruning…".into()),
        }]
    }
}
