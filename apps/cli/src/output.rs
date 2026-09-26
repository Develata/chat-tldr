use std::io::{self, Write};
use std::time::Instant;

use chat_tldr_core::{CliEvent, DonePayload, EventBody, RunId, RunStatus};

pub struct Output<W: Write> {
    writer: W,
    run_id: RunId,
    seq: u64,
    started: Instant,
}

impl<W: Write> Output<W> {
    pub fn new(writer: W) -> Self {
        let now = chrono::Utc::now();
        let suffix = (now.timestamp_subsec_nanos() ^ std::process::id()) & 0xffff;
        Self {
            writer,
            run_id: RunId(format!("r_{}_{suffix:04x}", now.format("%Y%m%dT%H%M%S"))),
            seq: 0,
            started: Instant::now(),
        }
    }

    pub fn emit(&mut self, body: EventBody) -> io::Result<()> {
        let event = CliEvent::new(self.run_id.clone(), self.seq, body);
        serde_json::to_writer(&mut self.writer, &event)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        self.seq += 1;
        Ok(())
    }

    pub fn run_id(&self) -> RunId {
        self.run_id.clone()
    }

    pub fn done(&mut self, exit_code: u8) -> io::Result<()> {
        self.emit(EventBody::Done(DonePayload {
            status: if exit_code == 0 {
                RunStatus::Complete
            } else {
                RunStatus::Failed
            },
            exit_code: i32::from(exit_code),
            finish_reason: None,
            elapsed_ms: self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        }))
    }
}
