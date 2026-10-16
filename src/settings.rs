//! Settings window: every option of config.toml without editing the file. Opens from the tray
//! menu and runs modeless on the tray thread. Saving writes the file and restarts Mentis, because
//! model, key and microphone are set up at startup.

use crate::config::{self, Config};
use crate::i18n::{german, key_name, t, windows_language};
use crate::models::{self, CATALOG, Gpu};
use crate::ui::{self, Fonts, STATIC_PATH, STATIC_PLAIN, set_text};
use crate::{audio, autostart, log, setup, theme, tray};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::UI::Controls::{BST_CHECKED, EM_SETCUEBANNER};
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CB_ADDSTRING,
    CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL, CB_SETDROPPEDWIDTH, CBS_DROPDOWNLIST, DC_HASDEFID,
    DM_GETDEFID, DefWindowProcW, DestroyWindow, DispatchMessageW, ES_AUTOHSCROLL, ES_NUMBER,
    GetMessageW, IsDialogMessageW, MSG, PostQuitMessage, SW_RESTORE, SW_SHOW, SW_SHOWNORMAL,
    SendMessageW, SetForegroundWindow, ShowWindow, TranslateMessage, WM_CLOSE, WM_COMMAND,
    WM_DESTROY, WM_DPICHANGED, WS_BORDER, WS_TABSTOP, WS_VSCROLL,
};

const ID_SAVE: usize = 1; // IDOK: Enter saves
const ID_CANCEL: usize = 2; // IDCANCEL: Esc closes
const ID_MODEL_FILE: usize = 10;
const ID_FOLDER: usize = 11;
const ID_DOWNLOAD: usize = 12;
const ID_CONFIG_FILE: usize = 13;

/// Input languages offered besides "like Windows" and "detect"; Whisper knows many more,
/// which can still be set in the config file.
const LANGUAGES: [(&str, &str); 18] = [
    ("de", "Deutsch"),
    ("en", "English"),
    ("fr", "Français"),
    ("es", "Español"),
    ("it", "Italiano"),
    ("nl", "Nederlands"),
    ("pl", "Polski"),
    ("pt", "Português"),
    ("cs", "Čeština"),
    ("da", "Dansk"),
    ("sv", "Svenska"),
    ("fi", "Suomi"),
    ("tr", "Türkçe"),
    ("ru", "Русский"),
    ("uk", "Українська"),
    ("ja", "日本語"),
    ("zh", "中文"),
    ("ko", "한국어"),
];

const KEYS: [&str; 6] = [
    "RightCtrl",
    "RightShift",
    "CapsLock",
    "ScrollLock",
    "Pause",
    "Insert",
];

/// A drop-down list and the config value behind each entry.
#[derive(Clone, Copy)]
struct Choice {
    combo: HWND,
}

#[derive(Clone, Copy)]
struct Controls {
    window: HWND,
    heading: HWND,
    sections: [HWND; 4],
    labels: [HWND; 9],
    hotkey: Choice,
    microphone: Choice,
    style: Choice,
    language: Choice,
    prompt: HWND,
    model: Choice,
    model_file: HWND,
    folder_text: HWND,
    folder: HWND,
    download: HWND,
    backend: Choice,
    sounds: HWND,
    overlay: HWND,
    live_preview: HWND,
    autostart: HWND,
    update_check: HWND,
    insert: Choice,
    trailing_space: HWND,
    max_seconds: HWND,
    seconds: HWND,
    note: HWND,
    status: HWND,
    config_file: HWND,
    save: HWND,
    cancel: HWND,
}

struct Context {
    config_path: PathBuf,
    /// The settings Mentis runs with; saving compares against them.
    config: Config,
    gpu: Option<Gpu>,
}

#[derive(Default)]
struct State {
    /// Config value of every entry, per combo box.
    values: Vec<(HWND, Vec<String>)>,
    error: bool,
    fonts: Option<Fonts>,
}

thread_local! {
    static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
    static CONTROLS: Cell<Option<Controls>> = const { Cell::new(None) };
    // Borrowed only briefly: window calls may re-enter the window procedure.
    static STATE: RefCell<State> = RefCell::new(State::default());
    // Test helper --settings: the window runs without the tray and ends the process when closed.
    static ALONE: Cell<bool> = const { Cell::new(false) };
}

/// Remembers what the settings window needs; call on the tray thread at startup.
pub fn init(config_path: &Path, config: &Config, gpu: Option<Gpu>) {
    CONTEXT.set(Some(Context {
        config_path: config_path.to_owned(),
        config: config.clone(),
        gpu,
    }));
}

/// Test helper `--settings`: only this window with its own message loop, also while Mentis runs.
pub fn run_alone(config_path: &Path, config: &Config, gpu: Option<Gpu>) {
    ALONE.set(true);
    init(config_path, config, gpu);
    open();
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        if dialog_message(&msg) {
            continue;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Configuration file and the settings Mentis runs with.
pub fn current() -> Option<(PathBuf, Config)> {
    CONTEXT.with_borrow(|c| {
        c.as_ref()
            .map(|c| (c.config_path.clone(), c.config.clone()))
    })
}

/// Tab and Enter handling for the open window; call from the message loop first.
pub fn dialog_message(msg: &MSG) -> bool {
    CONTROLS
        .get()
        .is_some_and(|c| unsafe { IsDialogMessageW(c.window, msg) } != 0)
}

/// Opens the window, or brings it to the front if it is already open.
pub fn open() {
    if let Some(c) = CONTROLS.get() {
        unsafe {
            ShowWindow(c.window, SW_RESTORE);
            SetForegroundWindow(c.window);
        }
        return;
    }
    let Some(config) = CONTEXT.with_borrow(|c| c.as_ref().map(|c| c.config.clone())) else {
        return;
    };
    match create(&config) {
        Ok(c) => unsafe {
            ShowWindow(c.window, SW_SHOW);
            SetForegroundWindow(c.window);
        },
        Err(e) => log::line(&format!("Einstellungen: {e}")),
    }
}

fn create(config: &Config) -> Result<Controls, String> {
    let window = ui::new_window(
        "MentisSettings",
        t("Mentis – Einstellungen", "Mentis – Settings"),
        Some(wndproc),
    )?;
    STATE.set(State::default());
    let label = |text: &str| ui::child(window, "STATIC", text, STATIC_PLAIN, 0);
    let combo = |entries: Vec<(String, String)>, selected: &str| {
        let combo = ui::child(
            window,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32 | WS_VSCROLL | WS_TABSTOP,
            0,
        );
        fill(combo, entries, selected);
        Choice { combo }
    };
    let check = |text: &str, on: bool| {
        let hwnd = ui::child(
            window,
            "BUTTON",
            text,
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            0,
        );
        unsafe { SendMessageW(hwnd, BM_SETCHECK, usize::from(on), 0) };
        hwnd
    };
    let button = |text: &str, style: i32, id: usize| {
        ui::child(window, "BUTTON", text, style as u32 | WS_TABSTOP, id)
    };
    let edit = |text: &str, style: i32| {
        ui::child(
            window,
            "EDIT",
            text,
            (ES_AUTOHSCROLL | style) as u32 | WS_BORDER | WS_TABSTOP,
            0,
        )
    };

    // Created in tab order: left column, right column, buttons.
    let heading = label(t("Einstellungen", "Settings"));
    let sections = [
        label(t("Diktat", "Dictation")),
        label(t("Anzeige und Ton", "Display and sound")),
        label(t("Spracherkennung", "Speech recognition")),
        label(t("Erweitert", "Advanced")),
    ];
    let labels = [
        label(t("Diktiertaste", "Dictation key")),
        label(t("Mikrofon", "Microphone")),
        label(t("Schreibstil", "Writing style")),
        label(t("Eingabesprache", "Input language")),
        label(t("Fachbegriffe", "Special terms")),
        label(t("Modell", "Model")),
        label(t("Rechnen auf", "Compute on")),
        label(t("Einfügen", "Insert text")),
        label(t("Längste Aufnahme", "Longest recording")),
    ];
    let hotkey = combo(hotkey_entries(&config.hotkey), &config.hotkey);
    let microphone = combo(microphone_entries(&config.microphone), &config.microphone);
    let style = combo(
        vec![
            (
                "clean".into(),
                t(
                    "Sauber – ohne Füllwörter wie „äh“",
                    "Clean – without fillers like “uh”",
                )
                .into(),
            ),
            (
                "exact".into(),
                t(
                    "Exakt – alles, was gesagt wird",
                    "Exact – everything that is said",
                )
                .into(),
            ),
        ],
        &config.style,
    );
    let language = combo(language_entries(&config.language), &config.language);
    let prompt = edit(&config.prompt, 0);
    let cue = ui::wide(t(
        "Namen, Produkte, z. B. ComfyUI, Mentis",
        "Names, products, e.g. ComfyUI, Mentis",
    ));
    unsafe { SendMessageW(prompt, EM_SETCUEBANNER, 1, cue.as_ptr() as isize) };
    let sounds = check(
        t(
            "Signaltöne bei Beginn und Ende der Aufnahme",
            "Sounds when recording starts and ends",
        ),
        config.sounds,
    );
    let overlay = check(
        t("Anzeigefeld über der Taskleiste", "Panel above the taskbar"),
        config.overlay,
    );
    let live_preview = check(
        t(
            "Text schon beim Sprechen zeigen (mit Grafikkarte)",
            "Show text while speaking (with graphics card)",
        ),
        config.live_preview,
    );
    let autostart = check(
        t("Mit Windows starten", "Start with Windows"),
        autostart::enabled(),
    );
    let update_check = check(
        t(
            "Nach neuen Versionen suchen (github.com)",
            "Check for new versions (github.com)",
        ),
        config.update_check,
    );
    let model = combo(Vec::new(), "");
    let model_file = button(
        t("Eigene Datei …", "Own file …"),
        BS_PUSHBUTTON,
        ID_MODEL_FILE,
    );
    let download = button(
        t("Modelle herunterladen …", "Download models …"),
        BS_PUSHBUTTON,
        ID_DOWNLOAD,
    );
    let folder_text = ui::child(window, "STATIC", "", STATIC_PATH, 0);
    let folder = button(t("Ändern …", "Change …"), BS_PUSHBUTTON, ID_FOLDER);
    let backend = combo(
        vec![
            (
                "auto".into(),
                t(
                    "Automatisch (Grafikkarte, sonst Prozessor)",
                    "Automatic (graphics card, else processor)",
                )
                .into(),
            ),
            (
                "vulkan".into(),
                t("Grafikkarte (Vulkan)", "Graphics card (Vulkan)").into(),
            ),
            ("cpu".into(), t("Prozessor", "Processor").into()),
        ],
        &config.backend,
    );
    let insert = combo(
        vec![
            (
                "clipboard".into(),
                t(
                    "Über die Zwischenablage (Strg+V)",
                    "Via the clipboard (Ctrl+V)",
                )
                .into(),
            ),
            (
                "type".into(),
                t("Zeichen einzeln tippen", "Type character by character").into(),
            ),
        ],
        &config.insert,
    );
    let trailing_space = check(
        t(
            "Leerzeichen nach jedem Diktat",
            "Space after each dictation",
        ),
        config.trailing_space,
    );
    let max_seconds = edit(&config.max_seconds.to_string(), ES_NUMBER);
    let seconds = label(t("Sekunden", "seconds"));
    let note = label(t(
        "Beim Speichern startet Mentis kurz neu, damit alles übernommen wird.",
        "Saving restarts Mentis briefly so that everything takes effect.",
    ));
    let status = label("");
    let config_file = button(
        t("Konfigurationsdatei öffnen", "Open configuration file"),
        BS_PUSHBUTTON,
        ID_CONFIG_FILE,
    );
    let save = button(t("Speichern", "Save"), BS_DEFPUSHBUTTON, ID_SAVE);
    let cancel = button(t("Abbrechen", "Cancel"), BS_PUSHBUTTON, ID_CANCEL);

    let controls = Controls {
        window,
        heading,
        sections,
        labels,
        hotkey,
        microphone,
        style,
        language,
        prompt,
        model,
        model_file,
        folder_text,
        folder,
        download,
        backend,
        sounds,
        overlay,
        live_preview,
        autostart,
        update_check,
        insert,
        trailing_space,
        max_seconds,
        seconds,
        note,
        status,
        config_file,
        save,
        cancel,
    };
    refresh_models(&controls, &config.model.to_string_lossy());
    CONTROLS.set(Some(controls));
    layout(&controls, unsafe { GetDpiForSystem() }, None);
    theme::apply(window);
    Ok(controls)
}

/// Replaces the entries of `combo` and selects `selected` (or the first entry).
fn fill(combo: HWND, entries: Vec<(String, String)>, selected: &str) {
    unsafe { SendMessageW(combo, CB_RESETCONTENT, 0, 0) };
    let mut index = 0;
    let mut values = Vec::new();
    for (i, (value, text)) in entries.into_iter().enumerate() {
        unsafe { SendMessageW(combo, CB_ADDSTRING, 0, ui::wide(&text).as_ptr() as isize) };
        if value.eq_ignore_ascii_case(selected) {
            index = i;
        }
        values.push(value);
    }
    unsafe { SendMessageW(combo, CB_SETCURSEL, index, 0) };
    STATE.with_borrow_mut(|s| {
        s.values.retain(|(hwnd, _)| *hwnd != combo);
        s.values.push((combo, values));
    });
}

fn value(choice: Choice) -> String {
    let index = unsafe { SendMessageW(choice.combo, CB_GETCURSEL, 0, 0) };
    STATE.with_borrow(|s| {
        s.values
            .iter()
            .find(|(hwnd, _)| *hwnd == choice.combo)
            .and_then(|(_, values)| values.get(usize::try_from(index).ok()?).cloned())
            .unwrap_or_default()
    })
}

fn checked(hwnd: HWND) -> bool {
    let state = unsafe { SendMessageW(hwnd, BM_GETCHECK, 0, 0) };
    state == BST_CHECKED as isize
}

fn hotkey_entries(current: &str) -> Vec<(String, String)> {
    let mut keys: Vec<String> = KEYS.iter().map(|k| k.to_string()).collect();
    keys.extend((1..=24).map(|n| format!("F{n}")));
    if !keys.iter().any(|k| k.eq_ignore_ascii_case(current)) {
        keys.push(current.to_owned());
    }
    keys.into_iter()
        .map(|k| {
            let name = key_name(&k);
            (k, name)
        })
        .collect()
}

fn microphone_entries(current: &str) -> Vec<(String, String)> {
    let mut entries = vec![(
        String::new(),
        t("Windows-Standardmikrofon", "Windows default microphone").to_owned(),
    )];
    let devices = audio::device_names();
    // The config may hold only part of a name ("USB"); keep it selectable as it is.
    if !current.is_empty() && !devices.iter().any(|d| d == current) {
        entries.push((current.to_owned(), current.to_owned()));
    }
    entries.extend(devices.into_iter().map(|d| (d.clone(), d)));
    entries
}

fn language_name(code: &str) -> String {
    LANGUAGES
        .iter()
        .find(|(c, _)| *c == code)
        .map_or_else(|| code.to_owned(), |(_, name)| name.to_string())
}

fn language_entries(current: &str) -> Vec<(String, String)> {
    let windows = language_name(&windows_language());
    let mut entries = vec![
        (
            String::new(),
            if german() {
                format!("Wie Windows ({windows})")
            } else {
                format!("Same as Windows ({windows})")
            },
        ),
        (
            "auto".into(),
            t(
                "Automatisch erkennen (jedes Diktat)",
                "Detect automatically (each dictation)",
            )
            .into(),
        ),
    ];
    entries.extend(
        LANGUAGES
            .iter()
            .map(|(c, n)| (c.to_string(), n.to_string())),
    );
    let current = current.trim();
    if !current.is_empty() && !entries.iter().any(|(c, _)| c == current) {
        entries.push((current.to_owned(), current.to_owned()));
    }
    entries
}

/// Model choices: automatic, every catalog model in the model folder, and a custom file.
fn refresh_models(c: &Controls, selected: &str) {
    let (gpu, language) = CONTEXT.with_borrow(|ctx| {
        ctx.as_ref()
            .map(|x| (x.gpu.clone(), x.config.language()))
            .unwrap_or_default()
    });
    let auto = models::pick_installed(gpu.as_ref(), language.as_deref())
        .and_then(|path| CATALOG.iter().find(|m| m.path() == path))
        .map(|m| m.name);
    let mut entries = vec![(
        String::new(),
        match auto {
            Some(name) if german() => format!("Automatisch ({name})"),
            Some(name) => format!("Automatic ({name})"),
            None => t("Automatisch", "Automatic").into(),
        },
    )];
    for model in CATALOG.iter().filter(|m| m.installed()) {
        entries.push((
            model.path().to_string_lossy().into_owned(),
            model.name.into(),
        ));
    }
    if !selected.is_empty()
        && !entries
            .iter()
            .any(|(v, _)| v.eq_ignore_ascii_case(selected))
    {
        let name = Path::new(selected)
            .file_name()
            .map_or_else(|| selected.to_owned(), |n| n.to_string_lossy().into_owned());
        entries.push((selected.to_owned(), name));
    }
    fill(c.model.combo, entries, selected);
    set_text(c.folder_text, &ui::model_folder_text());
}

/// Positions all controls for `dpi` and sizes the window; centers it unless `origin` is given.
fn layout(c: &Controls, dpi: u32, origin: Option<(i32, i32)>) {
    let px = |v: i32| v * dpi as i32 / 96;
    let fonts = Fonts::new(dpi);
    let place = ui::place;
    let all = [
        c.labels.as_slice(),
        &[
            c.hotkey.combo,
            c.microphone.combo,
            c.style.combo,
            c.language.combo,
            c.prompt,
            c.model.combo,
            c.model_file,
            c.folder_text,
            c.folder,
            c.download,
            c.backend.combo,
            c.sounds,
            c.overlay,
            c.live_preview,
            c.autostart,
            c.update_check,
            c.insert.combo,
            c.trailing_space,
            c.max_seconds,
            c.seconds,
            c.note,
            c.status,
            c.config_file,
            c.save,
            c.cancel,
        ],
    ]
    .concat();
    for hwnd in all {
        ui::set_font(hwnd, fonts.normal);
    }
    for hwnd in c.sections {
        ui::set_font(hwnd, fonts.bold);
    }
    ui::set_font(c.heading, fonts.heading);
    for choice in [
        c.hotkey,
        c.microphone,
        c.style,
        c.language,
        c.model,
        c.backend,
        c.insert,
    ] {
        unsafe { SendMessageW(choice.combo, CB_SETDROPPEDWIDTH, px(380) as usize, 0) };
    }
    STATE.with_borrow_mut(|s| s.fonts = Some(fonts));

    let (margin, gap, width) = (px(24), px(32), px(880));
    let column = (width - 2 * margin - gap) / 2;
    let label_width = px(130);
    let row = px(34);
    let field = column - label_width - px(6);
    // Label and control in one row; drop-down lists get room for their open list.
    let field_row = |x: i32, y: i32, label: HWND, control: HWND, w: i32| {
        place(label, x, y + px(4), label_width, px(20));
        place(control, x + label_width + px(6), y, w, px(300));
    };
    let section = |hwnd: HWND, x: i32, y: i32| place(hwnd, x, y, column, px(22));
    let line = |hwnd: HWND, x: i32, y: i32| place(hwnd, x, y, column, px(24));

    place(c.heading, margin, px(16), width - 2 * margin, px(34));
    let top = px(64);

    // Left column.
    let x = margin;
    let mut y = top;
    section(c.sections[0], x, y);
    y += px(30);
    for (label, control) in [
        (c.labels[0], c.hotkey.combo),
        (c.labels[1], c.microphone.combo),
        (c.labels[2], c.style.combo),
        (c.labels[3], c.language.combo),
    ] {
        field_row(x, y, label, control, field);
        y += row;
    }
    place(c.labels[4], x, y + px(4), label_width, px(20));
    place(c.prompt, x + label_width + px(6), y, field, px(24));
    y += row + px(14);
    section(c.sections[1], x, y);
    y += px(30);
    for hwnd in [
        c.sounds,
        c.overlay,
        c.live_preview,
        c.autostart,
        c.update_check,
    ] {
        line(hwnd, x, y);
        y += px(28);
    }
    let left_bottom = y;

    // Right column.
    let x = margin + column + gap;
    let mut y = top;
    section(c.sections[2], x, y);
    y += px(30);
    field_row(x, y, c.labels[5], c.model.combo, field);
    y += row;
    // Below the model: own file and download, side by side under the drop-down list.
    let file_width = (field - px(6)) * 2 / 5;
    let buttons_x = x + label_width + px(6);
    place(c.model_file, buttons_x, y - px(1), file_width, px(26));
    place(
        c.download,
        buttons_x + file_width + px(6),
        y - px(1),
        field - file_width - px(6),
        px(26),
    );
    y += row;
    place(c.folder_text, x, y + px(4), column - px(96), px(20));
    place(c.folder, x + column - px(90), y - px(1), px(90), px(26));
    y += row;
    field_row(x, y, c.labels[6], c.backend.combo, field);
    y += row + px(14);
    section(c.sections[3], x, y);
    y += px(30);
    field_row(x, y, c.labels[7], c.insert.combo, field);
    y += row;
    line(c.trailing_space, x, y);
    y += px(30);
    place(c.labels[8], x, y + px(4), label_width, px(20));
    place(c.max_seconds, x + label_width + px(6), y, px(64), px(24));
    place(
        c.seconds,
        x + label_width + px(76),
        y + px(4),
        px(120),
        px(20),
    );
    let right_bottom = y + row;

    let mut y = left_bottom.max(right_bottom) + px(10);
    place(c.status, margin, y, width - 2 * margin, px(20));
    y += px(24);
    place(c.note, margin, y, width - 2 * margin, px(20));
    y += px(34);
    let button = px(30);
    place(c.config_file, margin, y, px(220), button);
    place(c.cancel, width - margin - px(110), y, px(110), button);
    place(c.save, width - margin - px(230), y, px(112), button);
    y += button + px(20);
    ui::fit_window(c.window, width, y, dpi, origin);
}

fn show_error(c: &Controls, message: &str) {
    STATE.with_borrow_mut(|s| s.error = true);
    set_text(c.status, message);
}

fn config_path() -> PathBuf {
    CONTEXT.with_borrow(|c| {
        c.as_ref()
            .map(|c| c.config_path.clone())
            .unwrap_or_default()
    })
}

/// The settings as shown in the window.
fn read(c: &Controls) -> Result<Config, String> {
    let max_seconds = ui::text(c.max_seconds)
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|s| (1..=600).contains(s))
        .ok_or_else(|| {
            t(
                "Längste Aufnahme: bitte 1 bis 600 Sekunden eintragen.",
                "Longest recording: please enter 1 to 600 seconds.",
            )
            .to_owned()
        })?;
    let dir = models::dir();
    Ok(Config {
        model: PathBuf::from(value(c.model)),
        models_dir: if dir == models::default_dir() {
            PathBuf::new()
        } else {
            dir
        },
        language: value(c.language),
        hotkey: value(c.hotkey),
        backend: value(c.backend),
        insert: value(c.insert),
        microphone: value(c.microphone),
        prompt: ui::text(c.prompt).trim().to_owned(),
        style: value(c.style),
        sounds: checked(c.sounds),
        overlay: checked(c.overlay),
        live_preview: checked(c.live_preview),
        trailing_space: checked(c.trailing_space),
        max_seconds,
        update_check: checked(c.update_check),
    })
}

fn save(c: &Controls) {
    let Some(running) = CONTEXT.with_borrow(|x| x.as_ref().map(|x| x.config.clone())) else {
        return;
    };
    let result = read(c).and_then(|new| {
        config::save(&config_path(), &new)?;
        if checked(c.autostart) != autostart::enabled() {
            autostart::set(checked(c.autostart))?;
        }
        Ok(new)
    });
    match result {
        Err(e) => show_error(c, &e),
        Ok(new) if new == running => {
            unsafe { DestroyWindow(c.window) };
        }
        Ok(_) => {
            log::line("Einstellungen gespeichert, Mentis startet neu.");
            unsafe { DestroyWindow(c.window) };
            tray::restart();
        }
    }
}

fn choose_model_file(c: &Controls) {
    if let Some(path) = ui::choose_model_file(c.window) {
        refresh_models(c, &path.to_string_lossy());
    }
}

fn change_folder(c: &Controls) {
    let selected = value(c.model);
    match ui::change_model_folder(c.window, &config_path()) {
        Ok(true) => {
            STATE.with_borrow_mut(|s| s.error = false);
            set_text(c.status, "");
            refresh_models(c, &selected);
        }
        Ok(false) => {}
        Err(e) => show_error(c, &e),
    }
}

/// The setup window, for models that are not in the model folder yet.
fn download_models(c: &Controls) {
    let Some((config, gpu)) =
        CONTEXT.with_borrow(|x| x.as_ref().map(|x| (x.config.clone(), x.gpu.clone())))
    else {
        return;
    };
    let selected = value(c.model);
    unsafe { EnableWindow(c.window, 0) };
    let result = setup::run(&config_path(), &config, gpu.as_ref());
    unsafe {
        EnableWindow(c.window, 1);
        SetForegroundWindow(c.window);
    }
    match result {
        Ok(Some(path)) => refresh_models(c, &path.to_string_lossy()),
        Ok(None) => refresh_models(c, &selected),
        Err(e) => show_error(c, &e),
    }
}

fn open_config_file(c: &Controls) {
    let (verb, file) = (ui::wide("open"), ui::wide(&config_path().to_string_lossy()));
    unsafe {
        ShellExecuteW(
            c.window,
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(c) = CONTROLS.get() else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    let label = |control: HWND| {
        if control == c.status && STATE.with_borrow(|s| s.error) {
            Some(theme::colors().error)
        } else if control == c.note {
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
            match wparam & 0xffff {
                ID_SAVE => save(&c),
                ID_CANCEL => unsafe {
                    DestroyWindow(c.window);
                },
                ID_MODEL_FILE => choose_model_file(&c),
                ID_FOLDER => change_folder(&c),
                ID_DOWNLOAD => download_models(&c),
                ID_CONFIG_FILE => open_config_file(&c),
                _ => {}
            }
            0
        }
        DM_GETDEFID => ((DC_HASDEFID as usize) << 16 | ID_SAVE) as LRESULT,
        WM_DPICHANGED => {
            // SAFETY: lparam points to the suggested window rectangle.
            let suggested = unsafe { &*(lparam as *const RECT) };
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            layout(&c, dpi, Some((suggested.left, suggested.top)));
            0
        }
        WM_CLOSE => {
            unsafe { DestroyWindow(c.window) };
            0
        }
        WM_DESTROY => {
            CONTROLS.set(None);
            STATE.set(State::default());
            if ALONE.get() {
                unsafe { PostQuitMessage(0) };
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
