use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub model: PathBuf,
    pub models_dir: PathBuf,
    pub language: String,
    pub hotkey: String,
    pub backend: String,
    pub insert: String,
    pub microphone: String,
    pub prompt: String,
    pub style: String,
    pub sounds: bool,
    pub overlay: bool,
    pub live_preview: bool,
    pub trailing_space: bool,
    pub max_seconds: u32,
    pub update_check: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: PathBuf::new(),
            models_dir: PathBuf::new(),
            language: String::new(),
            hotkey: "RightCtrl".into(),
            backend: "auto".into(),
            insert: "clipboard".into(),
            microphone: String::new(),
            prompt: String::new(),
            style: "clean".into(),
            sounds: true,
            overlay: true,
            live_preview: true,
            trailing_space: true,
            max_seconds: 120,
            update_check: true,
        }
    }
}

fn template() -> String {
    format!(
        r#"# Mentis configuration. Restart Mentis after changes.

# Speech recognition model (GGUF for transcribe.cpp). Empty = the matching model from the
# model folder (Mentis downloads it on first start).
model = ''

# Folder for downloaded models. Empty = {}
models_dir = ''

# Input language as ISO code ("de", "en", ...), "auto", or empty = Windows language.
language = ""

# Dictation key: hold = speak, short tap = record until the next tap.
# Possible: RightCtrl, RightShift, CapsLock, ScrollLock, Pause, Insert, F1 ... F24
hotkey = "RightCtrl"

# "auto" (GPU if possible, else CPU), "vulkan" or "cpu".
backend = "auto"

# "clipboard" (Ctrl+V, the clipboard is restored) or "type" (types character by character).
insert = "clipboard"

# Empty = Windows default microphone, else part of the device name, e.g. "USB".
microphone = ""

# Optional context for Whisper, e.g. names and terms: "ComfyUI, Vulkan, Mentis".
prompt = ""

# Writing style: "clean" (without fillers like "uh") or "exact" (everything that is said).
style = "clean"

# Short sounds when recording starts and ends.
sounds = true

# Small panel above the taskbar while recording and recognizing.
overlay = true

# Show recognized text in the panel while speaking (GPU only).
live_preview = true

# Space after the inserted text, so consecutive dictations do not stick together.
trailing_space = true

# Longest recording in seconds.
max_seconds = 120

# Ask github.com once a day whether a new version exists (only a hint in the tray; nothing is
# downloaded or installed).
update_check = true
"#,
        crate::models::default_dir().display()
    )
}

fn known_folder(var: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_path() -> PathBuf {
    known_folder("APPDATA").join("Mentis").join("config.toml")
}

/// Loads the config, writing a commented default file on first start.
pub fn load(path: &Path) -> Result<Config, String> {
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, template()).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let config: Config = toml::from_str(&text).map_err(|e| format!("{}:\n{e}", path.display()))?;
    config.validate()?;
    Ok(config)
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !["clean", "exact"].contains(&self.style.as_str()) {
            return Err(format!(
                "style = \"{}\": erlaubt sind clean, exact",
                self.style
            ));
        }
        if !["auto", "vulkan", "cpu"].contains(&self.backend.as_str()) {
            return Err(format!(
                "backend = \"{}\": erlaubt sind auto, vulkan, cpu",
                self.backend
            ));
        }
        if !["clipboard", "type"].contains(&self.insert.as_str()) {
            return Err(format!(
                "insert = \"{}\": erlaubt sind clipboard, type",
                self.insert
            ));
        }
        if crate::hotkey::parse(&self.hotkey).is_none() {
            return Err(format!(
                "hotkey = \"{}\" wird nicht unterstützt",
                self.hotkey
            ));
        }
        if !(1..=600).contains(&self.max_seconds) {
            return Err("max_seconds muss zwischen 1 und 600 liegen".into());
        }
        Ok(())
    }

    /// The model file named in the configuration; `None` lets Mentis choose from its model folder.
    pub fn configured_model(&self, config_path: &Path) -> Result<Option<PathBuf>, String> {
        if self.model.as_os_str().is_empty() {
            return Ok(None);
        }
        if self.model.is_file() {
            return Ok(Some(self.model.clone()));
        }
        Err(format!(
            "Modelldatei nicht gefunden: {}\n'model' in {} anpassen oder leer lassen.",
            self.model.display(),
            config_path.display()
        ))
    }

    /// Input language for Whisper: the configured one, else the Windows language; None = detect.
    /// Writing style "exact": filler words stay in the text.
    pub fn exact(&self) -> bool {
        self.style == "exact"
    }

    /// Every setting as `key = value` line content, in file order.
    fn entries(&self) -> Vec<(&'static str, toml::Value)> {
        use toml::Value;
        let text = |s: &str| Value::String(s.to_owned());
        let path = |p: &Path| Value::String(p.to_string_lossy().into_owned());
        vec![
            ("model", path(&self.model)),
            ("models_dir", path(&self.models_dir)),
            ("language", text(&self.language)),
            ("hotkey", text(&self.hotkey)),
            ("backend", text(&self.backend)),
            ("insert", text(&self.insert)),
            ("microphone", text(&self.microphone)),
            ("prompt", text(&self.prompt)),
            ("style", text(&self.style)),
            ("sounds", Value::Boolean(self.sounds)),
            ("overlay", Value::Boolean(self.overlay)),
            ("live_preview", Value::Boolean(self.live_preview)),
            ("trailing_space", Value::Boolean(self.trailing_space)),
            ("max_seconds", Value::Integer(i64::from(self.max_seconds))),
            ("update_check", Value::Boolean(self.update_check)),
        ]
    }

    pub fn language(&self) -> Option<String> {
        match self.language.trim() {
            "auto" => None,
            "" => Some(crate::i18n::windows_language()),
            lang => Some(lang.to_owned()),
        }
    }
}

/// Stores a path chosen by the user (`model`, `models_dir`) in the config file, keeping its
/// comments.
pub fn set_path(config_path: &Path, key: &str, path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(config_path)
        .map_err(|e| format!("{}: {e}", config_path.display()))?;
    let value = toml::Value::String(path.to_string_lossy().into_owned());
    std::fs::write(
        config_path,
        with_line(&text, key, &format!("{key} = {value}")),
    )
    .map_err(|e| format!("{}: {e}", config_path.display()))
}

/// Writes all settings into the config file. Comments and unchanged lines stay; keys the file
/// does not have yet (older versions) are appended.
pub fn save(config_path: &Path, config: &Config) -> Result<(), String> {
    config.validate()?;
    let mut text = std::fs::read_to_string(config_path).unwrap_or_else(|_| template());
    for (key, value) in config.entries() {
        text = with_line(&text, key, &format!("{key} = {value}"));
    }
    std::fs::write(config_path, text).map_err(|e| format!("{}: {e}", config_path.display()))
}

/// Replaces the line that sets `key`, or appends `line` if the file has none (older files).
fn with_line(text: &str, key: &str, line: &str) -> String {
    let sets_key = |l: &&str| l.split('=').next().map(str::trim) == Some(key);
    let mut lines: Vec<&str> = text.lines().collect();
    match lines.iter().position(sets_key) {
        Some(i) => lines[i] = line,
        None => lines.push(line),
    }
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses_to_defaults() {
        let config: Config = toml::from_str(&template()).unwrap();
        config.validate().unwrap();
        assert_eq!(config.configured_model(Path::new("c.toml")), Ok(None));
        assert_eq!(config.language(), Some(crate::i18n::windows_language()));
        assert_eq!(config.hotkey, "RightCtrl");
        assert!(config.update_check);
        assert!(template().contains(r"Mentis\models"));
    }

    #[test]
    fn named_model_must_exist() {
        let custom: Config = toml::from_str(r"model = 'C:\nicht\da.gguf'").unwrap();
        let err = custom
            .configured_model(Path::new("config.toml"))
            .unwrap_err();
        assert!(err.contains(r"C:\nicht\da.gguf"));
    }

    #[test]
    fn chosen_paths_replace_only_their_own_line() {
        let line = format!("model = {}", toml::Value::String(r"D:\KI\it's.gguf".into()));
        let text = with_line(&template(), "model", &line);
        let config: Config = toml::from_str(&text).unwrap();
        assert_eq!(config.model, PathBuf::from(r"D:\KI\it's.gguf"));
        assert_eq!(config.models_dir, PathBuf::new());
        assert_eq!(text.matches("model =").count(), 1);
        assert!(text.contains("# Speech recognition model"));

        let line = format!("models_dir = {}", toml::Value::String(r"E:\Modelle".into()));
        let config: Config = toml::from_str(&with_line(&text, "models_dir", &line)).unwrap();
        assert_eq!(config.models_dir, PathBuf::from(r"E:\Modelle"));
        assert_eq!(config.model, PathBuf::from(r"D:\KI\it's.gguf"));
        // Config files from 0.5 and older have no models_dir line yet.
        let old = "language = \"de\"\n";
        let config: Config = toml::from_str(&with_line(old, "models_dir", &line)).unwrap();
        assert_eq!(config.models_dir, PathBuf::from(r"E:\Modelle"));
        assert_eq!(config.language, "de");
    }

    #[test]
    fn save_writes_every_setting_and_keeps_comments() {
        let path = std::env::temp_dir().join(format!("mentis-test-{}.toml", std::process::id()));
        // A file from an older version: no models_dir, no style.
        std::fs::write(
            &path,
            "# eigener Kommentar\nlanguage = \"de\"\nsounds = true\n",
        )
        .unwrap();
        let changed = Config {
            model: PathBuf::from(r"D:\KI\eigenes 'Modell'.gguf"),
            models_dir: PathBuf::from(r"E:\Modelle"),
            language: "auto".into(),
            hotkey: "F9".into(),
            microphone: "USB \"Studio\"".into(),
            prompt: "ComfyUI, Mentis".into(),
            style: "exact".into(),
            sounds: false,
            max_seconds: 300,
            update_check: false,
            ..Default::default()
        };
        save(&path, &changed).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let loaded = load(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(text.starts_with("# eigener Kommentar\n"));
        assert_eq!(text.matches("language =").count(), 1);
        assert_eq!(loaded.model, changed.model);
        assert_eq!(loaded.models_dir, changed.models_dir);
        assert_eq!(loaded.language, "auto");
        assert_eq!(loaded.hotkey, "F9");
        assert_eq!(loaded.microphone, changed.microphone);
        assert_eq!(loaded.prompt, changed.prompt);
        assert!(loaded.exact());
        assert!(!loaded.sounds && loaded.overlay);
        assert_eq!(loaded.max_seconds, 300);
        assert!(!loaded.update_check);
        let invalid = Config {
            style: "freundlich".into(),
            ..Default::default()
        };
        assert!(save(&path, &invalid).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn rejects_typos_and_invalid_values() {
        assert!(toml::from_str::<Config>("hotkye = \"F9\"").is_err());
        let config: Config = toml::from_str("backend = \"cuda\"").unwrap();
        assert!(config.validate().is_err());
        let config: Config = toml::from_str("hotkey = \"A\"").unwrap();
        assert!(config.validate().is_err());
        let config: Config = toml::from_str("language = \"auto\"").unwrap();
        assert_eq!(config.language(), None);
        let config: Config = toml::from_str("language = \"fr\"").unwrap();
        assert_eq!(config.language(), Some("fr".into()));
    }
}
