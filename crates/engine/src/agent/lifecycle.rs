//! Message-time topic expiry. Closing never moves messages or invalidates drafts.
use super::*;

#[derive(Clone, Copy)]
struct Extent {
    count: u64,
    first: DateTime<FixedOffset>,
    last: DateTime<FixedOffset>,
}

pub(super) struct TopicLifecycle {
    positions: BTreeMap<TopicId, usize>,
    activity: BTreeSet<(DateTime<FixedOffset>, TopicId)>,
    indexed: BTreeMap<TopicId, DateTime<FixedOffset>>,
    extents: BTreeMap<TopicId, Extent>,
}

impl TopicLifecycle {
    pub(super) fn new(snapshot: &AnalysisSnapshot) -> Self {
        let mut this = Self {
            positions: BTreeMap::new(),
            activity: BTreeSet::new(),
            indexed: BTreeMap::new(),
            extents: BTreeMap::new(),
        };
        for (position, topic) in snapshot.topics.iter().enumerate() {
            this.track(topic, position);
        }
        for message in &snapshot.messages {
            if let Some(topic) = &message.topic_id {
                this.add_member(topic, message.message.sent_at);
            }
        }
        this
    }

    fn track(&mut self, topic: &TopicRecord, position: usize) {
        self.positions.insert(topic.id.clone(), position);
        if let Some(previous) = self.indexed.remove(&topic.id) {
            self.activity.remove(&(previous, topic.id.clone()));
        }
        if topic.state == TopicState::Active {
            self.indexed.insert(topic.id.clone(), topic.last_message_at);
            self.activity
                .insert((topic.last_message_at, topic.id.clone()));
        }
    }

    fn add_member(&mut self, topic: &TopicId, sent_at: DateTime<FixedOffset>) {
        let extent = self.extents.entry(topic.clone()).or_insert(Extent {
            count: 0,
            first: sent_at,
            last: sent_at,
        });
        extent.count += 1;
        extent.first = extent.first.min(sent_at);
        extent.last = extent.last.max(sent_at);
    }

    /// The controller calls this once for each successfully assigned batch of
    /// previously unassigned messages. No message bodies are retained here.
    pub(super) fn assigned(
        &mut self,
        topic: &TopicRecord,
        position: usize,
        members: &[StoredMessage],
    ) -> TopicPayload {
        self.track(topic, position);
        for member in members {
            self.add_member(&topic.id, member.message.sent_at);
        }
        self.payload(topic)
    }

    pub(super) fn current<'a>(
        &self,
        topics: &'a [TopicRecord],
        id: &TopicId,
    ) -> Option<&'a TopicRecord> {
        self.positions
            .get(id)
            .and_then(|position| topics.get(*position))
    }

    pub(super) fn committed(&mut self, topic: &TopicRecord) {
        if let Some(position) = self.positions.get(&topic.id).copied() {
            self.track(topic, position);
        }
    }

    fn payload(&self, topic: &TopicRecord) -> TopicPayload {
        let extent = self.extents.get(&topic.id).copied().unwrap_or(Extent {
            count: 0,
            first: topic.last_message_at,
            last: topic.last_message_at,
        });
        TopicPayload {
            topic_id: topic.id.clone(),
            chat_id: topic.chat_id.clone(),
            title: topic.title.clone(),
            title_is_provisional: topic.provisional,
            state: topic.state,
            message_count: extent.count,
            first_message_at: extent.first,
            last_message_at: extent.last,
            is_chitchat: topic.is_chitchat,
            merged_into: None,
        }
    }

    /// Walk only expired active topics, not every topic for every burst. Store
    /// rechecks the latest persistent activity in the same transaction as close.
    pub(super) fn close_at(
        &mut self,
        at: DateTime<FixedOffset>,
        topics: &mut [TopicRecord],
        links: &mut TopicLinks,
        runtime: &mut Runtime<'_, '_>,
        sink: &mut dyn FnMut(EventBody) -> Result<()>,
    ) -> Result<()> {
        let limit = runtime.config.segment.topic_close_secs;
        let mut candidates = Vec::new();
        while let Some((last, id)) = self.activity.first() {
            let elapsed = at.signed_duration_since(*last);
            if elapsed <= chrono::TimeDelta::zero()
                || (elapsed.num_seconds() as u64) < limit
                || (elapsed.num_seconds() as u64 == limit && elapsed.subsec_nanos() == 0)
            {
                break;
            }
            candidates.push(id.clone());
            self.activity.pop_first();
        }
        if candidates.is_empty() {
            return Ok(());
        }
        let closed = runtime.session.close_topics(
            &candidates,
            at,
            runtime.config.segment.topic_close_secs,
        )?;
        // Count the entire transaction before any fallible JSONL output.
        runtime.stats.topics_updated += closed.len() as u64;
        for topic in &closed {
            let position = self.positions[&topic.id];
            topics[position] = topic.clone();
            self.indexed.remove(&topic.id);
            links.mark_closed(&topic.id, at);
        }
        for topic in closed {
            sink(EventBody::Topic(self.payload(&topic)))?;
        }
        Ok(())
    }
}
