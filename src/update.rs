//! Looks on GitHub once a day for a newer Mentis release and points to it in the tray. Nothing is
//! downloaded or installed; the release page opens in the browser on request.

use crate::{download, log, tray};
use std::path::PathBuf;
use std::time::Duration;

const LATEST: &str = "https://api.github.com/repos/Farbfinsternis/mentis/releases/latest";
pub const PAGE: &str = "https://github.com/Farbfinsternis/mentis/releases/latest";
/// After the model is ready, so the check never competes with the start.
const FIRST: Duration = Duration::from_secs(10);
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// Checks in the background now and then once a day; call once Mentis is ready.
pub fn start() {
    std::thread::spawn(|| {
        std::thread::sleep(FIRST);
        loop {
            check();
            std::thread::sleep(EVERY);
        }
    });
}

/// Remembers the last version a notification was shown for, so it appears only once.
fn notified_path() -> PathBuf {
    log::path().with_file_name("update.txt")
}

fn check() {
    // MENTIS_UPDATE_URL points the check at another release feed for tests.
    let url = std::env::var("MENTIS_UPDATE_URL").unwrap_or_else(|_| LATEST.into());
    let latest = match download::fetch(&url, 512 * 1024) {
        Ok((200, body)) => tag_name(&String::from_utf8_lossy(&body)).and_then(|t| version(&t)),
        Ok((404, _)) => {
            log::line("Updateprüfung: keine Veröffentlichung gefunden.");
            return;
        }
        Ok((status, _)) => {
            log::line(&format!("Updateprüfung: HTTP {status}"));
            return;
        }
        Err(e) => {
            log::line(&format!("Updateprüfung fehlgeschlagen: {e}"));
            return;
        }
    };
    let (Some(latest), Some(current)) = (latest, version(env!("CARGO_PKG_VERSION"))) else {
        log::line("Updateprüfung: keine Versionsnummer in der Antwort.");
        return;
    };
    let name = format!("{}.{}.{}", latest[0], latest[1], latest[2]);
    if latest <= current {
        log::line(&format!("Updateprüfung: aktuell (neueste Version {name})."));
        return;
    }
    let path = notified_path();
    let notify = std::fs::read_to_string(&path).map_or(true, |seen| seen.trim() != name);
    log::line(&format!("Updateprüfung: neue Version {name} verfügbar."));
    tray::offer_update(&name, notify);
    if notify {
        let _ = std::fs::write(&path, &name);
    }
}

/// The value of `"tag_name"` in a GitHub release (JSON), without a JSON parser.
fn tag_name(json: &str) -> Option<String> {
    let rest = &json[json.find("\"tag_name\"")? + "\"tag_name\"".len()..];
    let rest = rest
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_owned())
}

/// "v1.2.3" or "1.2" as [major, minor, patch]; a suffix like "-beta" is ignored.
fn version(text: &str) -> Option<[u32; 3]> {
    let text = text.trim().trim_start_matches(['v', 'V']);
    let core = text.split(['-', '+']).next()?;
    let mut parts = [0u32; 3];
    let mut count = 0;
    for (slot, part) in parts.iter_mut().zip(core.split('.')) {
        *slot = part.parse().ok()?;
        count += 1;
    }
    (count > 0).then_some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_tag_of_a_release() {
        let json = r#"{"url":"x","html_url":"y","id":1,"tag_name" : "v0.9.0","name":"Mentis 0.9"}"#;
        assert_eq!(tag_name(json).as_deref(), Some("v0.9.0"));
        assert_eq!(tag_name(r#"{"message":"Not Found"}"#), None);
        assert_eq!(tag_name(r#"{"tag_name":"v1"#), None);
    }

    #[test]
    fn compares_versions() {
        assert_eq!(version("v0.9.0"), Some([0, 9, 0]));
        assert_eq!(version("1.2"), Some([1, 2, 0]));
        assert_eq!(version("V1.10.3-beta.1"), Some([1, 10, 3]));
        assert_eq!(version("latest"), None);
        assert_eq!(version(""), None);
        assert!(version("0.10.0") > version("0.9.9"));
        assert!(version("1.0.0") > version(env!("CARGO_PKG_VERSION")));
        assert!(version(env!("CARGO_PKG_VERSION")) <= version(env!("CARGO_PKG_VERSION")));
    }
}
