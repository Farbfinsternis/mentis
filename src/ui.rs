//! Shared pieces of Mentis' windows (setup, settings): creation, fonts, layout, dialogs.

use crate::i18n::{german, t};
use crate::{config, download, log, models, theme};
use std::path::{Path, PathBuf};
use std::sync::Once;
use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, GetMonitorInfoW, HDC, HFONT, MONITOR_DEFAULTTONEAREST,
    MONITORINFO, MonitorFromWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Controls::{
    ICC_PROGRESS_CLASS, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
};
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, SystemParametersInfoForDpi};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetWindowTextLengthW, GetWindowTextW, ICON_BIG, ICON_SMALL, IDC_ARROW,
    IMAGE_ICON, LR_DEFAULTSIZE, LoadCursorW, LoadImageW, NONCLIENTMETRICSW, RegisterClassW,
    SPI_GETNONCLIENTMETRICS, SWP_NOZORDER, SendMessageW, SetWindowPos, SetWindowTextW,
    WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_NOTIFY, WM_SETFONT,
    WM_SETICON, WM_SETTINGCHANGE, WNDCLASSW, WNDPROC, WS_CAPTION, WS_CHILD, WS_EX_CONTROLPARENT,
    WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_VISIBLE,
};

/// Fixed-size dialog-like window.
pub const STYLE: u32 = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
/// SS_NOPREFIX: "&" in texts is shown, not turned into a shortcut.
pub const STATIC_PLAIN: u32 = 0x80;
/// SS_PATHELLIPSIS | SS_NOPREFIX: a long path is shortened in the middle.
pub const STATIC_PATH: u32 = 0x8080;

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

pub fn set_text(hwnd: HWND, text: &str) {
    unsafe { SetWindowTextW(hwnd, wide(text).as_ptr()) };
}

pub fn text(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
    let mut buffer = vec![0u16; len + 1];
    let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

/// Creates a hidden top-level window with Mentis' icon; tab navigation via IsDialogMessage.
pub fn new_window(class: &str, title: &str, wndproc: WNDPROC) -> Result<HWND, String> {
    static COMMON_CONTROLS: Once = Once::new();
    COMMON_CONTROLS.call_once(|| unsafe {
        theme::init();
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS | ICC_STANDARD_CLASSES,
        });
    });
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide(class);
        let wc = WNDCLASSW {
            lpfnWndProc: wndproc,
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: theme::window_brush(),
            ..std::mem::zeroed()
        };
        // Fails harmlessly when the class exists from an earlier window.
        RegisterClassW(&wc);
        let window = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            wide(title).as_ptr(),
            STYLE,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if window.is_null() {
            return Err(format!(
                "Fenster fehlgeschlagen ({})",
                std::io::Error::last_os_error()
            ));
        }
        // Program icon from the resources (build.rs).
        for (kind, size) in [(ICON_BIG, 32), (ICON_SMALL, 16)] {
            let icon = LoadImageW(instance, 1 as _, IMAGE_ICON, size, size, LR_DEFAULTSIZE);
            SendMessageW(window, WM_SETICON, kind as usize, icon as isize);
        }
        Ok(window)
    }
}

/// A visible child control of a system class ("STATIC", "BUTTON", "EDIT", ...).
pub fn child(parent: HWND, class: &str, text: &str, style: u32, id: usize) -> HWND {
    child_class(parent, wide(class).as_ptr(), text, style, id)
}

pub fn child_class(parent: HWND, class: *const u16, text: &str, style: u32, id: usize) -> HWND {
    unsafe {
        CreateWindowExW(
            0,
            class,
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            0,
            0,
            parent,
            id as _,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    }
}

/// The system message font at one DPI, plus a large heading and a semibold variant.
pub struct Fonts {
    pub normal: HFONT,
    pub heading: HFONT,
    pub bold: HFONT,
}

impl Fonts {
    pub fn new(dpi: u32) -> Self {
        unsafe {
            let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
            metrics.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS,
                metrics.cbSize,
                (&raw mut metrics).cast(),
                0,
                dpi,
            );
            let base = metrics.lfMessageFont;
            let mut heading = base;
            heading.lfHeight = base.lfHeight * 16 / 10;
            heading.lfWeight = 600;
            let mut bold = base;
            bold.lfWeight = 600;
            Self {
                normal: CreateFontIndirectW(&base),
                heading: CreateFontIndirectW(&heading),
                bold: CreateFontIndirectW(&bold),
            }
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for font in [self.normal, self.heading, self.bold] {
            unsafe { DeleteObject(font as _) };
        }
    }
}

pub fn set_font(hwnd: HWND, font: HFONT) {
    unsafe { SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 1) };
}

pub fn place(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    unsafe { SetWindowPos(hwnd, std::ptr::null_mut(), x, y, w, h, SWP_NOZORDER) };
}

/// Sizes the window for a client area of `width`×`height` pixels; centers it on its monitor
/// unless `origin` is given (after a DPI change).
pub fn fit_window(window: HWND, width: i32, height: i32, dpi: u32, origin: Option<(i32, i32)>) {
    unsafe {
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        AdjustWindowRectExForDpi(&mut frame, STYLE, 0, WS_EX_CONTROLPARENT, dpi);
        let (w, h) = (frame.right - frame.left, frame.bottom - frame.top);
        let (x, y) = origin.unwrap_or_else(|| {
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(
                MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST),
                &mut info,
            );
            let work = info.rcWork;
            (
                work.left + (work.right - work.left - w) / 2,
                (work.top + (work.bottom - work.top - h) / 2).max(work.top),
            )
        });
        place(window, x, y, w, h);
    }
}

/// Colors, dark drawing and app mode changes shared by Mentis' windows; `text` picks a special
/// text color for a label (errors, hints). None: not a theme message, handle it as usual.
pub fn theme_message(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    text: impl Fn(HWND) -> Option<COLORREF>,
) -> Option<LRESULT> {
    let dc = wparam as HDC;
    match msg {
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => Some(theme::static_colors(dc, text(lparam as HWND))),
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => Some(theme::field_colors(dc)),
        WM_NOTIFY => theme::custom_draw(lparam),
        WM_SETTINGCHANGE if theme::is_mode_change(lparam) => {
            theme::refresh();
            theme::apply(hwnd);
            None
        }
        _ => None,
    }
}

/// "1,5" / "1.5" below 10, whole numbers above.
pub fn number(value: f64) -> String {
    let text = if value < 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    };
    if german() {
        text.replace('.', ",")
    } else {
        text
    }
}

/// "Speicherort (120 GB frei): D:\…\models"; use with STATIC_PATH.
pub fn model_folder_text() -> String {
    let dir = models::dir();
    let free = download::free_space(&dir).map(|bytes| {
        let gb = number(bytes as f64 / f64::from(1 << 30));
        if german() {
            format!(" ({gb} GB frei)")
        } else {
            format!(" ({gb} GB free)")
        }
    });
    format!(
        "{}{}: {}",
        t("Speicherort", "Location"),
        free.unwrap_or_default(),
        dir.display()
    )
}

/// Lets the user move the model folder; saved right away as `models_dir`.
/// `Ok(false)`: the dialog was cancelled.
pub fn change_model_folder(owner: HWND, config_path: &Path) -> Result<bool, String> {
    let Some(dir) = crate::folder::pick(
        owner,
        t(
            "Ordner für Sprachmodelle wählen",
            "Choose a folder for speech models",
        ),
        &models::dir(),
    ) else {
        return Ok(false);
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    config::set_path(config_path, "models_dir", &dir)?;
    log::line(&format!("Modellordner: {}", dir.display()));
    models::set_dir(Some(dir));
    Ok(true)
}

/// Open dialog for a GGUF model file that is already on the PC.
pub fn choose_model_file(owner: HWND) -> Option<PathBuf> {
    let mut file = [0u16; 1024];
    let filter: Vec<u16> = t("Sprachmodelle (*.gguf)", "Speech models (*.gguf)")
        .encode_utf16()
        .chain([0])
        .chain("*.gguf".encode_utf16())
        .chain([0, 0])
        .collect();
    let title = wide(t(
        "Vorhandenes Sprachmodell wählen",
        "Choose an existing speech model",
    ));
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = file.as_mut_ptr();
    dialog.nMaxFile = file.len() as u32;
    dialog.lpstrTitle = title.as_ptr();
    dialog.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return None;
    }
    let len = file.iter().position(|&c| c == 0).unwrap_or(file.len());
    Some(PathBuf::from(String::from_utf16_lossy(&file[..len])))
}
