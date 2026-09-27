//! Bounded background transport for one CLI child owned by this GUI.
use std::{
    ffi::OsString,
    io::{self, BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::Duration,
};

use chat_tldr_core::settings::SecretString;
use chat_tldr_core::{ChatId, CliEvent, DonePayload, EventBody, EventStreamValidator, RunStatus};

const CHANNEL_CAPACITY: usize = 64;
const MAX_STDOUT_LINE: usize = 1024 * 1024;
const MAX_STDERR_LINE_CHARS: usize = 2048;
// A Unicode scalar value occupies at most four UTF-8 bytes. Keeping only this
// prefix lets us retain MAX_STDERR_LINE_CHARS complete characters while a
// noisy, unterminated stderr line is drained without unbounded allocation.
const MAX_STDERR_LINE_BYTES: usize = MAX_STDERR_LINE_CHARS * 4;
type Repaint = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug)]
pub struct CliSettings {
    pub executable: PathBuf,
    pub data_dir: Option<PathBuf>,
    pub config: Option<PathBuf>,
}

pub fn default_cli_path() -> io::Result<PathBuf> {
    let executable = std::env::current_exe()?;
    let filename = if cfg!(windows) {
        "chat-tldr.exe"
    } else {
        "chat-tldr"
    };
    Ok(executable.with_file_name(filename))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    Version,
    Chats,
    Inbox,
    Overview,
    Analyze,
    Import,
    Feedback,
    Resolve,
    MarkRead,
    Stats,
    Decisions,
    JevLog,
    ConfigShow,
    ConfigSet,
    QceVersion,
    QceStatus,
    QceLogin,
    QceChats,
    QceExport,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestTag {
    pub id: u64,
    pub kind: CommandKind,
    pub chat: Option<ChatId>,
}

#[derive(Clone, Debug)]
pub struct Request {
    pub tag: RequestTag,
    /// Separate argv values, never a shell command string.
    pub args: Vec<OsString>,
}

#[derive(Clone, Debug)]
pub struct Completion {
    pub exit_code: Option<i32>,
    pub done: Option<DonePayload>,
    pub error: Option<String>,
    /// The GUI requested termination; this never fabricates CLI exit code 130.
    pub cancelled: bool,
}

impl Completion {
    pub fn is_success(&self) -> bool {
        !self.cancelled
            && self.error.is_none()
            && self.exit_code == Some(0)
            && self
                .done
                .as_ref()
                .is_some_and(|done| done.status == RunStatus::Complete && done.exit_code == 0)
    }
}

#[derive(Clone, Debug)]
pub enum BridgePayload {
    Event(Box<CliEvent>),
    Stderr(String),
    Finished(Completion),
}

#[derive(Clone, Debug)]
pub struct BridgeEvent {
    pub request: RequestTag,
    pub payload: BridgePayload,
}

pub struct Bridge {
    settings: CliSettings,
    sender: SyncSender<BridgeEvent>,
    receiver: Receiver<BridgeEvent>,
    repaint: Repaint,
    active: Option<(RequestTag, Arc<AtomicBool>)>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Bridge {
    pub fn new(settings: CliSettings, repaint: impl Fn() + Send + Sync + 'static) -> Self {
        let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);
        Self {
            settings,
            sender,
            receiver,
            repaint: Arc::new(repaint),
            active: None,
            worker: None,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.active.is_some()
    }

    pub fn start(&mut self, request: Request) -> Result<(), String> {
        self.start_with_input(request, None)
    }

    pub fn start_with_input(
        &mut self,
        request: Request,
        input: Option<SecretString>,
    ) -> Result<(), String> {
        if self.is_busy() {
            return Err("已有 CLI 命令正在运行".into());
        }
        if input
            .as_ref()
            .is_some_and(|input| input.expose().len() > 64 * 1024)
        {
            return Err("配置输入超过 64 KiB".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let settings = self.settings.clone();
        let sender = self.sender.clone();
        let repaint = self.repaint.clone();
        let cancellation = cancel.clone();
        let tag = request.tag.clone();
        // A prior Finished event means that the worker has no more child I/O.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let worker = thread::Builder::new()
            .name("chat-tldr-command".into())
            .spawn(move || {
                let completion =
                    run_child(settings, &request, &sender, &repaint, &cancellation, input);
                send(
                    &sender,
                    &repaint,
                    &request.tag,
                    BridgePayload::Finished(completion),
                );
            })
            .map_err(|error| format!("无法启动后台线程：{error}"))?;
        self.worker = Some(worker);
        self.active = Some((tag, cancel));
        Ok(())
    }

    pub fn try_recv(&mut self) -> Option<BridgeEvent> {
        let event = self.receiver.try_recv().ok()?;
        if matches!(event.payload, BridgePayload::Finished(_))
            && self
                .active
                .as_ref()
                .is_some_and(|(tag, _)| *tag == event.request)
        {
            self.active = None;
        }
        Some(event)
    }

    pub fn cancel(&self) -> bool {
        if let Some((_, cancel)) = &self.active {
            cancel.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.cancel();
        // Disconnect before joining: either reader may be waiting on a full
        // bounded queue. The worker owns and reaps the CLI before GUI exit.
        let (_, disconnected) = mpsc::sync_channel(0);
        drop(std::mem::replace(&mut self.receiver, disconnected));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn send(
    sender: &SyncSender<BridgeEvent>,
    repaint: &Repaint,
    tag: &RequestTag,
    payload: BridgePayload,
) -> bool {
    if sender
        .send(BridgeEvent {
            request: tag.clone(),
            payload,
        })
        .is_err()
    {
        return false;
    }
    repaint();
    true
}

fn command(settings: &CliSettings, request: &Request) -> Command {
    let mut command = Command::new(&settings.executable);
    if let Some(path) = &settings.data_dir {
        command.arg("--data-dir").arg(path);
    }
    if let Some(path) = &settings.config {
        command.arg("--config").arg(path);
    }
    command
        .args(&request.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

fn run_child(
    settings: CliSettings,
    request: &Request,
    sender: &SyncSender<BridgeEvent>,
    repaint: &Repaint,
    cancel: &AtomicBool,
    input: Option<SecretString>,
) -> Completion {
    let mut completion = Completion {
        exit_code: None,
        done: None,
        error: None,
        cancelled: false,
    };
    let mut cmd = command(&settings, request);
    if input.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            completion.error = Some(format!("无法启动 CLI：{error}"));
            return completion;
        }
    };
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_sender = sender.clone();
    let stdout_repaint = repaint.clone();
    let stdout_tag = request.tag.clone();
    let stdout_thread =
        thread::spawn(move || read_stdout(stdout, &stdout_sender, &stdout_repaint, &stdout_tag));
    let stderr_sender = sender.clone();
    let stderr_repaint = repaint.clone();
    let stderr_tag = request.tag.clone();
    let stderr_thread =
        thread::spawn(move || read_stderr(stderr, &stderr_sender, &stderr_repaint, &stderr_tag));
    // A separate writer keeps cancellation responsive even if a bad CLI never
    // reads its pipe. The secret never enters argv, request Debug, or preferences.
    let mut input_thread = input.map(|input| {
        let mut stdin = child.stdin.take().expect("piped config input");
        thread::spawn(move || stdin.write_all(input.expose().as_bytes()))
    });
    let mut stdout_thread = Some(stdout_thread);
    let mut stream = None;
    loop {
        if input_thread
            .as_ref()
            .is_some_and(thread::JoinHandle::is_finished)
            && !matches!(input_thread.take().unwrap().join(), Ok(Ok(())))
        {
            completion.error = Some("无法向 CLI 传送配置，密钥未回显".into());
            let _ = child.kill();
        }
        if stdout_thread
            .as_ref()
            .is_some_and(thread::JoinHandle::is_finished)
        {
            stream = Some(
                stdout_thread
                    .take()
                    .unwrap()
                    .join()
                    .unwrap_or_else(|_| Stream::failed("stdout 读取线程异常".into())),
            );
            if stream.as_ref().is_some_and(|result| result.error.is_some()) {
                let _ = child.kill();
            }
        }
        if cancel.load(Ordering::Relaxed) && !completion.cancelled {
            completion.cancelled = true;
            if let Err(error) = child.kill() {
                completion.error = Some(format!("终止 CLI 失败：{error}"));
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                completion.exit_code = status.code();
                break;
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                completion.error = Some(format!("读取 CLI 退出状态失败：{error}"));
                let _ = child.kill();
                completion.exit_code = child.wait().ok().and_then(|status| status.code());
                break;
            }
        }
    }
    // The direct child has been reaped. A user-supplied wrapper may leave
    // descendants holding inherited pipes, so cancellation must not require EOF.
    // Detached readers retain their original request tag; later events cannot
    // publish into another request. Normal completion still drains both pipes.
    let stream = stream.unwrap_or_else(|| {
        match join_reader_unless_cancelled(stdout_thread.unwrap(), cancel) {
            Some(Ok(stream)) => stream,
            Some(Err(_)) => Stream::failed("stdout 读取线程异常".into()),
            None => Stream::default(),
        }
    });
    if let Some(writer) = input_thread
        && !matches!(
            join_reader_unless_cancelled(writer, cancel),
            Some(Ok(Ok(())))
        )
        && !cancel.load(Ordering::Relaxed)
    {
        completion
            .error
            .get_or_insert_with(|| "配置传送未完成".into());
    }
    if let Some(Err(_)) = join_reader_unless_cancelled(stderr_thread, cancel) {
        completion
            .error
            .get_or_insert_with(|| "stderr 读取线程异常".into());
    }
    completion.cancelled |= cancel.load(Ordering::Relaxed);
    completion.done = stream.done;
    if let Some(error) = stream.error {
        completion.error.get_or_insert(error);
    }
    if completion.cancelled {
        completion
            .error
            .get_or_insert_with(|| "已终止本 GUI 启动的 CLI；将刷新可能已保存的结果".into());
    } else if let Some(code) = completion.exit_code {
        if let Err(error) = stream.validator.finish(code) {
            completion
                .error
                .get_or_insert_with(|| format!("CLI 协议错误：{error}"));
        }
    } else {
        completion
            .error
            .get_or_insert_with(|| "CLI 异常终止，未返回退出码".into());
    }
    completion
}

/// Called only after reaping the CLI. Dropping an unfinished JoinHandle detaches
/// the pipe reader, without killing or assuming ownership of descendant processes.
fn join_reader_unless_cancelled<T>(
    reader: thread::JoinHandle<T>,
    cancel: &AtomicBool,
) -> Option<thread::Result<T>> {
    loop {
        if reader.is_finished() {
            return Some(reader.join());
        }
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[derive(Default)]
struct Stream {
    validator: EventStreamValidator,
    done: Option<DonePayload>,
    error: Option<String>,
}
impl Stream {
    fn failed(error: String) -> Self {
        Self {
            error: Some(error),
            ..Self::default()
        }
    }
}

fn read_stdout(
    reader: impl Read,
    sender: &SyncSender<BridgeEvent>,
    repaint: &Repaint,
    tag: &RequestTag,
) -> Stream {
    let mut reader = BufReader::new(reader);
    let mut result = Stream::default();
    let mut line_number = 0_u64;
    loop {
        let line = match limited_line(&mut reader) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                result.error = Some(format!("读取 CLI stdout 失败：{error}"));
                break;
            }
        };
        line_number += 1;
        let event: CliEvent = match serde_json::from_slice(&line) {
            Ok(event) => event,
            Err(error) => {
                result.error = Some(format!("CLI JSONL 无效（第 {line_number} 行）：{error}"));
                break;
            }
        };
        if let Err(error) = result.validator.accept(&event) {
            result.error = Some(format!("CLI 协议错误：{error}"));
            break;
        }
        if let EventBody::Done(done) = &event.body {
            result.done = Some(done.clone());
        }
        if !send(sender, repaint, tag, BridgePayload::Event(Box::new(event))) {
            break;
        }
    }
    if result.error.is_none() && result.done.is_none() {
        result.error = Some("CLI 协议错误：stream ended without done".into());
    }
    result
}

fn limited_line(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        let end = chunk
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1);
        let take = end.unwrap_or(chunk.len());
        if line.len() + take > MAX_STDOUT_LINE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "单行 JSONL 超出 1 MiB 上限",
            ));
        }
        line.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if end.is_some() {
            return Ok(Some(line));
        }
    }
}

fn read_stderr(
    reader: impl Read,
    sender: &SyncSender<BridgeEvent>,
    repaint: &Repaint,
    tag: &RequestTag,
) {
    let mut reader = BufReader::new(reader);
    loop {
        match limited_stderr_line(&mut reader) {
            Ok(Some(line)) => {
                let event = BridgeEvent {
                    request: tag.clone(),
                    payload: BridgePayload::Stderr(line),
                };
                match sender.try_send(event) {
                    Ok(()) => repaint(),
                    Err(mpsc::TrySendError::Full(_)) => {} // Drain noisy stderr without delaying protocol events.
                    Err(mpsc::TrySendError::Disconnected(_)) => break,
                }
            }
            Ok(None) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

fn limited_stderr_line(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut bytes = Vec::with_capacity(MAX_STDERR_LINE_BYTES);
    let mut saw_input = false;
    let mut truncated = false;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            if !saw_input {
                return Ok(None);
            }
            break;
        }
        saw_input = true;
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let content_end = newline.unwrap_or(chunk.len());
        let copy_len = content_end.min(MAX_STDERR_LINE_BYTES.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&chunk[..copy_len]);
        truncated |= copy_len < content_end;
        let consume_len = newline.map_or(chunk.len(), |index| index + 1);
        reader.consume(consume_len);
        if newline.is_some() {
            break;
        }
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    let mut line = String::from_utf8_lossy(&bytes).into_owned();
    if let Some((index, _)) = line.char_indices().nth(MAX_STDERR_LINE_CHARS) {
        line.truncate(index);
        truncated = true;
    }
    if truncated {
        line.push('…');
    }
    Ok(Some(line))
}

#[cfg(test)]
mod tests;
