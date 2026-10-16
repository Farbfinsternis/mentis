//! Text output into the focused window: clipboard + Ctrl+V, or typed Unicode input.

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{GlobalFree, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, SendInput, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetForegroundWindow, HWND_MESSAGE,
};

const CF_UNICODETEXT: u32 = 13;
/// Upper bound for clipboard content we back up; larger content switches to typing.
const MAX_BACKUP_BYTES: usize = 64 * 1024 * 1024;
/// Time for the target app to read the clipboard before it is restored.
const PASTE_SETTLE: Duration = Duration::from_millis(500);

/// Clipboard owner window; created on the thread that pumps messages.
static OWNER: AtomicIsize = AtomicIsize::new(0);

pub fn create_clipboard_owner() -> Result<(), String> {
    let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err(format!(
            "Hilfsfenster fehlgeschlagen ({})",
            std::io::Error::last_os_error()
        ));
    }
    OWNER.store(hwnd as isize, Ordering::Relaxed);
    Ok(())
}

pub fn foreground() -> isize {
    unsafe { GetForegroundWindow() as isize }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Pasted,
    /// Clipboard was in use by the user afterwards, so it was not restored.
    PastedNoRestore,
    Typed,
    /// Focus moved to another window: text left in the clipboard only.
    LeftInClipboard,
}

pub fn deliver(text: &str, target: isize, use_clipboard: bool) -> Result<Outcome, String> {
    wait_for_modifiers_release();
    if foreground() != target {
        let clipboard = Clipboard::open()?;
        clipboard.set_text(text)?;
        return Ok(Outcome::LeftInClipboard);
    }
    if !use_clipboard {
        type_text(text)?;
        return Ok(Outcome::Typed);
    }
    let backup = {
        let clipboard = Clipboard::open()?;
        match clipboard.backup() {
            Some(backup) => {
                clipboard.set_text(text)?;
                backup
            }
            None => {
                drop(clipboard);
                // Content we cannot faithfully restore is never overwritten.
                type_text(text)?;
                return Ok(Outcome::Typed);
            }
        }
    };
    let sequence = unsafe { GetClipboardSequenceNumber() };
    send_keys(&[
        key(VK_CONTROL, 0),
        key(u16::from(b'V'), 0),
        key(u16::from(b'V'), KEYEVENTF_KEYUP),
        key(VK_CONTROL, KEYEVENTF_KEYUP),
    ])?;
    std::thread::sleep(PASTE_SETTLE);
    if unsafe { GetClipboardSequenceNumber() } != sequence {
        return Ok(Outcome::PastedNoRestore);
    }
    Clipboard::open()?.restore(&backup)?;
    Ok(Outcome::Pasted)
}

/// Releasing the hotkey must not combine with our Ctrl+V or typed characters.
fn wait_for_modifiers_release() {
    let deadline = Instant::now() + Duration::from_millis(1500);
    let pressed = || {
        [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
            .iter()
            .any(|vk| unsafe { GetAsyncKeyState(i32::from(*vk)) } as u16 & 0x8000 != 0)
    };
    while pressed() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn key(vk: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn unicode(unit: u16, flags: u32) -> INPUT {
    let mut input = key(0, KEYEVENTF_UNICODE | flags);
    input.Anonymous.ki.wScan = unit;
    input
}

fn send_keys(inputs: &[INPUT]) -> Result<(), String> {
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent as usize != inputs.len() {
        return Err(format!(
            "Tastatureingabe blockiert ({})",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn type_text(text: &str) -> Result<(), String> {
    // Newlines would act as Enter and could submit forms; dictation never sends Enter.
    let text = text.replace(['\r', '\n'], " ");
    for chunk in text.encode_utf16().collect::<Vec<_>>().chunks(64) {
        let inputs: Vec<INPUT> = chunk
            .iter()
            .flat_map(|&u| [unicode(u, 0), unicode(u, KEYEVENTF_KEYUP)])
            .collect();
        send_keys(&inputs)?;
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

struct Clipboard;

struct Entry {
    format: u32,
    data: Vec<u8>,
}

impl Clipboard {
    fn open() -> Result<Self, String> {
        let owner = OWNER.load(Ordering::Relaxed) as HWND;
        for _ in 0..20 {
            if unsafe { OpenClipboard(owner) } != 0 {
                return Ok(Self);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Err("Zwischenablage ist durch ein anderes Programm belegt".into())
    }

    /// Copies all memory-based formats; `None` if something cannot be restored.
    fn backup(&self) -> Option<Vec<Entry>> {
        let mut entries = Vec::new();
        let mut total = 0;
        let mut format = 0;
        loop {
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                return Some(entries);
            }
            if is_gdi_format(format) {
                continue; // Windows synthesizes these from the memory formats (e.g. CF_DIB).
            }
            let handle = unsafe { GetClipboardData(format) };
            if handle.is_null() {
                continue;
            }
            let data = unsafe { read_global(handle) }?;
            total += data.len();
            if total > MAX_BACKUP_BYTES {
                return None;
            }
            entries.push(Entry { format, data });
        }
    }

    fn set_text(&self, text: &str) -> Result<(), String> {
        let mut bytes: Vec<u8> = text
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        unsafe { EmptyClipboard() };
        self.put(CF_UNICODETEXT, &bytes)?;
        // Keep dictations out of Win+V history and cloud clipboard.
        bytes = 0u32.to_le_bytes().to_vec();
        for name in [
            "ExcludeClipboardContentFromMonitorProcessing",
            "CanIncludeInClipboardHistory",
            "CanUploadToCloudClipboard",
        ] {
            let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
            let format = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
            if format != 0 {
                self.put(format, &bytes)?;
            }
        }
        Ok(())
    }

    fn restore(&self, entries: &[Entry]) -> Result<(), String> {
        unsafe { EmptyClipboard() };
        for entry in entries {
            self.put(entry.format, &entry.data)?;
        }
        Ok(())
    }

    fn put(&self, format: u32, data: &[u8]) -> Result<(), String> {
        unsafe {
            let handle = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1));
            if handle.is_null() {
                return Err("Speicher für Zwischenablage nicht verfügbar".into());
            }
            let target = GlobalLock(handle) as *mut u8;
            if target.is_null() {
                GlobalFree(handle);
                return Err("Zwischenablage-Speicher nicht sperrbar".into());
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), target, data.len());
            GlobalUnlock(handle);
            if SetClipboardData(format, handle).is_null() {
                GlobalFree(handle);
                return Err(format!(
                    "Zwischenablage konnte nicht gesetzt werden ({})",
                    std::io::Error::last_os_error()
                ));
            }
        }
        Ok(())
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

fn is_gdi_format(format: u32) -> bool {
    // CF_BITMAP, CF_METAFILEPICT, CF_PALETTE, CF_ENHMETAFILE, owner-display/DSP formats, GDI objects.
    matches!(format, 2 | 3 | 9 | 14 | 0x80..=0x8E | 0x300..=0x3FF)
}

unsafe fn read_global(handle: *mut c_void) -> Option<Vec<u8>> {
    unsafe {
        let size = GlobalSize(handle);
        if size == 0 || size > MAX_BACKUP_BYTES {
            return None;
        }
        let source = GlobalLock(handle) as *const u8;
        if source.is_null() {
            return None;
        }
        let data = std::slice::from_raw_parts(source, size).to_vec();
        GlobalUnlock(handle);
        Some(data)
    }
}
