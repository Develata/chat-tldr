use crate::error::{Failure, Result};
use chat_tldr_core::{
    AckPayload, CliEvent, DonePayload, ErrorPayload, EventBody, ProgressPayload, RunId, RunStatus,
};
use serde_json::Value;
use std::{io::Write, time::Instant};

pub struct Output<W> {
    writer: W,
    run: RunId,
    seq: u64,
    start: Instant,
}

impl<W: Write> Output<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            run: RunId(format!("r_{}", uuid::Uuid::new_v4().simple())),
            seq: 0,
            start: Instant::now(),
        }
    }

    pub fn emit(&mut self, body: EventBody) -> Result<()> {
        serde_json::to_writer(
            &mut self.writer,
            &CliEvent::new(self.run.clone(), self.seq, body),
        )
        .map_err(|_| Failure::io())?;
        self.writer
            .write_all(b"\n")
            .and_then(|_| self.writer.flush())
            .map_err(|_| Failure::io())?;
        self.seq += 1;
        Ok(())
    }

    pub fn ack(&mut self, command: &str, changed: bool, detail: Value) -> Result<()> {
        self.emit(EventBody::Ack(AckPayload {
            command: command.into(),
            target: None,
            changed,
            detail,
        }))
    }

    pub fn progress(&mut self, stage: &str, message: &str) -> Result<()> {
        self.emit(EventBody::Progress(ProgressPayload {
            stage: stage.into(),
            current: 0,
            total: None,
            message: message.into(),
        }))
    }

    pub fn error(&mut self, error: &Failure) -> Result<()> {
        self.emit(EventBody::Error(ErrorPayload {
            stage: "qce_manager".into(),
            code: error.code.into(),
            retryable: false,
            message: error.message.clone(),
            topic_id: None,
        }))
    }

    pub fn done(&mut self, exit: u8) -> Result<()> {
        self.emit(EventBody::Done(DonePayload {
            status: match exit {
                0 => RunStatus::Complete,
                130 => RunStatus::Cancelled,
                _ => RunStatus::Failed,
            },
            exit_code: i32::from(exit),
            finish_reason: None,
            elapsed_ms: self.start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        }))
    }
}
