//! Global push-to-talk key via a low-level keyboard hook (RegisterHotKey reports no key-up).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, SetWindowsHookExW, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_SYSKEYDOWN,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Press,
    Release,
    /// Another key went down while the hotkey was held (e.g. RightCtrl+C).
    Other,
    Escape,
}

const VK_ESCAPE: u32 = 0x1B;

/// Returns the virtual-key code and whether the key is a modifier.
pub fn parse(name: &str) -> Option<(u32, bool)> {
    let name = name.trim().to_ascii_lowercase();
    let key = match name.as_str() {
        "rightctrl" => (0xA3, true),
        "rightshift" => (0xA1, true),
        "capslock" => (0x14, false),
        "scrolllock" => (0x91, false),
        "pause" => (0x13, false),
        "insert" => (0x2D, false),
        _ => {
            let n: u32 = name.strip_prefix('f')?.parse().ok()?;
            if !(1..=24).contains(&n) {
                return None;
            }
            (0x6F + n, false)
        }
    };
    Some(key)
}

/// Pure key filter; kept separate from the hook so it can be tested.
pub struct Filter {
    vk: u32,
    modifier: bool,
    held: bool,
}

impl Filter {
    pub fn new(vk: u32, modifier: bool) -> Self {
        Self {
            vk,
            modifier,
            held: false,
        }
    }

    /// Returns the event to report and whether the key should be swallowed.
    pub fn on_key(&mut self, vk: u32, down: bool, recording: bool) -> (Option<Key>, bool) {
        if vk == self.vk {
            // Modifiers pass through so shortcuts like RightCtrl+C keep working.
            let swallow = !self.modifier;
            let event = match (down, self.held) {
                (true, false) => Some(Key::Press),
                (false, true) => Some(Key::Release),
                _ => None, // auto-repeat or stray key-up
            };
            self.held = down;
            return (event, swallow);
        }
        if down && vk == VK_ESCAPE && recording {
            return (Some(Key::Escape), true);
        }
        if down && self.held {
            return (Some(Key::Other), false);
        }
        (None, false)
    }
}

static FILTER: Mutex<Option<Filter>> = Mutex::new(None);
static EVENTS: OnceLock<Sender<Key>> = OnceLock::new();
/// Set by the controller; Escape is only swallowed while recording.
pub static RECORDING: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, lparam points to a KBDLLHOOKSTRUCT.
        let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        // Ignore our own SendInput (Ctrl+V or typed text).
        if info.flags & LLKHF_INJECTED == 0 {
            let down = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            let recording = RECORDING.load(Ordering::Relaxed);
            let (event, swallow) = match FILTER.lock().ok().as_mut().and_then(|f| f.as_mut()) {
                Some(filter) => filter.on_key(info.vkCode, down, recording),
                None => (None, false),
            };
            if let (Some(event), Some(tx)) = (event, EVENTS.get()) {
                let _ = tx.send(event);
            }
            if swallow {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// Installs the hook on the calling thread, which must then run a message loop.
pub fn install(vk: u32, modifier: bool, events: Sender<Key>) -> Result<(), String> {
    *FILTER.lock().map_err(|_| "hotkey state poisoned")? = Some(Filter::new(vk, modifier));
    EVENTS
        .set(events)
        .map_err(|_| "hotkey already installed".to_string())?;
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(hook_proc),
            GetModuleHandleW(std::ptr::null()),
            0,
        )
    };
    if hook.is_null() {
        return Err(format!(
            "Tastatur-Hook konnte nicht installiert werden ({})",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_keys_only() {
        assert_eq!(parse("RightCtrl"), Some((0xA3, true)));
        assert_eq!(parse("f9"), Some((0x78, false)));
        assert_eq!(parse("F24"), Some((0x87, false)));
        assert_eq!(parse("F25"), None);
        assert_eq!(parse("A"), None);
    }

    #[test]
    fn ignores_auto_repeat_and_reports_combos() {
        let mut f = Filter::new(0xA3, true);
        assert_eq!(f.on_key(0xA3, true, false), (Some(Key::Press), false));
        assert_eq!(f.on_key(0xA3, true, true), (None, false));
        assert_eq!(f.on_key(0x43, true, true), (Some(Key::Other), false));
        assert_eq!(f.on_key(0xA3, false, true), (Some(Key::Release), false));
        assert_eq!(f.on_key(0xA3, false, false), (None, false));
        assert_eq!(f.on_key(0x43, true, false), (None, false));
    }

    #[test]
    fn swallows_plain_hotkey_and_escape_while_recording() {
        let mut f = Filter::new(0x14, false);
        assert_eq!(f.on_key(0x14, true, false), (Some(Key::Press), true));
        assert_eq!(f.on_key(0x14, false, true), (Some(Key::Release), true));
        assert_eq!(f.on_key(VK_ESCAPE, true, true), (Some(Key::Escape), true));
        assert_eq!(f.on_key(VK_ESCAPE, true, false), (None, false));
    }
}
