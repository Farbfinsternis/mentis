//! First-start window: downloads the speech model that suits this PC, or lets the user pick an
//! existing model file. Runs its own message loop (before the tray starts, or from settings).

use crate::download::{self, Failure};
use crate::i18n::{german, t};
use crate::models::{self, CATALOG, Gpu, Model};
use crate::ui::{self, Fonts, STATIC_PATH, STATIC_PLAIN, set_text};
use crate::{config, log, theme};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::UI::Controls::{PBM_SETPOS, PBM_SETRANGE32, PROGRESS_CLASSW};
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BS_AUTORADIOBUTTON, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, DC_HASDEFID,
    DM_GETDEFID, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IsDialogMessageW,
    MSG, PostMessageW, PostQuitMessage, SW_SHOW, SendMessageW, SetForegroundWindow, ShowWindow,
    TranslateMessage, WM_APP, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_DPICHANGED, WS_GROUP,
    WS_TABSTOP,
};

/// IDOK and IDCANCEL, so Enter and Esc work through IsDialogMessage.
const ID_DOWNLOAD: usize = 1;
const ID_CANCEL: usize = 2;
const ID_FILE: usize = 10;
const ID_FOLDER: usize = 11;
const ID_RADIO: usize = 100;

const WM_PROGRESS: u32 = WM_APP + 20;
const WM_FINISHED: u32 = WM_APP + 21;

#[derive(Clone, Copy)]
struct Controls {
    window: HWND,
    heading: HWND,
    intro: HWND,
    hardware: HWND,
    radios: [HWND; 3],
    progress: HWND,
    status: HWND,
    source: HWND,
    location: HWND,
    folder: HWND,
    file: HWND,
    download: HWND,
    cancel: HWND,
}

#[derive(Default)]
struct State {
    config_path: PathBuf,
    /// Set while a download runs.
    cancel: Option<Arc<AtomicBool>>,
    /// The window closes as soon as the running download has stopped.
    closing: bool,
    error: bool,
    /// Time and byte count of the first progress report, for speed and remaining time.
    started: Option<(Instant, u64)>,
    result: Option<PathBuf>,
    fonts: Option<Fonts>,
}

thread_local! {
    static CONTROLS: Cell<Option<Controls>> = const { Cell::new(None) };
    // Borrowed only briefly: window calls below may re-enter the window procedure.
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Shows the window until a model is ready. `None`: the user closed it without a model.
pub fn run(
    config_path: &Path,
    config: &config::Config,
    gpu: Option<&Gpu>,
) -> Result<Option<PathBuf>, String> {
    STATE.set(State {
        config_path: config_path.to_owned(),
        ..State::default()
    });
    let window = ui::new_window("MentisSetup", "Mentis", Some(wndproc))?;
    let recommended = models::recommended(gpu, config.language().as_deref());
    let static_text = |text: &str| ui::child(window, "STATIC", text, STATIC_PLAIN, 0);
    let button = |text: &str, style: i32, id: usize| {
        ui::child(window, "BUTTON", text, style as u32 | WS_TABSTOP, id)
    };
    let mut radios = [std::ptr::null_mut(); 3];
    for (i, (slot, model)) in radios.iter_mut().zip(CATALOG.iter()).enumerate() {
        let group = if i == 0 { WS_GROUP | WS_TABSTOP } else { 0 };
        let mut label = format!(
            "{} – {} ({} MB)",
            model.name,
            model.summary(),
            model.megabytes()
        );
        if std::ptr::eq(model, recommended) {
            label.push_str(t(" · empfohlen", " · recommended"));
        }
        *slot = ui::child(
            window,
            "BUTTON",
            &label,
            BS_AUTORADIOBUTTON as u32 | group,
            ID_RADIO + i,
        );
        if std::ptr::eq(model, recommended) {
            unsafe { SendMessageW(*slot, BM_SETCHECK, 1, 0) };
        }
    }
    let controls = Controls {
        window,
        heading: static_text(t("Mentis einrichten", "Set up Mentis")),
        intro: static_text(t(
            "Mentis erkennt Sprache direkt auf diesem PC, ohne Cloud. Dafür lädt Mentis einmalig \
             ein Sprachmodell herunter; danach funktioniert die Erkennung auch ohne Internet.",
            "Mentis recognizes speech right on this PC, without the cloud. For that it downloads \
             a speech model once; after that, recognition also works offline.",
        )),
        hardware: static_text(&hardware_text(gpu)),
        radios,
        progress: ui::child_class(window, PROGRESS_CLASSW, "", 0, 0),
        status: static_text(""),
        source: static_text(""),
        location: ui::child(window, "STATIC", &ui::model_folder_text(), STATIC_PATH, 0),
        folder: button(t("Ändern …", "Change …"), BS_PUSHBUTTON, ID_FOLDER),
        file: button(
            t("Vorhandene Datei …", "Use existing file …"),
            BS_PUSHBUTTON | WS_GROUP as i32,
            ID_FILE,
        ),
        download: button(
            t("Herunterladen", "Download"),
            BS_DEFPUSHBUTTON,
            ID_DOWNLOAD,
        ),
        cancel: button(t("Abbrechen", "Cancel"), BS_PUSHBUTTON, ID_CANCEL),
    };
    unsafe {
        SendMessageW(controls.progress, PBM_SETRANGE32, 0, 1000);
        CONTROLS.set(Some(controls));
        update_source(&controls);
        layout(&controls, GetDpiForSystem(), None);
        theme::apply(window);
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(window, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    CONTROLS.set(None);
    Ok(STATE.take().result)
}

fn hardware_text(gpu: Option<&Gpu>) -> String {
    let gb = |bytes: u64| (bytes as f64 / f64::from(1 << 30)).round();
    match gpu {
        Some(g) if g.integrated && german() => {
            format!("Grafik: {} (im Prozessor integriert)", g.name)
        }
        Some(g) if g.integrated => format!("Graphics: {} (integrated)", g.name),
        Some(g) if german() => format!("Grafikkarte: {} ({} GB)", g.name, gb(g.memory)),
        Some(g) => format!("Graphics card: {} ({} GB)", g.name, gb(g.memory)),
        None => t(
            "Keine passende Grafikkarte gefunden – Mentis rechnet auf dem Prozessor.",
            "No suitable graphics card found – Mentis will use the processor.",
        )
        .into(),
    }
}

/// Positions all controls for `dpi` and sizes the window; centers it unless `origin` is given.
fn layout(c: &Controls, dpi: u32, origin: Option<(i32, i32)>) {
    let px = |v: i32| v * dpi as i32 / 96;
    let fonts = Fonts::new(dpi);
    let all = [
        c.heading,
        c.intro,
        c.hardware,
        c.radios[0],
        c.radios[1],
        c.radios[2],
        c.status,
        c.source,
        c.location,
        c.folder,
        c.file,
        c.download,
        c.cancel,
    ];
    for hwnd in all {
        let font = if hwnd == c.heading {
            fonts.heading
        } else {
            fonts.normal
        };
        ui::set_font(hwnd, font);
    }
    // The old fonts are deleted only after the controls switched to the new ones.
    STATE.with_borrow_mut(|s| s.fonts = Some(fonts));

    let place = ui::place;
    let (margin, width) = (px(24), px(600));
    let inner = width - 2 * margin;
    let mut y = px(18);
    place(c.heading, margin, y, inner, px(34));
    y += px(42);
    place(c.intro, margin, y, inner, px(40));
    y += px(50);
    place(c.hardware, margin, y, inner, px(20));
    y += px(30);
    for radio in c.radios {
        place(radio, margin, y, inner, px(24));
        y += px(28);
    }
    y += px(8);
    place(c.location, margin, y + px(4), inner - px(120), px(18));
    place(c.folder, width - margin - px(110), y, px(110), px(26));
    y += px(40);
    place(c.progress, margin, y, inner, px(14));
    y += px(22);
    place(c.status, margin, y, inner, px(40));
    y += px(44);
    place(c.source, margin, y, inner, px(18));
    y += px(30);
    let button = px(30);
    place(c.file, margin, y, px(180), button);
    place(c.cancel, width - margin - px(110), y, px(110), button);
    place(c.download, width - margin - px(230), y, px(112), button);
    y += button + px(20);
    ui::fit_window(c.window, width, y, dpi, origin);
}

fn selected(c: &Controls) -> &'static Model {
    let index = c
        .radios
        .iter()
        .position(|r| unsafe { SendMessageW(*r, BM_GETCHECK, 0, 0) } == 1)
        .unwrap_or(0);
    &CATALOG[index]
}

fn update_source(c: &Controls) {
    let model = selected(c);
    let text = if german() {
        format!(
            "Quelle: huggingface.co/handy-computer · Lizenz: {}",
            model.license
        )
    } else {
        format!(
            "Source: huggingface.co/handy-computer · License: {}",
            model.license
        )
    };
    set_text(c.source, &text);
}

fn show_error(c: &Controls, message: &str) {
    STATE.with_borrow_mut(|s| s.error = true);
    set_text(c.status, message);
}

fn choose_folder(c: &Controls) {
    let config_path = STATE.with_borrow(|s| s.config_path.clone());
    match ui::change_model_folder(c.window, &config_path) {
        Ok(false) => {}
        Ok(true) => {
            STATE.with_borrow_mut(|s| s.error = false);
            set_text(c.location, &ui::model_folder_text());
            set_text(c.status, "");
        }
        Err(e) => show_error(c, &e),
    }
}

fn enable_choice(c: &Controls, on: bool) {
    for hwnd in c.radios.iter().chain([&c.file, &c.folder, &c.download]) {
        unsafe { EnableWindow(*hwnd, i32::from(on)) };
    }
}

fn downloading() -> bool {
    STATE.with_borrow(|s| s.cancel.is_some())
}

fn start_download(c: &Controls) {
    let model = selected(c);
    let cancel = Arc::new(AtomicBool::new(false));
    STATE.with_borrow_mut(|s| {
        s.cancel = Some(cancel.clone());
        s.started = None;
        s.error = false;
    });
    enable_choice(c, false);
    set_text(
        c.status,
        t(
            "Verbinde mit huggingface.co …",
            "Connecting to huggingface.co …",
        ),
    );
    log::line(&format!("Lade Modell {} herunter.", model.file));
    let window = c.window as isize;
    std::thread::spawn(move || {
        let mut shown = Instant::now() - Duration::from_secs(1);
        let result = download::download(model, &cancel, |done, total| {
            if shown.elapsed() >= Duration::from_millis(100) || done == total {
                shown = Instant::now();
                unsafe { PostMessageW(window as HWND, WM_PROGRESS, done as usize, total as isize) };
            }
        });
        let result = Box::into_raw(Box::new(result));
        unsafe { PostMessageW(window as HWND, WM_FINISHED, 0, result as isize) };
    });
}

fn show_progress(c: &Controls, done: u64, total: u64) {
    let started = STATE.with_borrow_mut(|s| *s.started.get_or_insert((Instant::now(), done)));
    unsafe {
        SendMessageW(
            c.progress,
            PBM_SETPOS,
            (done * 1000 / total.max(1)) as usize,
            0,
        )
    };
    let mb = models::megabytes;
    let seconds = started.0.elapsed().as_secs_f64();
    let speed = (done - started.1) as f64 / seconds.max(0.001);
    let mut text = if german() {
        format!("{} von {} MB", mb(done), mb(total))
    } else {
        format!("{} of {} MB", mb(done), mb(total))
    };
    if seconds >= 1.0 && speed > 0.0 {
        let left = ((total - done) as f64 / speed).ceil() as u64;
        let left = if left >= 90 {
            format!("{} min", left.div_ceil(60))
        } else {
            format!("{left} s")
        };
        text += &format!(
            " · {} MB/s · {} {left}",
            ui::number(speed / 1048576.0),
            t("noch etwa", "about")
        );
        if !german() {
            text += " left";
        }
    }
    set_text(c.status, &text);
}

fn finished(c: &Controls, result: Result<PathBuf, Failure>) {
    let closing = STATE.with_borrow_mut(|s| {
        s.cancel = None;
        s.closing
    });
    match result {
        Ok(path) => {
            log::line(&format!("Modell bereit: {}", path.display()));
            STATE.with_borrow_mut(|s| s.result = Some(path));
            unsafe { DestroyWindow(c.window) };
            return;
        }
        _ if closing => {
            unsafe { DestroyWindow(c.window) };
            return;
        }
        Err(Failure::Cancelled) => set_text(
            c.status,
            t(
                "Download angehalten. Beim nächsten Versuch geht es an dieser Stelle weiter.",
                "Download paused. The next attempt continues where it stopped.",
            ),
        ),
        Err(Failure::Error(message)) => {
            log::line(&format!("Download fehlgeschlagen: {message}"));
            show_error(c, &message);
            set_text(c.download, t("Erneut versuchen", "Try again"));
        }
    }
    enable_choice(c, true);
}

fn choose_file(c: &Controls) {
    let Some(path) = ui::choose_model_file(c.window) else {
        return;
    };
    let config_path = STATE.with_borrow(|s| s.config_path.clone());
    match config::set_path(&config_path, "model", &path) {
        Ok(()) => {
            log::line(&format!("Vorhandenes Modell gewählt: {}", path.display()));
            STATE.with_borrow_mut(|s| s.result = Some(path));
            unsafe { DestroyWindow(c.window) };
        }
        Err(e) => show_error(c, &e),
    }
}

fn close(c: &Controls) {
    let cancel = STATE.with_borrow_mut(|s| {
        s.closing = true;
        s.cancel.clone()
    });
    match cancel {
        // The download thread reports back, then the window closes.
        Some(flag) => flag.store(true, Ordering::Relaxed),
        None => unsafe {
            DestroyWindow(c.window);
        },
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(c) = CONTROLS.get() else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    let label = |control: HWND| {
        if control == c.status && STATE.with_borrow(|s| s.error) {
            Some(theme::colors().error)
        } else if control == c.source {
            Some(theme::colors().quiet)
        } else {
            None
        }
    };
    if let Some(result) = ui::theme_message(hwnd, msg, wparam, lparam, label) {
        return result;
    }
    match msg {
        WM_COMMAND => {
            let id = wparam & 0xffff;
            match id {
                ID_DOWNLOAD if !downloading() => start_download(&c),
                ID_CANCEL if downloading() => {
                    if let Some(flag) = STATE.with_borrow(|s| s.cancel.clone()) {
                        flag.store(true, Ordering::Relaxed);
                    }
                }
                ID_CANCEL => close(&c),
                ID_FILE if !downloading() => choose_file(&c),
                ID_FOLDER if !downloading() => choose_folder(&c),
                _ if (ID_RADIO..ID_RADIO + c.radios.len()).contains(&id) => update_source(&c),
                _ => {}
            }
            0
        }
        DM_GETDEFID => ((DC_HASDEFID as usize) << 16 | ID_DOWNLOAD) as LRESULT,
        WM_PROGRESS => {
            show_progress(&c, wparam as u64, lparam as u64);
            0
        }
        WM_FINISHED => {
            // SAFETY: posted once by the download thread with a leaked Box.
            let result = unsafe { Box::from_raw(lparam as *mut Result<PathBuf, Failure>) };
            finished(&c, *result);
            0
        }
        WM_DPICHANGED => {
            // SAFETY: lparam points to the suggested window rectangle.
            let suggested = unsafe { &*(lparam as *const RECT) };
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            layout(&c, dpi, Some((suggested.left, suggested.top)));
            0
        }
        WM_CLOSE => {
            close(&c);
            0
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
