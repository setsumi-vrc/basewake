//! Desktop side: the window and the tray icon.

pub mod tray;
pub mod window;

use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::HSTRING;

/// Shows an error dialog (the GUI exe has no console to print to).
pub fn fatal(text: &str) {
    unsafe {
        MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("Basewake"), MB_OK | MB_ICONERROR);
    }
}
