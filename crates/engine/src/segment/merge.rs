//! Reply-supported topic merge candidates, independent of models and persistence.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
};

use chat_tldr_core::{MessageId, TopicId, TopicState};

use crate::store::{StoredMessage, TopicRecord};

type Pair = (TopicId, TopicId);
type Rank = (Reverse<usize>, TopicId, TopicId);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MergeCandidate {
    pub(crate) into: TopicId,
    pub(crate) from: TopicId,
    pub(crate) reply_edges: usize,
    pub(crate) evidence: Vec<MessageId>,
}

/// Each source has at most one resolved reply. Representatives prefer a source
/// in the requested time range, then stable message IDs; at most three survive.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ReplyEvidence {
    outside_scope: bool,
    source: MessageId,
    target: MessageId,
}

#[derive(Default)]
struct Support {
    count: usize,
    in_scope: bool,
    evidence: BTreeSet<ReplyEvidence>,
}

impl Support {
    fn add(&mut self, evidence: ReplyEvidence) {
        self.count += 1;
        self.in_scope |= !evidence.outside_scope;
        self.evidence.insert(evidence);
        self.trim_evidence();
    }

    fn combine(&mut self, other: Self) {
        // Topic-pair supports are disjoint: each unique source is counted once.
        self.count += other.count;
        self.in_scope |= other.in_scope;
        self.evidence.extend(other.evidence);
        self.trim_evidence();
    }

    fn trim_evidence(&mut self) {
        while self.evidence.len() > 3 {
            self.evidence.pop_last();
        }
    }
}

/// Build once after assignment and update only incident edges after each merge.
/// The constructor uses borrowed message indexing, never copies message bodies,
/// and runs in O((N + T + E + R) log(N + T + E + R)); graph storage is O(T + E + R).
/// Here E is distinct topic-pair edges and R is persisted rejected pairs. The
/// temporary message index is O(N). Selecting/counting candidates never scans
/// messages, and merging visits only the removed topic's neighbors/rejections.
#[derive(Default)]
pub(crate) struct MergeGraph {
    edges: BTreeMap<Pair, Support>,
    neighbors: BTreeMap<TopicId, BTreeSet<TopicId>>,
    rejected: BTreeMap<TopicId, BTreeSet<TopicId>>,
    ranked: BTreeSet<Rank>,
    aliases: BTreeMap<TopicId, TopicId>,
}

impl MergeGraph {
    pub(crate) fn new(
        messages: &[StoredMessage],
        assignments: &BTreeMap<MessageId, TopicId>,
        topics: &[TopicRecord],
        rejected: &BTreeSet<Pair>,
        in_scope: &BTreeSet<MessageId>,
    ) -> Self {
        let active: BTreeMap<_, _> = topics
            .iter()
            .filter(|topic| topic.state == TopicState::Active)
            .map(|topic| (&topic.id, &topic.chat_id))
            .collect();
        // A repeated input row cannot manufacture additional support. Database
        // snapshots have unique IDs; use the first row defensively for duplicates.
        let mut indexed = BTreeMap::new();
        for message in messages {
            indexed.entry(&message.message.id).or_insert(message);
        }
        let mut graph = Self::default();
        for (left, right) in rejected {
            if left != right && active.contains_key(left) && active.get(left) == active.get(right) {
                graph.block(pair(left, right));
            }
        }
        for source in indexed.values() {
            if source.message.recalled {
                continue;
            }
            let Some(target_id) = source
                .message
                .reply_to
                .as_ref()
                .and_then(|reply| reply.resolved.as_ref())
            else {
                continue;
            };
            let Some(target) = indexed.get(target_id) else {
                continue;
            };
            if target.message.recalled
                || target.message.chat_id != source.message.chat_id
                || target.cursor >= source.cursor
            {
                continue;
            }
            let (Some(source_topic), Some(target_topic)) = (
                assignments.get(&source.message.id),
                assignments.get(target_id),
            ) else {
                continue;
            };
            if source_topic == target_topic
                || active.get(source_topic).copied() != Some(&source.message.chat_id)
                || active.get(target_topic).copied() != Some(&target.message.chat_id)
            {
                continue;
            }
            graph
                .edges
                .entry(pair(source_topic, target_topic))
                .or_default()
                .add(ReplyEvidence {
                    outside_scope: !in_scope.contains(&source.message.id),
                    source: source.message.id.clone(),
                    target: target_id.clone(),
                });
        }
        for ((left, right), support) in &graph.edges {
            graph
                .neighbors
                .entry(left.clone())
                .or_default()
                .insert(right.clone());
            graph
                .neighbors
                .entry(right.clone())
                .or_default()
                .insert(left.clone());
            if graph.is_candidate(left, right, support) {
                graph
                    .ranked
                    .insert((Reverse(support.count), left.clone(), right.clone()));
            }
        }
        graph
    }

    pub(crate) fn count(&self) -> usize {
        self.ranked.len()
    }

    pub(crate) fn next(&self) -> Option<MergeCandidate> {
        let (Reverse(reply_edges), into, from) = self.ranked.first()?;
        let support = self.edges.get(&(into.clone(), from.clone()))?;
        let mut evidence = Vec::with_capacity(6);
        for edge in &support.evidence {
            for id in [&edge.source, &edge.target] {
                if !evidence.contains(id) {
                    evidence.push(id.clone());
                }
            }
        }
        Some(MergeCandidate {
            into: into.clone(),
            from: from.clone(),
            reply_edges: *reply_edges,
            evidence,
        })
    }

    pub(crate) fn reject(&mut self, candidate: &MergeCandidate) {
        let left = self.canonical(&candidate.into);
        let right = self.canonical(&candidate.from);
        if left != right {
            self.block(pair(&left, &right));
        }
    }

    /// Call only after persistence accepts the merge. Supports from the removed
    /// node join existing supports, and any rejected relation is inherited.
    pub(crate) fn merge(&mut self, candidate: &MergeCandidate) {
        let left = self.canonical(&candidate.into);
        let right = self.canonical(&candidate.from);
        if left == right {
            return;
        }
        let (into, from) = pair(&left, &right);
        if !self.edges.contains_key(&(into.clone(), from.clone())) {
            return;
        }
        let neighbors = self.neighbors.remove(&from).unwrap_or_default();
        for neighbor in neighbors {
            let Some(mut support) = self.take_edge(&from, &neighbor) else {
                continue;
            };
            if neighbor == into {
                continue;
            }
            if let Some(existing) = self.take_edge(&into, &neighbor) {
                support.combine(existing);
            }
            self.put_edge(pair(&into, &neighbor), support);
        }
        let rejected = self.rejected.remove(&from).unwrap_or_default();
        for neighbor in rejected {
            if let Some(reverse) = self.rejected.get_mut(&neighbor) {
                reverse.remove(&from);
            }
            if neighbor != into {
                self.block(pair(&into, &neighbor));
            }
        }
        self.aliases.insert(from, into);
    }

    fn canonical(&self, id: &TopicId) -> TopicId {
        let mut canonical = id;
        while let Some(parent) = self.aliases.get(canonical) {
            canonical = parent;
        }
        canonical.clone()
    }

    fn is_candidate(&self, left: &TopicId, right: &TopicId, support: &Support) -> bool {
        support.count >= 2
            && support.in_scope
            && !self
                .rejected
                .get(left)
                .is_some_and(|neighbors| neighbors.contains(right))
    }

    fn block(&mut self, (left, right): Pair) {
        if let Some(support) = self.edges.get(&(left.clone(), right.clone())) {
            self.ranked
                .remove(&(Reverse(support.count), left.clone(), right.clone()));
        }
        self.rejected
            .entry(left.clone())
            .or_default()
            .insert(right.clone());
        self.rejected.entry(right).or_default().insert(left);
    }

    fn take_edge(&mut self, left: &TopicId, right: &TopicId) -> Option<Support> {
        let pair = pair(left, right);
        let support = self.edges.remove(&pair)?;
        self.ranked
            .remove(&(Reverse(support.count), pair.0, pair.1));
        for (node, neighbor) in [(left, right), (right, left)] {
            if let Some(neighbors) = self.neighbors.get_mut(node) {
                neighbors.remove(neighbor);
            }
        }
        Some(support)
    }

    fn put_edge(&mut self, (left, right): Pair, support: Support) {
        if self.is_candidate(&left, &right, &support) {
            self.ranked
                .insert((Reverse(support.count), left.clone(), right.clone()));
        }
        self.neighbors
            .entry(left.clone())
            .or_default()
            .insert(right.clone());
        self.neighbors
            .entry(right.clone())
            .or_default()
            .insert(left.clone());
        self.edges.insert((left, right), support);
    }
}

fn pair(left: &TopicId, right: &TopicId) -> Pair {
    if left < right {
        (left.clone(), right.clone())
    } else {
        (right.clone(), left.clone())
    }
}

#[cfg(test)]
mod tests;
