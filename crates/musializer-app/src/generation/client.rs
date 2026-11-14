//! The four OpenRouter calls a generation makes, through `curl`.
//!
//! As read on 2026-11-14 (`docs/VIDEO_GENERATION.md`, "The API as found"):
//! `POST /videos` submits and answers `202` with `{id, polling_url, status}`;
//! `GET /videos/{id}` polls (`pending`, `in_progress`, then `completed`,
//! `failed`, `cancelled` or `expired`, with `error` and `usage.cost`);
//! `GET /videos/{id}/content?index=0` streams the MP4; `GET /videos/models`
//! is the public price list. There is no call that cancels a job.
//!
//! `curl` reads its whole configuration — URL, headers, the body's file — from
//! **stdin**, the way the key Test does (`ui/assist_settings.rs`): no flag
//! carries the key, so `/proc/<pid>/cmdline` never has it (E2), and nothing
//! with the key is written to disk. The key goes only to the endpoint's own
//! host, never to a URL a response named.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use musializer_core::assist::secret::Secret;
use serde_json::Value;

/// Where the API is. `MUSIALIZER_OPENROUTER_URL` points it at the gate's fake
/// provider; anything but `https://`, or `http://` to this machine, is refused
/// rather than sent a key.
pub const DEFAULT_BASE: &str = "https://openrouter.ai/api/v1";

/// The longest answer read from a JSON call. A poll is a few hundred bytes.
const MAX_JSON_BYTES: u64 = 1024 * 1024;

/// The longest provider message carried into a sentence.
const MAX_MESSAGE: usize = 300;

/// The client name OpenRouter sees: generic, naming no person or project.
const USER_AGENT: &str = "musializer-video/1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    base: String,
}

impl Endpoint {
    /// The endpoint the environment names, or OpenRouter's.
    ///
    /// # Errors
    ///
    /// An override that is neither `https://` nor `http://` to loopback.
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("MUSIALIZER_OPENROUTER_URL") {
            Ok(value) if !value.is_empty() => Self::new(&value),
            _ => Self::new(DEFAULT_BASE),
        }
    }

    /// # Errors
    ///
    /// As [`Self::from_env`].
    pub fn new(base: &str) -> Result<Self, String> {
        let base = base.trim_end_matches('/');
        let loopback = ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"]
            .iter()
            .any(|prefix| base.starts_with(prefix));
        if !(base.starts_with("https://") || loopback)
            || base
                .chars()
                .any(|c| c.is_whitespace() || c == '"' || c == '\\')
        {
            return Err(format!(
                "{base} is not an endpoint a key may be sent to: https, or http to this machine"
            ));
        }
        Ok(Self {
            base: base.to_owned(),
        })
    }

    #[must_use]
    pub fn describe(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

/// Why a call did not give an answer.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    #[error("curl could not be started ({0}); generating clips needs curl")]
    NoCurl(String),
    /// `sent`: the request was written to the connection (curl's
    /// `size_request`), or that cannot be told; a submit that failed after
    /// it may have been accepted.
    #[error("{}", network_text(.host, .detail, *.sent))]
    Network {
        host: String,
        detail: String,
        sent: bool,
    },
    #[error("stopped")]
    Stopped,
    #[error("{0}")]
    Http(HttpRefusal),
    #[error("the answer was not what OpenRouter documents: {0}")]
    Format(String),
    #[error("the download is larger than the {0} bytes a clip may be")]
    TooLarge(u64),
    #[error("{0}")]
    Local(String),
}

fn network_text(host: &str, detail: &str, sent: bool) -> String {
    if sent {
        format!("the call to {host} failed: {detail}")
    } else {
        format!("could not reach {host}: {detail}")
    }
}

/// A refusal with an HTTP status, in the words a person should read. E10: the
/// status, the provider's own message, never the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRefusal {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for HttpRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self.status {
            400 => "OpenRouter refused the request",
            401 => "OpenRouter rejected the key",
            402 => "the OpenRouter account has too little credit",
            403 => "OpenRouter refused this key the request",
            404 => "OpenRouter does not know that job or model",
            408 | 504 => "OpenRouter timed out",
            413 => "the request is too large for OpenRouter (the frame images)",
            429 => "OpenRouter is rate-limiting; wait and try again",
            500..=599 => "OpenRouter or the model's provider failed",
            _ => "OpenRouter answered with an error",
        };
        if self.message.is_empty() {
            write!(f, "{what} ({})", self.status)
        } else {
            write!(f, "{what} ({}): {}", self.status, self.message)
        }
    }
}

/// A provider message, bounded and on one line.
#[must_use]
pub fn bounded_message(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.trim();
    if flat.chars().count() <= MAX_MESSAGE {
        flat.to_owned()
    } else {
        format!("{}…", flat.chars().take(MAX_MESSAGE).collect::<String>())
    }
}

/// `{"error":{"message":…}}`, `{"error":"…"}` or `{"message":…}`.
#[must_use]
pub fn error_message(body: &[u8]) -> String {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return String::new();
    };
    let text = value
        .get("error")
        .and_then(|error| error.get("message").or(Some(error)))
        .and_then(Value::as_str)
        .or_else(|| value.get("message").and_then(Value::as_str))
        .unwrap_or("");
    bounded_message(text)
}

/// The `curl` to run: `MUSIALIZER_CURL` when it is an absolute path, else the
/// first on `PATH`, else the usual install places. Not a bare name: the
/// isolated launcher runs this application with a `PATH` of its own
/// (`tools/headless_run.sh`), and a bare `curl` then is no curl at all.
fn curl_program() -> PathBuf {
    if let Some(path) = std::env::var_os("MUSIALIZER_CURL").map(PathBuf::from) {
        if path.is_absolute() {
            return path;
        }
    }
    let on_path = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|folder| folder.join("curl"))
            .find(|candidate| candidate.is_file())
    });
    on_path
        .or_else(|| {
            ["/usr/bin/curl", "/usr/local/bin/curl", "/bin/curl"]
                .iter()
                .map(PathBuf::from)
                .find(|candidate| candidate.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("curl"))
}

/// One finished `curl` call.
struct Answer {
    status: u16,
    content_type: String,
}

/// A value for a `curl` config line, quoted.
fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Runs one call: `config` is everything but the output, which goes to
/// `output`. Bounded by `limit`, and stopped within a tenth of a second of
/// `stop`.
fn run(
    endpoint: &Endpoint,
    config: &str,
    output: &Path,
    limit: Duration,
    stop: &AtomicBool,
) -> Result<Answer, ClientError> {
    let mut child = Command::new(curl_program())
        .arg("--config")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // E1: curl needs no credential from the environment; the key, when
        // there is one, is in the configuration on stdin.
        .env_remove("OPENROUTER_API_KEY")
        .spawn()
        .map_err(|error| ClientError::NoCurl(error.to_string()))?;
    let full = format!(
        "{config}output = {}\nsilent\nshow-error\nmax-time = {}\nuser-agent = {}\n\
         write-out = \"%{{http_code}}\\n%{{content_type}}\\n%{{size_request}}\\n\"\n",
        quoted(&output.to_string_lossy()),
        limit.as_secs().max(1),
        quoted(USER_AGENT),
    );
    let wrote = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(full.as_bytes()))
        .is_some_and(|written| written.is_ok());
    if !wrote {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ClientError::Local("curl closed its input".into()));
    }
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ClientError::Network {
                    host: endpoint.describe().to_owned(),
                    detail: error.to_string(),
                    sent: true,
                });
            }
        }
        if stop.load(Ordering::Relaxed) || started.elapsed() > limit + Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if stop.load(Ordering::Relaxed) {
                ClientError::Stopped
            } else {
                ClientError::Network {
                    host: endpoint.describe().to_owned(),
                    detail: "curl did not return".into(),
                    sent: true,
                }
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.by_ref().take(4096).read_to_string(&mut stdout);
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.by_ref().take(4096).read_to_string(&mut stderr);
    }
    let status = child.wait().map_err(|error| ClientError::Network {
        host: endpoint.describe().to_owned(),
        detail: error.to_string(),
        sent: true,
    })?;
    let mut lines = stdout.lines();
    let code: u16 = lines
        .next()
        .and_then(|code| code.trim().parse().ok())
        .unwrap_or(0);
    let content_type = lines.next().unwrap_or("").trim().to_ascii_lowercase();
    // Whether any of the request went out: curl prints `size_request` even
    // when the transfer failed. Unreadable is "it may have": 6 (no such
    // host) and 7 (refused) are the only failures known to precede it.
    let sent = match lines
        .next()
        .and_then(|bytes| bytes.trim().parse::<u64>().ok())
    {
        Some(bytes) => bytes > 0,
        None => !matches!(status.code(), Some(6 | 7)),
    };
    // 63: `max-filesize` exceeded.
    if status.code() == Some(63) {
        return Err(ClientError::TooLarge(0));
    }
    if code == 0 {
        return Err(ClientError::Network {
            host: endpoint.describe().to_owned(),
            detail: bounded_message(&stderr),
            sent,
        });
    }
    // An answer that began and was cut short (18: a partial file, 56: the
    // connection reset): not an answer.
    if !status.success() {
        return Err(ClientError::Network {
            host: endpoint.describe().to_owned(),
            detail: format!(
                "the answer was cut short (curl {}){}",
                status.code().unwrap_or(-1),
                if stderr.trim().is_empty() {
                    String::new()
                } else {
                    format!(": {}", bounded_message(&stderr))
                }
            ),
            sent: true,
        });
    }
    Ok(Answer {
        status: code,
        content_type,
    })
}

fn authorization(secret: &Secret) -> String {
    let key = secret.expose();
    // A key with a line break would end the configuration line early; such a
    // key is no key, and the call goes without one (and is refused, 401).
    if key.chars().any(char::is_control) {
        return String::new();
    }
    format!(
        "header = {}\n",
        quoted(&format!("Authorization: Bearer {key}"))
    )
}

/// An answer's JSON. Every failure here is a [`ClientError::Format`]: the
/// call was answered, so a submit that ends here may have made a job.
fn read_json(path: &Path) -> Result<Value, ClientError> {
    let file = std::fs::File::open(path).map_err(|error| ClientError::Format(error.to_string()))?;
    let mut bytes = Vec::new();
    file.take(MAX_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| ClientError::Format(error.to_string()))?;
    if bytes.len() as u64 > MAX_JSON_BYTES {
        return Err(ClientError::Format(
            "the answer is larger than 1 MiB".into(),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| ClientError::Format(error.to_string()))
}

fn refusal(answer: &Answer, body: &Path) -> ClientError {
    let bytes = std::fs::read(body).unwrap_or_default();
    ClientError::Http(HttpRefusal {
        status: answer.status,
        message: error_message(&bytes[..bytes.len().min(MAX_JSON_BYTES as usize)]),
    })
}

/// A submit that gave no job id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submitted {
    pub error: ClientError,
    /// The request may have reached OpenRouter and been accepted: its answer
    /// was lost after it was written to the connection, or OpenRouter failed
    /// in a way that does not say no job was made.
    pub may_be_billed: bool,
}

/// A job's state, as a poll or the submit answered.
#[derive(Clone, Debug, PartialEq)]
pub struct JobState {
    pub id: String,
    pub status: String,
    pub error: Option<String>,
    /// `usage.cost`, when the answer carries one.
    pub cost_usd: Option<f64>,
}

impl JobState {
    /// # Errors
    ///
    /// No id or status, or an id that is not a plain token.
    pub fn from_json(value: &Value) -> Result<Self, ClientError> {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| {
                !id.is_empty()
                    && id.len() <= 128
                    && id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
            .ok_or_else(|| ClientError::Format("no job id".into()))?
            .to_owned();
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .filter(|status| status.len() <= 32)
            .ok_or_else(|| ClientError::Format("no job status".into()))?
            .to_owned();
        let error = value.get("error").and_then(|error| {
            error
                .as_str()
                .or_else(|| error.get("message").and_then(Value::as_str))
                .map(bounded_message)
        });
        let cost_usd = value
            .get("usage")
            .and_then(|usage| usage.get("cost"))
            .and_then(Value::as_f64)
            .filter(|cost| cost.is_finite() && *cost >= 0.0);
        Ok(Self {
            id,
            status,
            error,
            cost_usd,
        })
    }

    #[must_use]
    pub fn is_terminal(&self) -> bool {
        !matches!(self.status.as_str(), "pending" | "in_progress" | "queued")
    }
}

/// The scratch files of one call live in `work`, a private folder.
pub struct Client<'a> {
    pub endpoint: &'a Endpoint,
    pub secret: &'a Secret,
    pub work: &'a Path,
    pub stop: &'a AtomicBool,
}

impl Client<'_> {
    fn scratch(&self, name: &str) -> PathBuf {
        self.work.join(name)
    }

    /// `POST /videos` with the body in `body` (a file in the work folder).
    ///
    /// # Errors
    ///
    /// Any refusal, by status and the provider's message, with whether the
    /// request may have been accepted all the same ([`Submitted::may_be_billed`]).
    pub fn submit(&self, body: &Path) -> Result<JobState, Submitted> {
        self.submit_inner(body).map_err(|error| {
            let may_be_billed = match &error {
                // Never started, or never wrote a byte of the request (the
                // only `Local` a submit returns is curl closing its input).
                ClientError::NoCurl(_) | ClientError::Local(_) => false,
                ClientError::Network { sent, .. } => *sent,
                // The provider answered and refused: no job. A 5xx or a
                // timeout from OpenRouter may come after the model's
                // provider took the job.
                ClientError::Http(refusal) => refusal.status >= 500 || refusal.status == 408,
                // Killed mid-call, or a 2xx whose answer could not be read.
                ClientError::Stopped | ClientError::Format(_) | ClientError::TooLarge(_) => true,
            };
            Submitted {
                error,
                may_be_billed,
            }
        })
    }

    fn submit_inner(&self, body: &Path) -> Result<JobState, ClientError> {
        let answer_path = self.scratch("submit.json");
        let config = format!(
            "url = {}\nrequest = \"POST\"\n{}header = \"Content-Type: application/json\"\n\
             data-binary = {}\n",
            quoted(&self.endpoint.url("/videos")),
            authorization(self.secret),
            quoted(&format!("@{}", body.to_string_lossy())),
        );
        let answer = run(
            self.endpoint,
            &config,
            &answer_path,
            Duration::from_secs(120),
            self.stop,
        )?;
        if !(200..300).contains(&answer.status) {
            return Err(refusal(&answer, &answer_path));
        }
        JobState::from_json(&read_json(&answer_path)?)
    }

    /// `GET /videos/{id}`.
    ///
    /// # Errors
    ///
    /// Any refusal, by status and the provider's message.
    pub fn poll(&self, id: &str) -> Result<JobState, ClientError> {
        let answer_path = self.scratch("poll.json");
        let config = format!(
            "url = {}\n{}",
            quoted(&self.endpoint.url(&format!("/videos/{id}"))),
            authorization(self.secret),
        );
        let answer = run(
            self.endpoint,
            &config,
            &answer_path,
            Duration::from_secs(60),
            self.stop,
        )?;
        if !(200..300).contains(&answer.status) {
            return Err(refusal(&answer, &answer_path));
        }
        JobState::from_json(&read_json(&answer_path)?)
    }

    /// `GET /videos/{id}/content?index=0` into `destination`, refusing more
    /// than `max_bytes` (curl's `max-filesize`, which also stops a stream that
    /// grows past it) and an answer that names itself as not a video.
    ///
    /// # Errors
    ///
    /// A refusal, a download past the cap, or a body that says it is JSON,
    /// text or HTML.
    pub fn download(
        &self,
        id: &str,
        destination: &Path,
        max_bytes: u64,
    ) -> Result<u64, ClientError> {
        let config = format!(
            "url = {}\n{}max-filesize = {max_bytes}\n",
            quoted(&self.endpoint.url(&format!("/videos/{id}/content?index=0"))),
            authorization(self.secret),
        );
        let answer = run(
            self.endpoint,
            &config,
            destination,
            Duration::from_secs(600),
            self.stop,
        )
        .map_err(|error| match error {
            ClientError::TooLarge(_) => ClientError::TooLarge(max_bytes),
            other => other,
        })?;
        if !(200..300).contains(&answer.status) {
            let refused = refusal(&answer, destination);
            let _ = std::fs::remove_file(destination);
            return Err(refused);
        }
        let bytes = std::fs::metadata(destination)
            .map_err(|error| ClientError::Local(error.to_string()))?
            .len();
        if bytes > max_bytes {
            return Err(ClientError::TooLarge(max_bytes));
        }
        if ["application/json", "text/", "application/xml"]
            .iter()
            .any(|kind| answer.content_type.starts_with(kind))
        {
            return Err(ClientError::Format(format!(
                "the download is {}, not a video",
                answer.content_type
            )));
        }
        Ok(bytes)
    }
}

/// `GET /videos/models`, keyless, into `destination`; the bytes, bounded.
///
/// # Errors
///
/// Unreachable, a refusal, or more than the catalog's cap.
pub fn fetch_models(
    endpoint: &Endpoint,
    destination: &Path,
    stop: &AtomicBool,
) -> Result<Vec<u8>, ClientError> {
    let config = format!(
        "url = {}\nmax-filesize = {}\n",
        quoted(&endpoint.url("/videos/models")),
        musializer_core::generation::catalog::MAX_BYTES
    );
    let answer = run(
        endpoint,
        &config,
        destination,
        Duration::from_secs(20),
        stop,
    )?;
    let bytes =
        std::fs::read(destination).map_err(|error| ClientError::Local(error.to_string()))?;
    let _ = std::fs::remove_file(destination);
    if !(200..300).contains(&answer.status) {
        return Err(ClientError::Http(HttpRefusal {
            status: answer.status,
            message: error_message(&bytes),
        }));
    }
    Ok(bytes)
}

/// Standard base64 with padding, for a data URL.
#[must_use]
pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let triple = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(TABLE[(triple >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A data URL of an image's bytes, by their magic number.
///
/// # Errors
///
/// Bytes that are neither PNG nor JPEG.
pub fn data_url(bytes: &[u8]) -> Result<String, String> {
    let mime = image_mime(bytes).ok_or("a frame image is a PNG or a JPEG")?;
    Ok(format!("data:{mime};base64,{}", base64(bytes)))
}

/// An image's MIME type by its magic number: PNG or JPEG, else `None`.
#[must_use]
pub fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_is_rfc_4648() {
        for (input, output) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foobar", "Zm9vYmFy"),
            (&[0xFF, 0xFE, 0x00][..], "//4A"),
        ] {
            assert_eq!(base64(input), output);
        }
        assert!(data_url(b"\x89PNG\r\n\x1a\nxyz")
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert!(data_url(b"GIF89a").is_err());
    }

    #[test]
    fn a_key_goes_only_to_https_or_this_machine() {
        assert_eq!(
            Endpoint::new("https://openrouter.ai/api/v1/")
                .unwrap()
                .describe(),
            DEFAULT_BASE
        );
        assert!(Endpoint::new("http://127.0.0.1:8765/api/v1").is_ok());
        for refused in [
            "http://openrouter.ai/api/v1",
            "http://127.0.0.1.evil.example/api",
            "ftp://x",
            "https://a\"b",
        ] {
            assert!(Endpoint::new(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn a_refusal_reads_as_the_status_and_the_providers_words() {
        let message =
            error_message(br#"{"error":{"code":402,"message":"Insufficient credits\nadd more"}}"#);
        assert_eq!(message, "Insufficient credits add more");
        let refusal = HttpRefusal {
            status: 402,
            message,
        };
        assert_eq!(
            refusal.to_string(),
            "the OpenRouter account has too little credit (402): Insufficient credits add more"
        );
        assert_eq!(error_message(br#"{"error":"bad"}"#), "bad");
        assert_eq!(error_message(b"<html>"), "");
        assert_eq!(
            bounded_message(&"x".repeat(400)).chars().count(),
            MAX_MESSAGE + 1
        );
    }

    #[test]
    fn a_job_state_is_read_from_the_documented_shape() {
        let state = JobState::from_json(&serde_json::json!({
            "id": "gen-vid-1789480874-Ab3dEf9hIjKlMnOpQrSt",
            "status": "completed",
            "usage": { "cost": 0.5, "is_byok": false },
            "unsigned_urls": ["/api/v1/videos/x/content?index=0"]
        }))
        .unwrap();
        assert_eq!(state.cost_usd, Some(0.5));
        assert!(state.is_terminal());
        let failed = JobState::from_json(&serde_json::json!({
            "id": "a", "status": "failed", "error": "content policy"
        }))
        .unwrap();
        assert_eq!(failed.error.as_deref(), Some("content policy"));
        assert!(
            !JobState::from_json(&serde_json::json!({"id":"a","status":"pending"}))
                .unwrap()
                .is_terminal()
        );
        assert!(JobState::from_json(&serde_json::json!({"id":"../x","status":"pending"})).is_err());
    }

    #[test]
    fn a_config_value_cannot_break_out_of_its_quotes() {
        assert_eq!(quoted("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }
}
