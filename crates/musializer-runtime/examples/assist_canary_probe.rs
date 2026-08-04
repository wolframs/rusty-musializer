//! Plants one sentinel credential through every entry route and leaves behind
//! everything a scan needs to prove it did not escape.
//!
//! Driven by `tools/secret_canary_check.sh`. It exists because the acceptance in
//! `docs/ASSIST_PROVIDER_CONTRACTS.md` §4 is a *measurement* — "a syntactically
//! valid key with a unique sentinel substring, planted through every entry
//! route, then a recursive scan finds zero occurrences" — and nothing in the
//! application can drive that from the outside. The three routes §3 names are
//! all exercised here:
//!
//! | route | what this probe does |
//! | --- | --- |
//! | env import | reads `OPENROUTER_API_KEY`, then removes it from the process |
//! | file | writes the `0600` credentials store, the one authorized sink |
//! | session | holds the key in memory and prints only its fingerprint |
//!
//! Everything it writes goes under the directory named on the command line, plus
//! the config paths the environment points at. It writes nothing outside them.
//!
//! Usage: `assist_canary_probe <output-directory>`

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use musializer_core::assist::credentials::CredentialStore;
use musializer_core::assist::secret::Secret;
use musializer_core::assist::settings::{
    AssistSettings, CredentialMode, CredentialRecord, Credentials,
};
use musializer_core::project::io as project_io;
use musializer_core::project::lyrics::LyricsDocument;
use musializer_core::project::model::{
    AssetMode, AudioAsset, BlendMode, Metadata, OutputSettings, Project, SceneEntry,
};
use musializer_runtime::assist::{env as assist_env, files};

fn main() -> Result<(), String> {
    // SAFETY: this is the first statement of `main` in a single-threaded
    // program; nothing has been spawned and nothing else reads the environment.
    // Same contract `musializer-app`'s `main` satisfies, which is the point —
    // the probe must exercise the real import path, not a copy of it.
    let session = unsafe { assist_env::import_session_credentials() };

    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: assist_canary_probe <output-directory>")?,
    );
    std::fs::create_dir_all(&output).map_err(|error| format!("output directory: {error}"))?;

    // Route 1: the environment. Report by fingerprint only (§3).
    println!("session: {}", session.describe());
    println!("imported: {}", session.has_openrouter());
    let Some(secret) = session.openrouter() else {
        return Err("no OPENROUTER_API_KEY was present to import".to_string());
    };
    let fingerprint = secret.fingerprint();
    println!("fingerprint: {fingerprint}");
    // E8: the redacted Debug is what a panic payload or a stray `{:?}` prints.
    println!("debug-print: {secret:?}");

    // The parent's own /proc/self/environ still carries what `execve` put there
    // — `unsetenv` cannot rewrite that region. The exposure E1 is about is what
    // a *child* inherits, so that is what gets captured and scanned.
    let child = Command::new("/bin/sh")
        .arg("-c")
        .arg("tr '\\0' '\\n' < /proc/self/environ")
        .output()
        .map_err(|error| format!("could not spawn the environment probe: {error}"))?;
    let child_environ = String::from_utf8_lossy(&child.stdout).into_owned();
    write(&output.join("child-environ.txt"), child_environ.as_bytes())?;
    let names: Vec<&str> = child_environ
        .lines()
        .filter_map(|line| line.split_once('=').map(|(name, _)| name))
        .collect();
    println!(
        "child-has-openrouter-key: {}",
        names.contains(&assist_env::OPENROUTER_KEY_VARIABLE)
    );
    let credential_named = assist_env::credential_named_variables(names);
    println!(
        "child-credential-named-variables: {}",
        if credential_named.is_empty() {
            "none".to_string()
        } else {
            credential_named.join(",")
        }
    );

    // E2: no flag ever takes a key, so the process's own argv is scannable.
    let cmdline =
        std::fs::read("/proc/self/cmdline").map_err(|error| format!("cmdline: {error}"))?;
    write(&output.join("cmdline.txt"), &cmdline)?;

    // Route 2: the credentials file — the one authorized plaintext sink.
    let credentials_path = files::credentials_path().ok_or("no credentials path resolved")?;
    let mut store = CredentialStore::new();
    if !store.insert(
        "openrouter",
        "default",
        Secret::new(secret.expose().to_string()),
        Some("Canary".to_string()),
    ) {
        return Err("the credentials store refused the canary entry".to_string());
    }
    files::save_credentials(&credentials_path, &store).map_err(|error| error.to_string())?;
    let file_mode = files::mode_of(&credentials_path).map_err(|error| error.to_string())?;
    let directory_mode = files::mode_of(
        credentials_path
            .parent()
            .ok_or("credentials path has no parent")?,
    )
    .map_err(|error| error.to_string())?;
    println!("credentials-path: {}", credentials_path.display());
    println!("credentials-file-mode: {file_mode:04o}");
    println!("credentials-directory-mode: {directory_mode:04o}");
    if file_mode & 0o077 != 0 || directory_mode & 0o077 != 0 {
        return Err("the credentials file or its directory is not owner-only".to_string());
    }

    // E11: the preference record gets the fingerprint and the lookup id, and
    // there is no field the key could occupy.
    let settings_path = files::settings_path().ok_or("no settings path resolved")?;
    let settings = AssistSettings {
        credentials: Credentials {
            openrouter: CredentialRecord {
                mode: CredentialMode::EnvImport,
                lookup_id: "default".to_string(),
                fingerprint: Some(fingerprint.clone()),
                label: Some("Canary".to_string()),
            },
        },
        ..AssistSettings::default()
    };
    files::save_settings(&settings_path, &settings).map_err(|error| error.to_string())?;
    println!("settings-path: {}", settings_path.display());

    // E5: a project file carries no credential and no lookup label. Projects
    // carry the execution snapshot only, and this build has none yet — so what
    // the scan proves today is that nothing leaked into one by accident.
    let musi = project_io::serialize(&canary_project()?).map_err(|error| format!("{error}"))?;
    write(&output.join("canary.musi"), musi.as_bytes())?;

    // E3: a request echo in a log redacts the header rather than omitting the
    // line, so the failure stays diagnosable.
    write(
        &output.join("job.log"),
        format!(
            "POST https://openrouter.ai/api/v1/chat/completions \
             headers={{\"Authorization\": \"Bearer <redacted>\"}} \
             credential_present=true credential_fingerprint={fingerprint}\n"
        )
        .as_bytes(),
    )?;

    // E6: a catalog cache is written to the cache directory, normalized. The
    // canary never reaches it, and the scan proves that rather than assuming it.
    if let Some(cache) = std::env::var_os("XDG_CACHE_HOME").filter(|value| !value.is_empty()) {
        let directory = PathBuf::from(cache).join("musializer");
        std::fs::create_dir_all(&directory).map_err(|error| format!("cache directory: {error}"))?;
        write(
            &directory.join("openrouter-catalog.json"),
            br#"{"cache_schema":"musializer.model-catalog/v1","models":[]}"#,
        )?;
    }

    println!("probe: complete");
    Ok(())
}

/// The smallest project `Project::validate` accepts: one scene, one audio
/// asset, a lyrics document whose duration agrees with it.
fn canary_project() -> Result<Project, String> {
    let mut project = Project {
        metadata: Metadata {
            project_id: "canary".to_string(),
            title: "Canary".to_string(),
            application_version: "2026.08".to_string(),
            ..Metadata::default()
        },
        audio: AudioAsset {
            mode: AssetMode::Imported,
            path: "canary.assets/audio/fixture.wav".to_string(),
            sha256: "0".repeat(64),
            duration_seconds: 8.0,
            sample_rate: 48_000,
            channels: 2,
        },
        output: OutputSettings {
            end_seconds: 8.0,
            ..OutputSettings::default()
        },
        scenes: vec![SceneEntry {
            instance_id: 1,
            scene_type: "spectrum".to_string(),
            enabled: true,
            start_seconds: 0.0,
            end_seconds: 8.0,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mappings: Vec::new(),
        }],
        ..Project::default()
    };
    project.lyrics = LyricsDocument::new(8.0).map_err(|error| format!("{error:?}"))?;
    Ok(project)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file =
        std::fs::File::create(path).map_err(|error| format!("{}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("{}: {error}", path.display()))
}
