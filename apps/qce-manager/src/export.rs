use chrono::{DateTime, Utc};
use reqwest::Method;
use ring::digest::{Context, SHA256};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    time::Duration,
};

use crate::{
    args::ExportArgs,
    credentials::Credentials,
    error::{Failure, Result},
    files::Files,
    http::Http,
    output::Output,
    runtime::Budget,
    validation,
};

pub fn run<W: Write>(
    args: &ExportArgs,
    credentials: &Credentials<'_>,
    http: &Http,
    files: &Files,
    output: &mut Output<W>,
    budget: Budget<'_>,
) -> Result<()> {
    let since = parse_time(&args.since)?;
    let until = parse_time(&args.until)?;
    if since > until
        || args.peer.trim().is_empty()
        || args.peer.len() > 256
        || args.peer.chars().any(char::is_control)
    {
        return Err(Failure::usage("peer 必须非空且有效，since 不得晚于 until"));
    }
    let budget = budget.limited(Duration::from_secs(args.max_wait_secs), "E_QCE_TIMEOUT");
    let token = credentials.qce(budget)?;
    // Preflight local storage before creating an upstream task (never retry creation).
    let mut job = files.start()?;
    let request = json!({
        "peer":{"chatType":args.chat_type.number(),"peerUid":args.peer,"guildId":""},
        "format":"JSON", "filter":{"startTime":since.timestamp_millis(),"endTime":until.timestamp_millis()},
        "options":{"batchSize":200,"includeResourceLinks":false,"includeSystemMessages":true,
            "filterPureImageMessages":false,"prettyFormat":true,"exportAsZip":false,
            "embedAvatarsAsBase64":false,"debugExport":false,"useNameInFileName":false,
            "skipDownloadResourceTypes":["image","video","audio","file"]}
    });
    let created = http.json(
        Method::POST,
        "/api/messages/export",
        Some(&token),
        Some(&request),
        budget,
    )?;
    let task_id = created
        .get("taskId")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 256)
        .ok_or_else(Failure::protocol)?;
    let mut task_url = http
        .base
        .join("/api/tasks/")
        .map_err(|_| Failure::protocol())?;
    task_url
        .path_segments_mut()
        .map_err(|_| Failure::protocol())?
        .pop_if_empty()
        .push(task_id);
    output.progress("waiting_export", "等待 QCE 完成导出")?;
    let download = loop {
        let task = http.json(Method::GET, task_url.as_str(), Some(&token), None, budget)?;
        match task.get("status").and_then(Value::as_str) {
            Some("completed") => {
                if task.get("progress").and_then(Value::as_f64) != Some(100.0) {
                    return Err(Failure::protocol());
                }
                break task
                    .get("downloadUrl")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(Failure::protocol)?
                    .to_owned();
            }
            Some("failed" | "cancelled") => {
                return Err(Failure::new(
                    "E_QCE_TASK_FAILED",
                    5,
                    "上游导出任务失败或被取消；未产生成功路径",
                ));
            }
            Some("queued" | "pending" | "running" | "processing") => {
                budget.sleep(Duration::from_secs(args.poll_secs))?
            }
            _ => return Err(Failure::protocol()),
        }
    };
    let url = http.base.join(&download).map_err(|_| Failure::protocol())?;
    let mut response = http.request(Method::GET, url, Some(&token), None, budget)?;
    let expected = response.content_length();
    let path = job.path.join("messages.json");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| Failure::io())?;
    let mut hash = Context::new(&SHA256);
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        budget.check()?;
        let read = response.read(&mut buffer);
        budget.check()?;
        let count = read
            .map_err(|_| Failure::new("E_QCE_DOWNLOAD", 5, "下载中断或长度不符；未产生成功路径"))?;
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count])
            .map_err(|_| Failure::io())?;
        hash.update(&buffer[..count]);
        size += count as u64;
    }
    if expected.is_some_and(|expected| expected != size) {
        return Err(Failure::new("E_QCE_DOWNLOAD", 5, "下载长度不符"));
    }
    file.sync_all().map_err(|_| Failure::io())?;
    drop(file);
    let message_count = validation::count_messages(&path)?;
    let sha256 = hex(hash.finish().as_ref());
    budget.check()?;
    let manifest = json!({"schema_version":1,"export_id":job.id,"created_at":Utc::now().to_rfc3339(),
        "tool_version":env!("CARGO_PKG_VERSION"),"qce_base_url":http.base.as_str(),
        "chat_type":args.chat_type.name(),"peer_uid":args.peer,"since":since.to_rfc3339(),"until":until.to_rfc3339(),
        "task_id":task_id,"message_count":message_count,"file_size":size,"sha256":sha256});
    let mut file = File::create(job.path.join("manifest.json")).map_err(|_| Failure::io())?;
    serde_json::to_writer_pretty(&mut file, &manifest).map_err(|_| Failure::io())?;
    file.write_all(b"\n")
        .and_then(|_| file.sync_all())
        .map_err(|_| Failure::io())?;
    drop(file);
    budget.check()?;
    let published = job.publish()?;
    // If stdout breaks after this commit, retain the completed export for recovery.
    output.ack("export", true, json!({"path":published.join("messages.json"),"manifest":published.join("manifest.json"),"sha256":sha256,"message_count":message_count}))
}

fn parse_time(raw: &str) -> Result<DateTime<Utc>> {
    let value = DateTime::parse_from_rfc3339(raw)
        .map_err(|_| Failure::usage("时间必须为带时区的 RFC3339"))?;
    if value.timestamp_subsec_nanos() % 1_000_000 != 0 {
        return Err(Failure::usage("时间精度最多到毫秒，避免闭区间边界被截断"));
    }
    Ok(value.with_timezone(&Utc))
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(result, "{byte:02x}");
    }
    result
}

#[cfg(test)]
mod tests;
