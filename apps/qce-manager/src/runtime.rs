use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use crate::error::{Failure, Result};

#[derive(Clone, Copy)]
pub struct Budget<'a> {
    cancelled: &'a AtomicBool,
    deadline: Option<Instant>,
    timeout_code: &'static str,
}

impl<'a> Budget<'a> {
    pub fn new(cancelled: &'a AtomicBool) -> Self {
        Self {
            cancelled,
            deadline: None,
            timeout_code: "E_QCE_TIMEOUT",
        }
    }

    pub fn limited(self, duration: Duration, code: &'static str) -> Self {
        let deadline = Instant::now() + duration;
        Self {
            deadline: Some(self.deadline.map_or(deadline, |old| old.min(deadline))),
            timeout_code: code,
            ..self
        }
    }

    pub fn check(self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(Failure::new(
                "E_CANCELLED",
                130,
                "操作已取消；上游任务可能仍在执行",
            ));
        }
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(Failure::new(
                self.timeout_code,
                5,
                "等待超时；未产生成功路径",
            ));
        }
        Ok(())
    }

    pub fn timeout(self, maximum: Duration) -> Result<Duration> {
        self.check()?;
        Ok(self
            .deadline
            .map_or(maximum, |deadline| {
                maximum.min(deadline.saturating_duration_since(Instant::now()))
            })
            .max(Duration::from_millis(1)))
    }

    pub fn sleep(self, duration: Duration) -> Result<()> {
        let until = Instant::now() + duration;
        while Instant::now() < until {
            self.check()?;
            std::thread::sleep(
                until
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(40)),
            );
        }
        self.check()
    }
}
