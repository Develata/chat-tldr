use std::io::Write;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use chat_tldr_core::{
    AckPayload, ChatId, EventBody, FinishReason, InsightStats, RunStats, RunStatus, StatsPayload,
};
use chat_tldr_engine::{
    Config, EngineError,
    agent::{self, AnalyzeOptions, Models},
    decider::{Decider, JevDecider, LlmDecider},
    llm::Client,
    store,
};
use serde_json::json;

use crate::{Failure, args::AnalyzeArgs, commands::parse_time, output::Output, paths::Paths};

pub fn run<W: Write>(
    args: AnalyzeArgs,
    paths: &Paths,
    config: &Config,
    output: &mut Output<W>,
) -> Result<(), Failure> {
    if args.strategy != "ours" {
        return Err(Failure::new(
            "E_USAGE",
            2,
            "This binary supports only --strategy ours",
        ));
    }
    let mut options = AnalyzeOptions::from_config(config);
    options.since = parse_time(args.since.as_deref(), "--since")?;
    options.until = parse_time(args.until.as_deref(), "--until")?;
    options.decider = args.decider;
    if let Some(value) = args.max_steps {
        options.max_steps = value;
    }
    if let Some(value) = args.budget_usd {
        options.budget_usd = value;
    }
    options.validate()?;
    let html_output = args
        .html
        .as_deref()
        .map(|path| crate::html::prepare(path, paths))
        .transpose()?;
    let chat = ChatId(args.chat);
    let mut plan = agent::plan(&paths.database, &chat, &options)?;
    let has_llm_key = key_present(&config.llm.api_key_env);
    let has_jev_key = key_present(&config.jev.api_key_env);
    plan["readiness"] = json!({"read":true,"analyze":has_llm_key,"jev_key_present":has_jev_key,"llm_key_present":has_llm_key});
    if args.dry_run {
        output.emit(EventBody::Ack(AckPayload {
            command: "analyze".into(),
            target: Some(chat.to_string()),
            changed: false,
            detail: json!({"plan":plan}),
        }))?;
        return Ok(());
    }
    let run = output.run_id();
    if plan["messages"] == 0 {
        output.analysis_finished(RunStatus::Complete, FinishReason::Done);
        output.emit(EventBody::Stats(StatsPayload::Run(RunStats {
            run_id: run,
            chat_id: chat.clone(),
            messages_analyzed: 0,
            topics_created: 0,
            topics_updated: 0,
            insights: InsightStats::default(),
            usage: Vec::new(),
            cost_usd: 0.0,
            elapsed_ms: 0,
        })))?;
        if let Some(html_output) = html_output {
            export_html(paths, &chat, config, html_output, false, output)?;
        }
        return Ok(());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let handler_cancel = Arc::clone(&cancel);
    ctrlc::set_handler(move || handler_cancel.store(true, Ordering::Relaxed))
        .map_err(|_| Failure::new("E_INTERNAL", 1, "Cannot install cancellation handler"))?;
    let llm = Client::new(&config.llm).map_err(EngineError::from)?;
    let fallback = LlmDecider::new(&llm, config.llm.model.clone());
    let primary = if options.decider == "jev" && has_jev_key {
        Some(JevDecider::new(&config.jev).map_err(EngineError::from)?)
    } else {
        None
    };
    let mut committed = false;
    let mut output_error = None;
    let result = agent::analyze(
        &paths.database,
        &chat,
        &run,
        config,
        &options,
        Models {
            llm: &llm,
            primary: primary.as_ref().map(|decider| decider as &dyn Decider),
            fallback: &fallback,
        },
        &cancel,
        &mut |event| {
            if matches!(event, EventBody::Insight(_) | EventBody::Topic(_))
                || matches!(&event, EventBody::Progress(progress) if progress.stage == "store" && progress.current > 0)
            {
                committed = true;
            }
            output.emit(event).map_err(|error| {
                output_error = Some(error);
                EngineError::Io(std::io::Error::other("Cannot write JSONL output"))
            })
        },
    );
    if let Some(error) = output_error {
        return Err(error.into());
    }
    let result = result.map_err(|error| {
        let mut failure = Failure::from(error);
        if committed {
            failure.exit_code = 6;
        }
        output.analysis_finished(
            if committed {
                RunStatus::Partial
            } else {
                RunStatus::Failed
            },
            FinishReason::Error,
        );
        failure
    })?;
    output.analysis_finished(result.status, result.reason);
    committed |= result.stats.messages_analyzed > 0;
    output.emit(EventBody::Stats(StatsPayload::Run(result.stats)))?;
    if let Some(html_output) = html_output {
        export_html(paths, &chat, config, html_output, committed, output)?;
    }
    Ok(())
}

fn key_present(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

fn export_html<W: Write>(
    paths: &Paths,
    chat: &ChatId,
    config: &Config,
    html_output: crate::html::PreparedOutput,
    committed: bool,
    output: &mut Output<W>,
) -> Result<(), Failure> {
    let result = (|| {
        let snapshot = store::inbox(
            &paths.database,
            chat,
            &store::InboxOptions {
                all: false,
                include_resolved: false,
                include_rejected: false,
                now: chrono::Utc::now().with_timezone(&config.timezone_offset()?),
            },
        )?;
        crate::html::write(&snapshot, html_output, paths)
    })();
    result.map_err(|mut error: Failure| {
        if committed {
            error.exit_code = 6;
            error
                .message
                .push_str("; completed analysis data remains saved");
            output.analysis_finished(RunStatus::Partial, FinishReason::Error);
        } else {
            output.analysis_finished(RunStatus::Failed, FinishReason::Error);
        }
        error
    })
}
