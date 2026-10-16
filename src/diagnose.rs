//! Opt-in diagnostics (tray menu or MENTIS_DIAGNOSE=1): keeps the last recordings and their
//! transcripts locally so recognition problems can be reproduced with the user's real voice.
//! Always off after a restart.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

const KEEP: usize = 10;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Reads MENTIS_DIAGNOSE once at startup.
pub fn init() {
    if std::env::var("MENTIS_DIAGNOSE").is_ok_and(|v| v == "1") {
        set(true);
    }
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    crate::log::line(&if on {
        format!(
            "Diagnose an: Die letzten {KEEP} Aufnahmen und Texte werden in {} gespeichert.",
            dir().display()
        )
    } else {
        "Diagnose aus.".to_owned()
    });
}

pub fn dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Mentis")
        .join("diagnose")
}

/// Saves 16 kHz mono audio plus the recognized text and removes all but the newest recordings.
pub fn save(pcm: &[f32], text: &str) -> Result<PathBuf, String> {
    let dir = dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let wav = dir.join(format!("aufnahme-{stamp}.wav"));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: crate::audio::RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&wav, spec).map_err(|e| e.to_string())?;
    for sample in pcm {
        let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        writer.write_sample(value).map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())?;
    std::fs::write(wav.with_extension("txt"), text).map_err(|e| e.to_string())?;
    prune(&dir);
    Ok(wav)
}

fn prune(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut wavs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    // Names carry a millisecond timestamp of equal length, so they sort chronologically.
    wavs.sort();
    for old in wavs.iter().rev().skip(KEEP) {
        let _ = std::fs::remove_file(old);
        let _ = std::fs::remove_file(old.with_extension("txt"));
    }
}
