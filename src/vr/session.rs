//! Following a SteamVR session: connect, turn base stations on, show the dashboard panel, and turn
//! them off when SteamVR quits.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::dashboard::Dashboard;
use super::openvr::{self, OpenVr};
use crate::app::AppCtl;
use crate::engine::Command;

static ACTIVE: Mutex<Option<Arc<Mutex<OpenVr>>>> = Mutex::new(None);

/// The OpenVR connection of the running session, if any (reused for registering the app).
pub fn active() -> Option<Arc<Mutex<OpenVr>>> {
    ACTIVE.lock().unwrap().clone()
}

/// Runs until SteamVR quits. Blocks the calling thread.
pub fn run(ctl: &Arc<AppCtl>) {
    let vr = match connect(ctl) {
        Some(v) => v,
        None => return,
    };
    *ACTIVE.lock().unwrap() = Some(vr.clone());
    ctl.set_vr(|s| {
        s.connected = true;
        s.error = None;
    });
    tracing::info!("connected to SteamVR");
    ctl.handle.send(Command::SessionStart);

    let mut dashboard = match Dashboard::new(&vr.lock().unwrap()) {
        Ok(d) => Some(d),
        Err(e) => {
            tracing::error!("SteamVR dashboard panel unavailable: {e:#}");
            None
        }
    };

    loop {
        let quit = {
            let v = vr.lock().unwrap();
            let mut quit = false;
            while let Some(ev) = v.poll_event() {
                if ev.event_type == openvr::EVENT_QUIT || ev.event_type == openvr::EVENT_DRIVER_REQUESTED_QUIT {
                    quit = true;
                }
            }
            if !quit
                && let Some(d) = dashboard.as_mut() {
                    d.tick(&v, ctl);
                }
            quit
        };
        if quit {
            break;
        }
        let visible = dashboard.as_ref().is_some_and(|d| d.visible());
        thread::sleep(if visible { Duration::from_millis(11) } else { Duration::from_millis(100) });
    }

    tracing::info!("SteamVR is quitting");
    // Hand the "off" to a separate process first: SteamVR may end this one at any moment.
    spawn_detached_off();
    {
        let v = vr.lock().unwrap();
        if let Some(d) = dashboard.take() {
            d.destroy(&v);
        }
        v.acknowledge_quit_exiting();
    }
    *ACTIVE.lock().unwrap() = None;
    drop(vr); // VR_Shutdown
    ctl.set_vr(|s| s.connected = false);

    if ctl.handle.config().close_with_steamvr {
        ctl.quit();
    }
}

/// SteamVR starts us before it is fully up, so retry for a while.
fn connect(ctl: &Arc<AppCtl>) -> Option<Arc<Mutex<OpenVr>>> {
    let mut last = String::new();
    for attempt in 0..15 {
        match OpenVr::init(openvr::APP_OVERLAY) {
            Ok(v) => return Some(v),
            Err(e) => {
                last = format!("{e:#}");
                tracing::debug!("SteamVR connect attempt {attempt}: {last}");
                thread::sleep(Duration::from_secs(2));
            }
        }
    }
    tracing::error!("could not connect to SteamVR: {last}");
    ctl.set_vr(|s| s.error = Some(format!("Could not connect to SteamVR: {last}")));
    None
}

/// Starts `basewake.exe --local off` as an independent process (outside SteamVR's job when
/// allowed), so the base stations get turned off even if this process is killed.
pub fn spawn_detached_off() {
    use std::os::windows::process::CommandExt;
    use std::process::{Command as Proc, Stdio};
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let Ok(exe) = std::env::current_exe() else { return };
    let spawn = |flags: u32| {
        Proc::new(&exe)
            .args(["--local", "off"])
            .creation_flags(flags)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    };
    let base = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW;
    match spawn(base | CREATE_BREAKAWAY_FROM_JOB).or_else(|_| spawn(base)) {
        Ok(child) => tracing::info!("turning base stations off in process {}", child.id()),
        Err(e) => tracing::error!("could not start the turn-off process: {e}"),
    }
}
