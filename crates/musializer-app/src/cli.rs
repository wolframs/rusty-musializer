//! The command line.
//!
//! Ported from `../musializer/src/musializer.c:19-250` (the value parsers) and
//! `:315-662` (the whole of `main`). No other C file consumes `argv`, and the
//! grammar was verified flag-by-flag in `docs/PHASE0_INVENTORY.md` section 3 —
//! **work from that, not from `REWRITE_PLAN.md`'s older list, which was missing
//! eight flags.**
//!
//! Three properties of the C parser are load-bearing and reproduced here:
//!
//! 1. **`-h`/`--help`/`--version` are a pre-pass over all of `argv`**
//!    (`musializer.c:317-326`). They win from any position, they win even when
//!    the rest of the command line is invalid, they open no window, and they
//!    exit 0.
//! 2. **Order is semantics.** The C applies most flags the instant it reaches
//!    them, interleaved with input loading, so a later `--project` or positional
//!    really does overwrite earlier state. That is why parsing produces an
//!    ordered [`Action`] list rather than a flat struct of options: replaying the
//!    list left to right is the only way to preserve it.
//! 3. **Routes are deliberately deferred** until every positional and
//!    `--project` input is resolved (`musializer.c:433-452` for the deferral,
//!    `:553-561` for the application, rationale at `:446-448`). A project
//!    hydration would otherwise overwrite a route that happened to appear
//!    earlier in `argv`.
//!
//! The error model is also the C's: one shared `error` flag, a warning per
//! failure, and the loop keeps going (`musializer.c:384`, `:618`). Warnings are
//! collected rather than printed so the parser stays testable without a window.
//!
//! ## Deliberate divergences from the oracle
//!
//! - **An unknown flag is an error here.** The C has no unknown-flag diagnostic
//!   at all: any unrecognized `--flag` falls through to the positional arm and is
//!   loaded as an audio path, so `musializer --typo` warns "Could not load
//!   command-line track: --typo" (`musializer.c:546-550`). We keep the C's exit
//!   status but say what actually happened. Recorded as intentional in
//!   `REWRITE_PLAN.md`.
//! - **Number syntax is Rust's, not `strtod`'s.** `str::parse` rejects leading
//!   whitespace and hex float literals, both of which `strtod` accepts. Nobody
//!   types `--fps " 30"`, and treating it as an error is the safer direction.
//! - `--reload-once` parses and is reported unsupported when applied: hot reload
//!   is an explicit first-pass non-goal.

use std::path::PathBuf;

use musializer_core::scene::events::{EventRecord, EventType, VALUE_CAPACITY};
use musializer_core::scene::routes::{AnalysisSource, Interpolation, ParameterMapping};
use musializer_core::scene::{settings, SceneId};

/// Host-side cap on `--route` occurrences (`COMMAND_LINE_ROUTE_CAPACITY`,
/// `musializer.c:11`). The 257th warns and errors.
pub const ROUTE_CAPACITY: usize = 256;

/// `--ui-probe` spec length bound (`musializer.c:136-138`).
pub const UI_PROBE_SPEC_CAPACITY: usize = 256;

/// The window the C opens: factor 80 x 16:9 (`musializer.c:346-349`).
pub const DEFAULT_WINDOW: (i32, i32) = (1280, 720);

/// The minimum window the C project supports (`musializer.c:354`).
///
/// A panel's minimum size must be measured against the panel *this* actually
/// produces, not against a guessed threshold. GLFW clamps `SetWindowSize` to it,
/// which is why a deliberately tiny `--ui-probe size=` still photographs the
/// smallest layout the application permits.
pub const MIN_WINDOW: (i32, i32) = (960, 640);

/// What the pre-pass decided.
#[derive(Debug)]
pub enum Outcome {
    /// `-h`/`--help` seen anywhere in `argv`; print help, exit 0.
    Help,
    /// `--version` seen anywhere in `argv`; print the version, exit 0.
    Version,
    Parsed(Box<Cli>),
}

/// One immediate action, in `argv` order.
///
/// The C performs each of these inline in its `argv` loop. Keeping them ordered
/// is what preserves "a later input overwrites an earlier one", which in turn is
/// the reason routes have to be deferred.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// `--mute`: `SetMasterVolume(0.0f)`, applied immediately
    /// (`musializer.c:399-405`). Kills the output device volume only, so
    /// playback, analysis and export behave exactly as an audible session.
    Mute,
    /// `--scene NAME` (`musializer.c:406-412`).
    SelectScene(SceneId),
    /// `--ascii-image FILE`: import, then select `ascii` (`musializer.c:413-422`).
    AsciiImage(PathBuf),
    /// `--event TYPE:SECONDS:ID:VALUE` (`musializer.c:423-432`).
    RecordEvent(EventRecord),
    /// `--project FILE`, or a positional ending in `.musi` (`musializer.c:500-506`).
    OpenProject(PathBuf),
    /// A positional that is not `.musi` (`musializer.c:546-550`).
    LoadTrack(PathBuf),
}

/// Export quality (`plug_configure_render`, `plug.c:7157-7160`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Balanced,
    High,
    Master,
}

impl Quality {
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "balanced" => Some(Quality::Balanced),
            "high" => Some(Quality::High),
            "master" => Some(Quality::Master),
            _ => None,
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Quality::Balanced => "balanced",
            Quality::High => "high",
            Quality::Master => "master",
        }
    }
}

/// Which workspace panel a `--ui-probe` opens (`Plug_Ui_Panel`,
/// `musializer.c:111-119`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiPanel {
    #[default]
    None,
    Tune,
    Export,
    Lyrics,
    Assist,
}

impl UiPanel {
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "none" => Some(UiPanel::None),
            "tune" => Some(UiPanel::Tune),
            "export" => Some(UiPanel::Export),
            "lyrics" => Some(UiPanel::Lyrics),
            "assist" => Some(UiPanel::Assist),
            _ => None,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            UiPanel::None => "none",
            UiPanel::Tune => "tune",
            UiPanel::Export => "export",
            UiPanel::Lyrics => "lyrics",
            UiPanel::Assist => "assist",
        }
    }
}

/// A parsed `--ui-probe` request (`Command_Line_Ui_Probe`, `musializer.c:104-109`,
/// grammar at `:131-250`).
///
/// This exists so a headless capture can photograph a *specific* workspace state.
/// `../musializer/tools/UI_REVIEW.md` records that the two worst UI defects ever
/// found there were invisible to capture until `assist=confirm` and `lyric=N`
/// existed — if a surface cannot be photographed, it does not get reviewed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiProbe {
    pub panel: UiPanel,
    pub fullscreen: bool,
    /// The transport is parked unless this is set.
    pub playing: bool,
    /// Selects the nth lyric cue, 1-based. Needs `panel=assist`.
    pub lyric_selection: Option<u32>,
    /// Timeline strip zoom; 1 is the whole track.
    pub timeline_zoom: Option<f64>,
    /// `style=caption`: show the caption typography pane. Needs `panel=lyrics`.
    pub caption_style_pane: bool,
    /// `fonts=consent` shows the network-consent panel; `fonts=PATH` loads a
    /// family list from disk, so a capture never contacts Google.
    pub font_browser: Option<FontBrowserProbe>,
    /// `assist=confirm`: arm the Assist confirmation prompt.
    pub assist_confirmation: bool,
    /// Selects an authored lyric sheet for the next Assist lyrics run.
    pub lyrics_reference_path: Option<PathBuf>,
    /// `time=SECONDS`, which also sets the seek-requested flag.
    pub seek_seconds: Option<f64>,
    /// Host-side window geometry, not plug state (`musializer.c:238-243`).
    pub size: Option<(u32, u32)>,
}

/// `fonts=` in a `--ui-probe` spec (`musializer.c:202-217`).
#[derive(Clone, Debug, PartialEq)]
pub enum FontBrowserProbe {
    /// `fonts=consent`: the network-consent panel.
    Consent,
    /// `fonts=PATH`: a family list read from disk instead of the network.
    Catalogue(PathBuf),
}

/// Everything the command line asked for.
#[derive(Debug, Default)]
pub struct Cli {
    /// Replayed left to right; see [`Action`].
    pub actions: Vec<Action>,
    /// Applied only after every action, in `argv` order (`musializer.c:553-561`).
    pub routes: Vec<ParameterMapping>,
    pub render: Option<PathBuf>,
    /// `(start_seconds, duration_seconds)`; duration is always `> 0`.
    pub render_window: Option<(f64, f64)>,
    pub resolution: Option<(u32, u32)>,
    pub fps: Option<u32>,
    /// Stored unvalidated, exactly as the C does: the name is checked when the
    /// render configuration is applied, not when the flag is seen
    /// (`musializer.c:491-499` stores, `:563-569` validates).
    pub quality: Option<String>,
    pub save_project: Option<PathBuf>,
    pub analysis_bridge: Option<PathBuf>,
    pub auto_scenes: bool,
    pub reload_once: bool,
    pub ui_probe: Option<UiProbe>,

    /// One shared error flag, as in the C (`musializer.c:384`). Once set it
    /// poisons the later stages by short-circuit and the process exits 1.
    pub error: bool,
    /// Warnings in the order they were produced. Collected rather than printed
    /// so the parser is testable headlessly.
    pub warnings: Vec<String>,

    /// Diagnostics inherited from the Phase 1 vertical slice. Deliberately named
    /// so they cannot be confused with the product CLI: `tools/headless_check.sh`
    /// depends on them and on the report they produce.
    pub probe_frames: Option<u32>,
    pub probe_shot: Option<PathBuf>,
    /// Reopen this track halfway through a `--probe-frames` run.
    ///
    /// Exists to make the runtime track-swap path — detach, drop, drain, rebind
    /// the analyzer, reattach — reachable from a headless check. That sequence is
    /// otherwise only reachable by dropping a file on the window or clicking
    /// through a native picker, neither of which a capture script can drive, and it
    /// is the one path in this binary where getting an `unsafe` ordering wrong is a
    /// use-after-free rather than a wrong pixel.
    pub probe_reopen: Option<PathBuf>,
    /// `--size WxH`, the slice's own geometry flag. `--ui-probe size=` is the
    /// oracle's spelling; both are honoured.
    pub window: (i32, i32),
}

impl Cli {
    fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
        self.error = true;
    }

    /// `exit_status = command_line_error ? 1 : 0` (`musializer.c:618`).
    #[must_use]
    pub fn exit_status(&self) -> i32 {
        i32::from(self.error)
    }

    /// True when `--save-project` was given without `--render`, which
    /// **skips the main loop entirely** (`musializer.c:617`).
    #[must_use]
    pub fn exit_after_save(&self) -> bool {
        self.save_project.is_some() && self.render.is_none()
    }

    /// The last `--scene` on the command line, for the diagnostics report.
    #[must_use]
    pub fn requested_scene(&self) -> Option<SceneId> {
        self.actions.iter().rev().find_map(|action| match action {
            Action::SelectScene(id) => Some(*id),
            Action::AsciiImage(_) => Some(SceneId::AsciiField),
            _ => None,
        })
    }
}

/// Resolves a `--scene` name, including the six long aliases
/// (`scene_id_from_name`, `../musializer/src/plug.c:933-964`).
///
/// Comparison is exact and case-sensitive, as the C's `strcmp` is.
/// [`SceneId::from_stable_name`] handles only the ten persisted spellings — the
/// aliases are a CLI convenience and live here rather than in the shared
/// contract, because `.musi` never contains them.
#[must_use]
pub fn scene_from_cli_name(name: &str) -> Option<SceneId> {
    match name {
        "pulse-field" => Some(SceneId::PulseField),
        "orbital-lattice" => Some(SceneId::OrbitalLattice),
        "ascii-field" => Some(SceneId::AsciiField),
        "song-atlas" => Some(SceneId::SongAtlas),
        "spectral-terrarium" => Some(SceneId::SpectralTerrarium),
        "pentagram-orbits" => Some(SceneId::Pentagram),
        other => SceneId::from_stable_name(other),
    }
}

/// Every spelling `--scene` accepts, for the help text.
pub const SCENE_NAME_HELP: &str = "spectrum, pulse, orbital, ascii, atlas, terrarium, \
constellation, cadence, loom, pentagram (also pulse-field, orbital-lattice, ascii-field, \
song-atlas, spectral-terrarium, pentagram-orbits)";

/// The pre-pass, then the main loop. Mirrors `main` (`musializer.c:315-561`).
pub fn parse<I, S>(arguments: I) -> Outcome
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let argv: Vec<String> = arguments.into_iter().map(Into::into).collect();

    // The pre-pass scans all of argv before anything else. Both checks run per
    // index, so whichever of the two comes first in argv wins
    // (`musializer.c:317-326`).
    for argument in &argv {
        match argument.as_str() {
            "-h" | "--help" => return Outcome::Help,
            "--version" => return Outcome::Version,
            _ => {}
        }
    }

    let mut cli = Cli {
        window: DEFAULT_WINDOW,
        ..Cli::default()
    };

    // An index loop rather than an iterator, because `--render-window` consumes
    // two words and the C's index arithmetic there has a quirk worth keeping.
    let mut i = 0usize;
    while i < argv.len() {
        // The match arms are in the C's `if`-chain order (`musializer.c:398-551`),
        // which is also the order a parser has to match in to be identical.
        match argv[i].as_str() {
            "--mute" => cli.actions.push(Action::Mute),

            "--scene" => match value_of(&argv, i) {
                Some(name) => match scene_from_cli_name(name) {
                    Some(id) => cli.actions.push(Action::SelectScene(id)),
                    None => cli.warn("Unknown or missing command-line scene"),
                },
                None => cli.warn("Unknown or missing command-line scene"),
            },

            "--ascii-image" => match value_of(&argv, i) {
                Some(path) => cli.actions.push(Action::AsciiImage(PathBuf::from(path))),
                None => cli.warn("Could not load command-line ASCII image"),
            },

            "--event" => match value_of(&argv, i).and_then(parse_event) {
                Some(event) => cli.actions.push(Action::RecordEvent(event)),
                None => {
                    cli.warn("Invalid command-line event; expected type:seconds:id:value");
                }
            },

            "--route" => match value_of(&argv, i) {
                None => cli.warn(ROUTE_GRAMMAR_WARNING),
                Some(_) if cli.routes.len() >= ROUTE_CAPACITY => {
                    cli.warn(format!(
                        "Too many command-line routes (maximum {ROUTE_CAPACITY})"
                    ));
                }
                Some(spec) => match parse_route_spec(spec) {
                    // The C stores the raw spec and parses it in the deferred
                    // pass, so a malformed spec is reported there. Parsing here
                    // and deferring only the *application* reports it earlier
                    // with the same exit status and a better message.
                    Some(route) => cli.routes.push(route),
                    None => cli.warn(ROUTE_GRAMMAR_WARNING),
                },
            },

            "--render" => match value_of(&argv, i) {
                Some(path) => cli.render = Some(PathBuf::from(path)),
                None => cli.warn("Missing command-line render output path"),
            },

            "--render-window" => {
                let start = argv.get(i + 1).and_then(|text| parse_seconds(text));
                let duration = argv.get(i + 2).and_then(|text| parse_seconds(text));
                match (start, duration) {
                    (Some(start), Some(duration)) if duration > 0.0 => {
                        cli.render_window = Some((start, duration));
                    }
                    _ => cli.warn("Invalid render window; expected START_SECONDS DURATION_SECONDS"),
                }
                // `i += i + 2 < argc ? 2 : (argc - 1 - i);` (`musializer.c:473`).
                // With fewer than two values left it advances to the last index
                // instead of consuming two, so `--render-window 5` errors and
                // stops cleanly rather than looping. Reproduced: the loop must
                // terminate the same way.
                i += if i + 2 < argv.len() {
                    2
                } else {
                    argv.len().saturating_sub(1).saturating_sub(i)
                };
            }

            "--resolution" => match value_of(&argv, i).and_then(parse_resolution) {
                Some(size) => cli.resolution = Some(size),
                None => cli.warn("Invalid resolution; expected WIDTHxHEIGHT"),
            },

            "--fps" => match value_of(&argv, i).and_then(parse_positive_u32) {
                Some(fps) => cli.fps = Some(fps),
                None => cli.warn("Invalid render frame rate"),
            },

            "--quality" => match value_of(&argv, i) {
                Some(name) => cli.quality = Some(name.to_string()),
                None => cli.warn("Missing render quality"),
            },

            "--project" => match value_of(&argv, i) {
                Some(path) => cli.actions.push(Action::OpenProject(PathBuf::from(path))),
                None => cli.warn("Could not load command-line project"),
            },

            "--save-project" => match value_of(&argv, i) {
                Some(path) => cli.save_project = Some(PathBuf::from(path)),
                None => cli.warn("Missing project output path"),
            },

            "--analysis-bridge" => match value_of(&argv, i) {
                Some(path) => cli.analysis_bridge = Some(PathBuf::from(path)),
                None => cli.warn("Missing command-line analysis bridge path"),
            },

            "--auto-scenes" => cli.auto_scenes = true,
            "--reload-once" => cli.reload_once = true,

            "--ui-probe" => match value_of(&argv, i).and_then(parse_ui_probe) {
                Some(probe) => cli.ui_probe = Some(probe),
                None => cli.warn(
                    "Invalid --ui-probe spec; expected comma-separated panel=, \
                     fullscreen=, time=, or size= pairs",
                ),
            },

            // The slice's diagnostics. Not part of the oracle's surface.
            "--probe-frames" => match value_of(&argv, i).and_then(parse_positive_u32) {
                Some(frames) => cli.probe_frames = Some(frames),
                None => cli.warn("--probe-frames needs a positive frame count"),
            },
            "--probe-shot" => match value_of(&argv, i) {
                Some(path) => cli.probe_shot = Some(PathBuf::from(path)),
                None => cli.warn("--probe-shot needs a path"),
            },
            "--probe-reopen" => match value_of(&argv, i) {
                Some(path) => cli.probe_reopen = Some(PathBuf::from(path)),
                None => cli.warn("--probe-reopen needs a path"),
            },
            "--size" => match value_of(&argv, i).and_then(parse_resolution) {
                Some((width, height)) => cli.window = (width as i32, height as i32),
                None => cli.warn("--size wants WIDTHxHEIGHT"),
            },

            // Deliberate divergence: the C loads `--typo` as an audio path.
            other if other.starts_with("--") => {
                cli.warn(format!("Unknown option: {other}"));
            }

            positional => {
                // raylib's `IsFileExtension` is case-insensitive
                // (`musializer.c:546`), so `.MUSI` is a project too.
                let path = PathBuf::from(positional);
                if has_musi_extension(positional) {
                    cli.actions.push(Action::OpenProject(path));
                } else {
                    cli.actions.push(Action::LoadTrack(path));
                }
            }
        }

        // Every flag with a value consumed argv[i + 1] through `value_of`, so
        // step over it. `--render-window` already advanced past both of its own.
        if takes_one_value(argv[i].as_str()) && i + 1 < argv.len() {
            i += 1;
        }
        i += 1;
    }

    Outcome::Parsed(Box::new(cli))
}

const ROUTE_GRAMMAR_WARNING: &str = "Invalid, duplicate or missing command-line route; expected \
parameter:source:band:in_min:in_max:out_min:out_max[:curve][:noclamp]";

/// The flags that consume exactly one following argv word.
fn takes_one_value(flag: &str) -> bool {
    matches!(
        flag,
        "--scene"
            | "--ascii-image"
            | "--event"
            | "--route"
            | "--render"
            | "--resolution"
            | "--fps"
            | "--quality"
            | "--project"
            | "--save-project"
            | "--analysis-bridge"
            | "--ui-probe"
            | "--probe-frames"
            | "--probe-shot"
            | "--probe-reopen"
            | "--size"
    )
}

fn value_of(argv: &[String], index: usize) -> Option<&str> {
    argv.get(index + 1).map(String::as_str)
}

fn has_musi_extension(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("musi"))
}

/// `parse_positive_u32` (`musializer.c:63-73`). Non-zero decimal integer only.
#[must_use]
pub fn parse_positive_u32(text: &str) -> Option<u32> {
    let value: u32 = text.parse().ok()?;
    (value != 0).then_some(value)
}

/// `parse_seconds` (`musializer.c:75-85`). Finite and `>= 0`, whole string.
#[must_use]
pub fn parse_seconds(text: &str) -> Option<f64> {
    let value: f64 = text.parse().ok()?;
    (value.is_finite() && value >= 0.0).then_some(value)
}

/// `parse_resolution` (`musializer.c:87-100`).
///
/// The separator is the literal lowercase `x`; a second `x` anywhere after it is
/// an error, and the width text must be under 16 bytes.
#[must_use]
pub fn parse_resolution(text: &str) -> Option<(u32, u32)> {
    let (width_text, height_text) = text.split_once('x')?;
    if width_text.is_empty() || height_text.is_empty() || height_text.contains('x') {
        return None;
    }
    if width_text.len() >= 16 {
        return None;
    }
    Some((
        parse_positive_u32(width_text)?,
        parse_positive_u32(height_text)?,
    ))
}

/// `parse_command_line_event` (`musializer.c:19-61`).
///
/// `TYPE:SECONDS:ID:VALUE`. Note what is *not* checked: the host does not reject
/// a negative or non-finite timestamp, because the comment at `:60` puts
/// canonical validation in the plug. Reproduced — the bound lives downstream.
#[must_use]
pub fn parse_event(spec: &str) -> Option<EventRecord> {
    let mut fields = spec.splitn(4, ':');
    let type_text = fields.next()?;
    let seconds_text = fields.next()?;
    let id_text = fields.next()?;
    let value_text = fields.next()?;

    // Non-empty and strictly under 16 bytes (`musializer.c:23`).
    if type_text.is_empty() || type_text.len() >= 16 {
        return None;
    }
    let event_type = match type_text {
        "lyric" => EventType::Lyric,
        "semantic" => EventType::Semantic,
        "cue" => EventType::Cue,
        "custom" => EventType::Custom,
        _ => return None,
    };

    let timestamp_seconds: f64 = seconds_text.parse().ok()?;

    // Digits only: no sign, no whitespace, no hex (`musializer.c:33-40`).
    if id_text.is_empty() || !id_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let id: u64 = id_text.parse().ok()?;

    let value: f32 = value_text.parse().ok()?;

    let mut values = [0.0f32; VALUE_CAPACITY];
    values[0] = value;
    Some(EventRecord {
        timestamp_seconds,
        id,
        event_type: event_type as u32,
        value_count: 1,
        values,
    })
}

/// `scene_route_parse_spec` (`../musializer/src/scene_routes.c:109-189`).
///
/// `PARAM:SOURCE:BAND:IN_MIN:IN_MAX:OUT_MIN:OUT_MAX[:CURVE][:noclamp]`.
///
/// This lives here rather than in `core::scene::routes` because the CLI is its
/// only consumer and the persistence half of that module is still Agent B's to
/// finish. If a `ParameterMapping::parse_spec` lands there, this should delegate
/// to it rather than keep a second copy — a duplicated grammar drifts.
#[must_use]
pub fn parse_route_spec(spec: &str) -> Option<ParameterMapping> {
    // `SCENE_ROUTE_SPEC_CAPACITY` (`scene_routes.c:97`), checked at `:113`.
    if spec.is_empty() || spec.len() >= 256 {
        return None;
    }
    let fields: Vec<&str> = spec.split(':').collect();
    // At most 9 fields, at least 7 (`scene_routes.c:127`).
    if fields.len() < 7 || fields.len() > 9 {
        return None;
    }

    // The `settings.` prefix is optional; absent, it is prepended, so
    // `loom.weight` and `settings.loom.weight` are the same route
    // (`scene_routes.c:130-138`).
    let parameter = if fields[0].starts_with("settings.") {
        fields[0].to_string()
    } else {
        format!("settings.{}", fields[0])
    };
    // The scene is derived from the key, not from the selected scene
    // (`scene_routes.c:184-188`).
    let (scene, _index, _descriptor) = settings::descriptor_by_key(&parameter)?;

    let source = AnalysisSource::from_canonical_name(fields[1])?;

    let band: u32 = fields[2].parse().ok()?;
    let band_index = u16::try_from(band).ok()?;

    let input_min = parse_finite(fields[3])?;
    let input_max = parse_finite(fields[4])?;
    let output_min = parse_finite(fields[5])?;
    let output_max = parse_finite(fields[6])?;

    // Fields 8 and 9 are order-free optional tokens, repeats allowed and
    // last-wins (`scene_routes.c:163-182`). Defaults: linear, clamped.
    let mut interpolation = Interpolation::Linear;
    let mut clamp = true;
    for token in fields.iter().skip(7) {
        match *token {
            "clamp" => clamp = true,
            "noclamp" => clamp = false,
            other => interpolation = Interpolation::from_canonical_name(other)?,
        }
    }

    let route = ParameterMapping {
        parameter,
        source,
        band_index,
        input_min,
        input_max,
        output_min,
        output_max,
        interpolation,
        clamp,
    };
    // `scene_route_valid` carries the rest: band 0 unless the source is `band`,
    // a strictly increasing input range, and — the surprising one — a rejection
    // of equal output endpoints, because a flat mapping would be indistinguishable
    // from a persisted slider constant (`scene_routes.c:52-61`).
    route.is_valid_for(scene).then_some(route)
}

fn parse_finite(text: &str) -> Option<f64> {
    let value: f64 = text.parse().ok()?;
    value.is_finite().then_some(value)
}

/// `parse_ui_probe` (`musializer.c:131-250`). One argv word,
/// `key=value[,key=value...]`.
///
/// **Every key may appear at most once and an unknown key is an error**, not
/// last-wins and not a silent default. The rationale at `:128-130` is worth
/// keeping: a typo in a capture script must not quietly photograph the wrong UI
/// state.
#[must_use]
pub fn parse_ui_probe(spec: &str) -> Option<UiProbe> {
    if spec.is_empty() || spec.len() >= UI_PROBE_SPEC_CAPACITY {
        return None;
    }
    let mut probe = UiProbe::default();
    let mut seen: Vec<&str> = Vec::new();

    for pair in spec.split(',') {
        let (key, value) = pair.split_once('=')?;
        if key.is_empty() || value.is_empty() {
            return None;
        }
        if seen.contains(&key) {
            return None;
        }
        seen.push(key);

        match key {
            "panel" => probe.panel = UiPanel::from_name(value)?,
            "fullscreen" => probe.fullscreen = parse_probe_flag(value)?,
            "play" => probe.playing = parse_probe_flag(value)?,
            "lyric" => {
                // 1..=4096 inclusive; 0 is rejected (`musializer.c:177-187`).
                let selection: u32 = value.parse().ok()?;
                if !(1..=4096).contains(&selection) {
                    return None;
                }
                probe.lyric_selection = Some(selection);
            }
            "zoom" => {
                // 1.0..=100000.0 inclusive, finite (`musializer.c:188-197`).
                let zoom: f64 = value.parse().ok()?;
                if !zoom.is_finite() || !(1.0..=100_000.0).contains(&zoom) {
                    return None;
                }
                probe.timeline_zoom = Some(zoom);
            }
            "style" => {
                if value != "caption" {
                    return None;
                }
                probe.caption_style_pane = true;
            }
            "fonts" => {
                probe.font_browser = Some(if value == "consent" {
                    FontBrowserProbe::Consent
                } else {
                    FontBrowserProbe::Catalogue(PathBuf::from(value))
                });
            }
            "assist" => {
                if value != "confirm" {
                    return None;
                }
                probe.assist_confirmation = true;
            }
            "lyrics-file" => probe.lyrics_reference_path = Some(PathBuf::from(value)),
            "time" => probe.seek_seconds = Some(parse_seconds(value)?),
            "size" => probe.size = Some(parse_resolution(value)?),
            _ => return None,
        }
    }
    Some(probe)
}

/// `parse_ui_probe_flag` (`musializer.c:121-126`). Exactly `0` or `1`.
fn parse_probe_flag(text: &str) -> Option<bool> {
    match text {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

/// The help text. Modelled on `print_command_line_help`
/// (`musializer.c:252-313`) and kept in the same order, so somebody comparing
/// the two can do it top to bottom.
///
/// The C's first line is `Musializer 2026.07` with a capital M, while
/// `--version` prints `musializer 2026.07` in lowercase and a saved `.musi`
/// records `musializer-2026.07` with a hyphen — **three spellings of one
/// version, from three separate literals** (`musializer.c:255`, `:323`,
/// `plug.c:4293`). This build does not yet claim any of them; see the open
/// question in `REWRITE_PLAN.md`.
pub fn help_text(program: &str) -> String {
    format!(
        "\
Musializer (Rust rewrite) - deterministic music visualization workspace

Usage: {program} [options] [audio-file | project.musi]

Workspace:
  --project FILE          Open a .musi project
  --save-project FILE     Atomically save the current workspace
  --scene NAME            {SCENE_NAME_HELP}
  --ascii-image FILE      Import an image and select ASCII Field
  --event SPEC            Add type:seconds:id:value to the manual lane
  --route SPEC            Drive a scene setting from live audio:
                          parameter:source:band:in_min:in_max:
                          out_min:out_max[:curve][:noclamp], e.g.
                          loom.weight:band:2:0:1:0.4:2.2:smoothstep
                          Sources: rms, peak, spectral_flux,
                          beat_phase, band. Curves: step, linear,
                          smoothstep, ease_in, ease_out
  --analysis-bridge FILE  Import a verified analysis bridge
  --auto-scenes           Enable imported scene suggestions

Export:
  --render FILE           Render MP4 and exit
  --render-window S D     Render only D seconds starting at S.
                          Frames match the same span of a full render
  --resolution WIDTHxHEIGHT
  --fps N
  --quality NAME          balanced, high, or master

Diagnostics:
  --mute                  Start with the output volume at zero
  --reload-once           Exercise one hot-reload handoff (unsupported)
  --ui-probe SPEC         Open a workspace panel and park the transport
                          for reproducible headless UI capture. SPEC is
                          comma-separated key=value pairs:
                          panel=none|tune|export|lyrics|assist,
                          fullscreen=0|1, time=SECONDS, size=WIDTHxHEIGHT,
                          assist=confirm arms the Assist confirmation
                          prompt, lyric=N selects the nth lyric cue
                          (needs panel=assist),
                          zoom=FACTOR zooms the timeline strip about the
                          playhead (1 = whole track),
                          style=caption shows the caption typography
                          pane (needs panel=lyrics),
                          lyrics-file=PATH selects an authored lyric
                          sheet for the next Assist lyrics run,
                          fonts=consent shows the face browser's network
                          consent panel; fonts=PATH loads a family list
                          from disk instead, so a capture never opens a
                          network connection (needs panel=lyrics),
                          play=0|1. The transport is parked unless
                          play=1; audio-reactive scenes need play=1 but
                          then capture a frame that is not reproducible.
                          Every panel except none needs a loaded track
  --size WIDTHxHEIGHT     Preview window geometry
  --probe-frames N        Render N frames, print the report, and exit
  --probe-shot PATH       Write a PNG of the last rendered frame
  --probe-reopen PATH     Swap to PATH halfway through --probe-frames, to
                          exercise the runtime track-swap path
  -h, --help              Show this help without opening a window
  --version               Show the version
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(arguments: &[&str]) -> Cli {
        match parse(arguments.iter().copied()) {
            Outcome::Parsed(cli) => *cli,
            other => panic!("expected a parse, got {other:?}"),
        }
    }

    #[test]
    fn help_and_version_win_from_any_position_and_over_invalid_arguments() {
        // The pre-pass runs before anything else, so a broken command line still
        // prints help and exits 0 (`musializer.c:317-326`).
        assert!(matches!(parse(["--help"]), Outcome::Help));
        assert!(matches!(parse(["--fps", "0", "-h"]), Outcome::Help));
        assert!(matches!(parse(["--version"]), Outcome::Version));
        assert!(matches!(
            parse(["--nonsense", "--version"]),
            Outcome::Version
        ));
        // Whichever comes first in argv wins.
        assert!(matches!(parse(["--help", "--version"]), Outcome::Help));
        assert!(matches!(parse(["--version", "--help"]), Outcome::Version));
    }

    #[test]
    fn scene_names_accept_the_ten_short_and_six_long_spellings() {
        let expected = [
            ("spectrum", SceneId::Spectrum),
            ("pulse", SceneId::PulseField),
            ("pulse-field", SceneId::PulseField),
            ("orbital", SceneId::OrbitalLattice),
            ("orbital-lattice", SceneId::OrbitalLattice),
            ("ascii", SceneId::AsciiField),
            ("ascii-field", SceneId::AsciiField),
            ("atlas", SceneId::SongAtlas),
            ("song-atlas", SceneId::SongAtlas),
            ("terrarium", SceneId::SpectralTerrarium),
            ("spectral-terrarium", SceneId::SpectralTerrarium),
            ("constellation", SceneId::Constellation),
            ("cadence", SceneId::Cadence),
            ("loom", SceneId::Loom),
            ("pentagram", SceneId::Pentagram),
            ("pentagram-orbits", SceneId::Pentagram),
        ];
        for (name, id) in expected {
            assert_eq!(scene_from_cli_name(name), Some(id), "{name}");
        }
        // Case-sensitive, like the C's strcmp.
        assert_eq!(scene_from_cli_name("Spectrum"), None);
        assert_eq!(scene_from_cli_name("pulse field"), None);
        assert_eq!(scene_from_cli_name(""), None);
    }

    #[test]
    fn scene_flag_records_an_action_and_a_bad_name_errors() {
        let cli = parsed(&["--scene", "loom"]);
        assert_eq!(cli.actions, vec![Action::SelectScene(SceneId::Loom)]);
        assert!(!cli.error);
        assert_eq!(cli.requested_scene(), Some(SceneId::Loom));

        let cli = parsed(&["--scene", "nope"]);
        assert!(cli.error);
        assert_eq!(cli.exit_status(), 1);
        assert_eq!(cli.actions, vec![]);

        // A missing value is the same failure.
        assert!(parsed(&["--scene"]).error);
    }

    #[test]
    fn ascii_image_implies_the_ascii_scene() {
        let cli = parsed(&["--ascii-image", "logo.png"]);
        assert_eq!(cli.requested_scene(), Some(SceneId::AsciiField));
    }

    #[test]
    fn inputs_keep_argv_order_so_a_later_one_can_overwrite_an_earlier_one() {
        let cli = parsed(&["a.wav", "--project", "show.musi", "b.mp3"]);
        assert_eq!(
            cli.actions,
            vec![
                Action::LoadTrack(PathBuf::from("a.wav")),
                Action::OpenProject(PathBuf::from("show.musi")),
                Action::LoadTrack(PathBuf::from("b.mp3")),
            ]
        );
        assert!(!cli.error);
    }

    #[test]
    fn a_positional_musi_is_a_project_case_insensitively() {
        // raylib's IsFileExtension is case-insensitive (`musializer.c:546`).
        let cli = parsed(&["show.MUSI"]);
        assert_eq!(
            cli.actions,
            vec![Action::OpenProject(PathBuf::from("show.MUSI"))]
        );
        let cli = parsed(&["song.wav"]);
        assert_eq!(
            cli.actions,
            vec![Action::LoadTrack(PathBuf::from("song.wav"))]
        );
    }

    #[test]
    fn routes_are_collected_separately_from_the_ordered_actions() {
        // The deferral is the point: applying a route before a --project would
        // let project hydration overwrite it (`musializer.c:446-448`).
        let cli = parsed(&[
            "--route",
            "loom.weight:band:2:0:1:0.4:2.2:smoothstep",
            "song.wav",
        ]);
        assert_eq!(
            cli.actions,
            vec![Action::LoadTrack(PathBuf::from("song.wav"))]
        );
        assert_eq!(cli.routes.len(), 1);
        assert_eq!(cli.routes[0].parameter, "settings.loom.weight");
        assert_eq!(cli.routes[0].source, AnalysisSource::Band);
        assert_eq!(cli.routes[0].band_index, 2);
        assert_eq!(cli.routes[0].interpolation, Interpolation::Smoothstep);
        assert!(cli.routes[0].clamp);
        assert!(!cli.error);
    }

    #[test]
    fn a_route_spec_takes_the_settings_prefix_either_way() {
        let bare = parse_route_spec("loom.weight:rms:0:0:1:0.4:2.2").unwrap();
        let prefixed = parse_route_spec("settings.loom.weight:rms:0:0:1:0.4:2.2").unwrap();
        assert_eq!(bare, prefixed);
        assert_eq!(bare.parameter, "settings.loom.weight");
    }

    #[test]
    fn route_optional_tokens_are_order_free_and_last_wins() {
        let a = parse_route_spec("loom.weight:rms:0:0:1:0.4:2.2:noclamp:ease_in").unwrap();
        let b = parse_route_spec("loom.weight:rms:0:0:1:0.4:2.2:ease_in:noclamp").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.interpolation, Interpolation::EaseIn);
        assert!(!a.clamp);

        // Repeats are allowed and the last one wins (`scene_routes.c:163-182`).
        let c = parse_route_spec("loom.weight:rms:0:0:1:0.4:2.2:noclamp:clamp").unwrap();
        assert!(c.clamp);
    }

    #[test]
    fn route_specs_are_rejected_for_the_reasons_the_c_rejects_them() {
        // Fewer than seven fields.
        assert!(parse_route_spec("loom.weight:rms:0:0:1:0.4").is_none());
        // A tenth colon.
        assert!(parse_route_spec("loom.weight:rms:0:0:1:0.4:2.2:linear:clamp:extra").is_none());
        // Unknown setting key, unknown source, unknown token.
        assert!(parse_route_spec("loom.nope:rms:0:0:1:0.4:2.2").is_none());
        assert!(parse_route_spec("loom.weight:volume:0:0:1:0.4:2.2").is_none());
        assert!(parse_route_spec("loom.weight:rms:0:0:1:0.4:2.2:wobble").is_none());
        // A non-zero band with a non-band source.
        assert!(parse_route_spec("loom.weight:rms:3:0:1:0.4:2.2").is_none());
        // A band index past the analyzer's maximum.
        assert!(parse_route_spec("loom.weight:band:256:0:1:0.4:2.2").is_none());
        // A non-increasing input range.
        assert!(parse_route_spec("loom.weight:rms:0:1:1:0.4:2.2").is_none());
        // Equal output endpoints: a flat mapping is rejected outright, because it
        // would be byte-identical to a persisted slider constant on reopen
        // (`scene_routes.c:57-60`).
        assert!(parse_route_spec("loom.weight:rms:0:0:1:1.0:1.0").is_none());
        // Non-finite endpoints.
        assert!(parse_route_spec("loom.weight:rms:0:0:1:0.4:inf").is_none());
    }

    #[test]
    fn too_many_routes_errors_on_the_two_hundred_and_fifty_seventh() {
        let mut arguments: Vec<String> = Vec::new();
        for band in 0..ROUTE_CAPACITY + 1 {
            arguments.push("--route".into());
            // A distinct band per route keeps every spec valid on its own; the
            // duplicate-parameter rule belongs to the route table, not here.
            arguments.push(format!("loom.weight:band:{band}:0:1:0.4:2.2"));
        }
        let cli = match parse(arguments) {
            Outcome::Parsed(cli) => *cli,
            other => panic!("expected a parse, got {other:?}"),
        };
        assert_eq!(cli.routes.len(), ROUTE_CAPACITY);
        assert!(cli.error);
        assert!(cli.warnings.iter().any(|w| w.contains("Too many")));
    }

    #[test]
    fn events_parse_the_four_types_and_reject_everything_else() {
        let event = parse_event("lyric:12.5:1:0.75").unwrap();
        assert_eq!(event.timestamp_seconds, 12.5);
        assert_eq!(event.id, 1);
        assert_eq!(event.kind(), Some(EventType::Lyric));
        assert_eq!(event.value_count, 1);
        assert_eq!(event.values(), &[0.75]);

        for (spec, kind) in [
            ("semantic:0:9:1", EventType::Semantic),
            ("cue:0:9:1", EventType::Cue),
            ("custom:0:9:1", EventType::Custom),
        ] {
            assert_eq!(parse_event(spec).unwrap().kind(), Some(kind), "{spec}");
        }

        // An unknown type, an empty type, and a 16-byte type.
        assert!(parse_event("beat:1:1:1").is_none());
        assert!(parse_event(":1:1:1").is_none());
        assert!(parse_event("aaaaaaaaaaaaaaaa:1:1:1").is_none());
        // Missing fields.
        assert!(parse_event("lyric:1:1").is_none());
        // The id is digits only: no sign, no whitespace, no hex
        // (`musializer.c:33-40`).
        assert!(parse_event("lyric:1:-1:1").is_none());
        assert!(parse_event("lyric:1:0x1:1").is_none());
        assert!(parse_event("lyric:1::1").is_none());
        // Trailing junk after the value must not be tolerated.
        assert!(parse_event("lyric:1:1:1junk").is_none());
    }

    #[test]
    fn a_negative_event_timestamp_reaches_the_model_unrejected() {
        // Deliberate: the C's comment at `musializer.c:60` puts canonical
        // validation in the plug, so the host parser accepts it. A test pins
        // this so nobody "fixes" the CLI and moves the bound.
        let event = parse_event("lyric:-3.0:1:0.5").unwrap();
        assert_eq!(event.timestamp_seconds, -3.0);
    }

    #[test]
    fn render_window_takes_two_argv_words() {
        let cli = parsed(&["--render-window", "5", "2.5"]);
        assert_eq!(cli.render_window, Some((5.0, 2.5)));
        assert!(!cli.error);
    }

    #[test]
    fn render_window_with_one_value_errors_and_terminates() {
        // The C's index arithmetic advances to the last index rather than past
        // two words, so this errors and stops cleanly (`musializer.c:473`).
        let cli = parsed(&["--render-window", "5"]);
        assert!(cli.error);
        assert_eq!(cli.render_window, None);
    }

    #[test]
    fn render_window_rejects_a_non_positive_duration() {
        assert!(parsed(&["--render-window", "5", "0"]).error);
        assert!(parsed(&["--render-window", "-1", "2"]).error);
        assert!(parsed(&["--render-window", "nan", "2"]).error);
    }

    #[test]
    fn render_window_still_lets_later_flags_parse() {
        let cli = parsed(&["--render-window", "1", "2", "--fps", "30"]);
        assert_eq!(cli.render_window, Some((1.0, 2.0)));
        assert_eq!(cli.fps, Some(30));
        assert!(!cli.error);
    }

    #[test]
    fn resolution_wants_one_lowercase_x_and_two_positive_integers() {
        assert_eq!(parse_resolution("1920x1080"), Some((1920, 1080)));
        assert_eq!(parse_resolution("1920X1080"), None);
        assert_eq!(parse_resolution("1920x1080x2"), None);
        assert_eq!(parse_resolution("x1080"), None);
        assert_eq!(parse_resolution("1920x"), None);
        assert_eq!(parse_resolution("0x1080"), None);
        assert_eq!(parse_resolution("1920x0"), None);
        assert_eq!(parse_resolution("-1x1"), None);
        // The width text must be under 16 bytes (`musializer.c:93-95`).
        assert_eq!(parse_resolution("1234567890123456x1"), None);
    }

    #[test]
    fn fps_rejects_zero_and_anything_non_integral() {
        assert_eq!(parse_positive_u32("30"), Some(30));
        assert_eq!(parse_positive_u32("0"), None);
        assert_eq!(parse_positive_u32("-1"), None);
        assert_eq!(parse_positive_u32("29.97"), None);
        assert_eq!(parse_positive_u32(""), None);
        assert!(parsed(&["--fps", "0"]).error);
    }

    #[test]
    fn quality_is_stored_unvalidated_and_checked_when_applied() {
        // The C stores the string and validates it in plug_configure_render, so
        // the flag itself never fails on a bad name (`musializer.c:491-499`).
        let cli = parsed(&["--quality", "ludicrous"]);
        assert_eq!(cli.quality.as_deref(), Some("ludicrous"));
        assert!(!cli.error);
        assert_eq!(Quality::from_name("ludicrous"), None);
        assert_eq!(Quality::from_name("master"), Some(Quality::Master));
        // A missing value does fail here, as in the C.
        assert!(parsed(&["--quality"]).error);
    }

    #[test]
    fn save_project_without_render_skips_the_main_loop() {
        // `exit_after_save` (`musializer.c:617`).
        let cli = parsed(&["--save-project", "out.musi"]);
        assert!(cli.exit_after_save());
        let cli = parsed(&["--save-project", "out.musi", "--render", "out.mp4"]);
        assert!(!cli.exit_after_save());
    }

    #[test]
    fn the_valueless_flags_set_their_own_bits() {
        let cli = parsed(&["--mute", "--auto-scenes", "--reload-once"]);
        assert_eq!(cli.actions, vec![Action::Mute]);
        assert!(cli.auto_scenes);
        assert!(cli.reload_once);
        assert!(!cli.error);
    }

    #[test]
    fn ui_probe_parses_every_documented_key() {
        let probe = parse_ui_probe(
            "panel=assist,assist=confirm,lyric=3,play=1,fullscreen=0,\
             time=12.5,zoom=4,size=960x640,lyrics-file=/tmp/a.txt",
        )
        .unwrap();
        assert_eq!(probe.panel, UiPanel::Assist);
        assert!(probe.assist_confirmation);
        assert_eq!(probe.lyric_selection, Some(3));
        assert!(probe.playing);
        assert!(!probe.fullscreen);
        assert_eq!(probe.seek_seconds, Some(12.5));
        assert_eq!(probe.timeline_zoom, Some(4.0));
        assert_eq!(probe.size, Some((960, 640)));
        assert_eq!(
            probe.lyrics_reference_path,
            Some(PathBuf::from("/tmp/a.txt"))
        );

        let probe = parse_ui_probe("panel=lyrics,style=caption,fonts=consent").unwrap();
        assert!(probe.caption_style_pane);
        assert_eq!(probe.font_browser, Some(FontBrowserProbe::Consent));

        let probe = parse_ui_probe("panel=lyrics,fonts=/tmp/families.json").unwrap();
        assert_eq!(
            probe.font_browser,
            Some(FontBrowserProbe::Catalogue(PathBuf::from(
                "/tmp/families.json"
            )))
        );
    }

    #[test]
    fn ui_probe_rejects_a_repeated_key_an_unknown_key_and_a_missing_equals() {
        // Not last-wins: a typo in a capture script must not quietly photograph
        // the wrong UI state (`musializer.c:128-130`, `:244-246`).
        assert!(parse_ui_probe("panel=tune,panel=export").is_none());
        assert!(parse_ui_probe("pannel=tune").is_none());
        assert!(parse_ui_probe("panel").is_none());
        assert!(parse_ui_probe("=tune").is_none());
        assert!(parse_ui_probe("panel=").is_none());
        assert!(parse_ui_probe("").is_none());
    }

    #[test]
    fn ui_probe_value_ranges_are_the_c_ranges() {
        assert!(parse_ui_probe("lyric=0").is_none());
        assert!(parse_ui_probe("lyric=1").is_some());
        assert!(parse_ui_probe("lyric=4096").is_some());
        assert!(parse_ui_probe("lyric=4097").is_none());
        assert!(parse_ui_probe("zoom=0.999").is_none());
        assert!(parse_ui_probe("zoom=1").is_some());
        assert!(parse_ui_probe("zoom=100000").is_some());
        assert!(parse_ui_probe("zoom=100001").is_none());
        assert!(parse_ui_probe("zoom=inf").is_none());
        // Flags are exactly 0 or 1, not "true"/"yes".
        assert!(parse_ui_probe("play=true").is_none());
        assert!(parse_ui_probe("fullscreen=2").is_none());
        // style= and assist= accept exactly one word each.
        assert!(parse_ui_probe("style=body").is_none());
        assert!(parse_ui_probe("assist=cancel").is_none());
        // time= is parse_seconds: finite and non-negative.
        assert!(parse_ui_probe("time=-1").is_none());
    }

    #[test]
    fn a_too_long_ui_probe_spec_is_rejected() {
        let spec = format!("lyrics-file={}", "a".repeat(UI_PROBE_SPEC_CAPACITY));
        assert!(parse_ui_probe(&spec).is_none());
    }

    #[test]
    fn an_unknown_flag_is_an_error_rather_than_an_audio_path() {
        // The deliberate divergence. The C loads `--typo` as a track
        // (`musializer.c:546-550`); we say what happened instead.
        let cli = parsed(&["--typo"]);
        assert!(cli.error);
        assert_eq!(cli.actions, vec![]);
        assert!(cli.warnings.iter().any(|w| w.contains("Unknown option")));
    }

    #[test]
    fn one_error_does_not_stop_the_rest_of_the_line_from_parsing() {
        // The C's shared error flag plus a loop that keeps going
        // (`musializer.c:384`, `:618`).
        let cli = parsed(&["--fps", "0", "song.wav", "--auto-scenes"]);
        assert!(cli.error);
        assert_eq!(cli.exit_status(), 1);
        assert_eq!(
            cli.actions,
            vec![Action::LoadTrack(PathBuf::from("song.wav"))]
        );
        assert!(cli.auto_scenes);
    }

    #[test]
    fn the_slice_diagnostics_still_parse() {
        // tools/headless_check.sh depends on these three.
        let cli = parsed(&[
            "fixture.wav",
            "--size",
            "1280x720",
            "--probe-frames",
            "240",
            "--probe-shot",
            "build/shot.png",
        ]);
        assert!(!cli.error);
        assert_eq!(cli.window, (1280, 720));
        assert_eq!(cli.probe_frames, Some(240));
        assert_eq!(cli.probe_shot, Some(PathBuf::from("build/shot.png")));
    }

    #[test]
    fn an_empty_command_line_parses_to_defaults() {
        let cli = parsed(&[]);
        assert!(!cli.error);
        assert_eq!(cli.window, DEFAULT_WINDOW);
        assert!(cli.actions.is_empty());
        assert!(cli.routes.is_empty());
        assert_eq!(cli.exit_status(), 0);
    }

    #[test]
    fn help_text_names_every_scene() {
        let text = help_text("musializer");
        for id in SceneId::ALL {
            assert!(
                text.contains(id.stable_name()),
                "help omits {}",
                id.stable_name()
            );
        }
    }
}
