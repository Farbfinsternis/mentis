//! "Start with Windows" via the per-user Run key (no administrator rights needed).

use std::path::PathBuf;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "Mentis";
const OLD_VALUE: &str = "Echo";
/// Passed by the Run entry so Mentis starts quietly without the "ready" hint.
pub const FLAG: &str = "--autostart";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn exe() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(exe.to_string_lossy().into_owned())
}

/// The Run entry for this EXE.
fn command(exe: &str) -> String {
    format!("\"{exe}\" {FLAG}")
}

/// The program a Run entry starts.
fn program(entry: &str) -> &str {
    let entry = entry.trim();
    let path = match entry.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(rest),
        None => entry.split(" --").next().unwrap_or(entry),
    };
    path.trim()
}

/// Whether a Run entry starts this EXE (and not a copy in another folder).
fn starts(entry: &str, exe: &str) -> bool {
    program(entry).eq_ignore_ascii_case(exe)
}

fn read(key: &str, name: &str) -> Option<String> {
    let (key, name) = (wide(key), wide(name));
    let mut buffer = [0u16; 1024];
    let mut bytes = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let len = (bytes as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buffer[..len]))
}

fn write(key: &str, name: &str, value: &str) -> Result<(), String> {
    let (key, name, value) = (wide(key), wide(name), wide(value));
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            REG_SZ,
            value.as_ptr().cast(),
            (value.len() * 2) as u32,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(status as i32).to_string())
    }
}

fn delete(key: &str, name: &str) -> Result<(), String> {
    let (key, name) = (wide(key), wide(name));
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
    match status {
        ERROR_SUCCESS | windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND => Ok(()),
        _ => Err(std::io::Error::from_raw_os_error(status as i32).to_string()),
    }
}

pub fn enabled() -> bool {
    match (read(RUN_KEY, VALUE), exe()) {
        (Some(entry), Ok(exe)) => starts(&entry, &exe),
        _ => false,
    }
}

/// Turning it on also repairs an entry that points to an older Mentis folder.
pub fn set(on: bool) -> Result<(), String> {
    if on {
        write(RUN_KEY, VALUE, &command(&exe()?))
    } else {
        delete(RUN_KEY, VALUE)
    }
}

/// Removes the Run entry of Echo (Mentis' name up to 0.7) and returns the program it started,
/// so that Mentis can start with Windows in its place.
pub fn take_over_echo() -> Result<Option<PathBuf>, String> {
    take_over(RUN_KEY, OLD_VALUE)
}

fn take_over(key: &str, name: &str) -> Result<Option<PathBuf>, String> {
    let Some(entry) = read(key, name) else {
        return Ok(None);
    };
    delete(key, name)?;
    Ok(Some(PathBuf::from(program(&entry))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_entries_for_this_exe_only() {
        let exe = r"C:\Programme\Mentis\mentis.exe";
        assert_eq!(
            command(exe),
            r#""C:\Programme\Mentis\mentis.exe" --autostart"#
        );
        assert!(starts(&command(exe), exe));
        assert!(starts(r"c:\programme\mentis\MENTIS.EXE", exe));
        assert!(starts(r"C:\Programme\Mentis\mentis.exe --autostart", exe));
        assert!(!starts(r#""D:\Alt\Mentis\mentis.exe" --autostart"#, exe));
        assert!(!starts("", exe));
    }

    #[test]
    fn registry_round_trip() {
        // A test key of its own; the real Run entry is never touched.
        let key = r"Software\Mentis\Test";
        write(key, VALUE, "eins").unwrap();
        assert_eq!(read(key, VALUE).as_deref(), Some("eins"));
        delete(key, VALUE).unwrap();
        assert_eq!(read(key, VALUE), None);
        delete(key, VALUE).unwrap();

        write(key, OLD_VALUE, r#""D:\Alt\Echo\echo.exe" --autostart"#).unwrap();
        let old = take_over(key, OLD_VALUE).unwrap();
        assert_eq!(old, Some(PathBuf::from(r"D:\Alt\Echo\echo.exe")));
        assert_eq!(read(key, OLD_VALUE), None);
        assert_eq!(take_over(key, OLD_VALUE).unwrap(), None);
        let (sub, parent) = (wide(key), wide(r"Software\Mentis"));
        unsafe {
            windows_sys::Win32::System::Registry::RegDeleteKeyW(HKEY_CURRENT_USER, sub.as_ptr());
            // Removes Software\Mentis only if the test created it and it is empty now.
            windows_sys::Win32::System::Registry::RegDeleteKeyW(HKEY_CURRENT_USER, parent.as_ptr());
        }
    }
}
