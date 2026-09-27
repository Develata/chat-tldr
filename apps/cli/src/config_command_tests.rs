use super::*;

fn setup() -> (tempfile::TempDir, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(dir.path()), None).unwrap();
    (dir, paths)
}

#[test]
fn edits_preserve_unrelated_settings_and_replace_request_extensions() {
    let (_dir, paths) = setup();
    fs::write(
        &paths.config_file,
        "# retained comment\ntimezone='-03:00'\n[agent]\nmax_steps=7\n[custom]\nvalue='keep'\n",
    )
    .unwrap();
    let result = apply(
        &paths,
        "llm",
        ProviderUpdate {
            base_url: Some("https://example.test/v1".into()),
            model: Some("test-model".into()),
            key: Some(SecretString::new("test-private-key".into())),
            extra_body: Some(serde_json::json!({})),
            ..Default::default()
        },
        publish,
    )
    .unwrap();
    assert_eq!(result.config.agent.max_steps, 7);
    assert!(result.source.contains("# retained comment"));
    assert!(result.source.contains("value='keep'"));
    assert!(
        result
            .config
            .llm
            .extra_body
            .unwrap()
            .as_table()
            .unwrap()
            .is_empty()
    );
    let shown = serde_json::to_string(&snapshot(
        &paths,
        &result.source,
        &Config::parse(&result.source).unwrap(),
    ))
    .unwrap();
    assert!(!shown.contains("test-private-key"));
    assert!(!format!("{:?}", Config::parse(&result.source).unwrap()).contains("test-private-key"));
    assert_eq!(
        Config::load(&paths.config_file, true)
            .unwrap()
            .llm
            .api_key
            .unwrap()
            .expose(),
        "test-private-key"
    );
    assert!(!paths.database.exists());
}

#[test]
fn stale_revision_failed_publication_and_invalid_fields_preserve_original() {
    let (_dir, paths) = setup();
    let original = "[llm]\nmodel='before'\napi_key='synthetic-secret'\n";
    fs::write(&paths.config_file, original).unwrap();
    let updates = [
        ProviderUpdate {
            expected_revision: Some("stale".into()),
            model: Some("after".into()),
            ..Default::default()
        },
        ProviderUpdate {
            timeout_secs: Some(0),
            ..Default::default()
        },
        ProviderUpdate {
            base_url: Some("https://new.example/v1".into()),
            ..Default::default()
        },
        ProviderUpdate {
            base_url: Some("https://user:secret@example.test".into()),
            key: Some(SecretString::new("new-secret".into())),
            ..Default::default()
        },
        ProviderUpdate {
            key: Some(SecretString::new("bad\nkey".into())),
            ..Default::default()
        },
    ];
    for update in updates {
        assert!(apply(&paths, "llm", update, publish).is_err());
        assert_eq!(fs::read_to_string(&paths.config_file).unwrap(), original);
    }
    let update = ProviderUpdate {
        model: Some("after".into()),
        ..Default::default()
    };
    assert!(
        apply(&paths, "llm", update, |_, _| Err(Failure::new(
            "E_OUTPUT_WRITE",
            8,
            "simulated"
        )))
        .is_err()
    );
    assert_eq!(fs::read_to_string(&paths.config_file).unwrap(), original);
    let cleared = apply(
        &paths,
        "llm",
        ProviderUpdate {
            clear_key: true,
            ..Default::default()
        },
        publish,
    )
    .unwrap();
    assert!(cleared.config.llm.api_key.is_none());
    assert!(!cleared.source.contains("synthetic-secret"));
}

#[test]
fn rejects_oversized_and_invalid_input_without_echoing_secrets() {
    assert!(read_bounded(&vec![b'x'; MAX_INPUT as usize + 1][..]).is_err());
    assert!(read_bounded(&b"{}"[..]).is_ok());
    let (_dir, paths) = setup();
    assert!(
        apply(
            &paths,
            "jev",
            ProviderUpdate {
                api_format: Some("openai".into()),
                ..Default::default()
            },
            publish
        )
        .is_err()
    );
}
