//! Small custom controls: switch, two-way segmented control, quiet text button, "more" button.

use eframe::egui::{self, Color32, CornerRadius, Response, Sense, Stroke, StrokeKind, Ui, WidgetInfo, Role};

use super::{ACCENT, HAIRLINE, RAISED, TEXT, TEXT_2, TEXT_3, TRACK_OFF};

fn focus_ring(ui: &Ui, resp: &Response, rect: egui::Rect, radius: u8) {
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(3.0),
            CornerRadius::same(radius + 3),
            Stroke::new(1.5, ACCENT),
            StrokeKind::Outside,
        );
    }
}

/// An on/off switch. Returns the response; `clicked()` means the user wants the opposite of `on`.
pub fn switch(ui: &mut Ui, on: bool, enabled: bool, label: &str) -> Response {
    let size = egui::vec2(36.0, 20.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    resp.widget_info(|| WidgetInfo::selected(Role::Switch, enabled, on, label));
    if resp.hovered() && enabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(resp.id, on);
        let mut track = lerp_color(TRACK_OFF, ACCENT, t);
        let mut knob = lerp_color(Color32::from_rgb(0xC9, 0xCD, 0xD3), Color32::from_rgb(0x0F, 0x1A, 0x14), t);
        if resp.hovered() && enabled {
            track = track.gamma_multiply(1.12);
        }
        if !enabled {
            track = track.gamma_multiply(0.45);
            knob = knob.gamma_multiply(0.6);
        }
        let radius = rect.height() / 2.0;
        ui.painter().rect_filled(rect, CornerRadius::same(radius as u8), track);
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        ui.painter().circle_filled(egui::pos2(x, rect.center().y), radius - 3.0, knob);
        focus_ring(ui, &resp, rect, radius as u8);
    }
    resp
}

/// Two options side by side, the selected one raised. Returns the newly picked index, if any.
pub fn segmented(ui: &mut Ui, options: [&str; 2], selected: usize) -> Option<usize> {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galleys: Vec<_> =
        options.iter().map(|o| ui.painter().layout_no_wrap(o.to_string(), font.clone(), TEXT)).collect();
    let seg_w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 24.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(seg_w * 2.0 + 4.0, 28.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(7), Color32::TRANSPARENT, Stroke::new(1.0, HAIRLINE), StrokeKind::Inside);
    let mut picked = None;
    for (i, galley) in galleys.into_iter().enumerate() {
        let seg = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 2.0 + seg_w * i as f32, rect.top() + 2.0),
            egui::vec2(seg_w, rect.height() - 4.0),
        );
        let resp = ui.interact(seg, ui.scope_id().with(("seg", options[i])), Sense::click());
        resp.widget_info(|| WidgetInfo::selected(Role::RadioButton, true, i == selected, options[i]));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let color = if i == selected {
            ui.painter().rect_filled(seg, CornerRadius::same(5), Color32::from_rgb(0x2C, 0x30, 0x36));
            TEXT
        } else if resp.hovered() {
            TEXT
        } else {
            TEXT_2
        };
        let pos = seg.center() - galley.size() / 2.0;
        ui.painter().galley_with_override_text_color(pos, galley, color);
        focus_ring(ui, &resp, seg, 5);
        if resp.clicked() && i != selected {
            picked = Some(i);
        }
    }
    picked
}

/// A frameless text button that brightens on hover.
pub fn text_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let color = if enabled { TEXT_2 } else { TEXT_3 };
    let resp = ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(text).color(color)).frame(false));
    if resp.hovered() && enabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        // Repaint the label in the primary colour on hover.
        ui.painter().rect_filled(resp.rect.expand2(egui::vec2(6.0, 2.0)), CornerRadius::same(5), RAISED);
        let font = egui::TextStyle::Button.resolve(ui.style());
        ui.painter().text(resp.rect.center(), egui::Align2::CENTER_CENTER, text, font, TEXT);
    }
    resp
}

/// Three dots that open a menu. Muted until the row or the button is hovered.
pub fn more_button(ui: &mut Ui, row_hovered: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, "More"));
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter().rect_filled(rect, CornerRadius::same(6), RAISED);
    }
    let color = if resp.hovered() || resp.is_pointer_button_down_on() {
        TEXT
    } else if row_hovered {
        TEXT_2
    } else {
        TEXT_3
    };
    for dx in [-5.0, 0.0, 5.0] {
        ui.painter().circle_filled(rect.center() + egui::vec2(dx, 0.0), 1.6, color);
    }
    focus_ring(ui, &resp, rect, 6);
    resp
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}
