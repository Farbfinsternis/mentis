//! User interface language: German when Windows is set to German, English otherwise.

use std::sync::OnceLock;
use windows_sys::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};

const LANG_GERMAN: u16 = 0x07;

pub fn german() -> bool {
    static GERMAN: OnceLock<bool> = OnceLock::new();
    // The low 10 bits of a LANGID are the primary language (de-DE, de-AT, de-CH, ...).
    *GERMAN.get_or_init(|| unsafe { GetUserDefaultUILanguage() } & 0x3ff == LANG_GERMAN)
}

/// Two-letter code of the Windows display language ("de", "en", ...), "en" if unknown.
pub fn windows_language() -> String {
    let mut name = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
    let len = unsafe {
        LCIDToLocaleName(
            u32::from(GetUserDefaultUILanguage()),
            name.as_mut_ptr(),
            name.len() as i32,
            0,
        )
    };
    let name = String::from_utf16_lossy(&name[..(len.max(1) as usize - 1)]);
    language_code(&name).unwrap_or_else(|| "en".into())
}

/// "de-DE" -> "de"; the script/region part is irrelevant for speech recognition.
fn language_code(locale: &str) -> Option<String> {
    let code = locale.split('-').next()?.to_ascii_lowercase();
    (code.len() == 2 && code.chars().all(|c| c.is_ascii_lowercase())).then_some(code)
}

/// Picks the German or English text.
pub fn t(de: &'static str, en: &'static str) -> &'static str {
    if german() { de } else { en }
}

/// Readable name of the dictation key, e.g. "RightCtrl" -> "Strg rechts".
pub fn key_name(hotkey: &str) -> String {
    let name = match hotkey.trim().to_ascii_lowercase().as_str() {
        "rightctrl" => t("Strg rechts", "Right Ctrl"),
        "rightshift" => t("Umschalt rechts", "Right Shift"),
        "capslock" => t("Feststelltaste", "Caps Lock"),
        "scrolllock" => t("Rollen", "Scroll Lock"),
        "pause" => "Pause",
        "insert" => t("Einfg", "Insert"),
        _ => return hotkey.trim().to_ascii_uppercase(),
    };
    name.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_code_from_locale_name() {
        assert_eq!(language_code("de-DE").as_deref(), Some("de"));
        assert_eq!(language_code("en-US").as_deref(), Some("en"));
        assert_eq!(language_code("sr-Latn-RS").as_deref(), Some("sr"));
        assert_eq!(language_code(""), None);
        assert_eq!(language_code("fil-PH"), None);
        assert_eq!(windows_language().len(), 2);
    }
}
