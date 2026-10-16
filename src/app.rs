//! Dictation state machine: hotkey events -> recording -> transcription -> output.

use crate::audio::{self, Recorder};
use crate::config::Config;
use crate::diagnose;
use crate::hotkey::{self, Key};
use crate::insert::{self, Outcome};
use crate::overlay::{self, Phase};
use crate::transcriber::Transcriber;
use crate::tray::{self, Status};
use crate::{i18n::t, log};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// Releasing the key sooner than this counts as a tap: recording continues until the next tap.
const TAP: Duration = Duration::from_millis(300);
/// Shortest interval between live preview runs; grows with longer recognition times.
const PREVIEW_INTERVAL: Duration = Duration::from_millis(700);

struct Recording {
    recorder: Recorder,
    started: Instant,
    target: isize,
    toggled: bool,
    next_preview: Instant,
    preview: String,
}

struct Ui {
    overlay: bool,
    sounds: bool,
}

impl Ui {
    fn show(&self, phase: Phase, text: &str, hide_after_ms: u32) {
        if self.overlay {
            overlay::show(phase, text, hide_after_ms);
        }
    }

    fn hide(&self) {
        if self.overlay {
            overlay::hide();
        }
    }

    fn beep(&self, freq: u32, ms: u32) {
        if self.sounds {
            std::thread::spawn(move || unsafe {
                windows_sys::Win32::System::Diagnostics::Debug::Beep(freq, ms);
            });
        }
    }

    fn error(&self, message: &str) {
        self.beep(220, 250);
        self.show(
            Phase::Message,
            &format!("{}: {message}", t("Fehler", "Error")),
            4000,
        );
        log::line(&format!("Fehler: {message}"));
    }
}

pub fn run(config: Config, mut transcriber: Transcriber, events: Receiver<Key>) {
    let ui = Ui {
        overlay: config.overlay,
        sounds: config.sounds,
    };
    // On the CPU a preview run takes seconds and would delay the final result.
    let live = config.overlay && config.live_preview && transcriber.on_gpu();
    let modifier_hotkey = hotkey::parse(&config.hotkey).is_some_and(|(_, m)| m);
    let mut current: Option<Recording> = None;
    let max = Duration::from_secs(u64::from(config.max_seconds));
    loop {
        let wait = match &current {
            Some(rec) if live => rec.next_preview.saturating_duration_since(Instant::now()),
            _ => Duration::from_millis(200),
        };
        let event = match events.recv_timeout(wait.max(Duration::from_millis(10))) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let Some(rec) = current.as_mut() else {
            if event == Some(Key::Press) {
                current = start(&config, &ui);
            }
            continue;
        };
        match event {
            Some(Key::Release) if !rec.toggled && rec.started.elapsed() < TAP => {
                rec.toggled = true;
                status("Aufnahme läuft – zum Beenden Taste erneut antippen, Esc bricht ab.");
            }
            Some(Key::Release) if !rec.toggled => {
                finish(&config, &ui, &mut transcriber, current.take(), &events);
            }
            Some(Key::Press) if rec.toggled => {
                finish(&config, &ui, &mut transcriber, current.take(), &events);
            }
            Some(Key::Other) if modifier_hotkey && !rec.toggled => {
                // RightCtrl+C etc.: the user meant a shortcut, not a dictation; cancel silently.
                hotkey::RECORDING.store(false, Ordering::Relaxed);
                current = None;
                ui.hide();
                tray::set_status(Status::Ready);
                status("Verworfen (Tastenkombination erkannt).");
            }
            Some(Key::Escape) => {
                hotkey::RECORDING.store(false, Ordering::Relaxed);
                current = None;
                ui.beep(330, 120);
                ui.hide();
                tray::set_status(Status::Ready);
                status("Abgebrochen.");
            }
            None if rec.started.elapsed() >= max => {
                status("Maximale Aufnahmedauer erreicht.");
                finish(&config, &ui, &mut transcriber, current.take(), &events);
            }
            None if live && Instant::now() >= rec.next_preview => {
                preview(&ui, &mut transcriber, rec);
            }
            _ => {}
        }
    }
}

/// Recognizes everything recorded so far and shows it in the overlay.
fn preview(ui: &Ui, transcriber: &mut Transcriber, rec: &mut Recording) {
    let started = Instant::now();
    let pcm = rec.recorder.snapshot();
    if let Some((start, end)) = audio::speech_span(&pcm) {
        match transcriber.transcribe(&pcm[start..end]) {
            Ok(text) => {
                rec.preview = text;
                ui.show(Phase::Recording, &rec.preview, 0);
            }
            Err(e) => log::line(&format!("Vorschau fehlgeschlagen: {e}")),
        }
    }
    // Long recordings take longer to recognize; keep the GPU mostly free for the final run.
    rec.next_preview = Instant::now() + PREVIEW_INTERVAL.max(started.elapsed() * 3);
}

fn start(config: &Config, ui: &Ui) -> Option<Recording> {
    let device = match audio::input_device(&config.microphone) {
        Ok(device) => device,
        Err(e) => {
            ui.error(&e);
            return None;
        }
    };
    match Recorder::start(&device, config.max_seconds) {
        Ok(recorder) => {
            hotkey::RECORDING.store(true, Ordering::Relaxed);
            ui.beep(880, 60);
            ui.show(Phase::Recording, "", 0);
            tray::set_status(Status::Recording);
            status("● Aufnahme …");
            Some(Recording {
                recorder,
                started: Instant::now(),
                target: insert::foreground(),
                toggled: false,
                next_preview: Instant::now() + PREVIEW_INTERVAL,
                preview: String::new(),
            })
        }
        Err(e) => {
            ui.error(&e);
            None
        }
    }
}

fn finish(
    config: &Config,
    ui: &Ui,
    transcriber: &mut Transcriber,
    recording: Option<Recording>,
    events: &Receiver<Key>,
) {
    hotkey::RECORDING.store(false, Ordering::Relaxed);
    let Some(recording) = recording else { return };
    let seconds = recording.recorder.seconds();
    let target = recording.target;
    ui.show(Phase::Processing, &recording.preview, 0);
    tray::set_status(Status::Processing);
    let (pcm, gaps) = recording.recorder.finish();
    ui.beep(660, 60);
    let text = match process(config, ui, transcriber, &pcm, target, seconds) {
        Ok(text) => text,
        Err(e) => {
            ui.error(&e);
            String::new()
        }
    };
    if diagnose::enabled() {
        match diagnose::save(&pcm, &text) {
            Ok(path) => status(&format!("  Diagnose: {}", path.display())),
            Err(e) => log::line(&format!("Diagnose konnte nicht speichern: {e}")),
        }
    }
    if gaps > 0 {
        status(&format!(
            "  Hinweis: Der Mikrofontreiber meldete {gaps} Lücken im Audiostrom. Fehlen Wörter, \
             ein anderes Mikrofon testen (microphone in der Konfiguration, Liste: mentis.exe --devices)."
        ));
    }
    tray::set_status(Status::Ready);
    // Key presses during processing are stale; do not let them start a new recording.
    while events.try_recv().is_ok() {}
}

fn process(
    config: &Config,
    ui: &Ui,
    transcriber: &mut Transcriber,
    pcm: &[f32],
    target: isize,
    seconds: f32,
) -> Result<String, String> {
    let Some((start, end)) = audio::speech_span(pcm) else {
        // The peak level tells "wrong/muted microphone" apart from "spoke too quietly".
        let peak = pcm.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        ui.show(
            Phase::Message,
            t(
                "Keine Sprache erkannt – nichts eingefügt.",
                "No speech detected – nothing inserted.",
            ),
            2000,
        );
        status(&format!(
            "Keine Sprache erkannt ({seconds:.1} s Aufnahme, Spitzenpegel {:.0} dB) – nichts eingefügt.",
            20.0 * peak.max(1e-6).log10()
        ));
        return Ok(String::new());
    };
    status("Transkribiere …");
    let started = Instant::now();
    let mut text = transcriber.transcribe(&pcm[start..end])?;
    let elapsed = started.elapsed().as_secs_f64();
    if text.is_empty() {
        ui.show(
            Phase::Message,
            t(
                "Kein Text erkannt – nichts eingefügt.",
                "No text recognized – nothing inserted.",
            ),
            2000,
        );
        status(&format!(
            "Kein Text erkannt ({seconds:.1} s Audio, {elapsed:.2} s) – nichts eingefügt."
        ));
        return Ok(String::new());
    }
    if config.trailing_space {
        text.push(' ');
    }
    let outcome = insert::deliver(&text, target, config.insert == "clipboard")?;
    let chars = text.trim_end().chars().count();
    let how = match outcome {
        Outcome::Pasted => "eingefügt",
        Outcome::PastedNoRestore => {
            "eingefügt (Zwischenablage nicht wiederhergestellt, da zwischenzeitlich geändert)"
        }
        Outcome::Typed => "getippt",
        Outcome::LeftInClipboard => {
            ui.beep(440, 150);
            "NICHT eingefügt: Fenster wurde gewechselt – Text liegt in der Zwischenablage (Strg+V)"
        }
    };
    if outcome == Outcome::LeftInClipboard {
        ui.show(
            Phase::Message,
            t(
                "Fenster gewechselt – Text liegt in der Zwischenablage (Strg+V).",
                "Window changed – the text is on the clipboard (Ctrl+V).",
            ),
            4000,
        );
    } else {
        ui.hide();
    }
    status(&format!(
        "✓ {chars} Zeichen {how} · {seconds:.1} s Audio · Erkennung {elapsed:.2} s"
    ));
    Ok(text)
}

fn status(message: &str) {
    log::line(message);
}
