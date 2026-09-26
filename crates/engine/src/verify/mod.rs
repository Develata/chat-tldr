//! Deterministic evidence checks against the exact text presented to a model.

use chat_tldr_core::{InsightKind, MessageId, RenderProfile, VerificationStatus};

pub trait MessageLookup {
    /// None means missing message or unsupported rendering profile. Recalled
    /// messages may return an empty string; they must also be flagged below.
    fn rendered(&self, id: &MessageId, profile: RenderProfile) -> Option<String>;
    fn is_recalled(&self, id: &MessageId) -> bool;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftEvidence {
    pub message_id: MessageId,
    pub quote: String,
}

#[derive(Clone, Debug)]
pub struct VerifyInput<'a> {
    pub kind: InsightKind,
    pub deadline_raw: Option<&'a str>,
    pub evidence: &'a [DraftEvidence],
    pub profile: RenderProfile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyFailure {
    NoEvidence,
    MessageNotFound { index: usize },
    RecalledMessage { index: usize },
    QuoteTooShort { index: usize },
    QuoteNotFound { index: usize },
    DeadlineRawNotInEvidence,
    UnsupportedRenderProfile { version: u16 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyReport {
    pub status: VerificationStatus,
    pub evidence_ok: Vec<bool>,
    pub failures: Vec<VerifyFailure>,
}

pub fn verify(input: &VerifyInput<'_>, lookup: &dyn MessageLookup) -> VerifyReport {
    let mut report = VerifyReport {
        status: VerificationStatus::Rejected,
        evidence_ok: vec![false; input.evidence.len()],
        failures: Vec::new(),
    };
    if input.profile.version != 1 {
        report
            .failures
            .push(VerifyFailure::UnsupportedRenderProfile {
                version: input.profile.version,
            });
        return report;
    }
    if input.evidence.is_empty() {
        report.failures.push(VerifyFailure::NoEvidence);
    }
    let mut deadline_supported = input.deadline_raw.is_none();
    for (index, evidence) in input.evidence.iter().enumerate() {
        let Some(text) = lookup.rendered(&evidence.message_id, input.profile) else {
            report
                .failures
                .push(VerifyFailure::MessageNotFound { index });
            continue;
        };
        if lookup.is_recalled(&evidence.message_id) {
            report
                .failures
                .push(VerifyFailure::RecalledMessage { index });
            continue;
        }
        if evidence
            .quote
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .take(3)
            .count()
            < 3
        {
            report.failures.push(VerifyFailure::QuoteTooShort { index });
            continue;
        }
        if find_quote(&text, &evidence.quote).is_none() {
            report.failures.push(VerifyFailure::QuoteNotFound { index });
            continue;
        }
        report.evidence_ok[index] = true;
        // A missing/recalled message or fabricated quote cannot lend its
        // deadline to another evidence item that happened to verify.
        if let Some(raw) = input.deadline_raw
            && find_quote(&text, raw).is_some()
        {
            deadline_supported = true;
        }
    }
    if !deadline_supported {
        report
            .failures
            .push(VerifyFailure::DeadlineRawNotInEvidence);
    }
    report.status = if report.failures.is_empty() {
        VerificationStatus::Verified
    } else if input.kind == InsightKind::TopicSummary
        && input.deadline_raw.is_none()
        && report.evidence_ok.iter().any(|valid| *valid)
    {
        VerificationStatus::Unverified
    } else {
        VerificationStatus::Rejected
    };
    report
}

/// Find a whitespace-insensitive exact substring, returning Unicode scalar
/// indices into the original haystack. Leading/trailing whitespace is excluded;
/// whitespace between matched characters remains within the returned range.
/// Empty or whitespace-only quotes never match.
pub fn find_quote(haystack: &str, quote: &str) -> Option<(usize, usize)> {
    let needle: Vec<char> = quote.chars().filter(|ch| !ch.is_whitespace()).collect();
    if needle.is_empty() {
        return None;
    }
    let indexed: Vec<(usize, char)> = haystack
        .chars()
        .enumerate()
        .filter(|(_, ch)| !ch.is_whitespace())
        .collect();
    // Prefix matching keeps long repetitive model quotes linear in input size.
    let mut prefix = vec![0; needle.len()];
    let mut matched = 0;
    for index in 1..needle.len() {
        while matched > 0 && needle[index] != needle[matched] {
            matched = prefix[matched - 1];
        }
        if needle[index] == needle[matched] {
            matched += 1;
        }
        prefix[index] = matched;
    }
    matched = 0;
    for (index, (_, ch)) in indexed.iter().enumerate() {
        while matched > 0 && *ch != needle[matched] {
            matched = prefix[matched - 1];
        }
        if *ch == needle[matched] {
            matched += 1;
        }
        if matched == needle.len() {
            return Some((indexed[index + 1 - needle.len()].0, indexed[index].0 + 1));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    struct Lookup(HashMap<MessageId, (String, bool)>);

    impl MessageLookup for Lookup {
        fn rendered(&self, id: &MessageId, _: RenderProfile) -> Option<String> {
            self.0.get(id).map(|(text, _)| text.clone())
        }

        fn is_recalled(&self, id: &MessageId) -> bool {
            self.0.get(id).is_some_and(|(_, recalled)| *recalled)
        }
    }

    fn lookup() -> Lookup {
        Lookup(HashMap::from([
            ("m_report".into(), ("请在周五前提交报告。".into(), false)),
            ("m_template".into(), ("报告模板已上传。".into(), false)),
            ("m_recalled".into(), ("周五前的安排已撤回".into(), true)),
        ]))
    }

    fn evidence(id: &str, quote: &str) -> DraftEvidence {
        DraftEvidence {
            message_id: id.into(),
            quote: quote.into(),
        }
    }

    fn run(
        kind: InsightKind,
        deadline_raw: Option<&str>,
        evidence: &[DraftEvidence],
    ) -> VerifyReport {
        verify(
            &VerifyInput {
                kind,
                deadline_raw,
                evidence,
                profile: RenderProfile::default(),
            },
            &lookup(),
        )
    }

    #[test]
    fn unicode_whitespace_matching_uses_scalar_indices() {
        let original = "前🙂　周 五\n前交报告 后";
        assert_eq!(find_quote(original, "🙂周五前"), Some((1, 8)));
        assert_eq!(find_quote("前🙂abc后", "🙂a b c"), Some((1, 5)));
        assert_eq!(find_quote("甲e\u{301}🙂乙", "e\u{301}🙂"), Some((1, 4)));
        assert_eq!(find_quote(" \n周\t五　前 ", "　周五前\n"), Some((2, 7)));
        assert_eq!(find_quote("Friday 周五 前", "Friday周五前"), Some((0, 11)));
        assert_eq!(find_quote("周五前，周五前", "周五前"), Some((0, 3)));
        for invalid in ["", " \n\t　", "周四前", "周五之前"] {
            assert_eq!(find_quote(original, invalid), None, "{invalid:?}");
        }
        // Exact matching does not case-fold, normalize accents, or drop punctuation.
        assert_eq!(find_quote("e\u{301}", "é"), None);
        assert_eq!(find_quote("ABC", "abc"), None);
        assert_eq!(find_quote("周五，前", "周五前"), None);
    }

    #[test]
    fn repetitive_quotes_match_the_first_complete_occurrence() {
        assert_eq!(find_quote("ababababac", "ababac"), Some((4, 10)));
        let long = format!("{}乙", "甲".repeat(10_000));
        let quote = format!("{}乙", "甲".repeat(2_000));
        assert_eq!(find_quote(&long, &quote), Some((8_000, 10_001)));
        assert_eq!(find_quote("短文本", &quote), None);
    }

    #[test]
    fn all_documented_failures_are_reported_without_accepting_bad_evidence() {
        let empty = run(InsightKind::Todo, None, &[]);
        assert_eq!(empty.failures, [VerifyFailure::NoEvidence]);
        assert_eq!(empty.status, VerificationStatus::Rejected);
        let invalid = [
            evidence("missing", "周五前"),
            evidence("m_recalled", "周五前"),
            evidence("m_report", "周 五"),
            evidence("m_report", "周四前"),
        ];
        let report = run(InsightKind::Todo, Some("下个月"), &invalid);
        assert_eq!(report.evidence_ok, [false; 4]);
        assert_eq!(
            report.failures,
            [
                VerifyFailure::MessageNotFound { index: 0 },
                VerifyFailure::RecalledMessage { index: 1 },
                VerifyFailure::QuoteTooShort { index: 2 },
                VerifyFailure::QuoteNotFound { index: 3 },
                VerifyFailure::DeadlineRawNotInEvidence,
            ]
        );
        assert_eq!(report.status, VerificationStatus::Rejected);
    }

    #[test]
    fn topic_summary_can_be_partial_only_without_a_deadline() {
        let mixed = [
            evidence("m_report", "提交报告"),
            evidence("missing", "假的引用"),
        ];
        let summary = run(InsightKind::TopicSummary, None, &mixed);
        assert_eq!(summary.status, VerificationStatus::Unverified);
        assert_eq!(summary.evidence_ok, [true, false]);
        for kind in [
            InsightKind::Todo,
            InsightKind::MentionMe,
            InsightKind::Announcement,
            InsightKind::Decision,
        ] {
            assert_eq!(run(kind, None, &mixed).status, VerificationStatus::Rejected);
        }
        let with_deadline = run(InsightKind::TopicSummary, Some("周五前"), &mixed);
        assert_eq!(with_deadline.status, VerificationStatus::Rejected);
        assert!(
            !with_deadline
                .failures
                .contains(&VerifyFailure::DeadlineRawNotInEvidence)
        );
    }

    #[test]
    fn deadline_must_be_supported_by_a_valid_non_recalled_evidence_message() {
        let valid = [evidence("m_report", "提交报告")];
        // The raw expression can be elsewhere in the same valid message.
        let report = run(InsightKind::Todo, Some("周 五\n前"), &valid);
        assert_eq!(report.status, VerificationStatus::Verified);
        assert_eq!(report.evidence_ok, [true]);
        for second in [
            evidence("m_recalled", "周五前"),
            evidence("m_report", "不存在的引用"),
        ] {
            let mixed = [evidence("m_template", "模板已上传"), second];
            assert!(
                run(InsightKind::Todo, Some("周五前"), &mixed)
                    .failures
                    .contains(&VerifyFailure::DeadlineRawNotInEvidence)
            );
        }
        for raw in ["", "　\n", "下周三"] {
            assert!(
                run(InsightKind::Todo, Some(raw), &valid)
                    .failures
                    .contains(&VerifyFailure::DeadlineRawNotInEvidence)
            );
        }
    }

    #[test]
    fn future_render_profiles_cannot_silently_reuse_r1_evidence() {
        let evidence = [evidence("m_report", "提交报告")];
        let report = verify(
            &VerifyInput {
                kind: InsightKind::Todo,
                deadline_raw: None,
                evidence: &evidence,
                profile: RenderProfile { version: 2 },
            },
            &lookup(),
        );
        assert_eq!(report.status, VerificationStatus::Rejected);
        assert_eq!(report.evidence_ok, [false]);
        assert_eq!(
            report.failures,
            [VerifyFailure::UnsupportedRenderProfile { version: 2 }]
        );
    }
}
