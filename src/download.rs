//! Model download over HTTPS with Windows' own network stack (WinHTTP: system proxy, TLS,
//! redirects). Resumes from a `.part` file and checks size and SHA-256 before the model is used.

use crate::i18n::{german, t};
use crate::models::Model;
use std::ffi::c_void;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_ALG_HANDLE, BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALGORITHM, BCryptCloseAlgorithmProvider,
    BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash, BCryptHashData,
    BCryptOpenAlgorithmProvider,
};
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

const CHUNK: usize = 256 * 1024;
/// Kept free on the drive in addition to the model.
const SPARE_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    Cancelled,
    Error(String),
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Failure::Error(message)
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Downloads `model` into the model folder. `progress` receives (bytes done, bytes total).
pub fn download(
    model: &Model,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<PathBuf, Failure> {
    let target = model.path();
    // E.g. the user picked a folder that already holds this model.
    if model.installed() {
        return Ok(target);
    }
    let dir = target.parent().ok_or("Modellordner fehlt".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = dir.join(format!("{}.part", model.file));

    // Earlier attempts left a partial file: hash what is there and continue after it.
    let mut hash = Sha256::new()?;
    let mut done = match std::fs::metadata(&part) {
        Ok(m) if m.len() < model.bytes => hash_file(&part, &mut hash, cancel)?,
        Ok(_) => {
            let _ = std::fs::remove_file(&part);
            0
        }
        Err(_) => 0,
    };
    check_space(dir, model.bytes - done)?;
    progress(done, model.bytes);

    let response = Response::get(&model.url(), done)?;
    let mut file = match response.status {
        206 if done > 0 => std::fs::OpenOptions::new().append(true).open(&part),
        200 => {
            // The server ignored the range request: start over.
            hash = Sha256::new()?;
            done = 0;
            std::fs::File::create(&part)
        }
        status => {
            // E.g. 416 for a stale partial file: the next attempt starts from scratch.
            let _ = std::fs::remove_file(&part);
            return Err(Failure::Error(format!(
                "{} (HTTP {status})",
                t("Download abgelehnt", "Download refused")
            )));
        }
    }
    .map_err(|e| format!("{}: {e}", part.display()))?;

    let mut buffer = vec![0u8; CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Failure::Cancelled);
        }
        let read = response.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|e| format!("{}: {e}", part.display()))?;
        hash.update(&buffer[..read])?;
        done += read as u64;
        if done > model.bytes {
            break;
        }
        progress(done, model.bytes);
    }
    drop(file);
    if done != model.bytes || hash.finish()? != model.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(Failure::Error(
            t(
                "Die heruntergeladene Datei ist unvollständig oder beschädigt. Bitte erneut versuchen.",
                "The downloaded file is incomplete or damaged. Please try again.",
            )
            .into(),
        ));
    }
    std::fs::rename(&part, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    Ok(target)
}

/// Feeds an existing file into `hash`; returns its length.
fn hash_file(path: &Path, hash: &mut Sha256, cancel: &AtomicBool) -> Result<u64, Failure> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buffer = vec![0u8; CHUNK * 4];
    let mut total = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Failure::Cancelled);
        }
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if read == 0 {
            return Ok(total);
        }
        hash.update(&buffer[..read])?;
        total += read as u64;
    }
}

/// Free bytes on the drive of `dir`; the folder itself need not exist yet.
pub fn free_space(dir: &Path) -> Option<u64> {
    let existing = dir.ancestors().find(|p| p.is_dir())?;
    let mut free = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide(&existing.to_string_lossy()).as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

fn check_space(dir: &Path, needed: u64) -> Result<(), String> {
    let Some(free) = free_space(dir) else {
        return Ok(()); // unknown: let the write itself fail if the drive is full
    };
    if free < needed + SPARE_BYTES {
        let mb = |b: u64| b.div_ceil(1024 * 1024);
        return Err(if german() {
            format!(
                "Nicht genug Speicherplatz auf dem Laufwerk von {}: frei {} MB, benötigt {} MB.",
                dir.display(),
                mb(free),
                mb(needed + SPARE_BYTES)
            )
        } else {
            format!(
                "Not enough disk space on the drive of {}: {} MB free, {} MB needed.",
                dir.display(),
                mb(free),
                mb(needed + SPARE_BYTES)
            )
        });
    }
    Ok(())
}

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

/// An open GET request; the handles must outlive the reads.
struct Response {
    request: Handle,
    _connection: Handle,
    _session: Handle,
    status: u32,
    /// For error messages.
    host: String,
}

fn last_error(host: &str) -> String {
    let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    // WinHTTP codes (12xxx) have no text in the system message table.
    match code {
        12002 | 12007 | 12029 | 12030 if german() => {
            format!("Keine Verbindung zu {host}. Bitte die Internetverbindung prüfen.")
        }
        12002 | 12007 | 12029 | 12030 => {
            format!("Cannot reach {host}. Please check the internet connection.")
        }
        _ => format!("{} (WinHTTP {code})", t("Netzwerkfehler", "Network error")),
    }
}

/// GET `url` and return the status and at most `limit` bytes of the body (small API answers).
pub fn fetch(url: &str, limit: usize) -> Result<(u32, Vec<u8>), String> {
    let response = Response::get(url, 0)?;
    let mut body = Vec::new();
    let mut buffer = vec![0u8; 16 * 1024];
    while body.len() < limit {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&buffer[..read]);
    }
    body.truncate(limit);
    Ok((response.status, body))
}

impl Response {
    /// GET `url` (https only), starting at byte `offset`. Redirects are followed by WinHTTP.
    fn get(url: &str, offset: u64) -> Result<Self, String> {
        let rest = url.strip_prefix("https://").ok_or("nur https")?;
        let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let last_error = || last_error(host);
        let agent = wide(concat!("Mentis/", env!("CARGO_PKG_VERSION")));
        unsafe {
            let session = Handle(WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                std::ptr::null(),
                std::ptr::null(),
                0,
            ));
            if session.0.is_null() {
                return Err(last_error());
            }
            WinHttpSetTimeouts(session.0, 30_000, 30_000, 30_000, 60_000);
            let connection = Handle(WinHttpConnect(
                session.0,
                wide(host).as_ptr(),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            ));
            if connection.0.is_null() {
                return Err(last_error());
            }
            let request = Handle(WinHttpOpenRequest(
                connection.0,
                wide("GET").as_ptr(),
                wide(path).as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ));
            if request.0.is_null() {
                return Err(last_error());
            }
            let headers = if offset > 0 {
                wide(&format!("Range: bytes={offset}-"))
            } else {
                wide("")
            };
            let sent = WinHttpSendRequest(
                request.0,
                headers.as_ptr(),
                u32::MAX, // -1: header string is null-terminated
                std::ptr::null(),
                0,
                0,
                0,
            );
            if sent == 0 || WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0 {
                return Err(last_error());
            }
            let mut status = 0u32;
            let mut size = size_of::<u32>() as u32;
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                std::ptr::null(),
                (&raw mut status).cast(),
                &mut size,
                std::ptr::null_mut(),
            );
            Ok(Self {
                request,
                _connection: connection,
                _session: session,
                status,
                host: host.to_owned(),
            })
        }
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize, String> {
        let mut read = 0u32;
        let ok = unsafe {
            WinHttpReadData(
                self.request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut read,
            )
        };
        if ok == 0 {
            return Err(last_error(&self.host));
        }
        Ok(read as usize)
    }
}

/// Streaming SHA-256 from Windows CNG.
pub struct Sha256 {
    algorithm: BCRYPT_ALG_HANDLE,
    hash: BCRYPT_HASH_HANDLE,
}

impl Sha256 {
    pub fn new() -> Result<Self, String> {
        let mut algorithm: BCRYPT_ALG_HANDLE = std::ptr::null_mut();
        let mut hash: BCRYPT_HASH_HANDLE = std::ptr::null_mut();
        unsafe {
            if BCryptOpenAlgorithmProvider(
                &mut algorithm,
                BCRYPT_SHA256_ALGORITHM,
                std::ptr::null(),
                0,
            ) < 0
            {
                return Err("SHA-256 nicht verfügbar".into());
            }
            if BCryptCreateHash(
                algorithm,
                &mut hash,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
                0,
                0,
            ) < 0
            {
                BCryptCloseAlgorithmProvider(algorithm, 0);
                return Err("SHA-256 nicht verfügbar".into());
            }
        }
        Ok(Self { algorithm, hash })
    }

    pub fn update(&mut self, data: &[u8]) -> Result<(), String> {
        if unsafe { BCryptHashData(self.hash, data.as_ptr(), data.len() as u32, 0) } < 0 {
            return Err("SHA-256 fehlgeschlagen".into());
        }
        Ok(())
    }

    /// Lowercase hex digest.
    pub fn finish(self) -> Result<String, String> {
        let mut digest = [0u8; 32];
        if unsafe { BCryptFinishHash(self.hash, digest.as_mut_ptr(), 32, 0) } < 0 {
            return Err("SHA-256 fehlgeschlagen".into());
        }
        Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
    }
}

impl Drop for Sha256 {
    fn drop(&mut self) {
        unsafe {
            BCryptDestroyHash(self.hash);
            BCryptCloseAlgorithmProvider(self.algorithm, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_digests() {
        let digest = |chunks: &[&[u8]]| {
            let mut hash = Sha256::new().unwrap();
            for chunk in chunks {
                hash.update(chunk).unwrap();
            }
            hash.finish().unwrap()
        };
        assert_eq!(
            digest(&[]),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            digest(&[b"a", b"bc"]),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
