//! The one panel: header with a master switch, the station list, and settings. Drawn by the
//! desktop window and by the SteamVR dashboard overlay.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Layout, RichText, Sense};

use super::widgets::{more_button, segmented, switch, text_button};
use super::{DANGER, HAIRLINE, RAISED, TEXT, TEXT_2, TEXT_3, semibold, state_color};
use crate::app::AppCtl;
use crate::engine::{Command, Config, Mac, OffMode, PowerState, Snapshot, StationStatus, Target};
use crate::vr::VrStatus;

const ROW_H: f32 = 56.0;
const ROW_PAD: f32 = 10.0;

#[derive(Default)]
pub struct PanelState {
    /// Station being renamed, with the text typed so far.
    renaming: Option<(Mac, String)>,
    /// Switch positions the user just asked for, shown until the controller catches up.
    /// `None` is the master switch.
    pending: HashMap<Option<Mac>, (bool, Instant)>,
}

/// Draws the whole panel (no scrolling) and returns the height it needs, in points.
impl PanelState {
    /// A pending switch position expires shortly after the controller is idle again.
    pub fn expire_pending(&mut self, ctx: &egui::Context, busy: bool) {
        let now = Instant::now();
        for p in self.pending.values_mut() {
            if busy {
                p.1 = now;
            }
        }
        self.pending.retain(|_, (_, at)| at.elapsed() < Duration::from_millis(300));
        if !self.pending.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

/// The switch for every managed base station at once. Moves as soon as it is clicked.
pub fn master_switch(ui: &mut egui::Ui, ctl: &AppCtl, st: &mut PanelState, snap: &Snapshot, busy: bool) {
    let (on, total) = snap.powered_count();
    let shown = st.pending.get(&None).map(|p| p.0).unwrap_or(total > 0 && on == total);
    let resp = switch(ui, shown, total > 0 && !busy, "All base stations")
        .on_hover_text(if shown { "Turn all off" } else { "Turn all on" });
    if resp.clicked() {
        let target = !shown;
        st.pending.insert(None, (target, Instant::now()));
        ctl.handle.send(Command::PowerAll(if target { Target::On } else { Target::Off }));
    }
}

/// One base station's switch. Moves as soon as it is clicked.
pub fn station_switch(ui: &mut egui::Ui, ctl: &AppCtl, st: &mut PanelState, s: &StationStatus, busy: bool) {
    let mac = s.mac;
    let shown = st.pending.get(&Some(mac)).map(|p| p.0).unwrap_or(s.state.is_powered());
    if switch(ui, shown, !busy, &display_name(s)).clicked() {
        let target = !shown;
        st.pending.insert(Some(mac), (target, Instant::now()));
        ctl.handle.send(Command::Power(mac, if target { Target::On } else { Target::Off }));
    }
}

pub fn draw(ui: &mut egui::Ui, ctl: &Arc<AppCtl>, st: &mut PanelState) -> f32 {
    egui::Frame::new()
        .inner_margin(egui::Margin { left: 22, right: 22, top: 20, bottom: 22 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            draw_inner(ui, ctl, st);
        })
        .response
        .rect
        .height()
}

fn draw_inner(ui: &mut egui::Ui, ctl: &Arc<AppCtl>, st: &mut PanelState) {
    let snap = ctl.handle.snapshot();
    let cfg = ctl.handle.config();
    let vr = ctl.vr_status();
    let busy = snap.activity.is_some();

    st.expire_pending(ui.ctx(), busy);

    header(ui, ctl, st, &snap, &vr, busy);
    problems(ui, &snap);
    ui.add_space(18.0);

    if snap.stations.is_empty() {
        empty(ui, ctl, busy, &snap);
    } else {
        divider(ui);
        for s in &snap.stations {
            station_row(ui, ctl, st, s, busy);
            divider(ui);
        }
        ui.add_space(8.0);
        let label = if is_scanning(&snap) { "Scanning…" } else { "Scan for base stations" };
        if text_button(ui, label, !busy)
            .on_hover_text("Listens for about 5 seconds and adds any new base station")
            .clicked()
        {
            ctl.handle.send(Command::Scan);
        }
    }

    ui.add_space(30.0);
    settings(ui, ctl, &cfg, &vr);
}

fn is_scanning(snap: &Snapshot) -> bool {
    snap.activity.as_deref().is_some_and(|a| a.to_lowercase().starts_with("scan"))
}

fn header(ui: &mut egui::Ui, ctl: &Arc<AppCtl>, st: &mut PanelState, snap: &Snapshot, vr: &VrStatus, busy: bool) {
    let (on, total) = snap.powered_count();
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new("Base stations").text_style(egui::TextStyle::Heading).color(TEXT));
            let mut line = match &snap.activity {
                Some(a) => format!("{}…", a.trim_end_matches(['…', '.'])),
                None if total == 0 => "None managed".to_string(),
                None => format!("{on} of {total} on"),
            };
            if vr.connected {
                line.push_str("  ·  SteamVR running");
            }
            ui.label(RichText::new(line).color(TEXT_2));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // Line up with the station switches, which sit left of each row's "more" button.
            ui.add_space(26.0 + 8.0);
            master_switch(ui, ctl, st, snap, busy);
        });
    });
}

/// Errors from the last operation, in words a person can act on.
fn problems(ui: &mut egui::Ui, snap: &Snapshot) {
    let mut lines: Vec<(String, Option<String>)> = snap
        .stations
        .iter()
        .filter_map(|s| {
            let e = s.error.as_ref()?;
            let name = display_name(s);
            Some(if is_unreachable(e) {
                (format!("Couldn't reach {name}. Check that it has power and is in Bluetooth range."), Some(e.clone()))
            } else {
                (format!("{name}: {e}"), None)
            })
        })
        .collect();
    if lines.is_empty()
        && snap.message_is_error
        && let Some(m) = &snap.message
    {
        lines.push((capitalize(m), None));
    }
    if !lines.is_empty() {
        ui.add_space(10.0);
        for (text, detail) in lines {
            let r = ui.label(RichText::new(text).color(DANGER));
            if let Some(d) = detail {
                r.on_hover_text(d);
            }
        }
    }
}

fn empty(ui: &mut egui::Ui, ctl: &Arc<AppCtl>, busy: bool, snap: &Snapshot) {
    divider(ui);
    ui.add_space(18.0);
    ui.label(RichText::new("No base stations yet").color(TEXT));
    ui.label(RichText::new("Plug them in, then scan. They show up with names starting LHB-.").color(TEXT_2));
    ui.add_space(12.0);
    let label = if is_scanning(snap) { "Scanning…" } else { "Scan for base stations" };
    if ui.add_enabled(!busy, egui::Button::new(label).min_size(egui::vec2(0.0, 30.0))).clicked() {
        ctl.handle.send(Command::Scan);
    }
    ui.add_space(18.0);
    divider(ui);
}

fn divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, HAIRLINE));
}

pub fn display_name(s: &StationStatus) -> String {
    if s.name.trim().is_empty() { s.mac.to_string() } else { s.name.clone() }
}

fn is_unreachable(e: &str) -> bool {
    let e = e.to_lowercase();
    ["unreachable", "timed out", "timeout", "not found", "connect"].iter().any(|k| e.contains(k))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

pub fn state_text(s: &StationStatus) -> (&'static str, egui::Color32) {
    if let Some(e) = &s.error {
        return (if is_unreachable(e) { "Unreachable" } else { "Error" }, DANGER);
    }
    let text = match s.state {
        PowerState::On => "On",
        PowerState::Transition(_) => "Starting",
        PowerState::Sleep => "Sleep",
        PowerState::Standby => "Standby",
        PowerState::Unknown => "",
    };
    (text, state_color(s.state))
}

fn station_row(ui: &mut egui::Ui, ctl: &Arc<AppCtl>, st: &mut PanelState, s: &StationStatus, busy: bool) {
    let mac = s.mac;
    // Bleed the row's hover fill slightly past the content edges.
    let row = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), ROW_H));
    let row_resp = ui.interact(row, ui.scope_id().with(("row", mac.0)), Sense::click());
    let row_hovered = ui.rect_contains_pointer(row);
    if row_hovered {
        ui.painter().rect_filled(row.expand2(egui::vec2(ROW_PAD, 0.0)), egui::CornerRadius::same(6), RAISED);
    }

    ui.allocate_ui_with_layout(row.size(), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_size(row.size());

        // Name + address
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let renaming = matches!(&st.renaming, Some((m, _)) if *m == mac);
            if renaming {
                let mut done = None;
                if let Some((_, buf)) = st.renaming.as_mut() {
                    let te = ui.add(
                        egui::TextEdit::singleline(buf)
                            .desired_width(220.0)
                            .margin(egui::vec2(6.0, 2.0))
                            .hint_text(mac.to_string()),
                    );
                    if !te.has_focus() && !te.lost_focus() {
                        te.request_focus();
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        done = Some(None);
                    } else if te.lost_focus() {
                        done = Some(Some(buf.trim().to_string()));
                    }
                }
                match done {
                    Some(Some(name)) => {
                        st.renaming = None;
                        edit(ctl, move |c| {
                            if let Some(x) = c.find_mut(mac) {
                                x.name = name;
                            }
                        });
                    }
                    Some(None) => st.renaming = None,
                    None => {}
                }
            } else {
                let color = if s.enabled { TEXT } else { TEXT_2 };
                ui.label(RichText::new(display_name(s)).color(color));
            }
            let mut meta = mac.to_string();
            if !s.enabled {
                meta.push_str("  ·  not managed");
            }
            ui.label(RichText::new(meta).monospace().color(TEXT_3));
        });

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let more = more_button(ui, row_hovered).on_hover_text("More");
            menu(ctl, st, s, &more, &row_resp);
            ui.add_space(8.0);

            station_switch(ui, ctl, st, s, busy);
            ui.add_space(6.0);

            if s.busy {
                ui.add(egui::Spinner::new().size(14.0).color(TEXT_2));
            } else {
                let (text, color) = state_text(s);
                let label = ui.label(RichText::new(text).color(color));
                if let Some(e) = &s.error {
                    label.on_hover_text(e);
                }
            }
        });
    });
}

fn menu(ctl: &Arc<AppCtl>, st: &mut PanelState, s: &StationStatus, more: &egui::Response, row: &egui::Response) {
    let mac = s.mac;
    let enabled = s.enabled;
    let name = s.name.clone();
    let mut contents = |ui: &mut egui::Ui| {
        ui.set_min_width(200.0);
        if ui.button("Rename").clicked() {
            st.renaming = Some((mac, name.clone()));
            // The text field appears next frame; don't wait for more input to draw it.
            ui.ctx().request_repaint();
        }
        let manage = if enabled { "Stop managing" } else { "Manage" };
        if ui
            .button(manage)
            .on_hover_text("Managed base stations follow SteamVR and the main switch")
            .clicked()
        {
            edit(ctl, move |c| {
                if let Some(x) = c.find_mut(mac) {
                    x.enabled = !enabled;
                }
            });
        }
        ui.separator();
        if ui.button(RichText::new("Remove").color(DANGER)).clicked() {
            ctl.handle.send(Command::Forget(mac));
        }
    };
    egui::Popup::menu(more).show(&mut contents);
    egui::Popup::context_menu(row).show(&mut contents);
}

fn settings(ui: &mut egui::Ui, ctl: &Arc<AppCtl>, cfg: &Config, vr: &VrStatus) {
    ui.label(RichText::new("Settings").family(semibold()).size(15.0).color(TEXT));
    ui.add_space(6.0);

    setting_row(ui, "Turn off as", Some("Sleep is fully off. Standby wakes faster."), |ui| {
        let selected = if cfg.off_mode == OffMode::Sleep { 0 } else { 1 };
        if let Some(i) = segmented(ui, ["Sleep", "Standby"], selected) {
            let mode = if i == 0 { OffMode::Sleep } else { OffMode::Standby };
            edit(ctl, move |c| c.off_mode = mode);
        }
    });

    if crate::vr::SUPPORTED {
        steamvr_settings(ui, ctl, cfg, vr);
    }
    bool_row(ui, ctl, "Manage new base stations automatically", cfg.auto_enable_new, |c, v| c.auto_enable_new = v);

    ui.add_space(14.0);
    if text_button(ui, "Open settings and logs folder", true).clicked() {
        crate::desktop::chrome::open_in_browser(&Config::dir().to_string_lossy());
    }
}

fn steamvr_settings(ui: &mut egui::Ui, ctl: &Arc<AppCtl>, cfg: &Config, vr: &VrStatus) {
    let registered = vr.registered.unwrap_or(false);
    let note = match (&vr.error, vr.registered) {
        (Some(e), _) => Some(e.clone()),
        (None, None) => Some("Couldn't read SteamVR's settings".to_string()),
        _ => None,
    };
    setting_row(ui, "Start with SteamVR", note.as_deref(), |ui| {
        if switch(ui, registered, !vr.busy, "Start with SteamVR").clicked() {
            crate::vr::manifest::set_registered_async(ctl, !registered);
        }
        if vr.busy {
            ui.add(egui::Spinner::new().size(14.0).color(TEXT_2));
        }
    });

    bool_row(ui, ctl, "Find new base stations when SteamVR starts", cfg.scan_on_start, |c, v| c.scan_on_start = v);
    bool_row(ui, ctl, "Quit when SteamVR closes", cfg.close_with_steamvr, |c, v| c.close_with_steamvr = v);
}

fn setting_row(ui: &mut egui::Ui, label: &str, note: Option<&str>, control: impl FnOnce(&mut egui::Ui)) {
    let h = if note.is_some() { 48.0 } else { 36.0 };
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_size(egui::vec2(ui.available_width(), h));
        match note {
            Some(n) => {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(RichText::new(label).color(TEXT));
                    ui.label(RichText::new(n).small().color(TEXT_2));
                });
            }
            None => {
                ui.label(RichText::new(label).color(TEXT));
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
}

fn bool_row(ui: &mut egui::Ui, ctl: &Arc<AppCtl>, label: &str, value: bool, set: fn(&mut Config, bool)) {
    setting_row(ui, label, None, |ui| {
        if switch(ui, value, true, label).clicked() {
            let v = !value;
            edit(ctl, move |c| set(c, v));
        }
    });
}

fn edit(ctl: &AppCtl, f: impl FnOnce(&mut Config) + Send + 'static) {
    ctl.handle.send(Command::EditConfig(Box::new(f)));
}
