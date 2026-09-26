use super::*;
use crate::llm::tests::{Reply, provider, server};
use crate::llm::{LlmResponse, MockLlm};

fn request() -> DecisionRequest {
    DecisionRequest {
        state: json!({"message":"周五交报告"}),
        questions: BTreeMap::from([
            (
                "notice".into(),
                Question::Noul {
                    instructions: "Is this a notice?".into(),
                    criteria: Some(("Notice".into(), "Not a notice".into())),
                },
            ),
            (
                "topic".into(),
                Question::Choice {
                    instructions: "Choose the topic.".into(),
                    options: BTreeMap::from([
                        ("old".into(), "Old topic".into()),
                        ("new".into(), "New topic".into()),
                    ]),
                },
            ),
            (
                "urgency".into(),
                Question::Score {
                    instructions: "Rate urgency.".into(),
                    levels: vec!["Low".into(), "Medium".into(), "High".into()],
                },
            ),
        ]),
    }
}

fn jev_response() -> Value {
    json!({"model":"jev-1.13.0","answers":{
        "notice":{"type":"noul","noul":0.9},
        "topic":{"type":"choice","choice":"new","probabilities":{"old":0.1,"new":0.9},"confidence":0.8},
        "urgency":{"type":"score","score":1.25,"probabilities":{"0":0.0,"1":0.75,"2":0.25},"confidence":0.625,"legend":{"0":"Low","1":"Medium","2":"High"}}
    },"usage":{"input_tokens":100,"output_tokens":20}})
}

#[test]
fn jev_wire_shape_and_zero_based_scores_match_the_documented_api() {
    let (url, worker) = server(vec![Reply::json(jev_response())]);
    let mut config = provider(url);
    config.model = "jev-1.13.0".into();
    config.price_input_per_mtok = None;
    config.price_output_per_mtok = None;
    let response = JevDecider::testing(&config).decide(&request()).unwrap();
    assert!((response.usage.cost_usd - 0.0000042).abs() < 1e-12);
    assert!(matches!(response.answers["urgency"],Answer::Score{score,..} if score==1.25));
    let requests = worker.join().unwrap();
    assert!(requests[0].headers.starts_with("POST /v1/systemone "));
    let questions = &requests[0].body["questions"];
    assert_eq!(questions["notice"]["criteria"]["true"], "Notice");
    assert_eq!(questions["topic"]["criteria"]["new"], "New topic");
    assert_eq!(
        questions["urgency"]["criteria"],
        json!(["Low", "Medium", "High"])
    );
}

#[test]
fn rounded_jev_probabilities_are_normalized_but_malformed_distributions_are_rejected() {
    for (a, b, allowed) in [
        (0.34, 0.65, true),
        (0.35, 0.66, true),
        (0.3, 0.6, false),
        (0.341, 0.65, false),
    ] {
        let mut response = jev_response();
        response["answers"]["topic"]["probabilities"] = json!({"old":a,"new":b});
        let (url, worker) = server(vec![Reply::json(response)]);
        let mut config = provider(url);
        config.model = "jev-1.13.0".into();
        let result = JevDecider::testing(&config).decide(&request());
        worker.join().unwrap();
        assert_eq!(result.is_ok(), allowed);
        if let Ok(result) = result {
            let Answer::Choice { probabilities, .. } = &result.answers["topic"] else {
                panic!("choice")
            };
            assert!((probabilities.values().sum::<f32>() - 1.0).abs() < 1e-6);
        }
    }
}

#[test]
fn unknown_question_options_and_invalid_probabilities_are_rejected() {
    for mutation in 0..6 {
        let mut response = jev_response();
        match mutation {
            0 => response["answers"]["invented"] = json!({"type":"noul","noul":0.5}),
            1 => response["answers"]["topic"]["choice"] = json!("invented"),
            2 => response["answers"]["topic"]["probabilities"]["new"] = json!(0.1),
            3 => response["answers"]["notice"]["noul"] = json!(-0.1),
            4 => response["answers"]["urgency"]["legend"]["2"] = json!("invented"),
            _ => response["answers"]["urgency"]["score"] = json!(2.0),
        }
        let (url, worker) = server(vec![Reply::json(response)]);
        let mut config = provider(url);
        config.model = "jev-1.13.0".into();
        let error = JevDecider::testing(&config).decide(&request()).unwrap_err();
        assert_eq!(error.code(), "E_LLM_OUTPUT_INVALID");
        assert_eq!(error.usage().input_tokens, 100);
        worker.join().unwrap();
    }
}

fn llm_response(content: &str) -> Result<LlmResponse, ProviderError> {
    Ok(LlmResponse {
        content: content.into(),
        usage: Usage {
            input_tokens: 10,
            output_tokens: 10,
            cost_usd: 0.1,
        },
    })
}

#[test]
fn llm_fallback_retries_schema_once_normalizes_and_accounts_for_both_calls() {
    let answer = json!({"answers":{
        "notice":{"type":"noul","p_yes":0.9},
        "topic":{"type":"choice","probabilities":{"old":0.2,"new":0.6}},
        "urgency":{"type":"score","probabilities":{"0":0.0,"1":0.6,"2":0.2}}
    }});
    let mock = MockLlm::new(vec![
        llm_response("not json"),
        llm_response(&answer.to_string()),
    ]);
    let response = LlmDecider::new(&mock, "fixture")
        .decide(&request())
        .unwrap();
    assert_eq!(response.usage.input_tokens, 20);
    assert_eq!(response.usage.cost_usd, 0.2);
    assert!(
        matches!(&response.answers["topic"],Answer::Choice{choice,confidence,..} if choice=="new" && (*confidence-0.5).abs()<1e-5)
    );
    assert!(
        matches!(response.answers["urgency"],Answer::Score{score,..} if (score-1.25).abs()<1e-5)
    );
    assert_eq!(mock.requests().len(), 2);
    assert!(
        mock.requests()[1]
            .system
            .contains("previous result was invalid")
    );
    assert!(!mock.requests()[1].system.contains("not json"));
}

#[test]
fn unknown_llm_answer_keys_fail_after_exactly_one_schema_retry() {
    let mock = MockLlm::new(vec![
        llm_response("{\"answers\":{}}"),
        llm_response("{\"answers\":{}}"),
    ]);
    let error = LlmDecider::new(&mock, "fixture")
        .decide(&request())
        .unwrap_err();
    assert_eq!(error.code(), "E_LLM_OUTPUT_INVALID");
    assert_eq!(error.usage().input_tokens, 20);
    assert_eq!(mock.requests().len(), 2);
}

#[test]
fn invalid_typed_request_does_not_call_the_model() {
    let mock = MockLlm::new(Vec::new());
    let mut request = request();
    request.state = json!(["not an object"]);
    assert_eq!(
        LlmDecider::new(&mock, "fixture")
            .decide(&request)
            .unwrap_err()
            .code(),
        "E_PROVIDER_BAD_REQUEST"
    );
    assert!(mock.requests().is_empty());
}

#[test]
fn malformed_provider_envelope_is_retried_once_and_usage_is_retained() {
    let usage = Usage {
        input_tokens: 20,
        output_tokens: 5,
        cost_usd: 0.02,
    };
    let mock = MockLlm::new(vec![
        Err(ProviderError::invalid_output().with_usage(usage)),
        Err(ProviderError::invalid_output().with_usage(usage)),
    ]);
    let error = LlmDecider::new(&mock, "fixture")
        .decide(&request())
        .unwrap_err();
    assert_eq!(error.code(), "E_LLM_OUTPUT_INVALID");
    assert_eq!(error.usage().input_tokens, 40);
    assert_eq!(mock.requests().len(), 2);
}

#[test]
fn mock_decider_validates_probability_finiteness() {
    let req = DecisionRequest {
        state: json!({}),
        questions: BTreeMap::from([(
            "q".into(),
            Question::Noul {
                instructions: "Is it true?".into(),
                criteria: None,
            },
        )]),
    };
    let mock = MockDecider::new(
        "fixture",
        vec![Ok(DecisionResponse {
            model: "fixture".into(),
            answers: BTreeMap::from([("q".into(), Answer::Noul { p_yes: f32::NAN })]),
            usage: Usage::default(),
        })],
    );
    assert!(mock.decide(&req).is_err());
    assert_eq!(mock.requests().len(), 1);
}
