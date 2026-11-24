//! `basewake.exe`: tray app, SteamVR companion and silent command runner (no console window).
#![windows_subsystem = "windows"]

use clap::Parser;
use basewake::app::{self, StartMode};
use basewake::cli::{self, Cli};

fn main() {
    let cli = Cli::parse();
    let _log = basewake::engine::log::init(false);
    let code = match cli.command {
        // No console here: the exit code is the result (output is in the log).
        Some(cmd) => {
            let (code, text) = cli::run(cmd, !cli.local);
            tracing::info!("{} -> exit {code}\n{text}", cmd.as_str());
            code
        }
        None => app::run(if cli.steamvr { StartMode::SteamVr } else { StartMode::Normal }),
    };
    std::process::exit(code);
}
