//! "Mentis entfernen …": removes everything Mentis created outside its program folder. The program
//! folder itself cannot delete itself while Mentis runs; the user deletes it afterwards.
//! Only files Mentis knows are deleted; folders go only if they are empty afterwards, so other
//! files in a user-chosen model folder (and the user's own model file) stay.

use crate::config::Config;
use crate::i18n::{german, t};
use crate::models::{self, CATALOG};
use crate::{autostart, diagnose, log, tray, ui};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_DEFBUTTON2, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_YESNO, MessageBoxW,
    SW_SHOWNORMAL,
};

#[derive(Debug, PartialEq, Eq)]
enum Target {
    File(PathBuf),
    /// Removed only if nothing else is left in it.
    EmptyDir(PathBuf),
}

/// Everything Mentis may have created, in deletion order (files before their folders).
fn targets(config_path: &Path, model_dirs: &[PathBuf], local: &Path) -> Vec<Target> {
    let mut list = vec![Target::File(config_path.to_owned())];
    if let Some(dir) = config_path.parent() {
        list.push(Target::EmptyDir(dir.to_owned()));
    }
    for dir in model_dirs {
        for model in &CATALOG {
            list.push(Target::File(dir.join(model.file)));
            list.push(Target::File(dir.join(format!("{}.part", model.file))));
        }
        list.push(Target::EmptyDir(dir.clone()));
    }
    let recordings = local.join("diagnose");
    if let Ok(entries) = std::fs::read_dir(&recordings) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("aufnahme-") && (name.ends_with(".wav") || name.ends_with(".txt")) {
                list.push(Target::File(entry.path()));
            }
        }
    }
    list.push(Target::EmptyDir(recordings));
    for file in ["mentis.log", "mentis.old.log", "update.txt"] {
        list.push(Target::File(local.join(file)));
    }
    list.push(Target::EmptyDir(local.to_owned()));
    list
}

/// Deletes the targets; returns what could not be deleted (missing files do not count).
fn remove(targets: &[Target]) -> Vec<PathBuf> {
    let mut failed = Vec::new();
    for target in targets {
        match target {
            Target::File(path) => {
                if let Err(e) = std::fs::remove_file(path)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    failed.push(path.clone());
                }
            }
            // Fails for folders that still hold other files; that is intended.
            Target::EmptyDir(path) => {
                let _ = std::fs::remove_dir(path);
            }
        }
    }
    failed
}

fn local_dir() -> PathBuf {
    log::path()
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn message(owner: HWND, text: &str, style: u32) -> i32 {
    let (text, title) = (ui::wide(text), ui::wide("Mentis"));
    unsafe { MessageBoxW(owner, text.as_ptr(), title.as_ptr(), style) }
}

/// Asks, removes Mentis' data and ends Mentis.
pub fn run(owner: HWND, config_path: &Path, config: &Config) {
    let mut model_dirs = vec![models::dir(), models::default_dir()];
    model_dirs.dedup();
    let size: u64 = model_dirs
        .iter()
        .flat_map(|dir| CATALOG.iter().map(move |m| dir.join(m.file)))
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    let folders = model_dirs
        .iter()
        .map(|d| format!("  {}", d.display()))
        .collect::<Vec<_>>()
        .join("\n");
    let own_model = (!config.model.as_os_str().is_empty()).then(|| config.model.display());
    let question = if german() {
        let mut text = format!(
            "Mentis entfernen?\n\nGelöscht werden:\n\
             • der Autostart-Eintrag\n\
             • die Einstellungen: {}\n\
             • Protokoll und Diagnose-Aufnahmen: {}\n\
             • heruntergeladene Sprachmodelle ({} MB) in:\n{folders}\n\n\
             Andere Dateien in diesen Ordnern bleiben.",
            config_path.display(),
            local_dir().display(),
            models::megabytes(size)
        );
        if let Some(own) = &own_model {
            text += &format!(" Deine eigene Modelldatei bleibt ebenfalls: {own}");
        }
        text + "\n\nDen Programmordner löschst du danach selbst."
    } else {
        let mut text = format!(
            "Remove Mentis?\n\nThis deletes:\n\
             • the autostart entry\n\
             • the settings: {}\n\
             • log and diagnostic recordings: {}\n\
             • downloaded speech models ({} MB) in:\n{folders}\n\n\
             Other files in these folders stay.",
            config_path.display(),
            local_dir().display(),
            models::megabytes(size)
        );
        if let Some(own) = &own_model {
            text += &format!(" Your own model file stays as well: {own}");
        }
        text + "\n\nYou delete the program folder yourself afterwards."
    };
    if message(owner, &question, MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) != IDYES {
        return;
    }

    log::line("Mentis wird entfernt.");
    diagnose::set(false);
    if autostart::enabled()
        && let Err(e) = autostart::set(false)
    {
        log::line(&format!("Autostart nicht entfernt: {e}"));
    }
    // The open log file cannot be deleted.
    log::close();
    let failed = remove(&targets(config_path, &model_dirs, &local_dir()));

    let program = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let mut text = if german() {
        format!(
            "Mentis ist entfernt.\n\nZum Schluss den Programmordner löschen:\n{}\n\n\
             Er öffnet sich gleich im Explorer.",
            program.display()
        )
    } else {
        format!(
            "Mentis has been removed.\n\nFinally, delete the program folder:\n{}\n\n\
             It opens in Explorer in a moment.",
            program.display()
        )
    };
    if !failed.is_empty() {
        text += t(
            "\n\nDiese Dateien ließen sich nicht löschen:\n",
            "\n\nThese files could not be deleted:\n",
        );
        text += &failed
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
    }
    message(owner, &text, MB_OK | MB_ICONINFORMATION);
    let (verb, explorer) = (ui::wide("open"), ui::wide("explorer.exe"));
    let select = ui::wide(&format!("/select,\"{}\"", program.display()));
    unsafe {
        ShellExecuteW(
            owner,
            verb.as_ptr(),
            explorer.as_ptr(),
            select.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    tray::quit();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_what_mentis_created() {
        let root = std::env::temp_dir().join(format!("mentis-entfernen-{}", std::process::id()));
        let config_dir = root.join("Roaming").join("Mentis");
        let local = root.join("Local").join("Mentis");
        let models = root.join("D").join("KI");
        for dir in [&config_dir, &local.join("diagnose"), &models] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let config_path = config_dir.join("config.toml");
        let mentis_files = [
            config_path.clone(),
            local.join("mentis.log"),
            local.join("update.txt"),
            local.join("diagnose").join("aufnahme-1.wav"),
            local.join("diagnose").join("aufnahme-1.txt"),
            models.join(CATALOG[1].file),
            models.join(format!("{}.part", CATALOG[0].file)),
        ];
        let foreign = [
            models.join("eigenes-modell.gguf"),
            models.join("notizen.txt"),
        ];
        for file in mentis_files.iter().chain(&foreign) {
            std::fs::write(file, "x").unwrap();
        }

        let failed = remove(&targets(
            &config_path,
            std::slice::from_ref(&models),
            &local,
        ));

        assert!(failed.is_empty(), "{failed:?}");
        for file in &mentis_files {
            assert!(!file.exists(), "{} still there", file.display());
        }
        for file in &foreign {
            assert!(file.exists(), "{} was deleted", file.display());
        }
        assert!(!config_dir.exists() && !local.exists());
        assert!(models.exists(), "folder with other files stays");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
