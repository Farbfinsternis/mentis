//! Status lines for `%LOCALAPPDATA%\Mentis\mentis.log` and, when started from a terminal, the
//! console. Never pass dictated text here.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

/// The log is moved to mentis.old.log at startup once it grows beyond this.
const MAX_BYTES: u64 = 1024 * 1024;

static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);
static CONSOLE: AtomicBool = AtomicBool::new(false);

pub fn path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Mentis")
        .join("mentis.log")
}

/// Starts writing to the log file; earlier and failed attempts only reach the console.
pub fn open_file() -> Result<(), String> {
    let path = path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("old.log"));
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    // Byte order mark so every editor reads the umlauts as UTF-8.
    if file.metadata().is_ok_and(|m| m.len() == 0) {
        let _ = file.write_all("\u{feff}".as_bytes());
    }
    if let Ok(mut slot) = FILE.lock() {
        *slot = Some(file);
    }
    Ok(())
}

/// Stops writing to the log file (so it can be deleted).
pub fn close() {
    if let Ok(mut slot) = FILE.lock() {
        *slot = None;
    }
}

/// Also print lines to the console Mentis was attached to.
pub fn to_console(enabled: bool) {
    CONSOLE.store(enabled, Ordering::Relaxed);
}

pub fn line(message: &str) {
    if CONSOLE.load(Ordering::Relaxed) {
        println!("{message}");
    }
    if let Ok(mut slot) = FILE.lock()
        && let Some(file) = slot.as_mut()
    {
        let _ = writeln!(file, "{} {message}", timestamp());
    }
}

fn timestamp() -> String {
    let mut now: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut now) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond
    )
}
