//! Desktop side: the window with its title bar, the tray icon and the tray popup.

pub mod chrome;
pub mod tray;
pub mod window;

/// Shows an error dialog (the GUI exe has no console to print to).
#[cfg(windows)]
pub fn fatal(text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::HSTRING;
    unsafe {
        MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("Basewake"), MB_OK | MB_ICONERROR);
    }
}

/// Prints the error (Linux and macOS apps started from a terminal show it; it is also logged).
#[cfg(not(windows))]
pub fn fatal(text: &str) {
    eprintln!("Basewake: {text}");
}
