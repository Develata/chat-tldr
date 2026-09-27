use std::{
    io::Read,
    process::{Command, Stdio},
    time::Duration,
};

use crate::{
    error::{Failure, Result},
    runtime::Budget,
};

pub const CONFIG_LIMIT: u64 = 64 * 1024;

pub trait Docker {
    fn run(&self, arguments: &[&str], budget: Budget<'_>) -> Result<Vec<u8>>;
}

pub struct SystemDocker;

impl Docker for SystemDocker {
    fn run(&self, arguments: &[&str], budget: Budget<'_>) -> Result<Vec<u8>> {
        let failed = || Failure::new("E_QCE_DOCKER", 4, "Docker 只读命令失败；未回显子进程输出");
        budget.check()?;
        let mut command = Command::new("docker");
        command
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().map_err(|_| failed())?;
        let stdout = child.stdout.take().ok_or_else(failed)?;
        let reader = std::thread::spawn(move || {
            let mut data = Vec::new();
            stdout
                .take(CONFIG_LIMIT + 1)
                .read_to_end(&mut data)
                .map(|_| data)
        });
        let status = loop {
            if let Err(error) = budget.check() {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(error);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err(failed());
                }
            }
        };
        let bytes = reader.join().map_err(|_| failed())?.map_err(|_| failed())?;
        if !status.success() || bytes.len() as u64 > CONFIG_LIMIT {
            return Err(failed());
        }
        Ok(bytes)
    }
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.starts_with('-')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err(Failure::usage("容器名只接受字母、数字、下划线、点和连字符"));
    }
    Ok(())
}
