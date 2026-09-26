//! Attribution keeps its final candidate set stable across primary and LLM review.
use super::*;
use crate::store::TopicRecord;

const JEV_STATE_QUESTION_LIMIT: usize = 32_768;

impl Runtime<'_, '_> {
    pub(in crate::agent) fn choose_topic(
        &mut self,
        messages: &[StoredMessage],
        topics: &[&TopicRecord],
    ) -> Result<Option<(TopicRecord, &'static str)>> {
        let Some((request, count)) = bounded_request(messages, topics, &self.config.segment)?
        else {
            return Ok(None);
        };
        let candidates = &topics[..count];
        let subject = burst_subject(
            messages,
            &candidates
                .iter()
                .map(|topic| topic.id.clone())
                .collect::<Vec<_>>(),
        )?;
        let subjects = BTreeMap::from([("topic".into(), subject)]);
        let response = self.decide(&request, &subjects)?;
        let Some(Answer::Choice {
            choice, confidence, ..
        }) = response.answers.get("topic")
        else {
            return Err(crate::llm::ProviderError::invalid_output().into());
        };
        if choice == "new_topic" || *confidence < self.config.segment.tau_low {
            return Ok(None);
        }
        let (choice, method) = if *confidence >= self.config.segment.tau_high {
            (
                choice.as_str(),
                if self.fallback_active { "llm" } else { "jev" },
            )
        } else {
            let review = self.review_topic(&request, &subjects)?;
            let Some(Answer::Choice { choice, .. }) = review.answers.get("topic") else {
                return Err(crate::llm::ProviderError::invalid_output().into());
            };
            // A review is the final choice, not another confidence-gated recursion.
            return resolve_choice(choice, candidates, "llm_review");
        };
        resolve_choice(choice, candidates, method)
    }
}

fn resolve_choice(
    choice: &str,
    candidates: &[&TopicRecord],
    method: &'static str,
) -> Result<Option<(TopicRecord, &'static str)>> {
    if choice == "new_topic" {
        return Ok(None);
    }
    let topic = choice
        .strip_prefix('T')
        .and_then(|value| value.parse::<usize>().ok())
        .and_then(|index| candidates.get(index))
        .ok_or_else(crate::llm::ProviderError::invalid_output)?;
    Ok(Some(((*topic).clone(), method)))
}

/// Inputs are already ranked by the segment layer. Render the burst once and
/// account for removed JSON entries incrementally, avoiding repeated whole-burst
/// serialization while trimming candidates to the configured state budget.
fn bounded_request(
    messages: &[StoredMessage],
    topics: &[&TopicRecord],
    config: &crate::config::SegmentConfig,
) -> Result<Option<(DecisionRequest, usize)>> {
    let mut count = if topics.len() > 254 {
        (config.candidate_k as usize).min(254)
    } else {
        topics.len()
    };
    if count == 0 {
        return Ok(None);
    }
    let options = (0..count)
        .map(|n| (format!("T{n}"), format!("Continue candidate T{n}")))
        .chain([("new_topic".into(), "Start a new topic".into())])
        .collect();
    let mut request = DecisionRequest {
        state: json!({
            "candidate_topics":topics[..count].iter().enumerate().map(|(n,t)| json!({"ref":format!("T{n}"),"title":t.title})).collect::<Vec<_>>(),
            "new_messages":messages.iter().map(|m| crate::render::render(&m.message)).collect::<Vec<_>>()
        }),
        questions: BTreeMap::from([(
            "topic".into(),
            Question::Choice {
                instructions: "Which candidate topic do these messages continue?".into(),
                options,
            },
        )]),
    };
    // Count the provider wire shape: choice options are named `criteria` there.
    let mut size = {
        let wire = request.to_wire("");
        serde_json::to_string(&wire["state"])?.chars().count()
            + serde_json::to_string(&wire["questions"]["topic"])?
                .chars()
                .count()
    };
    let limit = (config.state_token_budget as usize).min(JEV_STATE_QUESTION_LIMIT);
    let keep = if size > limit {
        count.min(config.candidate_k as usize)
    } else {
        count
    };
    while count > keep || size > limit {
        if count == 1 {
            return Ok(None);
        }
        let row = request.state["candidate_topics"]
            .as_array_mut()
            .expect("array built above")
            .pop()
            .expect("count is nonzero");
        // There was at least one other row, so one comma disappears too.
        size -= serde_json::to_string(&row)?.chars().count() + 1;
        let key = format!("T{}", count - 1);
        let Question::Choice { options, .. } = request
            .questions
            .get_mut("topic")
            .expect("question built above")
        else {
            unreachable!("choice built above")
        };
        let value = options.remove(&key).expect("option built above");
        // A one-entry object adds two braces; removing an entry also removes a
        // comma because new_topic is retained, hence the net subtraction of one.
        size -= serde_json::to_string(&BTreeMap::from([(key, value)]))?
            .chars()
            .count()
            - 1;
        count -= 1;
    }
    Ok(Some((request, count)))
}

#[cfg(test)]
mod tests;
