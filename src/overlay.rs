//! Small always-on-top text panel above the taskbar: never takes focus, clicks pass through.

use crate::i18n::t;
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreatePen, CreateRoundRectRgn,
    CreateSolidBrush, DEFAULT_CHARSET, DT_CALCRECT, DT_NOPREFIX, DT_WORDBREAK, DeleteObject,
    DrawTextW, Ellipse, EndPaint, FF_SWISS, FW_NORMAL, FillRect, GetDC, GetMonitorInfoW,
    GetStockObject, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    NULL_BRUSH, NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, ReleaseDC, RoundRect,
    SelectObject, SetBkMode, SetTextColor, SetWindowRgn, TRANSPARENT,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetForegroundWindow, HTTRANSPARENT,
    HWND_TOPMOST, KillTimer, LWA_ALPHA, PostMessageW, RegisterClassW, SW_HIDE, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, WM_APP,
    WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Hidden,
    Recording,
    Processing,
    Message,
}

struct State {
    phase: Phase,
    text: String,
    hide_after_ms: u32,
}

static STATE: Mutex<State> = Mutex::new(State {
    phase: Phase::Hidden,
    text: String::new(),
    hide_after_ms: 0,
});
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static FONT: AtomicIsize = AtomicIsize::new(0);
static FONT_DPI: AtomicU32 = AtomicU32::new(0);

const WM_REFRESH: u32 = WM_APP + 1;
const HIDE_TIMER: usize = 1;
/// Only the end of long dictations is shown; the panel stays at a few lines.
const MAX_CHARS: usize = 320;

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}
const BACKGROUND: COLORREF = rgb(20, 40, 76);
const BORDER: COLORREF = rgb(110, 170, 245);
const TEXT: COLORREF = rgb(240, 240, 240);
const HINT: COLORREF = rgb(165, 190, 225);
/// Whole-panel opacity (0–255): translucent, text stays readable.
const OPACITY: u8 = 205;
const RECORDING: COLORREF = rgb(232, 64, 64);
const PROCESSING: COLORREF = rgb(240, 176, 48);

/// Creates the hidden panel; call on the thread that runs the message loop.
pub fn create() -> Result<(), String> {
    let class: Vec<u16> = "MentisOverlay\0".encode_utf16().collect();
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        lpszClassName: class.as_ptr(),
        ..unsafe { std::mem::zeroed() }
    };
    unsafe { RegisterClassW(&wc) };
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class.as_ptr(),
            std::ptr::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err(format!(
            "Anzeigefenster fehlgeschlagen ({})",
            std::io::Error::last_os_error()
        ));
    }
    unsafe { SetLayeredWindowAttributes(hwnd, 0, OPACITY, LWA_ALPHA) };
    WINDOW.store(hwnd as isize, Ordering::Relaxed);
    Ok(())
}

/// Updates the panel from any thread. `hide_after_ms` = 0 keeps it visible.
pub fn show(phase: Phase, text: &str, hide_after_ms: u32) {
    if let Ok(mut state) = STATE.lock() {
        state.phase = phase;
        state.text = tail(text, MAX_CHARS);
        state.hide_after_ms = hide_after_ms;
    }
    let hwnd = WINDOW.load(Ordering::Relaxed) as HWND;
    if !hwnd.is_null() {
        unsafe { PostMessageW(hwnd, WM_REFRESH, 0, 0) };
    }
}

pub fn hide() {
    show(Phase::Hidden, "", 0);
}

fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_owned();
    }
    let rest: String = text.chars().skip(count - max).collect();
    // Start at a word boundary so the panel does not begin mid-word.
    let rest = rest.split_once(' ').map_or(rest.as_str(), |(_, r)| r);
    format!("… {rest}")
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn font(dpi: u32) -> isize {
    if FONT_DPI.load(Ordering::Relaxed) != dpi {
        let face: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
        let height = -((20 * dpi / 96) as i32);
        let new = unsafe {
            CreateFontW(
                height,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                u32::from(DEFAULT_CHARSET),
                u32::from(OUT_DEFAULT_PRECIS),
                u32::from(CLIP_DEFAULT_PRECIS),
                u32::from(CLEARTYPE_QUALITY),
                u32::from(FF_SWISS),
                face.as_ptr(),
            )
        };
        let old = FONT.swap(new as isize, Ordering::Relaxed);
        if old != 0 {
            unsafe { DeleteObject(old as _) };
        }
        FONT_DPI.store(dpi, Ordering::Relaxed);
    }
    FONT.load(Ordering::Relaxed)
}

/// Text plus whether it is a placeholder hint rather than recognized speech.
fn content(state: &State) -> (String, bool) {
    match (state.phase, state.text.is_empty()) {
        (Phase::Recording, true) => (t("Höre zu …", "Listening …").into(), true),
        (Phase::Processing, true) => (t("Erkenne …", "Recognizing …").into(), true),
        (Phase::Message, _) => (state.text.clone(), true),
        _ => (state.text.clone(), false),
    }
}

struct Layout {
    scale: f32,
}

impl Layout {
    fn px(&self, value: i32) -> i32 {
        (value as f32 * self.scale).round() as i32
    }
    fn pad(&self) -> i32 {
        self.px(14)
    }
    fn dot(&self) -> i32 {
        self.px(10)
    }
    fn radius(&self) -> i32 {
        self.px(16)
    }
    fn text_left(&self) -> i32 {
        self.pad() + self.dot() + self.px(12)
    }
}

unsafe fn refresh(hwnd: HWND) {
    // Copy the state so no lock is held while window calls dispatch messages.
    let Some((phase, text, hide_after_ms)) = STATE
        .lock()
        .ok()
        .map(|s| (s.phase, content(&s).0, s.hide_after_ms))
    else {
        return;
    };
    unsafe {
        if phase == Phase::Hidden {
            KillTimer(hwnd, HIDE_TIMER);
            ShowWindow(hwnd, SW_HIDE);
            return;
        }
        // Show on the monitor the user is working on, just above the taskbar.
        let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(monitor, &mut info);
        let (mut dpi, mut _y) = (96, 96);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut _y);
        let layout = Layout {
            scale: dpi as f32 / 96.0,
        };
        let work = info.rcWork;
        let width = (work.right - work.left - layout.px(48)).min(layout.px(760));
        let mut text = wide(&text);
        let mut measure = RECT {
            left: 0,
            top: 0,
            right: width - layout.text_left() - layout.pad(),
            bottom: 0,
        };
        let dc = GetDC(hwnd);
        let previous = SelectObject(dc, font(dpi) as _);
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            text.len() as i32,
            &mut measure,
            DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(dc, previous);
        ReleaseDC(hwnd, dc);
        let height = measure.bottom.max(layout.px(20)) + 2 * layout.pad();
        let x = work.left + (work.right - work.left - width) / 2;
        let y = work.bottom - height - layout.px(16);
        let radius = layout.radius();
        SetWindowRgn(
            hwnd,
            CreateRoundRectRgn(0, 0, width + 1, height + 1, radius, radius),
            1,
        );
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        InvalidateRect(hwnd, std::ptr::null(), 1);
        if hide_after_ms > 0 {
            SetTimer(hwnd, HIDE_TIMER, hide_after_ms, None);
        } else {
            KillTimer(hwnd, HIDE_TIMER);
        }
    }
}

unsafe fn paint(hwnd: HWND) {
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let snapshot = STATE.lock().ok().map(|s| (s.phase, content(&s)));
        if let Some((phase, (text, hint))) = snapshot {
            let dpi = FONT_DPI.load(Ordering::Relaxed).max(96);
            let layout = Layout {
                scale: dpi as f32 / 96.0,
            };
            let mut client: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut client);
            let background = CreateSolidBrush(BACKGROUND);
            FillRect(dc, &client, background);
            DeleteObject(background as _);

            // Rounded frame just inside the window region.
            let width = layout.px(2).max(1);
            let pen = CreatePen(PS_SOLID, width, BORDER);
            let old_pen = SelectObject(dc, pen as _);
            let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
            let inset = width / 2;
            RoundRect(
                dc,
                inset,
                inset,
                client.right - inset,
                client.bottom - inset,
                layout.radius(),
                layout.radius(),
            );
            SelectObject(dc, old_brush);
            SelectObject(dc, old_pen);
            DeleteObject(pen as _);

            let dot_color = match phase {
                Phase::Recording => Some(RECORDING),
                Phase::Processing => Some(PROCESSING),
                _ => None,
            };
            if let Some(color) = dot_color {
                let brush = CreateSolidBrush(color);
                let old_brush = SelectObject(dc, brush as _);
                let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
                let top = layout.pad() + (layout.px(26) - layout.dot()) / 2;
                Ellipse(
                    dc,
                    layout.pad(),
                    top,
                    layout.pad() + layout.dot(),
                    top + layout.dot(),
                );
                SelectObject(dc, old_pen);
                SelectObject(dc, old_brush);
                DeleteObject(brush as _);
            }

            let mut text = wide(&text);
            let mut rect = RECT {
                left: layout.text_left(),
                top: layout.pad(),
                right: client.right - layout.pad(),
                bottom: client.bottom - layout.pad(),
            };
            let previous = SelectObject(dc, font(dpi) as _);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, if hint { HINT } else { TEXT });
            DrawTextW(
                dc,
                text.as_mut_ptr(),
                text.len() as i32,
                &mut rect,
                DT_WORDBREAK | DT_NOPREFIX,
            );
            SelectObject(dc, previous);
        }
        EndPaint(hwnd, &ps);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_REFRESH => {
                refresh(hwnd);
                0
            }
            WM_PAINT => {
                paint(hwnd);
                0
            }
            WM_TIMER if wparam == HIDE_TIMER => {
                if let Ok(mut state) = STATE.lock() {
                    state.phase = Phase::Hidden;
                }
                KillTimer(hwnd, HIDE_TIMER);
                ShowWindow(hwnd, SW_HIDE);
                0
            }
            WM_NCHITTEST => HTTRANSPARENT as LRESULT,
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tail;

    #[test]
    fn tail_keeps_short_text_and_cuts_long_text_at_a_word() {
        assert_eq!(tail("kurz", 10), "kurz");
        assert_eq!(tail("eins zwei drei vier", 9), "… vier");
        assert_eq!(tail("äöü äöü", 5), "… äöü");
    }
}
