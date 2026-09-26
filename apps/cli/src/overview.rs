use crate::{
    Failure, args::OverviewArgs, commands::parse_time, html, output::Output, paths::Paths,
};
use chat_tldr_core::*;
use chat_tldr_engine::{Config, store};
use chrono::{Duration, Utc};
use std::{collections::BTreeMap, fmt::Write as _, io::Write};

pub fn run<W: Write>(
    args: OverviewArgs,
    paths: &Paths,
    config: &Config,
    output: &mut Output<W>,
) -> Result<(), Failure> {
    let now = Utc::now().with_timezone(&config.timezone_offset()?);
    let until = parse_time(args.until.as_deref(), "--until")?.unwrap_or(now);
    let since = match parse_time(args.since.as_deref(), "--since")? {
        Some(since) => since,
        None => until
            .checked_sub_signed(Duration::hours(24))
            .ok_or_else(|| Failure::new("E_USAGE", 2, "Invalid window start"))?,
    };
    let target = args
        .html
        .as_deref()
        .map(|p| html::prepare(p, paths))
        .transpose()?;
    let mut report = store::overview(
        &paths.database,
        &ChatId(args.chat),
        &store::OverviewOptions { since, until, now },
    )?;
    if let Some(target) = target {
        html::write_page(&render(&report), target, paths)?;
    }
    let counts = report.counts();
    let rows = report.drain_parts();
    let chat = report.chat_id.to_string();
    output.emit(EventBody::Ack(AckPayload {
        command: "overview".into(),
        target: Some(report.chat_id.to_string()),
        changed: false,
        detail: serde_json::json!({"overview":report,"counts":counts}),
    }))?;
    for row in rows {
        if serde_json::to_vec(&row)
            .map_err(|_| Failure::new("E_OUTPUT_WRITE", 8, "Cannot serialize overview row"))?
            .len()
            > 900_000
        {
            return Err(Failure::new(
                "E_OUTPUT_WRITE",
                8,
                "An overview row exceeds the supported JSONL row size",
            ));
        }
        output.emit(EventBody::Ack(AckPayload {
            command: "overview.rows".into(),
            target: Some(chat.clone()),
            changed: false,
            detail: serde_json::to_value(row)
                .map_err(|_| Failure::new("E_OUTPUT_WRITE", 8, "Cannot serialize overview row"))?,
        }))?;
    }
    Ok(())
}

fn render(report: &Overview) -> String {
    let mut page = String::from(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>群聊分析总览</title><style>body{font:16px/1.65 system-ui,sans-serif;max-width:1000px;margin:32px auto;padding:0 20px;color:#20242c;background:#f6f7f9}section,article{background:white;border:1px solid #dce0e7;border-radius:12px;padding:16px;margin:14px 0}small{color:#536070}blockquote{white-space:pre-wrap;border-left:3px solid #899bb5;padding-left:12px}nav a{margin-right:16px}p{white-space:pre-wrap}</style></head><body><h1>群聊分析总览</h1>",
    );
    let escape = html::escape;
    let _ = write!(
        page,
        "<p>{}</p><p>讨论窗口：{} 至 {}（不含终点）</p><small>窗口内 {} 条消息，{} 条待分析；全群 {} 条待分析。数据覆盖 {} 至 {}。优先事项包含窗口前仍未处理的结论；查看不改变已读。</small>",
        escape(report.chat_id.as_ref()),
        escape(&report.since.to_rfc3339()),
        escape(&report.until.to_rfc3339()),
        report.window_messages,
        report.window_pending,
        report.pending_messages,
        escape(
            &report
                .data_start
                .map(|d| d.to_rfc3339())
                .unwrap_or_else(|| "无".into())
        ),
        escape(
            &report
                .data_end
                .map(|d| d.to_rfc3339())
                .unwrap_or_else(|| "无".into())
        )
    );
    page.push_str("<nav><a href=\"#hot\">热门话题</a><a href=\"#priority\">优先话题</a><a href=\"#mine\">与我有关</a><a href=\"#deadlines\">截止事项</a><a href=\"#unread\">未读回顾</a><a href=\"#resources\">资料入口</a></nav>");
    let topics: BTreeMap<_, _> = report
        .topics
        .iter()
        .map(|t| (&t.topic_id, t.title.as_str()))
        .collect();
    let title = |id: Option<&TopicId>| {
        id.and_then(|id| topics.get(id).copied())
            .unwrap_or("未归属话题")
    };
    let ids: BTreeMap<_, _> = report
        .insights
        .iter()
        .enumerate()
        .map(|(n, i)| (&i.insight.id, n))
        .collect();
    let link = |page: &mut String, id: &InsightId| {
        if let Some(index) = ids.get(id) {
            let item = &report.insights[*index].insight;
            let _ = write!(
                page,
                "<li><a href=\"#item-{index}\">{:?} · {}</a></li>",
                item.priority,
                escape(&item.title)
            );
        }
    };
    page.push_str("<section id=\"hot\"><h2>最近热门话题</h2><p>按参与广度与有效讨论活跃度排序，限制单人重复刷屏。</p>");
    for topic in &report.hot_topics {
        let _ = write!(
            page,
            "<p><b>{}</b> · {} 人 · {} 条有效讨论 / {} 条已分析消息</p>",
            escape(title(Some(&topic.topic_id))),
            topic.participants,
            topic.meaningful_messages,
            topic.message_count
        );
    }
    if report.hot_topics.is_empty() {
        page.push_str("<p>窗口内暂无已分析的有效话题讨论。</p>");
    }
    page.push_str("</section>");
    for (anchor, name, groups) in [
        ("priority", "优先话题", &report.priority_topics),
        ("unread", "未读回顾", &report.unread_topics),
    ] {
        let _ = write!(page, "<section id=\"{anchor}\"><h2>{name}</h2>");
        for group in groups {
            let _ = write!(
                page,
                "<h3>{:?} · {}</h3><p>{}</p><ul>",
                group.priority,
                escape(title(group.topic_id.as_ref())),
                escape(&group.reasons.join(" · "))
            );
            for id in &group.insight_ids {
                link(&mut page, id);
            }
            page.push_str("</ul>");
        }
        if groups.is_empty() {
            page.push_str("<p>暂无符合条件的结论。</p>");
        }
        page.push_str("</section>");
    }
    page.push_str("<section id=\"mine\"><h2>与我有关</h2><ul>");
    for row in &report.related {
        let _ = write!(page, "<li>{}</li>", escape(&row.reasons.join(" · ")));
        link(&mut page, &row.insight_id);
    }
    page.push_str("</ul><h3>窗口内提及原文</h3>");
    for row in &report.mentions {
        let _ = write!(
            page,
            "<blockquote>{}</blockquote><small>{} · {} · {}</small>",
            escape(&row.text),
            escape(&row.sender_display),
            escape(&row.reasons.join(" · ")),
            if row.analyzed {
                "已分析"
            } else {
                "尚未完成分析"
            }
        );
    }
    page.push_str("</section><section id=\"deadlines\"><h2>截止事项</h2>");
    for row in &report.deadlines {
        let label = match row.status {
            DeadlineStatus::Overdue => "已逾期",
            DeadlineStatus::Upcoming => "尚未到期",
            _ => "时间待确认",
        };
        let item = &report.insights[ids[&row.insight_id]].insight;
        let _ = write!(
            page,
            "<p>{label} · {} · 负责人 {:?}</p><ul>",
            escape(&item.deadline.as_ref().unwrap().raw),
            item.assignee
        );
        link(&mut page, &row.insight_id);
        page.push_str("</ul>");
    }
    page.push_str("</section><section id=\"resources\"><h2>资料入口</h2><p>仅提供原消息与附件元数据，未读取附件正文。</p>");
    for resource in &report.resources {
        let _ = write!(
            page,
            "<blockquote>{}</blockquote><small>{} · {}</small>",
            escape(&resource.source.text),
            escape(&resource.source.sender_display),
            escape(&resource.source.sent_at.to_rfc3339())
        );
        for attachment in &resource.attachments {
            let _ = write!(
                page,
                "<p>附件：{}</p>",
                escape(attachment.name.as_deref().unwrap_or("未命名附件"))
            );
        }
        for url in &resource.links {
            let _ = write!(
                page,
                "<p><a href=\"{}\">{}</a></p>",
                escape(url),
                escape(url)
            );
        }
    }
    page.push_str("</section><h2>结论与证据</h2>");
    for (index, row) in report.insights.iter().enumerate() {
        let _ = write!(
            page,
            "<article id=\"item-{index}\"><h3>{:?} · {}</h3><small>负责人 {:?} · {:?}</small><p>{}</p>",
            row.insight.priority,
            escape(&row.insight.title),
            row.insight.assignee,
            row.insight.verification_status,
            escape(&row.insight.summary)
        );
        for evidence in &row.evidence_view {
            let _ = write!(
                page,
                "<blockquote>{}</blockquote><small>{} · {}</small>",
                escape(&evidence.display_text),
                escape(&evidence.sender_display),
                escape(&evidence.sent_at.to_rfc3339())
            );
        }
        page.push_str("</article>");
    }
    page.push_str("</body></html>");
    page
}
