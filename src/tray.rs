//! Icon in the notification area: shows Mentis' state and offers the menu (start with Windows,
//! diagnostics, log, quit). Replaces the console window.

use crate::i18n::{german, t};
use crate::{autostart, diagnose, icon, log};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, GetDC, ReleaseDC,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIIF_RESPECT_QUIET_TIME,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_BALLOONUSERCLICK, NIN_SELECT,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW, ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, HICON, ICONINFO, MB_ICONERROR, MB_OK, MF_CHECKED, MF_DEFAULT, MF_GRAYED,
    MF_SEPARATOR, MF_STRING, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SM_CXSMICON, SW_SHOWNORMAL, SetForegroundWindow, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU,
    WM_DESTROY, WM_NULL, WM_SETTINGCHANGE, WNDCLASSW, WS_POPUP,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Loading,
    Ready,
    Recording,
    Processing,
}

const STATUSES: [Status; 4] = [
    Status::Loading,
    Status::Ready,
    Status::Recording,
    Status::Processing,
];

struct State {
    status: Status,
    /// "GPU" or "CPU" once the model is loaded.
    device: &'static str,
    key: String,
}

static STATE: Mutex<State> = Mutex::new(State {
    status: Status::Loading,
    device: "",
    key: String::new(),
});
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static ICONS: [AtomicIsize; 4] = [const { AtomicIsize::new(0) }; 4];
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
/// A newer version on GitHub, empty if none.
static UPDATE: Mutex<String> = Mutex::new(String::new());

const WM_TRAY: u32 = WM_APP + 10;
const WM_REFRESH: u32 = WM_APP + 11;
/// wparam 1: also show the notification.
const WM_UPDATE: u32 = WM_APP + 12;
/// NIN_SELECT | NINF_KEY: icon activated with the keyboard.
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;

const CMD_AUTOSTART: usize = 1;
const CMD_DIAGNOSE: usize = 2;
const CMD_LOG: usize = 3;
const CMD_QUIT: usize = 4;
const CMD_SETTINGS: usize = 5;
const CMD_UNINSTALL: usize = 6;
const CMD_UPDATE: usize = 7;

/// Set by `restart`: main starts a new Mentis after the message loop ended.
static RESTART: AtomicBool = AtomicBool::new(false);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Creates the hidden window and the icon; call on the thread that runs the message loop.
pub fn create(hotkey: &str) -> Result<(), String> {
    if let Ok(mut state) = STATE.lock() {
        state.key = crate::i18n::key_name(hotkey);
    }
    let class = wide("MentisTray");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        lpszClassName: class.as_ptr(),
        ..unsafe { std::mem::zeroed() }
    };
    unsafe { RegisterClassW(&wc) };
    // A hidden top-level window (not message-only) so it receives the TaskbarCreated broadcast.
    let title = wide("Mentis");
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
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
            "Tray-Fenster fehlgeschlagen ({})",
            std::io::Error::last_os_error()
        ));
    }
    // The menu follows the Windows app mode (light or dark).
    crate::theme::init();
    crate::theme::allow_menus(hwnd);
    WINDOW.store(hwnd as isize, Ordering::Relaxed);
    TASKBAR_CREATED.store(
        unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) },
        Ordering::Relaxed,
    );
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) }.max(16);
    for (slot, status) in ICONS.iter().zip(STATUSES) {
        slot.store(
            unsafe { make_icon(size, color(status)) } as isize,
            Ordering::Relaxed,
        );
    }
    // Right after logon Explorer may not be ready yet; TaskbarCreated adds the icon later.
    if !unsafe { add(hwnd) } {
        log::line("Tray-Symbol noch nicht angelegt, warte auf die Taskleiste.");
    }
    Ok(())
}

/// Updates the icon from any thread.
pub fn set_status(status: Status) {
    if let Ok(mut state) = STATE.lock() {
        state.status = status;
    }
    post(WM_REFRESH);
}

/// Model loaded: switches to Ready and names the compute device in the tooltip.
pub fn set_ready(on_gpu: bool) {
    if let Ok(mut state) = STATE.lock() {
        state.device = if on_gpu { "GPU" } else { "CPU" };
    }
    set_status(Status::Ready);
}

/// Removes the icon and ends the message loop; callable from any thread.
pub fn quit() {
    post(WM_CLOSE);
}

/// Like `quit`, but Mentis starts again with the saved settings.
pub fn restart() {
    RESTART.store(true, Ordering::Relaxed);
    quit();
}

pub fn restart_requested() -> bool {
    RESTART.load(Ordering::Relaxed)
}

/// A newer version is on GitHub: adds a menu entry and, if `notify`, shows a notification.
/// Callable from any thread.
pub fn offer_update(version: &str, notify: bool) {
    if let Ok(mut slot) = UPDATE.lock() {
        *slot = version.to_owned();
    }
    post_with(WM_UPDATE, usize::from(notify));
}

fn update() -> Option<String> {
    UPDATE
        .lock()
        .ok()
        .map(|v| v.clone())
        .filter(|v| !v.is_empty())
}

fn post(msg: u32) {
    post_with(msg, 0);
}

fn post_with(msg: u32, wparam: usize) {
    let hwnd = WINDOW.load(Ordering::Relaxed) as HWND;
    if !hwnd.is_null() {
        unsafe { PostMessageW(hwnd, msg, wparam, 0) };
    }
}

/// Windows notification about the new version; a click opens the release page.
unsafe fn notify_update(hwnd: HWND, version: &str) {
    unsafe {
        let mut data = icon_data(hwnd);
        data.uFlags |= NIF_INFO;
        data.dwInfoFlags = NIIF_INFO | NIIF_RESPECT_QUIET_TIME;
        let title = format!("Mentis {version}");
        let text = t(
            "Eine neue Version ist da. Klicken öffnet die Download-Seite.",
            "A new version is available. Click to open the download page.",
        );
        let title: Vec<u16> = title
            .encode_utf16()
            .take(data.szInfoTitle.len() - 1)
            .collect();
        data.szInfoTitle[..title.len()].copy_from_slice(&title);
        let text: Vec<u16> = text.encode_utf16().take(data.szInfo.len() - 1).collect();
        data.szInfo[..text.len()].copy_from_slice(&text);
        if Shell_NotifyIconW(NIM_MODIFY, &data) == 0 {
            log::line(&format!(
                "Hinweis auf neue Version nicht angezeigt ({})",
                std::io::Error::last_os_error()
            ));
        }
    }
}

fn open(hwnd: HWND, target: &str) -> bool {
    let (verb, target) = (wide("open"), wide(target));
    let result = unsafe {
        ShellExecuteW(
            hwnd,
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values up to 32 are errors.
    result as usize > 32
}

fn color(status: Status) -> [u8; 3] {
    match status {
        Status::Loading => [120, 120, 120],
        Status::Ready => icon::BLUE,
        Status::Recording => [220, 52, 52],
        Status::Processing => [222, 148, 20],
    }
}

fn status_text(state: &State) -> String {
    let (status, device, key) = (state.status, state.device, &state.key);
    match status {
        Status::Loading => t("lädt das Sprachmodell …", "loading the speech model …").into(),
        Status::Ready if german() => format!("bereit ({device})\n{key} halten und sprechen"),
        Status::Ready => format!("ready ({device})\nHold {key} and speak"),
        Status::Recording => t("hört zu …", "listening …").into(),
        Status::Processing => t("erkennt den Text …", "recognizing …").into(),
    }
}

fn tooltip() -> (Status, String) {
    match STATE.lock() {
        Ok(state) => (state.status, format!("Mentis – {}", status_text(&state))),
        Err(_) => (Status::Ready, "Mentis".into()),
    }
}

/// Fills the shared parts of the icon data: window, id, current icon and tooltip.
unsafe fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let (status, tip) = tooltip();
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    data.uFlags = NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    let index = STATUSES.iter().position(|s| *s == status).unwrap_or(0);
    data.hIcon = ICONS[index].load(Ordering::Relaxed) as HICON;
    let tip: Vec<u16> = tip.encode_utf16().take(data.szTip.len() - 1).collect();
    data.szTip[..tip.len()].copy_from_slice(&tip);
    data
}

unsafe fn add(hwnd: HWND) -> bool {
    unsafe {
        let mut data = icon_data(hwnd);
        data.uFlags |= NIF_MESSAGE;
        data.uCallbackMessage = WM_TRAY;
        if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
            return false;
        }
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        Shell_NotifyIconW(NIM_SETVERSION, &data);
        true
    }
}

unsafe fn menu(hwnd: HWND, x: i32, y: i32) {
    let (_, header) = tooltip();
    let header = header.lines().next().unwrap_or("Mentis").to_owned();
    let header = format!("{header}   ·   {}", env!("CARGO_PKG_VERSION"));
    let check = |on: bool| if on { MF_CHECKED } else { 0 };
    let update = update().map(|v| {
        if german() {
            format!("Neue Version {v} herunterladen …")
        } else {
            format!("Download new version {v} …")
        }
    });
    let mut items = vec![(MF_STRING | MF_GRAYED, 0, header.as_str())];
    if let Some(update) = &update {
        items.push((MF_STRING, CMD_UPDATE, update.as_str()));
    }
    items.extend([
        (MF_SEPARATOR, 0, ""),
        (
            MF_STRING | MF_DEFAULT,
            CMD_SETTINGS,
            t("Einstellungen …", "Settings …"),
        ),
        (
            MF_STRING | check(autostart::enabled()),
            CMD_AUTOSTART,
            t("Mit Windows starten", "Start with Windows"),
        ),
        (
            MF_STRING | check(diagnose::enabled()),
            CMD_DIAGNOSE,
            t("Diagnose-Aufnahmen speichern", "Save diagnostic recordings"),
        ),
        (MF_STRING, CMD_LOG, t("Protokoll öffnen", "Open log")),
        (MF_SEPARATOR, 0, ""),
        (
            MF_STRING,
            CMD_UNINSTALL,
            t("Mentis entfernen …", "Remove Mentis …"),
        ),
        (MF_SEPARATOR, 0, ""),
        (MF_STRING, CMD_QUIT, t("Beenden", "Quit")),
    ]);
    let command = unsafe {
        let popup = CreatePopupMenu();
        for (flags, id, text) in items {
            let text = wide(text);
            AppendMenuW(popup, flags, id, text.as_ptr());
        }
        // Without these two calls the menu does not close when clicking elsewhere.
        SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            popup,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            x,
            y,
            0,
            hwnd,
            std::ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(popup);
        command as usize
    };
    run_command(hwnd, command);
}

/// A menu command; also reachable as WM_COMMAND, e.g. for tests.
fn run_command(hwnd: HWND, command: usize) {
    match command {
        CMD_AUTOSTART => {
            let on = !autostart::enabled();
            match autostart::set(on) {
                Ok(()) => log::line(if on {
                    "Autostart an."
                } else {
                    "Autostart aus."
                }),
                Err(e) => error(&format!(
                    "{}: {e}",
                    t(
                        "Autostart konnte nicht geändert werden",
                        "Could not change autostart"
                    )
                )),
            }
        }
        CMD_DIAGNOSE => diagnose::set(!diagnose::enabled()),
        CMD_LOG => {
            if !open(hwnd, &log::path().to_string_lossy()) {
                error(&format!(
                    "{}: {}",
                    t("Protokoll nicht gefunden", "Log not found"),
                    log::path().display()
                ));
            }
        }
        CMD_UPDATE => {
            if !open(hwnd, crate::update::PAGE) {
                error(&format!(
                    "{}: {}",
                    t("Browser nicht geöffnet", "Could not open the browser"),
                    crate::update::PAGE
                ));
            }
        }
        CMD_SETTINGS => crate::settings::open(),
        CMD_UNINSTALL => {
            if let Some((config_path, config)) = crate::settings::current() {
                crate::uninstall::run(hwnd, &config_path, &config);
            }
        }
        CMD_QUIT => quit(),
        _ => {}
    }
}

fn error(message: &str) {
    log::line(&format!("Fehler: {message}"));
    let (text, title) = (wide(message), wide("Mentis"));
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TRAY => {
                // NOTIFYICON_VERSION_4: event in the low word of lparam, anchor point in wparam.
                // Click (or Enter on the focused icon) opens the settings, right click the menu.
                let event = (lparam & 0xffff) as u32;
                match event {
                    NIN_SELECT | NIN_KEYSELECT => crate::settings::open(),
                    NIN_BALLOONUSERCLICK => run_command(hwnd, CMD_UPDATE),
                    WM_CONTEXTMENU => {
                        let x = (wparam & 0xffff) as i16 as i32;
                        let y = ((wparam >> 16) & 0xffff) as i16 as i32;
                        menu(hwnd, x, y);
                    }
                    _ => {}
                }
                0
            }
            WM_COMMAND => {
                run_command(hwnd, wparam & 0xffff);
                0
            }
            WM_REFRESH => {
                let data = icon_data(hwnd);
                Shell_NotifyIconW(NIM_MODIFY, &data);
                0
            }
            WM_UPDATE => {
                if wparam != 0
                    && let Some(version) = update()
                {
                    notify_update(hwnd, &version);
                }
                0
            }
            WM_CLOSE => {
                let data = icon_data(hwnd);
                Shell_NotifyIconW(NIM_DELETE, &data);
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            WM_SETTINGCHANGE if crate::theme::is_mode_change(lparam) => {
                crate::theme::refresh();
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                // Explorer restarted (or started after Mentis): the icon has to be added again.
                add(hwnd);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Builds an icon with alpha channel from `render`.
unsafe fn make_icon(size: i32, base: [u8; 3]) -> HICON {
    let pixels = icon::render(size as usize, base);
    unsafe {
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size, // top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let dc = GetDC(std::ptr::null_mut());
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(
            dc,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        ReleaseDC(std::ptr::null_mut(), dc);
        if color.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u32>(), pixels.len());
        // All-zero AND mask: the alpha channel of the color bitmap decides transparency.
        // Monochrome rows are padded to 16 bits.
        let row_bytes = (size as usize).div_ceil(16) * 2;
        let mask_bits = vec![0u8; size as usize * row_bytes];
        let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr().cast());
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        });
        DeleteObject(color as _);
        DeleteObject(mask as _);
        icon
    }
}
