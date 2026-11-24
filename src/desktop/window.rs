//! The desktop window (eframe) with its own title bar, the tray icon and the tray popup.
//! Closing or minimising puts the window away in the tray; only Quit ends the app.

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Layout, Margin, Pos2, RichText, Ui, ViewportCommand, ViewportId, pos2, vec2};

use super::chrome::{self, Marks, TITLE_H, TitleAction};
use super::tray::{Tray, TrayAction};
use crate::app::{AppCtl, UiRequest};
use crate::engine::{Command, Target};
use crate::ui::{self, PanelState, TEXT, TEXT_2, TEXT_3, icons::Tone, panel};

const WIDTH: f32 = 460.0;
/// First guess for the height (3 stations); corrected after the first frame.
const START_HEIGHT: f32 = 700.0;
const POPUP_W: f32 = 260.0;
const POPUP_TITLE: &str = "Basewake tray";

struct Popup {
    opened: Instant,
    /// Tray click, in points; the popup opens above and to the left of it.
    anchor: Pos2,
    had_focus: bool,
    /// The popup window is created hidden and shown after its first frame.
    shown: bool,
}

impl Popup {
    fn new(anchor: Pos2) -> Self {
        Self { opened: Instant::now(), anchor, had_focus: false, shown: false }
    }

    /// Above the click when there's room (Windows taskbar at the bottom), below it otherwise
    /// (macOS menu bar at the top).
    fn pos(&self, h: f32) -> Pos2 {
        let x = (self.anchor.x - POPUP_W + 8.0).max(0.0);
        let y = if self.anchor.y - h - 8.0 >= 0.0 { self.anchor.y - h - 8.0 } else { self.anchor.y + 8.0 };
        pos2(x, y)
    }
}

struct WindowApp {
    ctl: Arc<AppCtl>,
    panel: PanelState,
    popup_panel: PanelState,
    marks: Marks,
    tray: Option<Tray>,
    visible: bool,
    /// Window height last applied.
    height: f32,
    popup: Option<Popup>,
    /// Measured height of the popup content (starts with an estimate).
    popup_h: f32,
    /// Passes run so far (capped); eframe shows the window after its first frame, so a window
    /// that starts hidden is hidden again during the first few.
    passes: u8,
}

impl WindowApp {
    fn set_visible(&mut self, ctx: &egui::Context, visible: bool) {
        self.visible = visible;
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Visible(visible));
        if visible {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Focus);
        }
    }

    /// Without a tray icon there is nowhere to hide to, so the window minimises instead.
    fn hide(&mut self, ctx: &egui::Context) {
        if self.tray.is_some() {
            self.set_visible(ctx, false);
        } else {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Minimized(true));
        }
    }

    fn handle_tray(&mut self, ctx: &egui::Context) {
        let Some(tray) = &mut self.tray else { return };
        tray.update(&self.ctl.handle.snapshot(), self.ctl.vr_status().connected);
        let actions: Vec<TrayAction> = tray.events.try_iter().collect();
        for a in actions {
            match a {
                TrayAction::ToggleWindow => {
                    self.popup = None;
                    let minimized = ctx.input(|i| i.viewport().minimized).unwrap_or(false);
                    if !self.visible || minimized {
                        self.ctl.show_window();
                    } else {
                        self.set_visible(ctx, false);
                    }
                }
                TrayAction::OpenPopup { x, y } => {
                    let ppp = ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(1.0);
                    self.popup = Some(Popup::new(pos2(x as f32 / ppp, y as f32 / ppp)));
                    if self.ctl.handle.snapshot().activity.is_none() {
                        self.ctl.handle.send(Command::Refresh);
                    }
                }
                TrayAction::ShowWindow => self.ctl.show_window(),
                TrayAction::AllOn => self.ctl.handle.send(Command::PowerAll(Target::On)),
                TrayAction::AllOff => self.ctl.handle.send(Command::PowerAll(Target::Off)),
                TrayAction::Scan => self.ctl.handle.send(Command::Scan),
                TrayAction::Quit => self.ctl.quit(),
            }
        }
    }

    /// The tray popup: state and master switch, a switch per base station, then actions.
    /// Created hidden, measured and placed on its first frame, then shown without the Windows
    /// open animation so it appears at once.
    fn show_popup(&mut self, ctx: &egui::Context) {
        let Some(popup) = &self.popup else { return };
        let builder = egui::ViewportBuilder::default()
            .with_title(POPUP_TITLE)
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_visible(false)
            .with_position(popup.pos(self.popup_h))
            .with_inner_size([POPUP_W, self.popup_h]);

        let mut close = false;
        let mut open_window = false;
        let mut quit = false;
        let mut had_focus = popup.had_focus;
        let shown = popup.shown;
        let young = popup.opened.elapsed() < Duration::from_millis(600);
        let mut measured = self.popup_h;
        let ctl = self.ctl.clone();
        let st = &mut self.popup_panel;
        let marks = &self.marks;
        let popup_id = ViewportId::from_hash_of("tray-popup");

        ctx.show_viewport_immediate(popup_id, builder, |ui, _| {
            // Closes when it loses focus (click outside), like a native menu. In the first
            // moments Windows may hand focus to another window: take it back instead.
            if shown {
                let focused = ui.input(|i| i.viewport().focused).unwrap_or(false);
                if focused {
                    had_focus = true;
                } else if young {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Focus);
                } else if had_focus {
                    close = true;
                }
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape) || i.viewport().close_requested()) {
                close = true;
            }

            let full = ui.max_rect();
            ui.painter().rect_filled(full, egui::CornerRadius::ZERO, ui::RAISED);
            ui.painter().rect_stroke(
                full,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0, ui::HAIRLINE),
                egui::StrokeKind::Inside,
            );

            let inner = egui::Frame::new().inner_margin(Margin::symmetric(8, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                (open_window, quit) = popup_contents(ui, &ctl, st, marks);
            });
            measured = inner.response.rect.height().ceil();

            if !shown {
                // First frame, still hidden: fit the height, turn off the fade-in, then show.
                ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(vec2(POPUP_W, measured)));
                chrome::disable_open_animation(POPUP_TITLE);
                ui.ctx().send_viewport_cmd(ViewportCommand::Visible(true));
                ui.ctx().send_viewport_cmd(ViewportCommand::Focus);
            }
        });

        let first = !shown;
        if let Some(p) = &mut self.popup {
            p.had_focus = had_focus;
            if first {
                ctx.send_viewport_cmd_to(popup_id, ViewportCommand::OuterPosition(p.pos(measured)));
                p.shown = true;
            }
        }
        self.popup_h = measured;
        if open_window {
            self.ctl.show_window();
            close = true;
        }
        if quit {
            self.ctl.quit();
            close = true;
        }
        if close {
            self.popup = None;
        }
        if first {
            ctx.request_repaint();
        }
    }
}

/// Returns (open window, quit).
fn popup_contents(ui: &mut Ui, ctl: &Arc<AppCtl>, st: &mut PanelState, marks: &Marks) -> (bool, bool) {
    let snap = ctl.handle.snapshot();
    let busy = snap.activity.is_some();
    st.expire_pending(ui.ctx(), busy);
    let (on, total) = snap.powered_count();

    egui::Frame::new().inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let tex = marks.state(ui::overall_tone(&snap));
            ui.add(egui::Image::new((tex.id(), vec2(28.0, 28.0))));
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new("Base stations").family(ui::semibold()).size(15.0).color(TEXT));
                let line = match &snap.activity {
                    Some(a) => format!("{}…", a.trim_end_matches(['…', '.'])),
                    None if total == 0 => "None managed".to_string(),
                    None => format!("{on} of {total} on"),
                };
                ui.label(RichText::new(line).size(12.5).color(TEXT_2));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                panel::master_switch(ui, ctl, st, &snap, busy);
            });
        });
    });
    ui.add_space(4.0);
    chrome::separator(ui);
    ui.add_space(2.0);

    if snap.stations.is_empty() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("No base stations yet").size(13.0).color(TEXT_2));
        });
        ui.add_space(4.0);
    }
    for s in &snap.stations {
        ui.allocate_ui_with_layout(vec2(ui.available_width(), 32.0), Layout::left_to_right(Align::Center), |ui| {
            ui.add_space(8.0);
            let color = if s.enabled { TEXT } else { TEXT_3 };
            ui.label(RichText::new(panel::display_name(s)).size(13.5).color(color));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);
                panel::station_switch(ui, ctl, st, s, busy);
                if s.busy {
                    ui.add(egui::Spinner::new().size(12.0).color(TEXT_2));
                } else {
                    let (text, color) = panel::state_text(s);
                    ui.label(RichText::new(text).size(12.5).color(if s.error.is_some() { color } else { TEXT_3 }));
                }
            });
        });
    }

    ui.add_space(2.0);
    chrome::separator(ui);
    ui.add_space(2.0);
    let scan = if busy { "Working…" } else { "Scan for base stations" };
    if chrome::menu_item(ui, scan, !busy).clicked() {
        ctl.handle.send(Command::Scan);
    }
    let open = chrome::menu_item(ui, "Open window", true).clicked();
    let quit = chrome::menu_item(ui, "Quit Basewake", true).clicked();
    (open, quit)
}

impl eframe::App for WindowApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        ui::BG.to_normalized_gamma_f32()
    }

    /// Runs even while the window is hidden in the tray: requests from other threads, tray
    /// events and the tray icon.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.passes < 4 {
            self.passes += 1;
            if !self.visible {
                ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Visible(false));
                ctx.request_repaint();
            }
        }
        for r in self.ctl.take_requests() {
            match r {
                UiRequest::ShowWindow => {
                    self.popup = None;
                    self.set_visible(ctx, true);
                }
                UiRequest::HideWindow => self.hide(ctx),
                UiRequest::Quit => {
                    self.popup = None;
                    ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
                }
            }
        }
        self.handle_tray(ctx);

        // Closing from the system (Alt+F4, taskbar) also goes to the tray.
        if ctx.input(|i| i.viewport().close_requested()) && !self.ctl.is_quitting() {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::CancelClose);
            self.hide(ctx);
        }
        // The popup is a child of the main window: while it is open the pass has to run even
        // if the main window is hidden.
        if self.popup.is_some() && !self.visible {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::RequestPaintWhileHidden);
        }
        // Keep spinners moving while something runs.
        if self.ctl.handle.snapshot().activity.is_some() || self.ctl.vr_status().busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.visible {
            let full = ui.max_rect();
            if let TitleAction::Hide = chrome::title_bar(ui, &self.marks) {
                self.hide(&ctx);
            }
            let panel_h = egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(ui::BG))
                .show(ui, |ui| ui::panel::draw(ui, &self.ctl, &mut self.panel))
                .inner;
            // Fixed width, no scrolling: the height follows the content (e.g. more base stations).
            let needed = (TITLE_H + panel_h).ceil();
            if (needed - self.height).abs() >= 1.0 {
                self.height = needed;
                ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::InnerSize(vec2(WIDTH, needed)));
            }
            // Thin border: without the system frame the window would blend into dark backgrounds.
            ui.painter().rect_stroke(
                full,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0, ui::HAIRLINE),
                egui::StrokeKind::Inside,
            );
        }
        if self.popup.is_some() {
            self.show_popup(&ctx);
        }
    }
}

pub fn run(ctl: Arc<AppCtl>, visible: bool) -> i32 {
    let icon = egui::IconData { rgba: ui::icons::rgba(Tone::On, 64), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Basewake")
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size([WIDTH, START_HEIGHT])
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
            c.set_egui(cc.egui_ctx.clone());
            let tray = Tray::new(&cc.egui_ctx);
            if tray.is_none() {
                tracing::warn!("no tray icon; closing the window minimises it instead");
            }
            Ok(Box::new(WindowApp {
                ctl: c,
                panel: PanelState::default(),
                popup_panel: PanelState::default(),
                marks: Marks::new(&cc.egui_ctx),
                tray,
                visible,
                height: START_HEIGHT,
                // Testing aid: open the tray popup at startup.
                popup: std::env::var_os("BASEWAKE_OPEN_POPUP").map(|_| Popup::new(pos2(1100.0, 700.0))),
                popup_h: 240.0,
                passes: 0,
            }))
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
