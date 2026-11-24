//! Registering Basewake as a SteamVR startup ("auto launch") overlay app.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};

use super::openvr::{self, OpenVr};
use crate::app::AppCtl;

pub const APP_KEY: &str = "local.basewake";
/// Registration left by versions released as "Lighthouse Manager".
const OLD_APP_KEY: &str = "local.lighthouse-manager";

pub fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Into::into)).unwrap_or_else(|| ".".into())
}

pub fn manifest_path() -> PathBuf {
    exe_dir().join("basewake.vrmanifest")
}

fn write_manifest() -> Result<PathBuf> {
    let path = manifest_path();
    let json = serde_json::json!({
        "source": "builtin",
        "applications": [{
            "app_key": APP_KEY,
            "launch_type": "binary",
            "binary_path_windows": "basewake.exe",
            "binary_path_linux": "basewake",
            "arguments": "--steamvr",
            "is_dashboard_overlay": true,
            "strings": { "en_us": {
                "name": "Basewake",
                "description": "Turns Lighthouse V2 base stations on with SteamVR and off when it closes"
            }}
        }]
    });
    std::fs::write(&path, serde_json::to_string_pretty(&json)?)
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}

/// Uses the running SteamVR session if there is one, otherwise a short "utility" connection.
fn with_openvr<T>(f: impl FnOnce(&OpenVr) -> Result<T>) -> Result<T> {
    let vr: Arc<Mutex<OpenVr>> = match super::session::active() {
        Some(v) => v,
        None => OpenVr::init(openvr::APP_UTILITY).context("cannot reach SteamVR")?,
    };
    let guard = vr.lock().unwrap();
    f(&guard)
}

pub fn register() -> Result<()> {
    let path = write_manifest()?;
    let p = path.to_string_lossy().to_string();
    with_openvr(|vr| {
        remove_old_registration(vr);
        vr.add_manifest(&p)?;
        vr.set_auto_launch(APP_KEY, true)?;
        Ok(())
    })?;
    tracing::info!("registered SteamVR startup app ({p})");
    Ok(())
}

/// Drops the startup entry of the old "Lighthouse Manager" install, so SteamVR doesn't start both.
fn remove_old_registration(vr: &OpenVr) {
    if !vr.is_installed(OLD_APP_KEY) {
        return;
    }
    let _ = vr.set_auto_launch(OLD_APP_KEY, false);
    if let Some(local) = directories::BaseDirs::new().map(|d| d.data_local_dir().to_path_buf()) {
        let old = local.join("Programs").join("LighthouseManager").join("lighthouse.vrmanifest");
        if let Err(e) = vr.remove_manifest(&old.to_string_lossy()) {
            tracing::warn!("could not remove the old Lighthouse Manager registration: {e:#}");
        } else {
            tracing::info!("removed the old Lighthouse Manager registration");
        }
    }
}

pub fn unregister() -> Result<()> {
    let p = manifest_path().to_string_lossy().to_string();
    with_openvr(|vr| {
        if vr.is_installed(APP_KEY) {
            let _ = vr.set_auto_launch(APP_KEY, false);
            vr.remove_manifest(&p)?;
        }
        Ok(())
    })?;
    tracing::info!("removed SteamVR startup app");
    Ok(())
}

/// Reads SteamVR's own config files to tell whether we are registered and set to auto-launch.
/// Never starts SteamVR. `None` if the files cannot be read.
pub fn registered_from_files() -> Option<bool> {
    // %LOCALAPPDATA%\openvr on Windows, ~/.config/openvr on Linux.
    let dirs = directories::BaseDirs::new()?;
    let base = if cfg!(windows) { dirs.data_local_dir() } else { dirs.config_dir() };
    let vrpath = base.join("openvr").join("openvrpaths.vrpath");
    let paths: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vrpath).ok()?).ok()?;
    let config_dir = PathBuf::from(paths["config"].as_array()?.first()?.as_str()?);

    let appconfig: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(config_dir.join("appconfig.json")).ok()?).ok()?;
    let ours = normalize(&manifest_path());
    let listed = appconfig["manifest_paths"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).any(|s| normalize(&PathBuf::from(s)) == ours))
        .unwrap_or(false);
    if !listed {
        return Some(false);
    }
    let auto = std::fs::read_to_string(config_dir.join("vrappconfig").join(format!("{APP_KEY}.vrappconfig")))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["autolaunch"].as_bool())
        .unwrap_or(false);
    Some(auto)
}

fn normalize(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// `basewake install` / `basewake uninstall`.
pub fn run_cli(install: bool) -> (i32, String) {
    let r = if install { register() } else { unregister() };
    match r {
        Ok(()) if install => (
            0,
            format!(
                "Registered with SteamVR: {}\nIt now starts with SteamVR (Settings → Startup/Shutdown → Choose Startup Overlay Apps).\n",
                manifest_path().display()
            ),
        ),
        Ok(()) => (0, "Removed from SteamVR.\n".into()),
        Err(e) => (1, format!("error: {e:#}\n")),
    }
}

pub fn refresh_status_async(ctl: Arc<AppCtl>) {
    thread::spawn(move || {
        let r = registered_from_files();
        ctl.set_vr(|s| s.registered = r);
    });
}

pub fn set_registered_async(ctl: &Arc<AppCtl>, on: bool) {
    let ctl = ctl.clone();
    ctl.set_vr(|s| {
        s.busy = true;
        s.error = None;
    });
    thread::spawn(move || {
        let r = if on { register() } else { unregister() };
        let registered = registered_from_files();
        ctl.set_vr(|s| {
            s.busy = false;
            s.registered = registered;
            s.error = r.err().map(|e| format!("{e:#}"));
        });
    });
}
