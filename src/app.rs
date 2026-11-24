//! The resident app: controller + tray + window + IPC server + SteamVR session, wired together.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread;
use std::time::Duration;

use crate::cli::{self, Cmd};
use crate::engine::{self, Handle};
use crate::ipc;
use crate::vr::{self, VrStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartMode {
    /// Opened by the user: show the window.
    Normal,
    /// Opened by SteamVR: stay in the tray and follow the session.
    SteamVr,
}

/// Something only the UI thread can do, asked for from another thread (IPC, SteamVR, tray).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRequest {
    ShowWindow,
    HideWindow,
    Quit,
}

/// Shared by every thread of the app.
pub struct AppCtl {
    pub handle: Handle,
    pub vr: RwLock<VrStatus>,
    /// Set when something the VR dashboard shows has changed.
    pub vr_dirty: AtomicBool,
    vr_running: AtomicBool,
    quitting: AtomicBool,
    requests: Mutex<Vec<UiRequest>>,
    egui: OnceLock<eframe::egui::Context>,
}

impl AppCtl {
    /// Something changed: repaint the window (which also refreshes the tray) and the VR panel.
    pub fn notify(&self) {
        if let Some(ctx) = self.egui.get() {
            ctx.request_repaint();
        }
        self.vr_dirty.store(true, Ordering::Release);
    }

    pub fn set_vr(&self, f: impl FnOnce(&mut VrStatus)) {
        f(&mut self.vr.write().unwrap());
        self.notify();
    }

    pub fn vr_status(&self) -> VrStatus {
        self.vr.read().unwrap().clone()
    }

    pub(crate) fn set_egui(&self, ctx: eframe::egui::Context) {
        let _ = self.egui.set(ctx);
    }

    fn request(&self, r: UiRequest) {
        self.requests.lock().unwrap().push(r);
        if let Some(ctx) = self.egui.get() {
            ctx.request_repaint();
        }
    }

    /// Requests waiting for the UI thread.
    pub(crate) fn take_requests(&self) -> Vec<UiRequest> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }

    /// Shows and focuses the window (from any thread).
    pub fn show_window(&self) {
        self.request(UiRequest::ShowWindow);
        // The window has no refresh button: re-read the stations whenever it is opened.
        if self.handle.snapshot().activity.is_none() {
            self.handle.send(engine::Command::Refresh);
        }
    }

    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::Acquire)
    }

    /// Closes the app: the window closes, which also removes the tray icon.
    pub fn quit(&self) {
        if self.quitting.swap(true, Ordering::AcqRel) {
            return;
        }
        tracing::info!("quitting");
        self.request(UiRequest::Quit);
        // Safety net in case the window never gets to process the close.
        thread::spawn(|| {
            thread::sleep(Duration::from_secs(4));
            tracing::warn!("window did not close in time; exiting");
            std::process::exit(0);
        });
    }

    /// Connects to SteamVR (once). Called at startup in SteamVR mode or when SteamVR launches
    /// a second copy of the exe.
    pub fn attach_steamvr(self: &Arc<Self>) {
        if self.vr_running.swap(true, Ordering::AcqRel) {
            tracing::info!("SteamVR session already active");
            return;
        }
        let ctl = self.clone();
        thread::Builder::new()
            .name("steamvr".into())
            .spawn(move || {
                vr::session::run(&ctl);
                ctl.vr_running.store(false, Ordering::Release);
            })
            .expect("spawn steamvr thread");
    }

    /// Answers a request from another `basewake.exe`.
    fn handle_ipc(self: &Arc<Self>, request: &str) -> (i32, String) {
        match request {
            "show" => {
                self.show_window();
                (0, String::new())
            }
            "hide" => {
                self.request(UiRequest::HideWindow);
                (0, String::new())
            }
            "quit" => {
                self.quit();
                (0, String::new())
            }
            "steamvr" => {
                self.attach_steamvr();
                (0, String::new())
            }
            other => match Cmd::parse(other) {
                Some(cmd) => cli::execute(&self.handle, cmd),
                None => (2, format!("unknown request: {other}\n")),
            },
        }
    }
}

/// Runs the resident app until it quits. Returns the process exit code.
pub fn run(mode: StartMode) -> i32 {
    // Another instance already running? Hand over to it.
    let server = match ipc::claim() {
        Ok(s) => s,
        Err(_) => {
            let request = if mode == StartMode::SteamVr { "steamvr" } else { "show" };
            return match ipc::send(request) {
                Some((code, _)) => code,
                None => {
                    tracing::error!("another instance owns the pipe but does not answer");
                    1
                }
            };
        }
    };

    let handle = match engine::start() {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("{e:#}");
            crate::desktop::fatal(&format!("Basewake could not start:\n\n{e:#}"));
            return 1;
        }
    };

    let ctl = Arc::new(AppCtl {
        handle: handle.clone(),
        vr: RwLock::new(VrStatus::default()),
        vr_dirty: AtomicBool::new(true),
        vr_running: AtomicBool::new(false),
        quitting: AtomicBool::new(false),
        requests: Mutex::new(Vec::new()),
        egui: OnceLock::new(),
    });
    {
        let weak = Arc::downgrade(&ctl);
        handle.on_change(move || {
            if let Some(c) = weak.upgrade() {
                c.notify();
            }
        });
    }
    {
        let c = ctl.clone();
        server.serve(Arc::new(move |req: &str| c.handle_ipc(req)));
    }
    vr::manifest::refresh_status_async(ctl.clone());

    if mode == StartMode::SteamVr {
        ctl.attach_steamvr();
    } else {
        // Show real states when the window opens.
        handle.send(engine::Command::Refresh);
    }

    crate::desktop::window::run(ctl, mode == StartMode::Normal)
}
