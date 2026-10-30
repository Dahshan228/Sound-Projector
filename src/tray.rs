use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, GetLastError};
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow};

pub const WINDOW_TITLE: &str = "Sound Projector";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "SoundProjector";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn find_window() -> Option<isize> {
    let title = wide(WINDOW_TITLE);
    let h = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    (!h.is_null()).then_some(h as isize)
}

pub fn show_window(hwnd: isize) {
    let h = hwnd as *mut core::ffi::c_void;
    unsafe {
        ShowWindow(h, SW_SHOW);
        ShowWindow(h, SW_RESTORE);
        SetForegroundWindow(h);
    }
}

/// A named mutex lives as long as this process; a second copy sees it already exists.
pub fn already_running() -> bool {
    let name = wide("SoundProjector.SingleInstance");
    unsafe {
        CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        GetLastError() == ERROR_ALREADY_EXISTS
    }
}

pub fn set_autostart(on: bool) -> Result<(), String> {
    let (key, name) = (wide(RUN_KEY), wide(RUN_VALUE));
    let err = if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let data = wide(&format!("\"{}\" --tray", exe.display()));
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32) }
    } else {
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => 0,
            e => e,
        }
    };
    if err == 0 { Ok(()) } else { Err(format!("Windows error {err}")) }
}

/// Tray icon with Show/Quit. Left-click or Show sets `show` and wakes the app (eframe still runs
/// `App::logic` while the window is hidden); the Win32 show is a fallback.
pub fn create_tray(ctx: egui::Context, show: Arc<AtomicBool>) -> Option<TrayIcon> {
    let menu = Menu::new();
    let show_item = MenuItem::new("Show Sound Projector", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    menu.append_items(&[&show_item, &quit_item]).ok()?;
    let (show_id, quit_id) = (show_item.id().clone(), quit_item.id().clone());

    let reveal = move || {
        show.store(true, Ordering::Relaxed);
        if let Some(h) = find_window() {
            show_window(h);
        }
        ctx.request_repaint();
    };
    let reveal2 = reveal.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if e.id == quit_id {
            // ponytail: settings are saved on every change, so a hard exit loses nothing but eframe's last window-geometry autosave
            std::process::exit(0);
        } else if e.id == show_id {
            reveal();
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            reveal2();
        }
    }));

    TrayIconBuilder::new()
        .with_icon(Icon::from_rgba(crate::icon::icon_rgba(32), 32, 32).ok()?)
        .with_tooltip(WINDOW_TITLE)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // writes the real HKCU Run key: cargo test autostart -- --ignored
    fn autostart_round_trip() {
        let query = || std::process::Command::new("reg").args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", RUN_VALUE]).output().unwrap();
        set_autostart(true).unwrap();
        let out = String::from_utf8_lossy(&query().stdout).into_owned();
        assert!(out.contains("--tray"), "{out}");
        set_autostart(false).unwrap();
        assert!(!query().status.success());
        set_autostart(false).unwrap(); // already gone is fine
    }
}
