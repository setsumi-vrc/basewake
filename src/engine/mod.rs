//! Everything that is not UI: Bluetooth, config, logging and the controller.

pub mod ble;
pub mod config;
pub mod controller;
pub mod log;
pub mod station;

pub use config::{Config, OffMode};
pub use controller::{Command, Handle, Report, Snapshot, StationStatus, Target};
pub use station::{Mac, PowerState};

use std::sync::Arc;

/// Loads the user's config and starts the controller on the real Bluetooth radio.
pub fn start() -> anyhow::Result<Handle> {
    let config = Config::load()?;
    Ok(controller::spawn(Arc::new(ble::WinRadio::new()), config, Some(Config::path())))
}
