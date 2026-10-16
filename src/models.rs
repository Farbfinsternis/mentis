//! Speech models Mentis can download, and the choice that suits this PC.

use crate::i18n::t;
use std::path::PathBuf;
use std::sync::RwLock;

pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub file: &'static str,
    repo: &'static str,
    /// Repository commit the file is pinned to; the hash check catches any other change.
    revision: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
    pub license: &'static str,
}

pub static CATALOG: [Model; 3] = [
    Model {
        id: "whisper-turbo",
        name: "Whisper large-v3-turbo",
        file: "whisper-large-v3-turbo-Q8_0.gguf",
        repo: "whisper-large-v3-turbo-gguf",
        revision: "ceea6c8a94a21ab85be244d311e874a39344dbf5",
        sha256: "b2e30cc286bc9f3aba4db9099fc7403543497c05ce7100d0d83091ddfd25a183",
        bytes: 886_381_760,
        license: "Apache-2.0",
    },
    Model {
        id: "parakeet-v3",
        name: "Parakeet TDT 0.6B v3",
        file: "parakeet-tdt-0.6b-v3-Q5_K_M.gguf",
        repo: "parakeet-tdt-0.6b-v3-gguf",
        revision: "90f082450fcbacdb54e5900c44ef697c9ea59622",
        sha256: "cc722e76adc1a629fc0b2535de879d99b8160d07ad4c0215e2ca7d7ea0ae4b8f",
        bytes: 548_946_272,
        license: "CC-BY-4.0",
    },
    Model {
        id: "whisper-small",
        name: "Whisper small",
        file: "whisper-small-Q8_0.gguf",
        repo: "whisper-small-gguf",
        revision: "a2073177cb69bd74b9ca9460b852d17fbfd5d68c",
        sha256: "9b9c8811bbcc82a7766f0fb0925614bdacb0923b2cc630daeac17108b655b860",
        bytes: 269_751_136,
        license: "Apache-2.0",
    },
];

impl Model {
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/handy-computer/{}/resolve/{}/{}",
            self.repo, self.revision, self.file
        )
    }

    pub fn path(&self) -> PathBuf {
        dir().join(self.file)
    }

    pub fn installed(&self) -> bool {
        // A finished download is renamed only after the hash check, so the size suffices here.
        std::fs::metadata(self.path()).is_ok_and(|m| m.len() == self.bytes)
    }

    pub fn megabytes(&self) -> u64 {
        megabytes(self.bytes)
    }

    /// One line for the setup window.
    pub fn summary(&self) -> &'static str {
        match self.id {
            "whisper-turbo" => t(
                "Beste Genauigkeit, braucht eine Grafikkarte",
                "Best accuracy, needs a graphics card",
            ),
            "parakeet-v3" => t(
                "Schnell auf dem Prozessor, 25 europäische Sprachen",
                "Fast on the processor, 25 European languages",
            ),
            _ => t(
                "Viele Sprachen weltweit, ohne Grafikkarte; ungenauer",
                "Many languages worldwide, without a graphics card; less accurate",
            ),
        }
    }
}

/// Whole megabytes (MiB), rounded, as shown in the setup window.
pub fn megabytes(bytes: u64) -> u64 {
    (bytes + (1 << 19)) >> 20
}

/// Folder chosen by the user (`models_dir`); `None` = default folder.
static DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// `%LOCALAPPDATA%\Mentis\models`, used until the user picks another folder.
pub fn default_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Mentis")
        .join("models")
}

/// Where models are downloaded to and looked for.
pub fn dir() -> PathBuf {
    DIR.read()
        .ok()
        .and_then(|d| d.clone())
        .unwrap_or_else(default_dir)
}

pub fn set_dir(dir: Option<PathBuf>) {
    if let Ok(mut slot) = DIR.write() {
        *slot = dir.filter(|d| !d.as_os_str().is_empty());
    }
}

pub fn by_id(id: &str) -> Option<&'static Model> {
    CATALOG.iter().find(|m| m.id == id)
}

/// Graphics card Mentis can use for the large model, as found by the compute backend.
#[derive(Debug, Clone, PartialEq)]
pub struct Gpu {
    pub name: String,
    pub memory: u64,
    pub integrated: bool,
}

/// Whisper turbo needs ~1 GiB of video memory; leave room for the desktop and other apps.
const MIN_GPU_MEMORY: u64 = 2 * 1024 * 1024 * 1024;

/// Languages of Parakeet TDT 0.6B v3 (ISO 639-1).
const PARAKEET_LANGUAGES: [&str; 25] = [
    "bg", "cs", "da", "de", "el", "en", "es", "et", "fi", "fr", "hr", "hu", "it", "lt", "lv", "mt",
    "nl", "pl", "pt", "ro", "ru", "sk", "sl", "sv", "uk",
];

/// Measured 15.10.2026 (21 s of German speech): Whisper turbo 0.16 s on an RTX 5070 Ti but 6.7 s
/// on the CPU (Ryzen 7 5800X3D); Parakeet 0.8 s on the CPU and 0.1 s on the GPU, slightly less
/// accurate than turbo; Whisper small 1.8 s on the CPU and noticeably less accurate.
/// `language`: the input language, `None` = detected per dictation.
pub fn recommended(gpu: Option<&Gpu>, language: Option<&str>) -> &'static Model {
    let fast_gpu = gpu.is_some_and(|g| !g.integrated && g.memory >= MIN_GPU_MEMORY);
    let european = language.is_none_or(|l| PARAKEET_LANGUAGES.contains(&l));
    let id = match (fast_gpu, european) {
        (true, _) => "whisper-turbo",
        (false, true) => "parakeet-v3",
        (false, false) => "whisper-small",
    };
    by_id(id).expect("catalog entry")
}

/// The model to use when the configuration names none: the recommended one if it is
/// installed, otherwise any installed catalog model.
pub fn pick_installed(gpu: Option<&Gpu>, language: Option<&str>) -> Option<PathBuf> {
    let first = recommended(gpu, language);
    std::iter::once(first)
        .chain(CATALOG.iter())
        .find(|m| m.installed())
        .map(Model::path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommends_turbo_only_for_a_dedicated_gpu() {
        let gpu = |memory, integrated| Gpu {
            name: "Test".into(),
            memory,
            integrated,
        };
        let de = Some("de");
        assert_eq!(
            recommended(Some(&gpu(16 << 30, false)), de).id,
            "whisper-turbo"
        );
        assert_eq!(
            recommended(Some(&gpu(16 << 30, false)), Some("ja")).id,
            "whisper-turbo"
        );
        assert_eq!(
            recommended(Some(&gpu(1 << 30, false)), de).id,
            "parakeet-v3"
        );
        assert_eq!(
            recommended(Some(&gpu(16 << 30, true)), de).id,
            "parakeet-v3"
        );
        assert_eq!(recommended(None, de).id, "parakeet-v3");
        assert_eq!(recommended(None, None).id, "parakeet-v3");
        assert_eq!(recommended(None, Some("ja")).id, "whisper-small");
    }

    #[test]
    fn catalog_is_consistent() {
        for model in &CATALOG {
            assert_eq!(model.sha256.len(), 64, "{}", model.id);
            assert!(model.url().ends_with(model.file));
            assert!(model.url().contains(model.revision));
            assert_eq!(by_id(model.id).map(|m| m.file), Some(model.file));
        }
    }
}
