use crate::settings::{Hotkey, PadId};
use std::sync::{Arc, Mutex};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyNameTextW, MAPVK_VK_TO_VSC, MOD_NOREPEAT, MapVirtualKeyW, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP, WM_HOTKEY};

pub const MOD_ALT: u32 = 1;
pub const MOD_CONTROL: u32 = 2;
pub const MOD_SHIFT: u32 = 4;
pub const MOD_WIN: u32 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HotkeyAction {
    Pad(PadId),
    Pause,
    Skip,
    Stop,
}

/// System-wide hotkeys via RegisterHotKey on a thread with its own message loop, so they keep
/// working while the window is hidden or a game has focus.
pub struct HotkeyService {
    thread_id: u32,
    bindings: Arc<Mutex<Vec<(HotkeyAction, Hotkey)>>>,
    failed: Arc<Mutex<Vec<HotkeyAction>>>,
}

impl HotkeyService {
    pub fn start(on_press: impl Fn(HotkeyAction) + Send + 'static) -> HotkeyService {
        let bindings: Arc<Mutex<Vec<(HotkeyAction, Hotkey)>>> = Arc::default();
        let failed: Arc<Mutex<Vec<HotkeyAction>>> = Arc::default();
        let (b, f) = (bindings.clone(), failed.clone());
        let (tid_tx, tid_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("sp-hotkeys".into())
            .spawn(move || {
                let mut msg: MSG = unsafe { std::mem::zeroed() };
                unsafe {
                    PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE); // creates this thread's queue
                    let _ = tid_tx.send(GetCurrentThreadId());
                }
                let mut registered: Vec<HotkeyAction> = Vec::new(); // hotkey id = index + 1
                while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
                    match msg.message {
                        WM_HOTKEY => {
                            if let Some(a) = (msg.wParam as usize).checked_sub(1).and_then(|i| registered.get(i)) {
                                on_press(*a);
                            }
                        }
                        WM_APP => {
                            for id in 1..=registered.len() {
                                unsafe { UnregisterHotKey(std::ptr::null_mut(), id as i32) };
                            }
                            registered.clear();
                            let mut bad = Vec::new();
                            for (a, h) in b.lock().unwrap().iter() {
                                registered.push(*a);
                                let ok = unsafe { RegisterHotKey(std::ptr::null_mut(), registered.len() as i32, h.mods | MOD_NOREPEAT, h.vk) };
                                if ok == 0 {
                                    bad.push(*a);
                                }
                            }
                            *f.lock().unwrap() = bad;
                        }
                        _ => {}
                    }
                }
            })
            .expect("spawn hotkey thread");
        let thread_id = tid_rx.recv().unwrap_or(0);
        HotkeyService { thread_id, bindings, failed }
    }

    pub fn set_bindings(&self, b: Vec<(HotkeyAction, Hotkey)>) {
        *self.bindings.lock().unwrap() = b;
        unsafe { PostThreadMessageW(self.thread_id, WM_APP, 0, 0) };
    }

    /// Bindings Windows refused, usually because another app owns that key.
    pub fn failed(&self) -> Vec<HotkeyAction> {
        self.failed.lock().unwrap().clone()
    }
}

/// "Ctrl+Num 1", "F13".
pub fn key_name(h: Hotkey) -> String {
    let mut s = String::new();
    for (bit, name) in [(MOD_CONTROL, "Ctrl+"), (MOD_ALT, "Alt+"), (MOD_SHIFT, "Shift+"), (MOD_WIN, "Win+")] {
        if h.mods & bit != 0 {
            s.push_str(name);
        }
    }
    if (0x70..=0x87).contains(&h.vk) {
        // Layouts often have no name for F13–F24.
        s.push_str(&format!("F{}", h.vk - 0x6F));
        return s;
    }
    let scan = unsafe { MapVirtualKeyW(h.vk, MAPVK_VK_TO_VSC) };
    // Navigation keys, Num /, and Num Lock live on the extended scan-code page.
    let extended = matches!(h.vk, 0x21..=0x28 | 0x2D | 0x2E | 0x6F | 0x90);
    let lparam = ((scan << 16) | if extended { 1 << 24 } else { 0 }) as i32;
    let mut buf = [0u16; 64];
    let n = unsafe { GetKeyNameTextW(lparam, buf.as_mut_ptr(), buf.len() as i32) };
    if n > 0 {
        s.push_str(&String::from_utf16_lossy(&buf[..n as usize]));
    } else {
        s.push_str(&format!("Key 0x{:02X}", h.vk));
    }
    s
}

pub fn keyboard_snapshot() -> [bool; 256] {
    let mut s = [false; 256];
    for (vk, down) in s.iter_mut().enumerate() {
        *down = unsafe { GetAsyncKeyState(vk as i32) } < 0;
    }
    s
}

pub enum Captured {
    Key(Hotkey),
    Cancel,
}

/// First key that's down now but wasn't in `baseline`, with the modifiers held at that moment.
pub fn poll_capture(baseline: &[bool; 256]) -> Option<Captured> {
    let down = |vk: u32| unsafe { GetAsyncKeyState(vk as i32) } < 0;
    for vk in 0x08u32..=0xFE {
        let modifier = matches!(vk, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C);
        if modifier || baseline[vk as usize] || !down(vk) {
            continue;
        }
        if vk == 0x1B {
            return Some(Captured::Cancel);
        }
        let mods = [(0x11, MOD_CONTROL), (0x12, MOD_ALT), (0x10, MOD_SHIFT), (0x5B, MOD_WIN), (0x5C, MOD_WIN)]
            .iter()
            .filter(|(k, _)| down(*k))
            .fold(0, |m, (_, b)| m | b);
        return Some(Captured::Key(Hotkey { mods, vk }));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names() {
        assert_eq!(key_name(Hotkey { mods: 0, vk: 0x7C }), "F13");
        assert_eq!(key_name(Hotkey { mods: 2 | 4, vk: 0x70 }), "Ctrl+Shift+F1");
    }

    #[test]
    fn registers_and_reports_failures() {
        let svc = HotkeyService::start(|_| {});
        let free = Hotkey { mods: 1 | 2 | 4, vk: 0x87 }; // Ctrl+Alt+Shift+F24
        svc.set_bindings(vec![(HotkeyAction::Skip, free), (HotkeyAction::Stop, free)]); // duplicate → second refused
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(svc.failed(), vec![HotkeyAction::Stop]);
        svc.set_bindings(vec![]);
    }
}
