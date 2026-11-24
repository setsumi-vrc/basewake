//! UI shared by the desktop window and the SteamVR dashboard panel.
//!
//! Visual language: a quiet native Windows utility. Segoe UI, one neutral dark surface, hairline
//! dividers instead of cards, switches for anything on/off, and a single green accent that only
//! ever means "on".

pub mod icons;
pub mod panel;
pub mod widgets;

pub use panel::PanelState;

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle};

use crate::engine::{PowerState, Snapshot};
use icons::Tone;

// Palette. Text colours are checked against BG: TEXT 15:1, TEXT_2 6.3:1, DANGER 5.2:1.
pub const BG: Color32 = Color32::from_rgb(0x12, 0x13, 0x16);
pub const RAISED: Color32 = Color32::from_rgb(0x1B, 0x1D, 0x21);
pub const HAIRLINE: Color32 = Color32::from_rgb(0x26, 0x29, 0x2E);
pub const TEXT: Color32 = Color32::from_rgb(0xE7, 0xE9, 0xEC);
pub const TEXT_2: Color32 = Color32::from_rgb(0x95, 0x9B, 0xA5);
pub const TEXT_3: Color32 = Color32::from_rgb(0x62, 0x68, 0x71);
pub const ACCENT: Color32 = Color32::from_rgb(0x5C, 0xC9, 0x8D);
pub const TRACK_OFF: Color32 = Color32::from_rgb(0x3A, 0x3E, 0x46);
pub const DANGER: Color32 = Color32::from_rgb(0xEC, 0x6F, 0x72);

/// Named font family for headings and emphasis.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Segoe UI (+ semibold) and Cascadia Mono from Windows, falling back to egui's fonts.
fn fonts() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let dir = std::path::Path::new(r"C:\Windows\Fonts");
    let load = |file: &str| std::fs::read(dir.join(file)).ok().map(|b| Arc::new(FontData::from_owned(b)));

    let mut semibold_chain = Vec::new();
    if let Some(f) = load("segoeui.ttf") {
        defs.font_data.insert("segoe".into(), f);
        defs.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    if let Some(f) = load("seguisb.ttf") {
        defs.font_data.insert("segoe-sb".into(), f);
        semibold_chain.push("segoe-sb".to_string());
    }
    semibold_chain.extend(defs.families[&FontFamily::Proportional].iter().cloned());
    defs.families.insert(semibold(), semibold_chain);
    if let Some(f) = load("CascadiaMono.ttf").or_else(|| load("consola.ttf")) {
        defs.font_data.insert("mono".into(), f);
        defs.families.entry(FontFamily::Monospace).or_default().insert(0, "mono".into());
    }
    defs
}

pub fn style(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_visuals(egui::Visuals::dark());
    ctx.global_style_mut(|s| {
        s.text_styles = [
            (TextStyle::Heading, FontId::new(19.0, semibold())),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 4.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.scroll.floating = true;

        let v = &mut s.visuals;
        v.panel_fill = BG;
        v.window_fill = RAISED;
        v.extreme_bg_color = RAISED;
        v.faint_bg_color = RAISED;
        v.window_stroke = Stroke::new(1.0, HAIRLINE);
        v.window_corner_radius = CornerRadius::same(8);
        v.menu_corner_radius = CornerRadius::same(8);
        v.override_text_color = None;
        v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.hyperlink_color = ACCENT;
        v.text_cursor.stroke = Stroke::new(1.5, TEXT);

        let w = &mut v.widgets;
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.bg_stroke = Stroke::new(1.0, HAIRLINE);
        for (state, fill, text) in [
            (&mut w.inactive, RAISED, TEXT),
            (&mut w.hovered, Color32::from_rgb(0x24, 0x27, 0x2C), TEXT),
            (&mut w.active, Color32::from_rgb(0x2C, 0x30, 0x36), TEXT),
            (&mut w.open, Color32::from_rgb(0x24, 0x27, 0x2C), TEXT),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::NONE;
            state.fg_stroke = Stroke::new(1.0, text);
            state.corner_radius = CornerRadius::same(6);
            state.expansion = 0.0;
        }
        w.hovered.bg_stroke = Stroke::new(1.0, HAIRLINE);
    });
}

pub fn state_color(s: PowerState) -> Color32 {
    match s {
        PowerState::On | PowerState::Transition(_) => TEXT,
        _ => TEXT_2,
    }
}

/// One colour for the whole set, used by the tray icon.
pub fn overall_tone(snap: &Snapshot) -> Tone {
    let managed: Vec<_> = snap.stations.iter().filter(|s| s.enabled).collect();
    if managed.iter().any(|s| s.error.is_some()) {
        return Tone::Error;
    }
    if snap.activity.is_some() {
        return Tone::Partial;
    }
    let on = managed.iter().filter(|s| s.state.is_powered()).count();
    match on {
        0 => Tone::Off,
        n if n == managed.len() => Tone::On,
        _ => Tone::Partial,
    }
}
