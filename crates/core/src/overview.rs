//! Additive, versioned payload for `overview`'s ack.detail; v1 events stay unchanged.
use crate::{
    Attachment, ChatId, Cursor, InsightId, InsightPayload, MessageId, Priority, TopicId,
    TopicPayload,
};
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Overview {
    pub version: u32,
    pub chat_id: ChatId,
    pub since: DateTime<FixedOffset>,
    pub until: DateTime<FixedOffset>,
    pub generated_at: DateTime<FixedOffset>,
    pub data_start: Option<DateTime<FixedOffset>>,
    pub data_end: Option<DateTime<FixedOffset>>,
    pub window_messages: u64,
    pub pending_messages: u64,
    pub window_pending: u64,
    pub last_reviewed: Option<Cursor>,
    pub hot_topics: Vec<HotTopic>,
    pub priority_topics: Vec<TopicDigest>,
    pub related: Vec<RelatedInsight>,
    pub mentions: Vec<OverviewMessage>,
    pub deadlines: Vec<DeadlineItem>,
    pub unread_topics: Vec<TopicDigest>,
    pub resources: Vec<ResourceItem>,
    pub topics: Vec<TopicPayload>,
    pub insights: Vec<InsightPayload>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HotTopic {
    pub topic_id: TopicId,
    pub message_count: u64,
    pub meaningful_messages: u64,
    pub participants: u64,
    pub activity_score: f64,
    pub last_message_at: DateTime<FixedOffset>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TopicDigest {
    pub topic_id: Option<TopicId>,
    pub priority: Priority,
    /// Preserves engine ranking; the first item explains this topic's position.
    pub insight_ids: Vec<InsightId>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelatedInsight {
    pub insight_id: InsightId,
    pub reasons: Vec<String>,
}

wire_enum!(DeadlineStatus {
    Overdue,
    Upcoming,
    Uncertain
});

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeadlineItem {
    pub insight_id: InsightId,
    pub status: DeadlineStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OverviewMessage {
    pub message_id: MessageId,
    pub topic_id: Option<TopicId>,
    pub sent_at: DateTime<FixedOffset>,
    pub sender_display: String,
    pub text: String,
    pub analyzed: bool,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceItem {
    pub source: OverviewMessage,
    pub attachments: Vec<Attachment>,
    pub links: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "section", content = "row", rename_all = "snake_case")]
pub enum OverviewPart {
    HotTopics(HotTopic),
    PriorityTopics(TopicDigest),
    Related(RelatedInsight),
    Mentions(OverviewMessage),
    Deadlines(DeadlineItem),
    UnreadTopics(TopicDigest),
    Resources(ResourceItem),
    Topics(TopicPayload),
    Insights(InsightPayload),
}

impl OverviewPart {
    pub fn section(&self) -> &'static str {
        match self {
            Self::HotTopics(_) => "hot_topics",
            Self::PriorityTopics(_) => "priority_topics",
            Self::Related(_) => "related",
            Self::Mentions(_) => "mentions",
            Self::Deadlines(_) => "deadlines",
            Self::UnreadTopics(_) => "unread_topics",
            Self::Resources(_) => "resources",
            Self::Topics(_) => "topics",
            Self::Insights(_) => "insights",
        }
    }
    pub fn append(self, report: &mut Overview) {
        match self {
            Self::HotTopics(row) => report.hot_topics.push(row),
            Self::PriorityTopics(row) => report.priority_topics.push(row),
            Self::Related(row) => report.related.push(row),
            Self::Mentions(row) => report.mentions.push(row),
            Self::Deadlines(row) => report.deadlines.push(row),
            Self::UnreadTopics(row) => report.unread_topics.push(row),
            Self::Resources(row) => report.resources.push(row),
            Self::Topics(row) => report.topics.push(row),
            Self::Insights(row) => report.insights.push(row),
        }
    }
}

impl Overview {
    pub fn counts(&self) -> BTreeMap<String, u64> {
        [
            ("hot_topics", self.hot_topics.len()),
            ("priority_topics", self.priority_topics.len()),
            ("related", self.related.len()),
            ("mentions", self.mentions.len()),
            ("deadlines", self.deadlines.len()),
            ("unread_topics", self.unread_topics.len()),
            ("resources", self.resources.len()),
            ("topics", self.topics.len()),
            ("insights", self.insights.len()),
        ]
        .into_iter()
        .map(|(key, len)| (key.into(), len as u64))
        .collect()
    }

    /// Drain rows without copying their text. The remaining report is a header.
    pub fn drain_parts(&mut self) -> impl Iterator<Item = OverviewPart> + use<> {
        std::mem::take(&mut self.hot_topics)
            .into_iter()
            .map(OverviewPart::HotTopics)
            .chain(
                std::mem::take(&mut self.priority_topics)
                    .into_iter()
                    .map(OverviewPart::PriorityTopics),
            )
            .chain(
                std::mem::take(&mut self.related)
                    .into_iter()
                    .map(OverviewPart::Related),
            )
            .chain(
                std::mem::take(&mut self.mentions)
                    .into_iter()
                    .map(OverviewPart::Mentions),
            )
            .chain(
                std::mem::take(&mut self.deadlines)
                    .into_iter()
                    .map(OverviewPart::Deadlines),
            )
            .chain(
                std::mem::take(&mut self.unread_topics)
                    .into_iter()
                    .map(OverviewPart::UnreadTopics),
            )
            .chain(
                std::mem::take(&mut self.resources)
                    .into_iter()
                    .map(OverviewPart::Resources),
            )
            .chain(
                std::mem::take(&mut self.topics)
                    .into_iter()
                    .map(OverviewPart::Topics),
            )
            .chain(
                std::mem::take(&mut self.insights)
                    .into_iter()
                    .map(OverviewPart::Insights),
            )
    }
}
