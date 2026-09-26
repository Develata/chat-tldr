//! Bounded background transport for one CLI child owned by this GUI.
use std::{
    ffi::OsString,
    io::{self, BufRead, BufReader, Read},
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

use chat_tldr_core::{ChatId, CliEvent, DonePayload, EventBody, EventStreamValidator, RunStatus};

const CHANNEL_CAPACITY: usize = 64;
const MAX_STDOUT_LINE: usize = 1024 * 1024;
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
    Analyze,
    Import,
    Feedback,
    Resolve,
    MarkRead,
    Stats,
    Decisions,
    JevLog,
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
        }
    }

    pub fn is_busy(&self) -> bool {
        self.active.is_some()
    }

    pub fn start(&mut self, request: Request) -> Result<(), String> {
        if self.is_busy() {
            return Err("已有 CLI 命令正在运行".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let settings = self.settings.clone();
        let sender = self.sender.clone();
        let repaint = self.repaint.clone();
        let cancellation = cancel.clone();
        let tag = request.tag.clone();
        thread::Builder::new()
            .name("chat-tldr-command".into())
            .spawn(move || {
                let completion = run_child(settings, &request, &sender, &repaint, &cancellation);
                send(
                    &sender,
                    &repaint,
                    &request.tag,
                    BridgePayload::Finished(completion),
                );
            })
            .map_err(|error| format!("无法启动后台线程：{error}"))?;
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
) -> Completion {
    let mut completion = Completion {
        exit_code: None,
        done: None,
        error: None,
        cancelled: false,
    };
    let mut child = match command(&settings, request).spawn() {
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
    let mut stdout_thread = Some(stdout_thread);
    let mut stream = None;
    loop {
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
    let stream = stream.unwrap_or_else(|| {
        stdout_thread
            .unwrap()
            .join()
            .unwrap_or_else(|_| Stream::failed("stdout 读取线程异常".into()))
    });
    if stderr_thread.join().is_err() {
        completion
            .error
            .get_or_insert_with(|| "stderr 读取线程异常".into());
    }
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
    mut reader: impl Read,
    sender: &SyncSender<BridgeEvent>,
    repaint: &Repaint,
    tag: &RequestTag,
) {
    let mut buffer = [0; 2048];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                let event = BridgeEvent {
                    request: tag.clone(),
                    payload: BridgePayload::Stderr(
                        String::from_utf8_lossy(&buffer[..size]).into_owned(),
                    ),
                };
                match sender.try_send(event) {
                    Ok(()) => repaint(),
                    Err(mpsc::TrySendError::Full(_)) => {} // Drain noisy stderr without delaying protocol events.
                    Err(mpsc::TrySendError::Disconnected(_)) => break,
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests;
