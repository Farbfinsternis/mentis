// No console window: Mentis lives in the notification area. The test helpers attach to the
// terminal they were started from.
#![windows_subsystem = "windows"]

mod app;
mod audio;
mod autostart;
mod config;
mod diagnose;
mod download;
mod folder;
mod hotkey;
mod i18n;
mod icon;
mod insert;
mod log;
mod migrate;
mod models;
mod overlay;
mod settings;
mod setup;
mod transcriber;
mod tray;
mod ui;
mod uninstall;
mod update;

use i18n::{german, t};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MESSAGEBOX_STYLE, MSG,
    MessageBoxW, TranslateMessage,
};

const TEST_HELPERS: [&str; 5] = [
    "--transcribe",
    "--paste-test",
    "--devices",
    "--download",
    "--setup",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = args.iter().any(|a| TEST_HELPERS.contains(&a.as_str()));
    if cli {
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
        log::to_console(true);
    }
    if let Err(e) = run(&args, cli) {
        log::line(&format!("Fehler: {e}"));
        if !cli {
            message_box(&e, MB_ICONERROR);
        }
        std::process::exit(1);
    }
}

pub fn message_box(text: &str, style: MESSAGEBOX_STYLE) {
    let text: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let title: Vec<u16> = "Mentis\0".encode_utf16().collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | style,
        )
    };
}

fn run(args: &[String], cli: bool) -> Result<(), String> {
    let config_path = match args.iter().position(|a| a == "--config") {
        Some(i) => PathBuf::from(args.get(i + 1).ok_or("--config braucht einen Pfad")?),
        None => config::default_path(),
    };
    if cli {
        return run_test_helper(args, &config_path);
    }

    let name: Vec<u16> = "Local\\MentisDictation\0".encode_utf16().collect();
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if mutex.is_null() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        message_box(
            t(
                "Mentis läuft bereits. Du findest es im Infobereich der Taskleiste (unten rechts, \
                 eventuell hinter dem Pfeil ^).",
                "Mentis is already running. You can find it in the notification area of the \
                 taskbar (bottom right, possibly behind the ^ arrow).",
            ),
            MB_ICONINFORMATION,
        );
        return Ok(());
    }
    if migrate::echo_running() {
        message_box(
            t(
                "Echo läuft noch. Echo heißt jetzt Mentis: Bitte Echo beenden (Rechtsklick auf \
                 das Symbol im Infobereich > Beenden) und Mentis dann erneut starten.",
                "Echo is still running. Echo is now called Mentis: please quit Echo (right-click \
                 its icon in the notification area > Quit), then start Mentis again.",
            ),
            MB_ICONINFORMATION,
        );
        return Ok(());
    }
    // Before the log opens: Echo's log folder becomes Mentis' log folder.
    let takeover = migrate::from_echo();
    // Without a writable log Mentis still works; status lines are then simply dropped.
    let _ = log::open_file();
    log::line(&format!("Mentis {} startet.", env!("CARGO_PKG_VERSION")));
    for line in &takeover.lines {
        log::line(line);
    }
    if takeover.happened() {
        message_box(&takeover.hint(), MB_ICONINFORMATION);
    }
    let config = config::load(&config_path)?;
    models::set_dir(Some(config.models_dir.clone()));
    log::line(&format!("Konfiguration: {}", config_path.display()));
    diagnose::init();

    // Per-monitor DPI awareness comes from the manifest (build.rs), before any window exists.
    let gpu = transcriber::init_backends(&config)?;
    log::line(&match &gpu {
        Some(g) => format!("Grafik: {} ({} MB)", g.name, g.memory >> 20),
        None => "Keine Grafikkarte, Erkennung auf der CPU.".into(),
    });
    let model = match config.configured_model(&config_path)? {
        Some(model) => model,
        None => match models::pick_installed(gpu.as_ref(), config.language().as_deref()) {
            Some(model) => model,
            None => match setup::run(&config_path, &config, gpu.as_ref())? {
                Some(model) => model,
                None => {
                    log::line("Einrichtung ohne Modell beendet.");
                    return Ok(());
                }
            },
        },
    };
    let device = audio::input_device(&config.microphone)?;
    log::line(&format!("Mikrofon: {}", audio::name(&device)));
    let (vk, modifier) = hotkey::parse(&config.hotkey).ok_or("Ungültiger Hotkey")?;

    tray::create(&config.hotkey)?;
    settings::init(&config_path, &config, gpu.clone());
    insert::create_clipboard_owner()?;
    if config.overlay {
        overlay::create()?;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    hotkey::install(vk, modifier, tx)?;

    // Loading takes seconds; the gray tray icon shows that Mentis has started meanwhile.
    let quiet = args.iter().any(|a| a == autostart::FLAG);
    std::thread::spawn(move || match load(&config, &model) {
        Ok(mut transcriber) => {
            match transcriber.warm_up() {
                Ok(seconds) => log::line(&format!("Aufwärmen {seconds:.1} s")),
                Err(e) => log::line(&format!("Aufwärmen fehlgeschlagen: {e}")),
            }
            tray::set_ready(transcriber.on_gpu());
            let key = i18n::key_name(&config.hotkey);
            log::line(&format!("Bereit. Diktiertaste: {}", config.hotkey));
            if config.update_check {
                update::start();
            }
            if config.overlay && !quiet {
                let hint = if german() {
                    format!("Mentis ist bereit: {key} halten und sprechen.")
                } else {
                    format!("Mentis is ready: hold {key} and speak.")
                };
                overlay::show(overlay::Phase::Message, &hint, 4000);
            }
            // Key presses while loading must not start a recording now.
            while rx.try_recv().is_ok() {}
            app::run(config, transcriber, rx);
        }
        Err(e) => {
            log::line(&format!("Fehler: {e}"));
            message_box(&e, MB_ICONERROR);
            tray::quit();
        }
    });

    // The tray, the keyboard hook and the clipboard owner window need this thread to pump messages.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        if settings::dialog_message(&msg) {
            continue;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    if tray::restart_requested() {
        // Free the single-instance lock first, or the new Mentis would find this one running.
        unsafe { CloseHandle(mutex) };
        restart(args)?;
        log::line("Neustart.");
        return Ok(());
    }
    log::line("Beendet.");
    Ok(())
}

/// Starts Mentis again with the same configuration file (after saving the settings).
fn restart(args: &[String]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let config: Vec<&String> = match args.iter().position(|a| a == "--config") {
        Some(i) => args[i..].iter().take(2).collect(),
        None => Vec::new(),
    };
    std::process::Command::new(exe)
        .args(config)
        .spawn()
        .map(drop)
        .map_err(|e| format!("Neustart fehlgeschlagen: {e}"))
}

fn run_test_helper(args: &[String], config_path: &Path) -> Result<(), String> {
    println!("Mentis {}", env!("CARGO_PKG_VERSION"));
    let config = config::load(config_path)?;
    models::set_dir(Some(config.models_dir.clone()));
    println!("Konfiguration: {}", config_path.display());
    if let Some(i) = args.iter().position(|a| a == "--transcribe") {
        let file = args
            .get(i + 1)
            .ok_or("--transcribe braucht eine WAV-Datei")?;
        return transcribe_file(&config, config_path, file);
    }
    if let Some(i) = args.iter().position(|a| a == "--download") {
        let ids: Vec<&str> = models::CATALOG.iter().map(|m| m.id).collect();
        let model = args
            .get(i + 1)
            .and_then(|id| models::by_id(id))
            .ok_or_else(|| format!("--download braucht eine Modell-ID: {}", ids.join(", ")))?;
        return download_model(model);
    }
    if args.iter().any(|a| a == "--setup") {
        // Shows only the first-start window, also while Mentis is running.
        let gpu = transcriber::init_backends(&config)?;
        match setup::run(config_path, &config, gpu.as_ref())? {
            Some(model) => println!("Modell: {}", model.display()),
            None => println!("Ohne Modell geschlossen."),
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--paste-test") {
        // Insertion check without speaking: focus the target field within 3 seconds.
        println!("In 3 Sekunden wird ein Testtext in das aktive Textfeld eingefügt …");
        std::thread::sleep(std::time::Duration::from_secs(3));
        insert::create_clipboard_owner()?;
        let text = "Mentis-Test: Größe, Übermaß, 12,5 € – \"Zitat\" ";
        let outcome = insert::deliver(text, insert::foreground(), config.insert == "clipboard")?;
        println!("Ergebnis: {outcome:?}");
        return Ok(());
    }
    for name in audio::device_names() {
        println!("  {name}");
    }
    Ok(())
}

fn download_model(model: &models::Model) -> Result<(), String> {
    println!(
        "Lade {} ({} MB) nach {} …",
        model.file,
        model.megabytes(),
        models::dir().display()
    );
    let started = std::time::Instant::now();
    let mut shown = std::time::Instant::now();
    let result = download::download(model, &AtomicBool::new(false), |done, total| {
        if shown.elapsed().as_secs() >= 2 {
            shown = std::time::Instant::now();
            println!("  {} / {} MB", done >> 20, total >> 20);
        }
    });
    match result {
        Ok(path) => {
            println!(
                "Fertig und Prüfsumme korrekt: {} ({:.0} s)",
                path.display(),
                started.elapsed().as_secs_f64()
            );
            Ok(())
        }
        Err(download::Failure::Error(e)) => Err(e),
        Err(download::Failure::Cancelled) => Err("abgebrochen".into()),
    }
}

fn load(config: &config::Config, model: &Path) -> Result<transcriber::Transcriber, String> {
    log::line(&format!("Lade Modell {} …", model.display()));
    let started = std::time::Instant::now();
    let transcriber = transcriber::Transcriber::load(config, model)?;
    log::line(&format!(
        "Modell geladen ({}, {:.1} s)",
        transcriber.backend,
        started.elapsed().as_secs_f64()
    ));
    Ok(transcriber)
}

/// Test path without microphone and hotkey: prints the cleaned transcript of a WAV file.
fn transcribe_file(config: &config::Config, config_path: &Path, file: &str) -> Result<(), String> {
    let mut reader = hound::WavReader::open(file).map_err(|e| format!("{file}: {e}"))?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels).max(1);
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>(),
        hound::SampleFormat::Int => {
            let scale = (1_i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()
        }
    }
    .map_err(|e| e.to_string())?;
    let mono: Vec<f32> = samples
        .chunks(channels)
        .map(|f| f.iter().sum::<f32>() / f.len() as f32)
        .collect();
    let pcm = audio::resample(&mono, spec.sample_rate, audio::RATE);
    let gpu = transcriber::init_backends(config)?;
    let model = match config.configured_model(config_path)? {
        Some(model) => model,
        None => models::pick_installed(gpu.as_ref(), config.language().as_deref())
            .ok_or("Kein Modell installiert (mentis.exe --download ID)")?,
    };
    let mut transcriber = load(config, &model)?;
    println!("Aufwärmen {:.1} s", transcriber.warm_up()?);
    let Some((start, end)) = audio::speech_span(&pcm) else {
        println!("Keine Sprache erkannt.");
        return Ok(());
    };
    let started = std::time::Instant::now();
    let text = transcriber.transcribe(&pcm[start..end])?;
    println!(
        "Erkennung {:.2} s für {:.1} s Audio",
        started.elapsed().as_secs_f64(),
        pcm.len() as f32 / audio::RATE as f32
    );
    println!("{text}");
    Ok(())
}
