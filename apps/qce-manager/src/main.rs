mod args;
mod commands;
mod credentials;
mod docker;
mod error;
mod export;
mod files;
mod http;
mod login;
mod output;
mod runtime;
mod validation;

#[cfg(test)]
mod test_support;

use clap::Parser;
use error::Failure;
use std::{
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn main() -> ExitCode {
    let mut output = output::Output::new(std::io::stdout().lock());
    let cli = match args::Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            let _ = error.print();
            return ExitCode::SUCCESS;
        }
        Err(_) => {
            return finish(
                &mut output,
                Err(Failure::usage("参数无效；请使用 --help 查看用法")),
            );
        }
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&cancelled);
    if ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed)).is_err() {
        return finish(
            &mut output,
            Err(Failure::new("E_INTERNAL", 1, "无法安装取消处理器")),
        );
    }
    let result = commands::run(
        &cli,
        &docker::SystemDocker,
        &mut login::Terminal,
        &mut output,
        runtime::Budget::new(&cancelled),
    );
    finish(&mut output, result)
}

fn finish<W: std::io::Write>(
    output: &mut output::Output<W>,
    result: error::Result<()>,
) -> ExitCode {
    let code = match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{}: {}", error.code, error.message);
            if output.error(&error).is_err() {
                return ExitCode::from(8);
            }
            error.exit
        }
    };
    if output.done(code).is_err() {
        ExitCode::from(8)
    } else {
        ExitCode::from(code)
    }
}
