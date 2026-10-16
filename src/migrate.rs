//! One-time takeover from Echo, Mentis' name up to version 0.7: settings, speech models, log and
//! autostart move over, so Echo's testers neither set up nor download anything again.

use crate::i18n::german;
use crate::models::CATALOG;
use crate::{autostart, config};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};

const OLD: &str = "Echo";
const NEW: &str = "Mentis";

/// What the takeover did, for the log and the one-time hint.
#[derive(Debug, Default)]
pub struct Takeover {
    pub lines: Vec<String>,
    /// Settings or models came over from Echo.
    pub data: bool,
    /// Echo started with Windows; Mentis does now.
    pub autostart: bool,
    /// Echo's program folder, known from its autostart entry.
    pub old_program: Option<PathBuf>,
}

impl Takeover {
    pub fn happened(&self) -> bool {
        self.data || self.autostart
    }

    /// Shown once after the takeover.
    pub fn hint(&self) -> String {
        let folder = self
            .old_program
            .as_ref()
            .map(|p| format!(":\n{}", p.display()))
            .unwrap_or_else(|| ".".into());
        let mut text = if german() {
            "Echo heißt jetzt Mentis.".to_owned()
        } else {
            "Echo is now called Mentis.".to_owned()
        };
        text += "\n\n";
        if self.data {
            text += if german() {
                "Einstellungen und Sprachmodelle von Echo wurden übernommen. "
            } else {
                "Echo's settings and speech models have been taken over. "
            };
        }
        if self.autostart {
            text += if german() {
                "Mentis startet jetzt statt Echo mit Windows. "
            } else {
                "Mentis now starts with Windows instead of Echo. "
            };
        }
        text.truncate(text.trim_end().len());
        text += &if german() {
            format!("\n\nDen alten Programmordner von Echo kannst du löschen{folder}")
        } else {
            format!("\n\nYou can delete Echo's old program folder{folder}")
        };
        text
    }
}

/// Whether Echo is running: both at once would insert every dictation twice.
pub fn echo_running() -> bool {
    let name: Vec<u16> = "Local\\EchoDictation\0".encode_utf16().collect();
    let mutex = unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) };
    if mutex.is_null() {
        return false;
    }
    unsafe { CloseHandle(mutex) };
    true
}

fn known_folder(var: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Takes over whatever Echo left; does nothing once that is done.
pub fn from_echo() -> Takeover {
    let mut done = folders(&known_folder("APPDATA"), &known_folder("LOCALAPPDATA"));
    match autostart::take_over_echo() {
        Ok(None) => {}
        Ok(Some(program)) => {
            match autostart::set(true) {
                Ok(()) => {
                    done.lines.push("Autostart von Echo übernommen.".into());
                    done.autostart = true;
                }
                Err(e) => done.lines.push(format!("Autostart nicht übernommen: {e}")),
            }
            done.old_program = program.parent().map(Path::to_path_buf);
        }
        Err(e) => done
            .lines
            .push(format!("Autostart-Eintrag von Echo nicht entfernt: {e}")),
    }
    done
}

/// Moves Echo's folders under `roaming` (settings) and `local` (log, models, diagnostics) and
/// fixes paths in the settings that point into the old folder.
fn folders(roaming: &Path, local: &Path) -> Takeover {
    let mut done = Takeover::default();
    let (old_settings, settings) = (roaming.join(OLD), roaming.join(NEW));
    if old_settings.is_dir() && !settings.exists() {
        match std::fs::rename(&old_settings, &settings) {
            Ok(()) => {
                done.lines.push(format!(
                    "Einstellungen von Echo übernommen: {}",
                    settings.display()
                ));
                done.data = true;
            }
            Err(e) => {
                // At least the small settings file, so that Mentis needs no new setup.
                let copied = std::fs::create_dir_all(&settings).and_then(|()| {
                    std::fs::copy(
                        old_settings.join("config.toml"),
                        settings.join("config.toml"),
                    )
                });
                done.lines.push(match copied {
                    Ok(_) => format!(
                        "{} ließ sich nicht verschieben ({e}), Einstellungen kopiert.",
                        old_settings.display()
                    ),
                    Err(c) => format!("Einstellungen von Echo nicht übernommen: {e}; Kopie: {c}"),
                });
                done.data = settings.join("config.toml").is_file();
            }
        }
    }

    let (old_local, new_local) = (local.join(OLD), local.join(NEW));
    let mut local_moved = false;
    if old_local.is_dir() && !new_local.exists() {
        match std::fs::rename(&old_local, &new_local) {
            Ok(()) => {
                for (from, to) in [
                    ("echo.log", "mentis.log"),
                    ("echo.old.log", "mentis.old.log"),
                ] {
                    let _ = std::fs::rename(new_local.join(from), new_local.join(to));
                }
                done.lines.push(format!(
                    "Protokoll und Modelle von Echo übernommen: {}",
                    new_local.display()
                ));
                done.data = true;
                local_moved = true;
            }
            Err(e) => done.lines.push(format!(
                "{} ließ sich nicht verschieben: {e}",
                old_local.display()
            )),
        }
    }

    let config_path = settings.join("config.toml");
    if !config_path.is_file() {
        return done;
    }
    let config = match config::load(&config_path) {
        Ok(config) => config,
        Err(e) => {
            done.lines.push(format!("Einstellungen nicht geprüft: {e}"));
            return done;
        }
    };
    if local_moved {
        for (key, path) in [("model", &config.model), ("models_dir", &config.models_dir)] {
            if let Some(moved) = moved_path(path, &old_local, &new_local) {
                done.lines
                    .push(match config::set_path(&config_path, key, &moved) {
                        Ok(()) => format!("{key} angepasst: {}", moved.display()),
                        Err(e) => format!("{key} nicht angepasst: {e}"),
                    });
            }
        }
    } else {
        // Echo's folder stayed (locked): keep using its models instead of downloading again.
        let old_models = old_local.join("models");
        let has_models = CATALOG.iter().any(|m| old_models.join(m.file).is_file());
        if has_models && config.models_dir.as_os_str().is_empty() {
            done.lines.push(
                match config::set_path(&config_path, "models_dir", &old_models) {
                    Ok(()) => format!("Modelle bleiben in {}", old_models.display()),
                    Err(e) => format!("models_dir nicht angepasst: {e}"),
                },
            );
        }
    }
    done
}

/// `path` with the folder `old` replaced by `new`, if it lies in `old` (Windows paths ignore case).
fn moved_path(path: &Path, old: &Path, new: &Path) -> Option<PathBuf> {
    let text = path.to_string_lossy();
    let old = old.to_string_lossy();
    let head = text.get(..old.len())?;
    let rest = &text[old.len()..];
    let inside = rest.is_empty() || rest.starts_with(['\\', '/']);
    (head.eq_ignore_ascii_case(&old) && inside)
        .then(|| PathBuf::from(format!("{}{rest}", new.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("mentis-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn paths_move_only_from_inside_the_old_folder() {
        let (old, new) = (
            Path::new(r"C:\Users\A\AppData\Local\Echo"),
            Path::new(r"C:\Users\A\AppData\Local\Mentis"),
        );
        assert_eq!(
            moved_path(Path::new(r"c:\users\a\appdata\local\ECHO\models"), old, new),
            Some(PathBuf::from(r"C:\Users\A\AppData\Local\Mentis\models"))
        );
        assert_eq!(moved_path(old, old, new), Some(new.to_path_buf()));
        assert_eq!(
            moved_path(
                Path::new(r"C:\Users\A\AppData\Local\Echo-Modelle"),
                old,
                new
            ),
            None
        );
        assert_eq!(moved_path(Path::new(r"E:\Modelle"), old, new), None);
        assert_eq!(moved_path(Path::new(""), old, new), None);
    }

    #[test]
    fn takes_over_settings_models_and_log_once() {
        let root = temp_root("takeover");
        let (roaming, local) = (root.join("Roaming"), root.join("Local"));
        let model = CATALOG[0].file;
        let old_models = local.join("Echo").join("models");
        write(
            &roaming.join("Echo").join("config.toml"),
            &format!(
                "# eigener Kommentar\nmodels_dir = '{}'\nlanguage = \"de\"\n",
                old_models.display().to_string().to_ascii_uppercase()
            ),
        );
        write(&old_models.join(model), "x");
        write(&local.join("Echo").join("echo.log"), "log");

        let done = folders(&roaming, &local);

        assert!(done.data, "{:?}", done.lines);
        assert!(!roaming.join("Echo").exists() && !local.join("Echo").exists());
        let config_path = roaming.join("Mentis").join("config.toml");
        let config = config::load(&config_path).unwrap();
        // Only Echo's folder is replaced; the rest keeps the spelling from the settings.
        let expected = local.join("Mentis").join("MODELS");
        assert_eq!(config.models_dir, expected);
        assert_eq!(config.language, "de");
        assert!(
            std::fs::read_to_string(&config_path)
                .unwrap()
                .starts_with("# eigener Kommentar")
        );
        assert!(local.join("Mentis").join("models").join(model).is_file());
        assert!(local.join("Mentis").join("mentis.log").is_file());

        let again = folders(&roaming, &local);
        assert!(
            !again.happened() && again.lines.is_empty(),
            "{:?}",
            again.lines
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn keeps_using_models_left_in_echos_folder() {
        let root = temp_root("takeover-locked");
        let (roaming, local) = (root.join("Roaming"), root.join("Local"));
        let old_models = local.join("Echo").join("models");
        write(
            &roaming.join("Echo").join("config.toml"),
            "language = \"de\"\n",
        );
        write(&old_models.join(CATALOG[1].file), "x");
        // Mentis' folder exists already, so Echo's cannot be moved there.
        std::fs::create_dir_all(local.join("Mentis")).unwrap();

        let done = folders(&roaming, &local);

        let config = config::load(&roaming.join("Mentis").join("config.toml")).unwrap();
        assert_eq!(config.models_dir, old_models, "{:?}", done.lines);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn hint_names_what_happened() {
        let done = Takeover {
            data: true,
            autostart: true,
            old_program: Some(PathBuf::from(r"D:\Alt\Echo")),
            ..Default::default()
        };
        let hint = done.hint();
        assert!(
            hint.contains("Mentis") && hint.ends_with(r"D:\Alt\Echo"),
            "{hint}"
        );
        let hint = Takeover {
            data: true,
            ..Default::default()
        }
        .hint();
        assert!(hint.ends_with('.') && !hint.contains(":\n"), "{hint}");
    }
}
