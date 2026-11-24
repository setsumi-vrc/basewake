//! Logging to `%APPDATA%\Basewake\logs\basewake.log.<date>` (and stderr for the CLI).

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::{EnvFilter, fmt};

use super::config::Config;

/// Keep the returned guard alive for the whole run, or buffered log lines are lost on exit.
pub fn init(to_stderr: bool) -> Option<WorkerGuard> {
    let dir = Config::dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("basewake")
        .filename_suffix("log")
        .max_log_files(14)
        .build(&dir)
        .ok();
    let filter = EnvFilter::try_from_env("BASEWAKE_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    match appender {
        Some(appender) => {
            let (file, guard) = tracing_appender::non_blocking(appender);
            let builder = fmt().with_env_filter(filter).with_ansi(false).with_target(false);
            if to_stderr {
                let _ = builder.with_writer(file.and(std::io::stderr.with_max_level(tracing::Level::WARN))).try_init();
            } else {
                let _ = builder.with_writer(file).try_init();
            }
            Some(guard)
        }
        None => {
            let _ = fmt().with_env_filter(filter).with_target(false).with_writer(std::io::stderr).try_init();
            None
        }
    }
}
