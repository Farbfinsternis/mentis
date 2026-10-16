use crate::config::Config;
use crate::models::Gpu;
use std::path::Path;
use std::time::Instant;
use transcribe_cpp::{
    Backend, BackendMask, DeviceType, Model, ModelOptions, RunExtension, RunOptions, Session,
    SessionOptions, WhisperRunOptions,
};

pub struct Transcriber {
    session: Session,
    options: RunOptions,
    pub backend: String,
    /// Writing style "exact": keep filler words.
    exact: bool,
}

fn backend(config: &Config) -> Backend {
    match config.backend.as_str() {
        "cpu" => Backend::Cpu,
        "vulkan" => Backend::Vulkan,
        _ => Backend::Auto,
    }
}

/// Loads the compute modules from the program folder; call once before `Transcriber::load`.
/// Returns the graphics card the model would run on, if any.
pub fn init_backends(config: &Config) -> Result<Option<Gpu>, String> {
    transcribe_cpp::disable_logging();
    let backend = backend(config);
    // The ggml backend modules (ggml-cpu-*.dll, optional ggml-vulkan.dll) sit next to the EXE.
    // The mask keeps the GPU driver out of the process entirely for backend = "cpu".
    let modules = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .ok_or("Programmordner nicht gefunden")?;
    transcribe_cpp::init_backends_with(Some(&modules), BackendMask::for_backend(backend)).map_err(
        |e| {
            format!(
                "Rechenmodule (ggml-cpu-*.dll) in {} konnten nicht geladen werden: {e}",
                modules.display()
            )
        },
    )?;
    if backend == Backend::Vulkan && !transcribe_cpp::backend_available(Backend::Vulkan) {
        return Err(
            "backend = \"vulkan\", aber keine Vulkan-fähige GPU gefunden \
             (Grafiktreiber ohne Vulkan oder ggml-vulkan.dll fehlt im Programmordner). \
             backend = \"auto\" nutzt dann die CPU."
                .into(),
        );
    }
    // A dedicated card beats an integrated one; among equals the one with more memory.
    let gpu = transcribe_cpp::devices()
        .into_iter()
        .filter(|d| matches!(d.device_type, DeviceType::Gpu | DeviceType::Igpu))
        .max_by_key(|d| (d.device_type == DeviceType::Gpu, d.memory_total))
        .map(|d| Gpu {
            name: d.description,
            memory: d.memory_total,
            integrated: d.device_type == DeviceType::Igpu,
        });
    Ok(gpu)
}

fn is_whisper(model: &Model) -> bool {
    model.arch().to_ascii_lowercase().contains("whisper")
}

impl Transcriber {
    pub fn load(config: &Config, model: &Path) -> Result<Self, String> {
        let model = Model::load_with(
            model,
            &ModelOptions {
                backend: backend(config),
                ..Default::default()
            },
        )
        .map_err(|e| format!("Modell konnte nicht geladen werden: {e}"))?;
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8) as i32)
            .unwrap_or(4);
        let session = model
            .session_with(&SessionOptions {
                n_threads: threads,
                ..Default::default()
            })
            .map_err(|e| e.to_string())?;
        // Parakeet detects the language itself and has no prompt; the options are Whisper's.
        let options = if is_whisper(&model) {
            let language = config.language();
            let prompt = context(language.as_deref(), &config.prompt, config.exact());
            RunOptions {
                language,
                family: Some(RunExtension::Whisper(WhisperRunOptions {
                    initial_prompt: (!prompt.is_empty()).then_some(prompt),
                    // No sampling fallback: retries at temperature > 0 drift into subtitle style
                    // without punctuation or capitals. Dictations are short, so greedy suffices.
                    temperature_inc: Some(0.0),
                    ..Default::default()
                })),
                ..Default::default()
            }
        } else {
            RunOptions::default()
        };
        Ok(Self {
            backend: model.backend().to_string(),
            exact: config.exact(),
            session,
            options,
        })
    }

    pub fn on_gpu(&self) -> bool {
        !self.backend.to_ascii_lowercase().contains("cpu")
    }

    /// Runs once on silence so GPU shader setup does not delay the first dictation.
    pub fn warm_up(&mut self) -> Result<f64, String> {
        let start = Instant::now();
        self.session
            .run(&vec![0.0; crate::audio::RATE as usize], &self.options)
            .map_err(|e| e.to_string())?;
        Ok(start.elapsed().as_secs_f64())
    }

    pub fn transcribe(&mut self, pcm: &[f32]) -> Result<String, String> {
        // Whisper works on >= 1 s windows; pad very short clips with silence.
        let mut audio = pcm.to_vec();
        audio.resize(audio.len().max(crate::audio::RATE as usize + 1), 0.0);
        let result = self
            .session
            .run(&audio, &self.options)
            .map_err(|e| e.to_string())?;
        Ok(clean(&result.text, self.exact))
    }
}

/// Built-in context per input language. Whisper continues in the style of this text: full
/// sentences with punctuation, English terms in English spelling. No context for "auto",
/// since a German or English sample would bias language detection.
const CONTEXT_DE: &str = "Hallo, hier ist ein diktierter Text mit sauberer Zeichensetzung. \
    Englische Begriffe bleiben englisch: GitHub, ComfyUI, Workflow, Pull Request, Release Build, \
    Feature, Update, Meeting, Work-Life-Balance.";
const CONTEXT_EN: &str =
    "Hello, this is a dictated text with proper punctuation and capitalization.";

/// Whisper mostly omits hesitations; a prompt that contains some makes it write them down.
const VERBATIM_DE: &str = "Ähm, also, äh, ich sage das jetzt, hm, genau so, wie ich es meine.";
const VERBATIM_EN: &str = "Um, so, uh, I'm saying this, hmm, exactly the way I mean it.";

/// Built-in context followed by the user's own terms.
fn context(language: Option<&str>, user: &str, exact: bool) -> String {
    let (builtin, verbatim) = match language {
        Some("de") => (CONTEXT_DE, VERBATIM_DE),
        Some("en") => (CONTEXT_EN, VERBATIM_EN),
        _ => ("", ""),
    };
    let verbatim = if exact { verbatim } else { "" };
    [builtin, verbatim, user.trim()]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Phrases Whisper invents for silence or noise (from subtitle training data).
const HALLUCINATIONS: &[&str] = &[
    "untertitel im auftrag des zdf",
    "untertitel der amara.org-community",
    "untertitelung im auftrag des zdf",
    "untertitelung aufgrund der amara.org-community",
    "vielen dank fürs zuschauen",
    "vielen dank für's zuschauen",
    "danke fürs zuschauen",
    "bis zum nächsten mal",
    "copyright wdr",
    "swr 2020",
    "thanks for watching",
    "thank you for watching",
    "subtitles by the amara.org community",
];

/// Hesitation sounds without meaning. Ambiguous words ("um", "eh", "also", "mhm") stay.
const FILLERS: &[&str] = &[
    "äh", "ähm", "ääh", "äähm", "öh", "öhm", "ehm", "hm", "hmm", "uh", "uhm", "umm", "erm",
];

/// Drops filler tokens and hesitation ellipses while keeping sentence punctuation and capitals.
fn remove_fillers(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut capitalize_next = false;
    for token in text.split_whitespace() {
        let core = token.trim_matches(|c: char| !c.is_alphanumeric());
        if FILLERS.contains(&core.to_lowercase().as_str()) {
            let sentence_start = out.last().is_none_or(|t| t.ends_with(['.', '?', '!']));
            let ends_sentence = token.ends_with(['.', '?', '!']);
            if ends_sentence && let Some(prev) = out.last_mut() {
                let prev_core_end = prev.trim_end_matches([',', ';', ':', '…', '.']).len();
                prev.truncate(prev_core_end);
                prev.push_str(&token[token.len() - 1..]);
            }
            capitalize_next |= sentence_start || ends_sentence;
            continue;
        }
        let mut word = token.to_owned();
        // "Ich denke … dass": an ellipsis before a lowercase word marks a pause, not an omission.
        if let Some(prev) = out.last_mut()
            && word.starts_with(char::is_lowercase)
            && (prev.ends_with("...") || prev.ends_with('…'))
        {
            let end = prev.trim_end_matches(['.', '…']).len();
            prev.truncate(end);
        }
        if word == "..." || word == "…" {
            continue;
        }
        if capitalize_next {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                word = first.to_uppercase().chain(chars).collect();
            }
            capitalize_next = false;
        }
        out.push(word);
    }
    out.retain(|w| !w.is_empty());
    out.join(" ")
}

/// Tidies whitespace, drops Whisper's silence phrases and, unless `exact`, filler words.
pub fn clean(text: &str, exact: bool) -> String {
    let text = if exact {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        remove_fillers(text)
    };
    let normalized = text
        .to_lowercase()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_owned();
    let is_hallucination = HALLUCINATIONS.iter().any(|h| {
        normalized == *h || (normalized.starts_with(h) && normalized.len() < h.len() + 25)
    });
    if is_hallucination || normalized.is_empty() {
        return String::new();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{CONTEXT_DE, VERBATIM_DE, clean, context};

    #[test]
    fn context_combines_builtin_and_user_terms() {
        assert_eq!(context(Some("de"), "", false), CONTEXT_DE);
        assert_eq!(
            context(Some("de"), " Farbfinsternis ", false),
            format!("{CONTEXT_DE} Farbfinsternis")
        );
        assert_eq!(
            context(Some("de"), "Mentis", true),
            format!("{CONTEXT_DE} {VERBATIM_DE} Mentis")
        );
        assert_eq!(context(None, "", true), "");
        assert_eq!(context(Some("fr"), "Mentis", false), "Mentis");
    }

    #[test]
    fn removes_known_hallucinations_only() {
        for exact in [false, true] {
            assert_eq!(clean(" Untertitel im Auftrag des ZDF, 2021 ", exact), "");
            assert_eq!(clean("Vielen Dank fürs Zuschauen!", exact), "");
            assert_eq!(clean(" ... ", exact), "");
            assert_eq!(
                clean(" Hallo  Welt,\n wie geht's? ", exact),
                "Hallo Welt, wie geht's?"
            );
        }
        assert_eq!(
            clean(
                "Vielen Dank fürs Zuschauen, das Video zeigt heute ausführlich, wie man Mentis einrichtet.",
                false
            ),
            "Vielen Dank fürs Zuschauen, das Video zeigt heute ausführlich, wie man Mentis einrichtet."
        );
    }

    #[test]
    fn removes_fillers_and_pause_ellipses() {
        assert_eq!(
            clean("Äh, ich denke, ähm, das passt.", false),
            "Ich denke, das passt."
        );
        assert_eq!(
            clean("Das ist gut, äh. dann weiter", false),
            "Das ist gut. Dann weiter"
        );
        assert_eq!(
            clean("Ich denke... dass es geht.", false),
            "Ich denke dass es geht."
        );
        assert_eq!(clean("Und dann... Pause.", false), "Und dann... Pause.");
        assert_eq!(clean("Ähm.", false), "");
        // Real words that only look like fillers stay.
        assert_eq!(
            clean("Treffen um 14 Uhr, das ist eh klar.", false),
            "Treffen um 14 Uhr, das ist eh klar."
        );
        assert_eq!(
            clean("Uh, the meeting is at noon.", false),
            "The meeting is at noon."
        );
    }

    #[test]
    fn exact_style_keeps_what_was_said() {
        assert_eq!(
            clean("Äh, ich denke, ähm, das passt.", true),
            "Äh, ich denke, ähm, das passt."
        );
        assert_eq!(
            clean("Ich denke... dass es geht.", true),
            "Ich denke... dass es geht."
        );
        assert_eq!(clean("Ähm.", true), "Ähm.");
    }
}
