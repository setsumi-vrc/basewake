//! Tray icon: its colour shows the state at a glance; left click shows or hides the window.
//!
//! Right click opens the app's own popup on Windows and macOS. Linux tray hosts
//! (StatusNotifierItem) don't report where a click happened and Wayland doesn't let apps place
//! windows, so there it opens a native menu with the same items instead.

use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui;
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::engine::Snapshot;
use crate::ui::{self, icons::Tone};

const ICON_SIZE: u32 = 32;

pub enum TrayAction {
    ToggleWindow,
    /// Physical screen position of the click.
    OpenPopup { x: f64, y: f64 },
    /// From the Linux tray menu.
    #[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
    ShowWindow,
    #[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
    AllOn,
    #[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
    AllOff,
    #[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
    Scan,
    #[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    pub events: Receiver<TrayAction>,
    shown: Option<(Tone, String)>,
    #[cfg(not(any(windows, target_os = "macos")))]
    _menu: linux::Menu,
}

fn icon(tone: Tone) -> Option<Icon> {
    Icon::from_rgba(ui::icons::tray_rgba(tone, ICON_SIZE), ICON_SIZE, ICON_SIZE).ok()
}

impl Tray {
    /// Must be created on the main thread (the one running the window).
    pub fn new(ctx: &egui::Context) -> Option<Self> {
        let (tx, rx) = channel();
        let send = {
            let ctx = ctx.clone();
            move |tx: &Sender<TrayAction>, a: TrayAction| {
                let _ = tx.send(a);
                ctx.request_repaint();
            }
        };

        {
            let tx = tx.clone();
            let send = send.clone();
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                let action = match e {
                    TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => {
                        Some(TrayAction::ToggleWindow)
                    }
                    TrayIconEvent::Click {
                        button: MouseButton::Right,
                        button_state: MouseButtonState::Up,
                        position,
                        ..
                    } => Some(TrayAction::OpenPopup { x: position.x, y: position.y }),
                    _ => None,
                };
                if let Some(a) = action {
                    send(&tx, a);
                }
            }));
        }

        let builder = TrayIconBuilder::new()
            .with_tooltip("Basewake")
            .with_icon(icon(Tone::Off)?)
            .with_menu_on_left_click(false);

        #[cfg(any(windows, target_os = "macos"))]
        {
            let _ = (&send, &tx);
            let icon = match builder.with_menu_on_right_click(false).build() {
                Ok(i) => i,
                Err(e) => {
                    tracing::error!("tray icon: {e}");
                    return None;
                }
            };
            Some(Self { icon, events: rx, shown: None })
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let menu = linux::Menu::new(move |a| send(&tx, a));
            let icon = match builder.with_menu(Box::new(menu.menu.clone())).build() {
                Ok(i) => i,
                Err(e) => {
                    tracing::error!("tray icon: {e}");
                    return None;
                }
            };
            Some(Self { icon, events: rx, shown: None, _menu: menu })
        }
    }

    /// Updates the icon and tooltip, only when they change.
    pub fn update(&mut self, snap: &Snapshot, steamvr: bool) {
        let now = describe(snap, steamvr);
        if self.shown.as_ref() == Some(&now) {
            return;
        }
        if self.shown.as_ref().map(|s| s.0) != Some(now.0) {
            let _ = self.icon.set_icon(icon(now.0));
        }
        let _ = self.icon.set_tooltip(Some(&now.1));
        self.shown = Some(now);
    }
}

/// Icon tone + tooltip for the current state.
fn describe(snap: &Snapshot, steamvr: bool) -> (Tone, String) {
    let (on, total) = snap.powered_count();
    let tone = ui::overall_tone(snap);
    let mut tip = if total == 0 {
        "Basewake · no base stations".to_string()
    } else {
        format!("Basewake · {on} of {total} on")
    };
    if let Some(a) = &snap.activity {
        tip = format!("{tip}\n{a}");
    }
    if steamvr {
        tip.push_str("\nSteamVR running");
    }
    (tone, tip)
}

#[cfg(not(any(windows, target_os = "macos")))]
mod linux {
    use tray_icon::menu::{Menu as NativeMenu, MenuEvent, MenuItem, PredefinedMenuItem};

    use super::TrayAction;

    pub struct Menu {
        pub menu: NativeMenu,
    }

    impl Menu {
        pub fn new(send: impl Fn(TrayAction) + Send + Sync + 'static) -> Self {
            let open = MenuItem::new("Open window", true, None);
            let on = MenuItem::new("Turn all on", true, None);
            let off = MenuItem::new("Turn all off", true, None);
            let scan = MenuItem::new("Scan for base stations", true, None);
            let quit = MenuItem::new("Quit Basewake", true, None);
            let menu = NativeMenu::new();
            let _ = menu.append_items(&[
                &open,
                &PredefinedMenuItem::separator(),
                &on,
                &off,
                &scan,
                &PredefinedMenuItem::separator(),
                &quit,
            ]);
            let ids = [open.id().clone(), on.id().clone(), off.id().clone(), scan.id().clone(), quit.id().clone()];
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                let a = match ids.iter().position(|id| *id == e.id) {
                    Some(0) => TrayAction::ShowWindow,
                    Some(1) => TrayAction::AllOn,
                    Some(2) => TrayAction::AllOff,
                    Some(3) => TrayAction::Scan,
                    Some(4) => TrayAction::Quit,
                    _ => return,
                };
                send(a);
            }));
            Self { menu }
        }
    }
}
