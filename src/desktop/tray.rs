//! Tray icon on its own thread with its own message loop, so it keeps working while the window
//! is hidden and eframe is idle.

use std::sync::Arc;
use std::thread;

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, TranslateMessage, WM_USER,
};

use crate::app::{AppCtl, WM_TRAY_REFRESH};
use crate::engine::{Command, Target};
use crate::ui::{self, icons::Tone};

const ICON_SIZE: u32 = 32;

fn icon(tone: Tone) -> Option<Icon> {
    Icon::from_rgba(ui::icons::rgba(tone, ICON_SIZE), ICON_SIZE, ICON_SIZE).ok()
}

pub fn spawn(ctl: Arc<AppCtl>) {
    thread::Builder::new()
        .name("tray".into())
        .spawn(move || {
            if let Err(e) = run(&ctl) {
                tracing::error!("tray icon failed: {e:#}");
            }
            ctl.set_tray_thread(0);
        })
        .expect("spawn tray thread");
}

fn run(ctl: &Arc<AppCtl>) -> anyhow::Result<()> {
    unsafe {
        // Create this thread's message queue before anyone posts to it.
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
    }

    let open = MenuItem::new("Open Basewake", true, None);
    let on = MenuItem::new("Turn all on", true, None);
    let off = MenuItem::new("Turn all off", true, None);
    let scan = MenuItem::new("Scan for base stations", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::new();
    menu.append_items(&[
        &open,
        &PredefinedMenuItem::separator(),
        &on,
        &off,
        &scan,
        &PredefinedMenuItem::separator(),
        &quit,
    ])?;

    let (tone, tip) = describe(ctl);
    let tray: TrayIcon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip(&tip)
        .with_icon(icon(tone).ok_or_else(|| anyhow::anyhow!("bad icon"))?)
        .build()?;

    {
        let c = ctl.clone();
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                c.show_window();
            }
        }));
    }
    {
        let c = ctl.clone();
        let ids = (open.id().clone(), on.id().clone(), off.id().clone(), scan.id().clone(), quit.id().clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let id = &e.id;
            if *id == ids.0 {
                c.show_window();
            } else if *id == ids.1 {
                c.handle.send(Command::PowerAll(Target::On));
            } else if *id == ids.2 {
                c.handle.send(Command::PowerAll(Target::Off));
            } else if *id == ids.3 {
                c.handle.send(Command::Scan);
            } else if *id == ids.4 {
                c.quit();
            }
        }));
    }

    ctl.set_tray_thread(unsafe { GetCurrentThreadId() });
    let mut last = (tone, tip);
    unsafe {
        let mut msg = MSG::default();
        // GetMessage returns 0 on WM_QUIT and -1 on error.
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            if msg.hwnd.is_invalid() && msg.message == WM_TRAY_REFRESH {
                let now = describe(ctl);
                if now != last {
                    if now.0 != last.0 {
                        let _ = tray.set_icon(icon(now.0));
                    }
                    let _ = tray.set_tooltip(Some(&now.1));
                    last = now;
                }
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
    MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
    drop(tray); // removes the icon from the taskbar
    Ok(())
}

/// Icon tone + tooltip for the current state.
fn describe(ctl: &AppCtl) -> (Tone, String) {
    let snap = ctl.handle.snapshot();
    let (on, total) = snap.powered_count();
    let tone = ui::overall_tone(&snap);
    let mut tip = if total == 0 {
        "Basewake · no base stations".to_string()
    } else {
        format!("Basewake · {on} of {total} on")
    };
    if let Some(a) = &snap.activity {
        tip = format!("{tip}\n{a}");
    }
    if ctl.vr_status().connected {
        tip.push_str("\nSteamVR running");
    }
    (tone, tip)
}
