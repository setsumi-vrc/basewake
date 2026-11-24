//! Desktop window (eframe). Closing or minimising hides it to the tray.

use std::sync::Arc;

use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::app::AppCtl;
use crate::ui::{self, PanelState, icons::Tone};

struct WindowApp {
    ctl: Arc<AppCtl>,
    panel: PanelState,
    /// Window height last applied.
    height: f32,
}

const WIDTH: f32 = 460.0;
/// First guess for the height (3 stations); corrected after the first frame.
const START_HEIGHT: f32 = 660.0;

impl eframe::App for WindowApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Close button → hide to tray (unless we are really quitting).
        if ctx.input(|i| i.viewport().close_requested()) && !self.ctl.is_quitting() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.ctl.hide_window();
        }
        // (Minimise → hide to tray is handled in `minimize_hook`, since eframe stops
        // running this function while the window is minimised.)
        let needed = egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(ui::BG))
            .show(ui, |ui| ui::panel::draw(ui, &self.ctl, &mut self.panel))
            .inner;
        // Fixed width, no scrolling: the height follows the content (e.g. more base stations).
        let needed = needed.ceil();
        if (needed - self.height).abs() >= 1.0 {
            self.height = needed;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(WIDTH, needed)));
        }
        // Keep spinners moving while something runs.
        if self.ctl.handle.snapshot().activity.is_some() || self.ctl.vr_status().busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

pub fn run(ctl: Arc<AppCtl>, visible: bool) -> i32 {
    let icon = egui::IconData { rgba: ui::icons::rgba(Tone::On, 64), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Basewake")
            .with_inner_size([WIDTH, START_HEIGHT])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_icon(Arc::new(icon))
            .with_visible(visible),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let c = ctl.clone();
    let result = eframe::run_native(
        "Basewake",
        options,
        Box::new(move |cc| {
            ui::style(&cc.egui_ctx);
            if let Ok(h) = cc.window_handle()
                && let RawWindowHandle::Win32(w) = h.as_raw() {
                    c.set_window(w.hwnd.get(), cc.egui_ctx.clone());
                    install_minimize_hook(w.hwnd.get());
                }
            Ok(Box::new(WindowApp { ctl: c, panel: PanelState::default(), height: START_HEIGHT }))
        }),
    );
    match result {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!("window: {e}");
            crate::desktop::fatal(&format!("Basewake could not open its window:\n\n{e}"));
            1
        }
    }
}

/// Hides the window when it gets minimised, so it lives in the tray instead of the taskbar.
fn install_minimize_hook(hwnd: isize) {
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{SIZE_MINIMIZED, SW_HIDE, ShowWindow, WM_SIZE};

    unsafe extern "system" fn proc(h: HWND, msg: u32, w: WPARAM, l: LPARAM, _id: usize, _data: usize) -> LRESULT {
        let r = unsafe { DefSubclassProc(h, msg, w, l) };
        if msg == WM_SIZE && w.0 as u32 == SIZE_MINIMIZED {
            unsafe {
                let _ = ShowWindow(h, SW_HIDE);
            }
        }
        r
    }
    unsafe {
        if !SetWindowSubclass(HWND(hwnd as *mut _), Some(proc), 1, 0).as_bool() {
            tracing::warn!("could not hook minimise; minimising keeps the window in the taskbar");
        }
    }
}
