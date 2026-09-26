//! Pure event reduction. A provisional stream never becomes a reviewed view.
use std::collections::{BTreeMap, HashSet, VecDeque};

use chat_tldr_core::{
    ChatId, ChatPayload, CliEvent, Cursor, DecisionPayload, EventBody, EventStreamValidator,
    InboxPayload, InsightId, InsightPayload, JevAnswerPayload, Overview, ProgressPayload, RunStats,
    RunStatus, StatsPayload, TopicId, TopicPayload,
};
use serde::Deserialize;

use crate::bridge::{BridgeEvent, BridgePayload, CommandKind, Completion, RequestTag};

const LOG_LIMIT: usize = 200;
const HISTORY_LIMIT: usize = 200;
const VIEW_LIMIT: usize = 50_000;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Capabilities {
    pub commands: Vec<String>,
    #[serde(default)]
    pub strategies: Vec<String>,
    #[serde(default)]
    pub deciders: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct VersionInfo {
    cli_version: String,
    schema_version: String,
    capabilities: Capabilities,
}

#[derive(Clone, Debug)]
pub struct InboxView {
    pub request_id: u64,
    pub meta: InboxPayload,
    pub topics: Vec<TopicPayload>,
    pub insights: Vec<InsightPayload>,
    fresh: bool,
    displayed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FollowUp {
    Chats,
    Inbox(ChatId),
}

#[derive(Default)]
struct Pending {
    overview: Option<Overview>,
    overview_expected: Option<BTreeMap<String, u64>>,
    version: Option<VersionInfo>,
    chats: Vec<ChatPayload>,
    meta: Option<InboxPayload>,
    topics: Vec<TopicPayload>,
    insights: Vec<InsightPayload>,
    ids: HashSet<String>,
    problem: Option<String>,
}

#[derive(Default)]
pub struct GuiModel {
    pub overview: Option<OverviewView>,
    pub capabilities: Option<Capabilities>,
    pub cli_version: Option<String>,
    pub chats: Vec<ChatPayload>,
    pub selected_chat: Option<ChatId>,
    pub inbox: Option<InboxView>,
    pub progress: Option<ProgressPayload>,
    pub decisions: VecDeque<DecisionPayload>,
    pub stats: Option<RunStats>,
    pub history_stats: VecDeque<StatsPayload>,
    pub jev_answers: VecDeque<JevAnswerPayload>,
    pub logs: VecDeque<String>,
    pub last_error: Option<String>,
    pub last_completion: Option<Completion>,
    pub active: Option<RequestTag>,
    pending: Pending,
    latest_request: Option<u64>,
}

pub struct OverviewView {
    pub report: Overview,
    pub titles: BTreeMap<TopicId, String>,
    pub items: BTreeMap<InsightId, usize>,
}

impl OverviewView {
    fn new(report: Overview) -> Self {
        Self {
            titles: report
                .topics
                .iter()
                .map(|t| (t.topic_id.clone(), t.title.clone()))
                .collect(),
            items: report
                .insights
                .iter()
                .enumerate()
                .map(|(n, i)| (i.insight.id.clone(), n))
                .collect(),
            report,
        }
    }
}

impl GuiModel {
    pub fn handshake_ok(&self) -> bool {
        self.capabilities.is_some()
    }

    pub fn has_capability(&self, command: &str) -> bool {
        self.capabilities
            .as_ref()
            .is_some_and(|caps| caps.commands.iter().any(|value| value == command))
    }

    pub fn begin(&mut self, request: RequestTag) -> Result<(), String> {
        if self.active.is_some() {
            return Err("已有命令尚未结束".into());
        }
        if self
            .latest_request
            .is_some_and(|previous| request.id <= previous)
        {
            return Err("请求编号必须递增".into());
        }
        self.latest_request = Some(request.id);
        self.pending = Pending::default();
        self.progress = None;
        if !matches!(
            request.kind,
            CommandKind::Stats | CommandKind::Decisions | CommandKind::JevLog
        ) {
            self.overview = None;
        }
        match request.kind {
            CommandKind::Version => {
                self.capabilities = None;
                self.cli_version = None;
                self.last_error = None;
            }
            CommandKind::Analyze => {
                self.decisions.clear();
                self.stats = None;
                self.last_error = None;
            }
            CommandKind::Decisions => self.decisions.clear(),
            CommandKind::JevLog => self.jev_answers.clear(),
            CommandKind::Stats => self.history_stats.clear(),
            CommandKind::Import => self.last_error = None,
            _ => {}
        }
        if matches!(
            request.kind,
            CommandKind::Chats
                | CommandKind::Inbox
                | CommandKind::Analyze
                | CommandKind::Import
                | CommandKind::Feedback
                | CommandKind::Resolve
                | CommandKind::MarkRead
                | CommandKind::Version
        ) && let Some(inbox) = &mut self.inbox
        {
            inbox.fresh = false;
            inbox.displayed = false;
        }
        self.active = Some(request);
        Ok(())
    }

    pub fn select_chat(&mut self, chat: ChatId) -> bool {
        if self.selected_chat.as_ref() == Some(&chat) {
            return false;
        }
        self.selected_chat = Some(chat);
        self.overview = None;
        self.inbox = None;
        self.stats = None;
        self.progress = None;
        self.decisions.clear();
        self.jev_answers.clear();
        true
    }

    pub fn mark_inbox_displayed(&mut self, request_id: u64) {
        if self.active.is_none()
            && let Some(inbox) = &mut self.inbox
            && inbox.fresh
            && inbox.request_id == request_id
            && self.selected_chat.as_ref() == Some(&inbox.meta.chat_id)
        {
            inbox.displayed = true;
        }
    }

    pub fn mark_read_cursor(&self) -> Option<Cursor> {
        let inbox = self.inbox.as_ref()?;
        (self.active.is_none()
            && inbox.fresh
            && inbox.displayed
            && self.selected_chat.as_ref() == Some(&inbox.meta.chat_id))
        .then_some(inbox.meta.view_cursor)
        .flatten()
    }

    pub fn apply(&mut self, event: BridgeEvent) -> Option<FollowUp> {
        if self.active.as_ref() != Some(&event.request) {
            return None;
        }
        let current_chat = event.request.chat.is_none() || event.request.chat == self.selected_chat;
        match event.payload {
            BridgePayload::Finished(completion) => {
                self.finish(event.request, completion, current_chat)
            }
            BridgePayload::Stderr(text) if current_chat => {
                self.log(text);
                None
            }
            BridgePayload::Event(wire) if current_chat => {
                self.reduce(&event.request, wire.body);
                None
            }
            _ => None,
        }
    }

    fn reduce(&mut self, request: &RequestTag, body: EventBody) {
        match body {
            EventBody::Ack(ack)
                if request.kind == CommandKind::Overview && ack.command == "overview.rows" =>
            {
                let row = serde_json::from_value::<chat_tldr_core::OverviewPart>(ack.detail);
                if let (Ok(row), Some(report), Some(expected)) = (
                    row,
                    self.pending.overview.as_mut(),
                    self.pending.overview_expected.as_ref(),
                ) {
                    if ack.changed || ack.target.as_deref() != Some(report.chat_id.as_ref()) {
                        self.problem("总览行会话或状态不匹配");
                        return;
                    }
                    if report.counts().get(row.section()).copied().unwrap_or(0)
                        >= expected.get(row.section()).copied().unwrap_or(0)
                    {
                        self.problem("总览行数超出声明");
                        return;
                    }
                    row.append(report);
                } else {
                    self.problem("总览行出现在元数据之前或格式无效");
                }
            }
            EventBody::Ack(ack) if request.kind == CommandKind::Overview => {
                if ack.command != "overview" || ack.changed || self.pending.overview.is_some() {
                    self.problem("总览响应重复或命令不匹配");
                    return;
                }
                let expected = serde_json::from_value::<BTreeMap<String, u64>>(
                    ack.detail.get("counts").cloned().unwrap_or_default(),
                );
                match serde_json::from_value::<Overview>(
                    ack.detail.get("overview").cloned().unwrap_or_default(),
                ) {
                    Ok(report)
                        if report.version == 1
                            && request.chat.as_ref() == Some(&report.chat_id)
                            && ack.target.as_deref() == Some(report.chat_id.as_ref())
                            && report.since < report.until
                            && report.insights.len() <= VIEW_LIMIT
                            && report.topics.len() <= VIEW_LIMIT
                            && report
                                .insights
                                .iter()
                                .all(|row| row.insight.chat_id == report.chat_id)
                            && report
                                .topics
                                .iter()
                                .all(|row| row.chat_id == report.chat_id) =>
                    {
                        let empty = report.counts();
                        if let Ok(expected) = expected
                            && expected.keys().eq(empty.keys())
                            && expected.values().all(|n| *n <= VIEW_LIMIT as u64)
                            && empty.values().all(|n| *n == 0)
                        {
                            self.pending.overview_expected = Some(expected);
                            self.pending.overview = Some(report);
                        } else {
                            self.problem("总览行数声明无效");
                        }
                    }
                    _ => self.problem("总览版本、会话或数据无效"),
                }
            }
            EventBody::Ack(ack) if request.kind == CommandKind::Version => {
                if ack.command != "version" || self.pending.version.is_some() {
                    self.problem("版本握手响应重复或命令不匹配");
                    return;
                }
                match serde_json::from_value::<VersionInfo>(ack.detail) {
                    Ok(version) if compatible_schema(&version.schema_version) => {
                        self.pending.version = Some(version)
                    }
                    Ok(_) => self.problem("CLI 协议版本与 GUI 不兼容，请更新 CLI 或修改路径"),
                    Err(_) => self.problem("CLI 版本握手缺少有效的版本或 capabilities"),
                }
            }
            EventBody::Chat(chat) if request.kind == CommandKind::Chats => {
                if self.pending.chats.len() >= VIEW_LIMIT
                    || !self.pending.ids.insert(chat.chat_id.to_string())
                {
                    self.problem("会话列表重复或超过显示上限");
                } else {
                    self.pending.chats.push(chat);
                }
            }
            EventBody::Inbox(meta) if request.kind == CommandKind::Inbox => {
                if request.chat.as_ref() != Some(&meta.chat_id) || self.pending.meta.is_some() {
                    self.problem("收件箱会话不匹配或元数据重复");
                } else {
                    self.pending.meta = Some(meta);
                }
            }
            EventBody::Topic(topic) if request.kind == CommandKind::Inbox => {
                if self.pending.meta.is_none()
                    || request.chat.as_ref() != Some(&topic.chat_id)
                    || self.pending.topics.len() >= VIEW_LIMIT
                    || !self.pending.ids.insert(format!("topic:{}", topic.topic_id))
                {
                    self.problem("收件箱话题顺序、会话或标识无效");
                } else {
                    self.pending.topics.push(topic);
                }
            }
            EventBody::Insight(insight) if request.kind == CommandKind::Inbox => {
                if self.pending.meta.is_none()
                    || request.chat.as_ref() != Some(&insight.insight.chat_id)
                    || self.pending.insights.len() >= VIEW_LIMIT
                    || !self
                        .pending
                        .ids
                        .insert(format!("insight:{}", insight.insight.id))
                {
                    self.problem("收件箱结论顺序、会话或标识无效");
                } else {
                    self.pending.insights.push(insight);
                }
            }
            EventBody::Progress(progress) => self.progress = Some(progress),
            EventBody::Decision(decision) => {
                push_bounded(&mut self.decisions, decision, HISTORY_LIMIT)
            }
            EventBody::JevAnswer(answer) => {
                push_bounded(&mut self.jev_answers, answer, HISTORY_LIMIT)
            }
            EventBody::Stats(stats) => {
                if let StatsPayload::Run(run) = &stats {
                    if request
                        .chat
                        .as_ref()
                        .is_some_and(|chat| *chat != run.chat_id)
                    {
                        self.problem("统计响应来自其他会话");
                        return;
                    }
                    self.stats = Some(run.clone());
                }
                push_bounded(&mut self.history_stats, stats, HISTORY_LIMIT);
            }
            EventBody::Warning(warning) => {
                self.log(format!("{}：{}", warning.code, warning.message))
            }
            EventBody::Error(error) => {
                let message = format!("{}：{}", error.code, error.message);
                self.last_error = Some(message.clone());
                self.log(message);
                if matches!(
                    request.kind,
                    CommandKind::Version | CommandKind::Chats | CommandKind::Inbox
                ) {
                    self.problem("CLI 返回了错误，当前视图不能发布");
                }
            }
            EventBody::Unknown { event, .. } => self.log(format!("忽略未知事件：{event}")),
            // Streaming analysis is provisional; only a fresh inbox publishes cards.
            EventBody::Done(_)
            | EventBody::Topic(_)
            | EventBody::Insight(_)
            | EventBody::Ack(_)
            | EventBody::Message(_) => {}
            _ => self.problem("CLI 返回了与当前命令不匹配的数据"),
        }
    }

    fn finish(
        &mut self,
        request: RequestTag,
        mut completion: Completion,
        current_chat: bool,
    ) -> Option<FollowUp> {
        self.active = None;
        self.progress = None;
        let pending = std::mem::take(&mut self.pending);
        if let Some(problem) = pending.problem {
            completion.error.get_or_insert(problem);
        }
        if current_chat && completion.is_success() {
            match request.kind {
                CommandKind::Overview => {
                    if let Some(report) = pending.overview
                        && pending.overview_expected.as_ref() == Some(&report.counts())
                        && overview_consistent(&report)
                    {
                        self.overview = Some(OverviewView::new(report));
                    } else {
                        completion.error = Some("CLI 未返回有效分析总览".into());
                    }
                }
                CommandKind::Version => {
                    if let Some(version) = pending.version {
                        self.cli_version = Some(version.cli_version);
                        self.capabilities = Some(version.capabilities);
                    } else {
                        completion.error = Some("CLI 未返回版本握手".into());
                    }
                }
                CommandKind::Chats => {
                    self.chats = pending.chats;
                    if !self
                        .chats
                        .iter()
                        .any(|chat| self.selected_chat.as_ref() == Some(&chat.chat_id))
                    {
                        self.selected_chat = self.chats.first().map(|chat| chat.chat_id.clone());
                        self.inbox = None;
                    }
                }
                CommandKind::Inbox => {
                    if let Some(meta) = pending.meta {
                        self.inbox = Some(InboxView {
                            request_id: request.id,
                            meta,
                            topics: pending.topics,
                            insights: pending.insights,
                            fresh: true,
                            displayed: false,
                        });
                    } else {
                        completion.error = Some("CLI 未返回收件箱元数据".into());
                    }
                }
                _ => {}
            }
        }
        if !completion.is_success() {
            let message = completion.error.clone().unwrap_or_else(|| {
                match completion.done.as_ref().map(|done| done.status) {
                    Some(RunStatus::Partial) => "命令部分完成；已保存的结果将通过收件箱刷新".into(),
                    Some(RunStatus::Cancelled) => "CLI 已取消；将刷新已保存的结果".into(),
                    Some(RunStatus::Unknown) => "CLI 返回未知结束状态，不能视为成功".into(),
                    _ => format!("CLI 未成功完成（退出码 {:?}）", completion.exit_code),
                }
            });
            self.last_error = Some(message.clone());
            self.log(message);
        }
        let success = completion.is_success();
        self.last_completion = Some(completion);
        if !current_chat {
            return self.inbox_refresh();
        }
        match request.kind {
            CommandKind::Version if success && self.has_capability("chats") => {
                Some(FollowUp::Chats)
            }
            CommandKind::Chats if success => self.inbox_refresh(),
            CommandKind::Import if self.has_capability("chats") => Some(FollowUp::Chats),
            CommandKind::Analyze
            | CommandKind::Feedback
            | CommandKind::Resolve
            | CommandKind::MarkRead => {
                if self.has_capability("chats") {
                    Some(FollowUp::Chats)
                } else {
                    self.inbox_refresh()
                }
            }
            _ => None,
        }
    }

    fn inbox_refresh(&self) -> Option<FollowUp> {
        self.has_capability("inbox")
            .then(|| self.selected_chat.clone().map(FollowUp::Inbox))
            .flatten()
    }

    fn problem(&mut self, message: &str) {
        self.pending.problem.get_or_insert_with(|| message.into());
    }

    fn log(&mut self, mut message: String) {
        if let Some((index, _)) = message.char_indices().nth(2048) {
            message.truncate(index);
            message.push('…');
        }
        push_bounded(&mut self.logs, message, LOG_LIMIT);
    }

    /// Compiled, synthetic examples use the same validator and reducer as live data.
    /// The application must label this view as demo and disable business actions.
    pub fn demo() -> Self {
        let mut model = Self::default();
        model.load_demo_stream(
            RequestTag {
                id: 1,
                kind: CommandKind::Chats,
                chat: None,
            },
            include_str!("../../../fixtures/jsonl/chats.jsonl"),
        );
        let chat = ChatId("qq:group:synthetic-course".into());
        model.select_chat(chat.clone());
        model.load_demo_stream(
            RequestTag {
                id: 2,
                kind: CommandKind::Inbox,
                chat: Some(chat),
            },
            include_str!("../../../fixtures/jsonl/inbox.jsonl"),
        );
        // The shared chats fixture precedes analysis, while inbox is a later
        // snapshot. Reconcile its displayed summary only for the combined demo.
        if let Some(inbox) = &model.inbox
            && let Some(chat) = model
                .chats
                .iter_mut()
                .find(|chat| chat.chat_id == inbox.meta.chat_id)
        {
            chat.open_p0 = inbox.meta.counts.p0;
        }
        model
    }

    fn load_demo_stream(&mut self, tag: RequestTag, source: &str) {
        if let Err(error) = self.begin(tag.clone()) {
            self.last_error = Some(error);
            return;
        }
        let mut validator = EventStreamValidator::new();
        let mut completion = Completion {
            exit_code: Some(0),
            done: None,
            error: None,
            cancelled: false,
        };
        for line in source.lines() {
            let event = match serde_json::from_str::<CliEvent>(line) {
                Ok(event) => event,
                Err(error) => {
                    completion.error = Some(error.to_string());
                    break;
                }
            };
            if let Err(error) = validator.accept(&event) {
                completion.error = Some(error.to_string());
                break;
            }
            if let EventBody::Done(done) = &event.body {
                completion.done = Some(done.clone());
            }
            self.apply(BridgeEvent {
                request: tag.clone(),
                payload: BridgePayload::Event(Box::new(event)),
            });
        }
        if let Err(error) = validator.finish(0) {
            completion.error.get_or_insert_with(|| error.to_string());
        }
        self.apply(BridgeEvent {
            request: tag,
            payload: BridgePayload::Finished(completion),
        });
    }
}

fn compatible_schema(version: &str) -> bool {
    version.split_once('.').is_some_and(|(major, minor)| {
        Some(major) == chat_tldr_core::SCHEMA_VERSION.split('.').next()
            && !minor.is_empty()
            && minor.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn overview_consistent(report: &Overview) -> bool {
    let topics: HashSet<_> = report.topics.iter().map(|t| &t.topic_id).collect();
    let items: HashSet<_> = report.insights.iter().map(|i| &i.insight.id).collect();
    let topic_ok = |topic: Option<&TopicId>| topic.is_none_or(|id| topics.contains(id));
    topics.len() == report.topics.len()
        && items.len() == report.insights.len()
        && report.topics.iter().all(|t| t.chat_id == report.chat_id)
        && report
            .insights
            .iter()
            .all(|i| i.insight.chat_id == report.chat_id && topic_ok(i.insight.topic_id.as_ref()))
        && report
            .hot_topics
            .iter()
            .all(|t| topics.contains(&t.topic_id) && t.activity_score.is_finite())
        && report
            .priority_topics
            .iter()
            .chain(&report.unread_topics)
            .all(|t| {
                topic_ok(t.topic_id.as_ref()) && t.insight_ids.iter().all(|id| items.contains(id))
            })
        && report.related.iter().all(|i| items.contains(&i.insight_id))
        && report
            .deadlines
            .iter()
            .all(|i| items.contains(&i.insight_id))
        && report
            .mentions
            .iter()
            .all(|m| topic_ok(m.topic_id.as_ref()))
        && report
            .resources
            .iter()
            .all(|r| topic_ok(r.source.topic_id.as_ref()))
}

fn push_bounded<T>(items: &mut VecDeque<T>, item: T, limit: usize) {
    if items.len() == limit {
        items.pop_front();
    }
    items.push_back(item);
}

#[cfg(test)]
mod tests;
