//! Light or dark windows, following the app mode in the Windows settings. Dark controls need a
//! few undocumented uxtheme functions (looked up by ordinal, skipped where missing). Check boxes
//! and radio buttons are drawn here, because their themed text stays black on dark.

use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows_sys::Win32::Foundation::{COLORREF, ERROR_SUCCESS, HWND, LPARAM, LRESULT, RECT, SIZE};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::{
    COLOR_WINDOW, COLOR_WINDOWTEXT, CreateSolidBrush, DT_CALCRECT, DT_END_ELLIPSIS, DT_LEFT,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DrawFocusRect, DrawTextW, FillRect, GetSysColor,
    GetSysColorBrush, HBRUSH, HDC, HFONT, RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME, RDW_INVALIDATE,
    RedrawWindow, SelectObject, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::Controls::BST_CHECKED;
use windows_sys::Win32::UI::Controls::{
    BP_CHECKBOX, BP_RADIOBUTTON, CDDS_PREPAINT, CDIS_DISABLED, CDIS_FOCUS, CDIS_HOT, CDIS_SELECTED,
    CDRF_SKIPDEFAULT, COMBOBOXINFO, CloseThemeData, DrawThemeBackground, GetComboBoxInfo,
    GetThemePartSize, NM_CUSTOMDRAW, NMCUSTOMDRAW, NMHDR, OpenThemeData, SetWindowTheme, TS_TRUE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, EnumChildWindows, GCLP_HBRBACKGROUND, GWL_STYLE, GetClassNameW, GetWindowLongW,
    GetWindowTextLengthW, GetWindowTextW, SPI_GETHIGHCONTRAST, SendMessageW, SetClassLongPtrW,
    SystemParametersInfoW, UISF_HIDEFOCUS, WM_GETFONT, WM_QUERYUISTATE,
};

pub struct Colors {
    pub window: COLORREF,
    pub text: COLORREF,
    /// Hints and secondary text.
    pub quiet: COLORREF,
    pub error: COLORREF,
    /// Edit fields and drop-down lists.
    pub field: COLORREF,
}

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

const DARK: Colors = Colors {
    window: rgb(32, 32, 32),
    text: rgb(240, 240, 240),
    quiet: rgb(160, 160, 160),
    error: rgb(255, 128, 112),
    field: rgb(45, 45, 45),
};

static DARK_NOW: AtomicBool = AtomicBool::new(false);
static WINDOW_BRUSH: AtomicIsize = AtomicIsize::new(0);
static FIELD_BRUSH: AtomicIsize = AtomicIsize::new(0);

pub fn dark() -> bool {
    DARK_NOW.load(Ordering::Relaxed)
}

pub fn colors() -> Colors {
    if dark() {
        return DARK;
    }
    let (window, text) = unsafe { (GetSysColor(COLOR_WINDOW), GetSysColor(COLOR_WINDOWTEXT)) };
    Colors {
        window,
        text,
        quiet: rgb(96, 96, 96),
        error: rgb(196, 43, 28),
        field: window,
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// "Choose your app mode: Dark" in the Windows settings; high contrast always wins.
fn system_dark() -> bool {
    // MENTIS_THEME=light|dark forces one design, for tests.
    if let Ok(forced) = std::env::var("MENTIS_THEME") {
        return forced.eq_ignore_ascii_case("dark");
    }
    let mut contrast: HIGHCONTRASTW = unsafe { std::mem::zeroed() };
    contrast.cbSize = size_of::<HIGHCONTRASTW>() as u32;
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            (&raw mut contrast).cast(),
            0,
        )
    };
    if ok != 0 && contrast.dwFlags & HCF_HIGHCONTRASTON != 0 {
        return false;
    }
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut value = 1u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut value).cast(),
            &mut size,
        )
    };
    status == ERROR_SUCCESS && value == 0
}

/// An undocumented uxtheme function by ordinal; None on Windows versions without it.
fn uxtheme(ordinal: usize) -> Option<unsafe extern "system" fn() -> isize> {
    unsafe {
        let module = LoadLibraryW(wide("uxtheme.dll").as_ptr());
        if module.is_null() {
            return None;
        }
        GetProcAddress(module, ordinal as *const u8)
    }
}

/// Ordinal 133, AllowDarkModeForWindow(HWND, BOOL).
fn allow_dark(hwnd: HWND, on: bool) {
    if let Some(f) = uxtheme(133) {
        let f: unsafe extern "system" fn(HWND, i32) -> i32 = unsafe { std::mem::transmute(f) };
        unsafe { f(hwnd, i32::from(on)) };
    }
}

/// Ordinals 104 (RefreshImmersiveColorPolicyState) and 136 (FlushMenuThemes).
fn refresh_menus() {
    for ordinal in [104, 136] {
        if let Some(f) = uxtheme(ordinal) {
            unsafe { f() };
        }
    }
}

/// Once per process, before the first window: reads the mode and lets menus follow it.
pub fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        DARK_NOW.store(system_dark(), Ordering::Relaxed);
        crate::log::line(if dark() {
            "Darstellung: dunkel"
        } else {
            "Darstellung: hell"
        });
        // Ordinal 135, SetPreferredAppMode(AllowDark = 1) (AllowDarkModeForApp(TRUE) on 1809).
        if let Some(f) = uxtheme(135) {
            let f: unsafe extern "system" fn(i32) -> i32 = unsafe { std::mem::transmute(f) };
            unsafe { f(1) };
        }
        refresh_menus();
    });
}

/// WM_SETTINGCHANGE: whether it announces a new app mode ("ImmersiveColorSet").
pub fn is_mode_change(lparam: LPARAM) -> bool {
    if lparam == 0 {
        return false;
    }
    let text = lparam as *const u16;
    let name: Vec<u16> = "ImmersiveColorSet".encode_utf16().collect();
    // SAFETY: lparam of WM_SETTINGCHANGE is a null-terminated string when non-zero.
    let len = (0..64)
        .take_while(|&i| unsafe { *text.add(i) } != 0)
        .count();
    let found = unsafe { std::slice::from_raw_parts(text, len) };
    found == name.as_slice()
}

/// Re-reads the app mode after a change; menus follow from the next one opened.
pub fn refresh() {
    DARK_NOW.store(system_dark(), Ordering::Relaxed);
    refresh_menus();
}

/// The hidden tray window owns the menu; allowing it lets the menu follow the mode.
pub fn allow_menus(hwnd: HWND) {
    allow_dark(hwnd, true);
}

fn cached_brush(slot: &AtomicIsize, color: COLORREF) -> HBRUSH {
    let brush = slot.load(Ordering::Relaxed);
    if brush != 0 {
        return brush as HBRUSH;
    }
    let brush = unsafe { CreateSolidBrush(color) };
    slot.store(brush as isize, Ordering::Relaxed);
    brush
}

pub fn window_brush() -> HBRUSH {
    if dark() {
        cached_brush(&WINDOW_BRUSH, DARK.window)
    } else {
        unsafe { GetSysColorBrush(COLOR_WINDOW) }
    }
}

fn field_brush() -> HBRUSH {
    if dark() {
        cached_brush(&FIELD_BRUSH, DARK.field)
    } else {
        unsafe { GetSysColorBrush(COLOR_WINDOW) }
    }
}

/// Applies the current mode to a top-level window and its controls and redraws them.
pub fn apply(window: HWND) {
    let dark = dark();
    allow_dark(window, dark);
    let flag = i32::from(dark);
    unsafe {
        DwmSetWindowAttribute(
            window,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&raw const flag).cast(),
            size_of::<i32>() as u32,
        );
        SetClassLongPtrW(window, GCLP_HBRBACKGROUND, window_brush() as isize);
        EnumChildWindows(window, Some(apply_child), 0);
        RedrawWindow(
            window,
            std::ptr::null(),
            std::ptr::null_mut(),
            RDW_ERASE | RDW_INVALIDATE | RDW_FRAME | RDW_ALLCHILDREN,
        );
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

fn set_theme(hwnd: HWND, dark_theme: &str) {
    let dark = dark();
    allow_dark(hwnd, dark);
    let name = wide(dark_theme);
    // Null restores the normal theme.
    let name = if dark {
        name.as_ptr()
    } else {
        std::ptr::null()
    };
    unsafe { SetWindowTheme(hwnd, name, std::ptr::null()) };
}

unsafe extern "system" fn apply_child(hwnd: HWND, _: LPARAM) -> i32 {
    match class_name(hwnd).to_ascii_lowercase().as_str() {
        "button" | "msctls_progress32" => set_theme(hwnd, "DarkMode_Explorer"),
        "edit" => set_theme(hwnd, "DarkMode_CFD"),
        "combobox" => {
            set_theme(hwnd, "DarkMode_CFD");
            let mut info: COMBOBOXINFO = unsafe { std::mem::zeroed() };
            info.cbSize = size_of::<COMBOBOXINFO>() as u32;
            if unsafe { GetComboBoxInfo(hwnd, &mut info) } != 0 && !info.hwndList.is_null() {
                set_theme(info.hwndList, "DarkMode_Explorer");
            }
        }
        _ => {}
    }
    1
}

/// WM_CTLCOLORSTATIC / WM_CTLCOLORBTN: text on the window background, optionally in `text`.
pub fn static_colors(dc: HDC, text: Option<COLORREF>) -> LRESULT {
    let colors = colors();
    unsafe {
        SetTextColor(dc, text.unwrap_or(colors.text));
        SetBkColor(dc, colors.window);
    }
    window_brush() as LRESULT
}

/// WM_CTLCOLOREDIT / WM_CTLCOLORLISTBOX: edit fields and the open list of a drop-down.
pub fn field_colors(dc: HDC) -> LRESULT {
    let colors = colors();
    unsafe {
        SetTextColor(dc, colors.text);
        SetBkColor(dc, colors.field);
    }
    field_brush() as LRESULT
}

/// WM_NOTIFY: draws check boxes and radio buttons in dark mode. None: not handled here.
pub fn custom_draw(lparam: LPARAM) -> Option<LRESULT> {
    if !dark() || lparam == 0 {
        return None;
    }
    // SAFETY: WM_NOTIFY always carries an NMHDR; NM_CUSTOMDRAW from a button an NMCUSTOMDRAW.
    let header = unsafe { &*(lparam as *const NMHDR) };
    if header.code != NM_CUSTOMDRAW {
        return None;
    }
    let hwnd = header.hwndFrom;
    // BS_TYPEMASK: 2/3/5/6 check boxes, 4/9 radio buttons.
    let part = match unsafe { GetWindowLongW(hwnd, GWL_STYLE) } & 0xF {
        2 | 3 | 5 | 6 => BP_CHECKBOX,
        4 | 9 => BP_RADIOBUTTON,
        _ => return None,
    };
    let draw = unsafe { &*(lparam as *const NMCUSTOMDRAW) };
    if draw.dwDrawStage != CDDS_PREPAINT {
        return None;
    }
    unsafe { paint_check(hwnd, draw, part) };
    Some(CDRF_SKIPDEFAULT as LRESULT)
}

unsafe fn paint_check(hwnd: HWND, draw: &NMCUSTOMDRAW, part: i32) {
    let dc = draw.hdc;
    let rect = draw.rc;
    let colors = colors();
    let item = draw.uItemState;
    let checked = unsafe { SendMessageW(hwnd, BM_GETCHECK, 0, 0) } == BST_CHECKED as isize;
    // CBS_*/RBS_*: unchecked 1–4, checked 5–8 (normal, hot, pressed, disabled).
    let state = if checked { 5 } else { 1 }
        + if item & CDIS_DISABLED != 0 {
            3
        } else if item & CDIS_SELECTED != 0 {
            2
        } else if item & CDIS_HOT != 0 {
            1
        } else {
            0
        };
    unsafe {
        FillRect(dc, &rect, window_brush());
        let class = wide("Button");
        let theme = OpenThemeData(hwnd, class.as_ptr());
        let mut size = SIZE { cx: 13, cy: 13 };
        if theme != 0 {
            GetThemePartSize(theme, dc, part, state, std::ptr::null(), TS_TRUE, &mut size);
        }
        let top = rect.top + (rect.bottom - rect.top - size.cy) / 2;
        let glyph = RECT {
            left: rect.left,
            top,
            right: rect.left + size.cx,
            bottom: top + size.cy,
        };
        if theme != 0 {
            DrawThemeBackground(theme, dc, part, state, &glyph, std::ptr::null());
            CloseThemeData(theme);
        }

        let len = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut text = vec![0u16; len + 1];
        let len = GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32).max(0);
        let font = SendMessageW(hwnd, WM_GETFONT, 0, 0) as HFONT;
        let old = SelectObject(dc, font as _);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(
            dc,
            if item & CDIS_DISABLED != 0 {
                colors.quiet
            } else {
                colors.text
            },
        );
        let mut label = RECT {
            left: glyph.right + size.cx / 3 + 1,
            ..rect
        };
        let flags = DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS;
        DrawTextW(dc, text.as_mut_ptr(), len, &mut label, flags);
        let hide_focus = SendMessageW(hwnd, WM_QUERYUISTATE, 0, 0) as u32 & UISF_HIDEFOCUS != 0;
        if item & CDIS_FOCUS != 0 && !hide_focus {
            let mut extent = label;
            DrawTextW(dc, text.as_mut_ptr(), len, &mut extent, flags | DT_CALCRECT);
            let height = extent.bottom - extent.top;
            let focus = RECT {
                left: label.left - 1,
                top: label.top + (label.bottom - label.top - height) / 2 - 1,
                right: extent.right.min(label.right) + 1,
                bottom: label.top + (label.bottom - label.top + height) / 2 + 1,
            };
            DrawFocusRect(dc, &focus);
        }
        SelectObject(dc, old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_the_app_mode_broadcast() {
        let text = wide("ImmersiveColorSet");
        assert!(is_mode_change(text.as_ptr() as LPARAM));
        let other = wide("WindowsThemeElement");
        assert!(!is_mode_change(other.as_ptr() as LPARAM));
        assert!(!is_mode_change(0));
    }

    #[test]
    fn colors_are_bgr() {
        assert_eq!(rgb(0x11, 0x22, 0x33), 0x0033_2211);
    }
}
