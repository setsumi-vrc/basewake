//! Command-line interface shared by `basewake.exe` (no console) and `basewakectl.exe` (console).

use std::fmt::Write as _;

use clap::{Parser, Subcommand};

use crate::engine::{self, Command, Handle, Report, Target};

#[derive(Parser, Debug)]
#[command(name = "basewake", version, about = "Lighthouse V2 base station manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Cmd>,
    /// Started by SteamVR: run in the tray and follow the SteamVR session.
    #[arg(long, hide = true)]
    pub steamvr: bool,
    /// Never forward to a running instance; run the command in this process.
    #[arg(long, hide = true)]
    pub local: bool,
}

#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    /// Turn all enabled base stations on
    On,
    /// Turn all enabled base stations off (sleep or standby, see settings)
    Off,
    /// Toggle each enabled base station
    Toggle,
    /// Show the state of every known base station
    Status,
    /// Look for base stations nearby and add new ones
    Scan,
    /// Register as a SteamVR startup app
    Install,
    /// Remove the SteamVR registration
    Uninstall,
    /// Close the running app (used when updating the exe)
    #[command(hide = true)]
    Quit,
}

impl Cmd {
    pub fn as_str(self) -> &'static str {
        match self {
            Cmd::On => "on",
            Cmd::Off => "off",
            Cmd::Toggle => "toggle",
            Cmd::Status => "status",
            Cmd::Scan => "scan",
            Cmd::Install => "install",
            Cmd::Uninstall => "uninstall",
            Cmd::Quit => "quit",
        }
    }

    pub fn parse(s: &str) -> Option<Cmd> {
        [Cmd::On, Cmd::Off, Cmd::Toggle, Cmd::Status, Cmd::Scan, Cmd::Install, Cmd::Uninstall, Cmd::Quit]
            .into_iter()
            .find(|c| c.as_str() == s)
    }

    /// Commands that use Bluetooth go to the running instance, so two processes never
    /// fight over the same base stations.
    fn uses_bluetooth(self) -> bool {
        !matches!(self, Cmd::Install | Cmd::Uninstall)
    }

    /// Commands that only make sense for a running instance.
    fn needs_instance(self) -> bool {
        matches!(self, Cmd::Quit)
    }
}

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;

/// Runs a command: through the running instance if there is one, otherwise in this process.
/// Returns (exit code, text to print).
pub fn run(cmd: Cmd, allow_forward: bool) -> (i32, String) {
    if cmd.needs_instance() {
        return crate::ipc::send(cmd.as_str()).unwrap_or((EXIT_OK, "Basewake is not running.
".into()));
    }
    if allow_forward && cmd.uses_bluetooth()
        && let Some(reply) = crate::ipc::send(cmd.as_str()) {
            return reply;
        }
    match cmd {
        Cmd::Install => crate::vr::manifest::run_cli(true),
        Cmd::Uninstall => crate::vr::manifest::run_cli(false),
        _ => match engine::start() {
            Ok(h) => execute(&h, cmd),
            Err(e) => (EXIT_FAILED, format!("error: {e:#}\n")),
        },
    }
}

/// Runs a Bluetooth command on an existing controller.
pub fn execute(handle: &Handle, cmd: Cmd) -> (i32, String) {
    let report = match cmd {
        Cmd::On => handle.run(Command::PowerAll(Target::On)),
        Cmd::Off => handle.run(Command::PowerAll(Target::Off)),
        Cmd::Toggle => handle.run(Command::PowerAll(Target::Toggle)),
        Cmd::Status => handle.run(Command::Refresh),
        Cmd::Scan => handle.run(Command::Scan),
        Cmd::Install => return crate::vr::manifest::run_cli(true),
        Cmd::Uninstall => return crate::vr::manifest::run_cli(false),
        Cmd::Quit => return (EXIT_OK, String::new()),
    };
    let code = if report.success() { EXIT_OK } else { EXIT_FAILED };
    (code, format_report(cmd, &report, handle))
}

pub fn format_report(cmd: Cmd, report: &Report, handle: &Handle) -> String {
    let mut out = String::new();
    if let Some(e) = &report.error {
        let _ = writeln!(out, "error: {e}");
    }
    if cmd == Cmd::Scan {
        if report.discovered.is_empty() {
            out.push_str("No base stations found.\n");
        }
        for d in &report.discovered {
            let tag = if report.new_stations.contains(&d.mac) { "  (new)" } else { "" };
            let _ = writeln!(out, "{}  {:<14} {:>4} dBm{tag}", d.mac, d.name, d.rssi);
        }
        return out;
    }
    let snap = handle.snapshot();
    for s in &snap.stations {
        let state = match report.failed.iter().find(|(m, _)| *m == s.mac) {
            Some((_, e)) => format!("FAILED: {e}"),
            None if report.ok.iter().any(|(m, _)| *m == s.mac) => s.state.label(),
            None => format!("{} (not touched)", s.state.label()),
        };
        let enabled = if s.enabled { "" } else { "  [disabled]" };
        let _ = writeln!(out, "{}  {:<14} {state}{enabled}", s.mac, s.name);
    }
    for m in &report.new_stations {
        let _ = writeln!(out, "New base station added: {m}");
    }
    out
}
