//! The resident app: controller + tray + window + IPC server + SteamVR session, wired together.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    IsIconic, PostMessageW, PostThreadMessageW, SW_HIDE, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
    WM_CLOSE, WM_QUIT,
};

use crate::cli::{self, Cmd};
use crate::engine::{self, Handle};
use crate::ipc;
use crate::vr::{self, VrStatus};

/// Thread message that tells the tray thread to redraw its icon/tooltip.
pub const WM_TRAY_REFRESH: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartMode {
    /// Opened by the user: show the window.
    Normal,
    /// Opened by SteamVR: stay in the tray and follow the session.
    SteamVr,
}

/// Shared by every thread of the app.
pub struct AppCtl {
    pub handle: Handle,
    pub vr: RwLock<VrStatus>,
    /// Set when something the VR dashboard shows has changed.
    pub vr_dirty: AtomicBool,
    vr_running: AtomicBool,
    hwnd: AtomicIsize,
    tray_thread: AtomicU32,
    quitting: AtomicBool,
    egui: OnceLock<eframe::egui::Context>,
}

impl AppCtl {
    /// Something changed: repaint the window, refresh the tray and the VR panel.
    pub fn notify(&self) {
        if let Some(ctx) = self.egui.get() {
            ctx.request_repaint();
        }
        let tid = self.tray_thread.load(Ordering::Acquire);
        if tid != 0 {
            unsafe {
                let _ = PostThreadMessageW(tid, WM_TRAY_REFRESH, WPARAM(0), LPARAM(0));
            }
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

    fn hwnd(&self) -> Option<HWND> {
        let h = self.hwnd.load(Ordering::Acquire);
        (h != 0).then_some(HWND(h as *mut _))
    }

    pub(crate) fn set_window(&self, hwnd: isize, ctx: eframe::egui::Context) {
        self.hwnd.store(hwnd, Ordering::Release);
        let _ = self.egui.set(ctx);
    }

    pub(crate) fn set_tray_thread(&self, tid: u32) {
        self.tray_thread.store(tid, Ordering::Release);
    }

    /// Shows and focuses the window (works from any thread, even while eframe is idle).
    pub fn show_window(&self) {
        if let Some(h) = self.hwnd() {
            unsafe {
                if IsIconic(h).as_bool() {
                    let _ = ShowWindow(h, SW_RESTORE);
                } else {
                    let _ = ShowWindow(h, SW_SHOW);
                }
                let _ = SetForegroundWindow(h);
            }
        }
        if let Some(ctx) = self.egui.get() {
            ctx.request_repaint();
        }
        // The window has no refresh button: re-read the stations whenever it is opened.
        if self.handle.snapshot().activity.is_none() {
            self.handle.send(engine::Command::Refresh);
        }
    }

    pub fn hide_window(&self) {
        if let Some(h) = self.hwnd() {
            unsafe {
                let _ = ShowWindow(h, SW_HIDE);
            }
        }
    }

    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::Acquire)
    }

    /// Closes the app: removes the tray icon and closes the window.
    pub fn quit(&self) {
        if self.quitting.swap(true, Ordering::AcqRel) {
            return;
        }
        tracing::info!("quitting");
        let tid = self.tray_thread.load(Ordering::Acquire);
        if tid != 0 {
            unsafe {
                let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(h) = self.hwnd() {
            unsafe {
                let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(ctx) = self.egui.get() {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
            ctx.request_repaint();
        }
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
        hwnd: AtomicIsize::new(0),
        tray_thread: AtomicU32::new(0),
        quitting: AtomicBool::new(false),
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
    crate::desktop::tray::spawn(ctl.clone());
    vr::manifest::refresh_status_async(ctl.clone());

    if mode == StartMode::SteamVr {
        ctl.attach_steamvr();
    } else {
        // Show real states when the window opens.
        handle.send(engine::Command::Refresh);
    }

    crate::desktop::window::run(ctl, mode == StartMode::Normal)
}
