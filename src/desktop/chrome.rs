//! Window chrome drawn in egui: the title bar (name, version, links, minimise, close) and the
//! rows used by the tray popup.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Rect, Response, Sense, Stroke, TextureHandle, Ui, ViewportCommand, pos2,
    vec2,
};

use crate::ui::{self, ACCENT, HAIRLINE, RAISED, TEXT, TEXT_2, TEXT_3, icons};

pub const TITLE_H: f32 = 44.0;
pub const REPO_URL: &str = "https://github.com/setsumi-vrc/basewake";
pub const X_URL: &str = "https://x.com/setsumi_vrc";
pub const ISSUES_URL: &str = "https://github.com/setsumi-vrc/basewake/issues/new";

/// Windows' own red for a hovered close button.
const CLOSE_HOVER: Color32 = Color32::from_rgb(0xC4, 0x2B, 0x1C);

/// Textures for the title bar: the app mark and the GitHub and X marks.
pub struct Marks {
    app: TextureHandle,
    links: [TextureHandle; 2],
    /// The base-station glyph in each state colour, for the tray popup.
    states: [TextureHandle; 4],
}

impl Marks {
    pub fn new(ctx: &egui::Context) -> Self {
        let glyph = |name: &str, tone| {
            let img = egui::ColorImage::from_rgba_unmultiplied([64, 64], &icons::tray_rgba(tone, 64));
            ctx.load_texture(name, img, smooth())
        };
        use icons::Tone;
        Self {
            app: glyph("mark-app", Tone::On),
            states: [
                glyph("state-on", Tone::On),
                glyph("state-partial", Tone::Partial),
                glyph("state-off", Tone::Off),
                glyph("state-error", Tone::Error),
            ],
            links: [
                mask_texture(ctx, "mark-github", include_bytes!("../ui/marks/github.a8")),
                mask_texture(ctx, "mark-x", include_bytes!("../ui/marks/x.a8")),
            ],
        }
    }
}

impl Marks {
    pub fn state(&self, tone: icons::Tone) -> &TextureHandle {
        use icons::Tone;
        &self.states[match tone {
            Tone::On => 0,
            Tone::Partial => 1,
            Tone::Off => 2,
            Tone::Error => 3,
        }]
    }
}

fn smooth() -> egui::TextureOptions {
    egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR }
}

/// A 64×64 alpha mask as a white texture to tint.
fn mask_texture(ctx: &egui::Context, name: &str, alpha: &[u8]) -> TextureHandle {
    let rgba: Vec<u8> = alpha.iter().flat_map(|&a| [255, 255, 255, a]).collect();
    ctx.load_texture(name, egui::ColorImage::from_rgba_unmultiplied([64, 64], &rgba), smooth())
}

pub enum TitleAction {
    None,
    /// Minimise and close both put the window away in the tray.
    Hide,
}

#[derive(Clone, Copy)]
enum Glyph {
    Minimize,
    Close,
    Bug,
}

/// The title bar: drag anywhere on it to move the window.
pub fn title_bar(ui: &mut Ui, marks: &Marks) -> TitleAction {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), TITLE_H), Sense::click_and_drag());
    if resp.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    let p = ui.painter();
    let mark = Rect::from_center_size(pos2(rect.left() + 26.0, rect.center().y), vec2(18.0, 18.0));
    p.image(marks.app.id(), mark, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    let name = p.text(
        pos2(rect.left() + 44.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Basewake",
        FontId::new(14.0, ui::semibold()),
        TEXT_2,
    );
    p.text(
        pos2(name.right() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        concat!("v", env!("CARGO_PKG_VERSION")),
        FontId::proportional(12.5),
        TEXT_3,
    );

    let mut action = TitleAction::None;
    let mut x = rect.right();
    let mut slot = |w: f32| {
        let r = Rect::from_min_size(pos2(x - w, rect.top()), vec2(w, TITLE_H));
        x -= w;
        r
    };
    if title_button(ui, slot(46.0), Glyph::Close, "Close to tray").clicked() {
        action = TitleAction::Hide;
    }
    if title_button(ui, slot(46.0), Glyph::Minimize, "Minimize to tray").clicked() {
        action = TitleAction::Hide;
    }
    slot(6.0);
    if title_button(ui, slot(36.0), Glyph::Bug, "Report a problem").clicked() {
        open_in_browser(ISSUES_URL);
    }
    for (tex, url) in marks.links.iter().zip([REPO_URL, X_URL]).rev() {
        link_button(ui, slot(36.0), tex, url);
    }
    action
}

/// Title-bar link: a tinted mark, the address as tooltip, opens the browser.
fn link_button(ui: &mut Ui, rect: Rect, tex: &TextureHandle, url: &str) {
    let resp = ui
        .interact(rect, ui.scope_id().with(url), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(url);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, url));
    if resp.clicked() {
        open_in_browser(url);
    }
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::ZERO, RAISED);
    }
    let tint = if resp.hovered() { TEXT } else { TEXT_2 };
    let icon = Rect::from_center_size(rect.center(), vec2(16.0, 16.0));
    p.image(tex.id(), icon, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
    focus(ui, &resp, rect);
}

fn title_button(ui: &mut Ui, rect: Rect, glyph: Glyph, tip: &str) -> Response {
    let resp = ui.interact(rect, ui.scope_id().with(tip), Sense::click()).on_hover_text(tip);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    let p = ui.painter();
    if resp.hovered() {
        let fill = if matches!(glyph, Glyph::Close) { CLOSE_HOVER } else { RAISED };
        p.rect_filled(rect, CornerRadius::ZERO, fill);
    }
    let color = if resp.hovered() { TEXT } else { TEXT_2 };
    let s = Stroke::new(1.3, color);
    let c = rect.center();
    match glyph {
        Glyph::Minimize => {
            p.line_segment([pos2(c.x - 5.0, c.y), pos2(c.x + 5.0, c.y)], s);
        }
        Glyph::Close => {
            p.line_segment([pos2(c.x - 5.0, c.y - 5.0), pos2(c.x + 5.0, c.y + 5.0)], s);
            p.line_segment([pos2(c.x + 5.0, c.y - 5.0), pos2(c.x - 5.0, c.y + 5.0)], s);
        }
        Glyph::Bug => {
            // Body, head, three legs a side and two antennae.
            let body = Rect::from_center_size(pos2(c.x, c.y + 1.5), vec2(8.0, 10.0));
            p.rect_stroke(body, CornerRadius::same(4), s, egui::StrokeKind::Middle);
            p.line_segment([pos2(c.x, c.y - 1.0), pos2(c.x, c.y + 6.5)], s);
            p.line_segment([pos2(c.x - 2.5, c.y - 3.5), pos2(c.x + 2.5, c.y - 3.5)], s);
            for dy in [-1.0, 2.0, 5.0] {
                p.line_segment([pos2(c.x - 4.0, c.y + dy), pos2(c.x - 7.0, c.y + dy - 1.5)], s);
                p.line_segment([pos2(c.x + 4.0, c.y + dy), pos2(c.x + 7.0, c.y + dy - 1.5)], s);
            }
            p.line_segment([pos2(c.x - 1.5, c.y - 4.0), pos2(c.x - 3.5, c.y - 7.0)], s);
            p.line_segment([pos2(c.x + 1.5, c.y - 4.0), pos2(c.x + 3.5, c.y - 7.0)], s);
        }
    }
    focus(ui, &resp, rect);
    resp
}

fn focus(ui: &Ui, resp: &Response, rect: Rect) {
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(2.0),
            CornerRadius::same(4),
            Stroke::new(1.5, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
}

/// Opens a link in the default browser, or a folder in the file manager.
pub fn open_in_browser(url: &str) {
    #[cfg(windows)]
    let cmd = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let cmd = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = cmd {
        tracing::warn!("could not open {url}: {e}");
    }
}

/// A full-width hairline.
pub fn separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, HAIRLINE);
}

/// Menu row: full width, text on the left, highlight on hover.
pub fn menu_item(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), sense);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, text));
    let p = ui.painter();
    if enabled && (resp.hovered() || resp.has_focus()) {
        p.rect_filled(rect, CornerRadius::same(6), RAISED);
    }
    p.text(
        pos2(rect.left() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(13.5),
        if enabled { TEXT } else { TEXT_3 },
    );
    resp
}

/// Turns off the DWM open/close animation of one of this process's windows, found by its title,
/// so the tray popup appears at once instead of fading in.
#[cfg(windows)]
pub fn disable_open_animation(title: &str) {
    use windows::Win32::Graphics::Dwm::{DWMWA_TRANSITIONS_FORCEDISABLED, DwmSetWindowAttribute};
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
    use windows::core::HSTRING;
    unsafe {
        if let Ok(hwnd) = FindWindowW(None, &HSTRING::from(title)) {
            let on: i32 = 1;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_TRANSITIONS_FORCEDISABLED,
                (&on as *const i32).cast(),
                std::mem::size_of::<i32>() as u32,
            );
        }
    }
}

#[cfg(not(windows))]
pub fn disable_open_animation(_title: &str) {}
