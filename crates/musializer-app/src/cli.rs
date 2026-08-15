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
//!    acts on the state left by everything before it. That is why parsing
//!    produces an ordered [`Action`] list rather than a flat struct of options:
//!    replaying the list left to right is the only way to preserve it.
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
//! - **Number syntax is Rust's, not `strtod`'s.** `str::parse` rejects leading
//!   whitespace and hex float literals, both of which `strtod` accepts. Nobody
//!   types `--fps " 30"`, and treating it as an error is the safer direction.
//! - `--reload-once` parses and is reported unsupported when applied: hot reload
//!   is an explicit first-pass non-goal.

use std::path::PathBuf;

use musializer_core::scene::events::{EventRecord, EventType, VALUE_CAPACITY};
use musializer_core::scene::routes::{self, ParameterMapping};
use musializer_core::scene::{settings, SceneId};

use crate::ui::scale::UiScalePreference;

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
    /// Arms a tap run at the seek position and presses the tap key this many
    /// times (UX0-C03).
    ///
    /// **Invented**, and for the reason every probe key here is: Xvfb has no
    /// keyboard, so without it the play-and-tap loop is unphotographable. An
    /// armed run and a disarmed one differ by one 11 px line of hint text, and a
    /// stamp that landed leaves exactly the picture a stamp that was refused
    /// does — the difference is in the cue spans, which is what the `lyrics:`
    /// report line carries.
    pub lyric_taps: Option<u32>,
    /// Presses Ctrl+Z once, after any `lyric-tap=` (UX0-B03).
    ///
    /// One step rather than a count: the assertion worth making is that the
    /// document *came back*, and one step against a known batch proves the
    /// recording, the restore and the label. Depth is unit-tested.
    pub lyric_undo: bool,
    /// Timeline strip zoom; 1 is the whole track.
    pub timeline_zoom: Option<f64>,
    /// `style=caption`: show the caption typography pane. Needs `panel=lyrics`.
    pub caption_style_pane: bool,
    /// `style=effects`: show the caption effects form inside that pane.
    pub caption_effects_pane: bool,
    /// `fonts=consent` shows the network-consent panel; `fonts=PATH` loads a
    /// family list from disk, so a capture never contacts Google.
    pub font_browser: Option<FontBrowserProbe>,
    /// `picker=ink|plate`: open the caption pane's free colour picker. Implies
    /// `style=caption`, since the picker is a disclosure inside that pane.
    pub caption_picker: Option<CaptionPickerProbe>,
    /// `tune=pulse|hue`: open the effects form's drive tuning editor
    /// (UX0-C14). Implies `style=effects`; refuses to combine with `picker=`,
    /// which claims the same column.
    pub caption_tune: Option<CaptionTuneProbe>,
    /// `route=KEY`: open the Tune inspector's route editor on the setting that
    /// `KEY` names, e.g. `route=loom.weight` or the full `settings.loom.weight`.
    ///
    /// **Invented, not the oracle's.** The frozen C has no probe for it because
    /// nothing in the C can drive a headless capture at all; the editor is a
    /// 260 px block of drawing that a click is otherwise the only way to reach,
    /// and a surface nothing photographs does not get reviewed. Same rationale as
    /// `--probe-frames` and `--probe-shot`. Needs `panel=tune`, and the key must
    /// belong to the scene being drawn.
    pub route_editor: Option<String>,
    /// `assist=confirm|candidate|running|failed`: put the Assist panel in one of
    /// its consequential states (review 4.2).
    pub assist: Option<AssistProbe>,
    /// Selects an authored lyric sheet for the next Assist lyrics run.
    pub lyrics_reference_path: Option<PathBuf>,
    /// `time=SECONDS`, which also sets the seek-requested flag.
    pub seek_seconds: Option<f64>,
    /// Host-side window geometry, not plug state (`musializer.c:238-243`).
    pub size: Option<(u32, u32)>,
    /// Diagnostic-only workspace split positions in logical UI units. These let
    /// the capture gate review layouts that are otherwise reachable only by a
    /// pointer drag.
    pub sidebar_width: Option<f32>,
    pub inspector_width: Option<f32>,
    pub timeline_height: Option<f32>,
    /// `hover=XxY`: park the pointer at a window coordinate.
    ///
    /// **Invented, and the only way a tooltip can be reviewed.** A headless run has
    /// no pointer — `GetMousePosition` is the origin forever — so every hover state
    /// in this interface was unphotographable, which by this project's own rule
    /// means unreviewed. That rule has been paid for twice already: the welcome
    /// screen ran a whole session in raylib's bitmap face because no capture
    /// included it, and three whole-track derivations drew plausible fallbacks for
    /// two bands for the same reason.
    ///
    /// Parks rather than moves: the position is reasserted every frame, so the tip
    /// is in the shot regardless of how many frames the run lasts.
    pub hover: Option<(f32, f32)>,
    /// `click=XxY`: park the pointer there and press it, once (EX1).
    ///
    /// **Invented, and it is the missing half of `hover=`.** `hover=` proved a
    /// control lights up under the pointer; nothing could prove it *takes* the
    /// press. That gap is exactly where the operator's "I can't pick different
    /// sizes" lived: the export panel's resolution buttons highlight correctly at
    /// every scale, so every check this repository had said the row was fine.
    ///
    /// Xvfb has no button any more than it has a pointer or a wheel, and raylib
    /// exposes no way to synthesize one, so the press is injected at
    /// [`Widgets`](crate::ui::widgets::Widgets)' own pointer seam rather than at
    /// the device. That bounds what it can reach — controls that go through
    /// `Widgets::button`/`::slider`, which is every button in the shell — and it
    /// deliberately does **not** drive the gestures that read raylib directly
    /// (timeline scrub, cue drags, the pan). Those own their own state machines
    /// and would need their own probe.
    ///
    /// Delivered across three consecutive frames — press, hold, release — because
    /// a claim is taken on the press edge and only ever cashed by the same widget
    /// on the release edge. A one-frame press and release in the same frame is
    /// precisely the case the claim rule drops, so a probe that did that would
    /// report a working control as broken.
    pub click: Option<(f32, f32)>,
    /// `wheel=NOTCHES`: deliver one wheel event, on one frame, wherever
    /// `hover=` parked the pointer (LX2).
    ///
    /// **Invented for the same reason as `hover=`, and it is the other half of
    /// it.** Xvfb has no wheel any more than it has a pointer, so "the wheel
    /// zooms the timeline from the lane you are aiming in" was a binding no
    /// capture could reach — and the lanes are drawn by three different modules
    /// against three different rectangles, which is precisely where a region
    /// test is wrong in a way that reads correctly in the source.
    ///
    /// One frame, not every frame: the shell consumes it on the first frame it
    /// draws, so a 30-frame probe zooms by one notch rather than by thirty.
    pub wheel: Option<f32>,
    /// `wheel-shift=0|1`: hold Shift for the `wheel=` notch, making it a pan
    /// across the timed lanes rather than a zoom (D4).
    ///
    /// **Invented for the same reason as `wheel=` itself.** Xvfb has no keyboard
    /// modifier under a wheel any more than it has a wheel, so "Shift-wheel pans"
    /// was a binding no capture could reach — and a pan and a zoom both move the
    /// view, so a report line that only said the view changed could not tell them
    /// apart. `timeline:` prints the span, which a pan leaves alone and a zoom
    /// does not, and that is what separates them.
    pub wheel_shift: bool,
    /// `middle-drag=FROMxTO`: middle-press at `FROM`, drag to `TO`, release (D4).
    ///
    /// **Invented, and it is to the middle button what `click=` is to the left
    /// one.** `click=` goes through `Widgets`' pointer seam, which the timeline's
    /// pan does not use — the pan reads `MOUSE_BUTTON_MIDDLE` from raylib
    /// directly, because it is not a widget and claims nothing from the bank. So
    /// the whole middle-drag pan, including the release that must not strand the
    /// claim, was unreachable from any capture.
    ///
    /// Both values are **x coordinates in the same logical space as `hover=`**,
    /// which is where the y comes from, because a pan is a one-axis gesture and a
    /// probe that could aim it off the lane vertically would just be a way to
    /// write a test that presses nothing. Spelled `FROMxTO` for the same reason
    /// `hover=` is `XxY`: the spec is comma-separated.
    pub middle_drag: Option<(f32, f32)>,
    /// `scene-pick=NAME`: click the scene browser's tile for `NAME`, once (LX3).
    ///
    /// **Invented, and it exists because the gate could not reach the one
    /// gesture the operator's bug reports were about.** `--scene NAME` is the
    /// command line's *startup* scene and deliberately still sets the base
    /// scene; a scene *click* is a different operation once an automatic plan
    /// is running, where it retargets one segment and leaves the plan driving.
    /// Nothing headless could press that tile, so the difference between "the
    /// plan survived the click" and "the click switched the plan off" — the
    /// whole of LX3-a — was unphotographable.
    ///
    /// Delivered once, at the end of the probe stage, so it lands after
    /// `time=` has parked the playhead and the segment it targets is the one a
    /// capture will show.
    pub scene_pick: Option<SceneId>,
    /// `tune-seed=N`: replace the Tune panel's Surprise/Nudge seed (PX6).
    ///
    /// **Invented, and it is what makes a random feature photographable at
    /// all.** The panel seeds from a press counter rather than a clock for
    /// exactly this reason, but the counter still starts wherever the session
    /// left it. Pinning the seed makes a Surprise capture reproducible, so the
    /// gate can assert the *values* it produced rather than only that something
    /// moved.
    pub tune_seed: Option<u64>,
    /// `tune-explore=A+B+...`: run Tune exploration actions before the first
    /// frame (PX6). One of `nudge`, `surprise`, `compare`, `revert`, `keep`.
    ///
    /// **Invented because the claim is about a *sequence*.** `click=` presses
    /// one control per run, and "Surprise then Revert restores the exact
    /// tuning" cannot be stated in one press. The two are complementary and the
    /// gate uses both: `click=` proves the button takes the press, this proves
    /// what the press means.
    pub tune_explore: Option<String>,
    /// `tune-type=KEY:VALUE`: commit a typed value through the Tune panel's own
    /// parse/clamp path (PX6).
    ///
    /// **Invented because Xvfb has no keyboard any more than it has a wheel.**
    /// The value goes through [`musializer_core::ui::tune_explore::parse_typed`]
    /// and the descriptor, so a capture can assert that typing `99` into a
    /// 0.40..2.00 slider writes 2.00 rather than being silently rejected.
    pub tune_type: Option<String>,
    /// `drop=PATH`: synthesize a file drop of `PATH`, once (D1).
    ///
    /// **Invented, for the reason `hover=`, `wheel=` and `scene-pick=` were.**
    /// Xvfb has no drag-and-drop, so the typed dispatch in
    /// `ui::shell::dropped_files` — a three-arm branch on an extension — was
    /// unreachable from any check this repository has. That is the exact shape
    /// the export panel's SIZE row had: a dispatch that reads correctly in the
    /// source while sending everything to one arm, with every gate green.
    ///
    /// Delivered on the first frame that draws and consumed there, so a 30-frame
    /// probe drops one file rather than thirty. It goes through the same
    /// classifier the device path does — not around it — so what the probe
    /// proves is what a real drop would do.
    pub drop_file: Option<PathBuf>,
    /// `save-to=PATH`: the destination a file picker would have returned
    /// (UX0-C01, UX0-C10).
    ///
    /// **Invented, and it is `click=`'s missing half for the two controls this
    /// application exists to reach.** The export panel's render button and its
    /// still button both open a modal picker, and Xvfb has no picker any more
    /// than it has a pointer — so a capture could press either one and prove
    /// only that the press was claimed. With this, the press produces the file,
    /// and the gate can assert an MP4's duration and frame count, or two runs of
    /// a still hashing the same.
    ///
    /// It substitutes for the *dialog*, not for the decision: the path is used
    /// exactly where the picker's answer would have been, and every refusal
    /// after it — the `.mp4` extension rule, the alias checks, the geometry —
    /// still applies.
    pub save_to: Option<PathBuf>,
    /// `share-frame=playhead`: seed the Export panel's first-frame choice from
    /// `time=` before a single `click=` is delivered.
    ///
    /// **Invented because the claim is about two presses.** A person first
    /// chooses the playhead frame and then presses Render, while `click=` can
    /// prove only one control per run. This seeds the first decision so the
    /// second still travels through the real button and destination seam.
    pub share_frame_playhead: bool,
    /// `audio-stall=MS`: block only the main/refill thread once during a probe.
    /// This is a bounded negative-control hook for the output-underrun counter;
    /// the audio device thread remains live and must observe the starvation.
    pub audio_stall_ms: Option<u64>,
    /// `protocol-answer=ID:CHOICE[+ID:CHOICE...]`: answer protocol items by id
    /// (HX-5).
    ///
    /// **Invented because Xvfb can neither hear the track nor press `2`**, and
    /// by *id* rather than by pixel because GX-1 is what a pixel-addressed
    /// probe costs: every coordinate in the gate is a latent hard-coded layout.
    pub protocol_answer: Option<String>,
    /// `protocol-flip=ID`: put the item's other look on record before the
    /// answer, so the gate can compare the recorded variant order against the
    /// JSONL (HX-5).
    pub protocol_flip: Option<String>,
}

/// `assist=` in a `--ui-probe` spec (review 4.2).
///
/// **Invented, and wider than the oracle's.** The frozen C has no probe at all;
/// this repository shipped `assist=confirm` and left the three states a user
/// actually has to read — a staged result awaiting Apply, a run in progress, and
/// a failure — unphotographable, which by this project's own rule means
/// unreviewed. The overpainted blocking reason found in review 1.13 lived in
/// exactly that gap.
///
/// Every state but [`Self::Confirm`] is synthesized in-process by
/// `ui::panels::assist::apply_probe_state`: no helper is spawned, no file is
/// read, and the content is fixed, so two runs of the same probe produce the
/// same pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistProbe {
    /// Arm the confirmation step, as `assist=confirm` always has.
    Confirm,
    /// Stage a fixed validated result whose target track is gone, so the review
    /// body draws *and* Apply is blocked with its reason beside it.
    Candidate,
    /// A running job with a fixed elapsed clock.
    Running,
    /// A terminal failure with a fixed detail and log name.
    Failed,
}

impl AssistProbe {
    /// The spec words, in the order the help text lists them.
    fn from_word(word: &str) -> Option<Self> {
        match word {
            "confirm" => Some(AssistProbe::Confirm),
            "candidate" => Some(AssistProbe::Candidate),
            "running" => Some(AssistProbe::Running),
            "failed" => Some(AssistProbe::Failed),
            _ => None,
        }
    }
}

/// `picker=` in a `--ui-probe` spec.
///
/// **Invented, and for the reason the whole probe grammar exists.** The free
/// colour picker is a disclosure behind the last cell of a swatch row — a click
/// is the only way a user reaches it, and a headless run has no click. Without
/// this key the picker would join the welcome screen and the three `None`
/// fallbacks on the list of surfaces this repository shipped unreviewed, which is
/// exactly what `../musializer/tools/UI_REVIEW.md` says costs the most.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionPickerProbe {
    /// `picker=glow`: the glow colour, which opens the effects form.
    Glow,
    /// `picker=ink`: the caption text colour.
    Ink,
    /// `picker=plate`: the backing colour.
    Plate,
}

/// `tune=` in a `--ui-probe` spec (UX0-C14), for the same reason as
/// [`CaptionPickerProbe`]: the tuning editor is a disclosure behind a button,
/// and a headless run has no click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionTuneProbe {
    /// `tune=pulse`: the glow pulse drive's mapping.
    Pulse,
    /// `tune=hue`: the hue drive's mapping.
    Hue,
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
    /// `--encoder NAME` (2026-08-08). Which encoder compresses the frames.
    ///
    /// Not in the frozen C's grammar, which is why it is additive: absent means
    /// `x264`, exactly what this application has always produced. See
    /// `AGENTS.md` for when an agent should reach for the GPU one.
    pub encoder: Option<String>,
    pub save_project: Option<PathBuf>,
    pub analysis_bridge: Option<PathBuf>,
    /// `--protocol PATH`: load a `*.protocol.json` feedback protocol and start
    /// its listening session (HX-2). Additive; nothing else on the grammar
    /// moves.
    pub protocol: Option<PathBuf>,
    pub auto_scenes: bool,
    pub reload_once: bool,
    pub ui_probe: Option<UiProbe>,
    /// Rust-side shell scaling. CLI wins over the per-user preference file.
    pub ui_scale: Option<UiScalePreference>,

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
    /// `--hud`: start with the diagnostic readout drawn over the preview.
    ///
    /// `None` means "decide from context", which is a probe run turning it on and
    /// an interactive run leaving it off. Three-valued rather than a `bool` so that
    /// `--hud=0` can turn it off *during* a probe run, which is how a capture of
    /// the clean preview is taken.
    pub hud: Option<bool>,
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
        // The scene's display name is two words, so the hyphenated spelling is
        // the one a user will reach for first.
        "phosphor-dream" => Some(SceneId::PhosphorDream),
        other => SceneId::from_stable_name(other),
    }
}

/// Every spelling `--scene` accepts, for the help text.
pub const SCENE_NAME_HELP: &str = "spectrum, pulse, orbital, ascii, atlas, terrarium, \
constellation, cadence, loom, pentagram, phosphor (also pulse-field, orbital-lattice, \
ascii-field, song-atlas, spectral-terrarium, pentagram-orbits, phosphor-dream)";

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

            "--encoder" => match value_of(&argv, i) {
                Some(name) => cli.encoder = Some(name.to_string()),
                None => cli.warn("Missing encoder name"),
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

            // HX-2, additive: a `*.protocol.json` listening session. The
            // protocol names its own audio, so this flag alone starts one.
            "--protocol" => match value_of(&argv, i) {
                Some(path) => cli.protocol = Some(PathBuf::from(path)),
                None => cli.warn("Missing command-line protocol path"),
            },

            "--auto-scenes" => cli.auto_scenes = true,
            "--reload-once" => cli.reload_once = true,

            // The message names the pair it refused. A capture script's typo that
            // warns "invalid spec" costs a round-trip to diagnose, which is
            // exactly what `hover=1121,449` cost once.
            "--ui-probe" => match value_of(&argv, i) {
                None => cli.warn("Missing --ui-probe spec"),
                Some(spec) => match parse_ui_probe_spec(spec) {
                    Ok(probe) => cli.ui_probe = Some(probe),
                    Err(message) => cli.warn(message),
                },
            },

            "--ui-scale" => match value_of(&argv, i).and_then(UiScalePreference::parse) {
                Some(scale) => cli.ui_scale = Some(scale),
                None => cli.warn("--ui-scale wants auto, 100, 125, 150, 175, or 200"),
            },

            // The slice's diagnostics. Not part of the oracle's surface.
            "--probe-frames" => match value_of(&argv, i).and_then(parse_positive_u32) {
                Some(frames) => cli.probe_frames = Some(frames),
                None => cli.warn("--probe-frames needs a positive frame count"),
            },
            // Not the oracle's: its readout has no toggle because it has no
            // readout. Spelled with an optional value so a probe run can turn the
            // HUD back off, which is the only way to photograph a clean preview.
            "--hud" => cli.hud = Some(true),
            "--hud=1" | "--hud=on" => cli.hud = Some(true),
            "--hud=0" | "--hud=off" => cli.hud = Some(false),
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
            | "--encoder"
            | "--project"
            | "--save-project"
            | "--analysis-bridge"
            | "--protocol"
            | "--ui-probe"
            | "--ui-scale"
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
/// **This used to be a second implementation of the grammar, and is now a
/// delegation.** Its own doc comment asked for that: "if a
/// `ParameterMapping::parse_spec` lands there, this should delegate to it rather
/// than keep a second copy — a duplicated grammar drifts." Item G moved the
/// persistence half of `scene_routes.c` into [`routes`], so the copy is gone.
///
/// The scene the key names is dropped here because `--route` is applied through
/// `App::routes_mut`, which re-derives it from the parameter — the same key, the
/// same table (`plug.c:1077-1080`).
#[must_use]
pub fn parse_route_spec(spec: &str) -> Option<ParameterMapping> {
    routes::parse_route_spec(spec).map(|(_scene, route)| route)
}

/// Accepted or refused, for the tests that only care which.
///
/// A thin `.ok()` rather than a second parser: a duplicated spec grammar drifts
/// from the one it was copied from.
#[cfg(test)]
#[must_use]
fn parse_ui_probe(spec: &str) -> Option<UiProbe> {
    parse_ui_probe_spec(spec).ok()
}

/// `parse_ui_probe` (`musializer.c:131-250`). One argv word,
/// `key=value[,key=value...]`.
///
/// **Every key may appear at most once and an unknown key is an error**, not
/// last-wins and not a silent default. The rationale at `:128-130` is worth
/// keeping: a typo in a capture script must not quietly photograph the wrong UI
/// state.
///
/// The refusal names the pair it refused, which the C's does not need to: this
/// repository has a probe grammar and the C does not, and `hover=1121,449` cost
/// a capture round-trip to diagnose because the warning said only "invalid
/// spec".
pub fn parse_ui_probe_spec(spec: &str) -> Result<UiProbe, String> {
    if spec.is_empty() {
        return Err("--ui-probe needs at least one key=value pair".to_string());
    }
    if spec.len() >= UI_PROBE_SPEC_CAPACITY {
        return Err(format!(
            "--ui-probe spec is longer than the {UI_PROBE_SPEC_CAPACITY}-byte limit"
        ));
    }
    let mut probe = UiProbe::default();
    let mut seen: Vec<&str> = Vec::new();

    for pair in spec.split(',') {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(format!("--ui-probe pair `{pair}` is not key=value"));
        };
        if key.is_empty() || value.is_empty() {
            return Err(format!(
                "--ui-probe pair `{pair}` has an empty key or value"
            ));
        }
        if seen.contains(&key) {
            // Not last-wins: a typo in a capture script must not quietly
            // photograph the wrong UI state (`musializer.c:128-130`).
            return Err(format!("--ui-probe key `{key}` appears more than once"));
        }
        seen.push(key);

        if apply_probe_key(&mut probe, key, value).is_none() {
            return Err(format!(
                "--ui-probe `{key}={value}` is not something this build understands"
            ));
        }
    }
    Ok(probe)
}

/// One `key=value` pair, applied. `None` is "this build does not understand it",
/// which the caller turns into a message naming the pair.
fn apply_probe_key(probe: &mut UiProbe, key: &str, value: &str) -> Option<()> {
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
        "lyric-tap" => {
            // At least one: `lyric-tap=0` would arm a run, stamp nothing and
            // leave a frame indistinguishable from one that never ran, which is
            // the `wheel=0` mistake this grammar already refuses.
            let taps: u32 = value.parse().ok()?;
            if !(1..=64).contains(&taps) {
                return None;
            }
            probe.lyric_taps = Some(taps);
        }
        "lyric-undo" => probe.lyric_undo = parse_probe_flag(value)?,
        "zoom" => {
            // 1.0..=100000.0 inclusive, finite (`musializer.c:188-197`).
            let zoom: f64 = value.parse().ok()?;
            if !zoom.is_finite() || !(1.0..=100_000.0).contains(&zoom) {
                return None;
            }
            probe.timeline_zoom = Some(zoom);
        }
        "style" => {
            // `caption` opens the style form; `effects` its sibling effects
            // form. Both are bodies of the same pane.
            match value {
                "caption" => probe.caption_style_pane = true,
                "effects" => {
                    probe.caption_style_pane = true;
                    probe.caption_effects_pane = true;
                }
                _ => return None,
            }
        }
        "fonts" => {
            probe.font_browser = Some(if value == "consent" {
                FontBrowserProbe::Consent
            } else {
                FontBrowserProbe::Catalogue(PathBuf::from(value))
            });
        }
        "picker" => {
            // Named rather than a flag, because the pane has two colours and a
            // capture that photographed the wrong one would still exit 0.
            probe.caption_picker = Some(match value {
                "ink" => CaptionPickerProbe::Ink,
                "plate" => CaptionPickerProbe::Plate,
                "glow" => CaptionPickerProbe::Glow,
                _ => return None,
            });
        }
        "tune" => {
            probe.caption_tune = Some(match value {
                "pulse" => CaptionTuneProbe::Pulse,
                "hue" => CaptionTuneProbe::Hue,
                _ => return None,
            });
        }
        "assist" => {
            probe.assist = Some(AssistProbe::from_word(value)?);
        }
        "route" => {
            // Resolved against the descriptor tables here rather than in the
            // panel, so a mistyped key fails the command line instead of
            // quietly photographing an unexpanded row.
            let key = if value.starts_with("settings.") {
                value.to_string()
            } else {
                format!("settings.{value}")
            };
            settings::descriptor_by_key(&key)?;
            probe.route_editor = Some(key);
        }
        "lyrics-file" => probe.lyrics_reference_path = Some(PathBuf::from(value)),
        // Deliberately not checked for existence here: "a dropped file that is
        // not there" is one of the branches this probe has to be able to reach,
        // and refusing it on the command line would make the failure path
        // unphotographable in exactly the way the probe exists to fix.
        "drop" => probe.drop_file = Some(PathBuf::from(value)),
        "time" => probe.seek_seconds = Some(parse_seconds(value)?),
        "share-frame" => {
            if value != "playhead" {
                return None;
            }
            probe.share_frame_playhead = true;
        }
        "size" => probe.size = Some(parse_resolution(value)?),
        "sidebar" => probe.sidebar_width = Some(parse_split_position(value)?),
        "inspector" => probe.inspector_width = Some(parse_split_position(value)?),
        "timeline-height" => probe.timeline_height = Some(parse_split_position(value)?),
        "hover" => probe.hover = Some(parse_point(value)?),
        "click" => probe.click = Some(parse_point(value)?),
        "wheel" => {
            let notches: f32 = value.parse().ok()?;
            // Bounded because the factor is `1.2^notches`: past a handful the
            // view is pinned at its own clamp and the capture proves nothing
            // about which lane accepted the event.
            if !notches.is_finite() || notches == 0.0 || notches.abs() > 8.0 {
                return None;
            }
            probe.wheel = Some(notches);
        }
        // Shift is a *modifier* on the notch `wheel=` delivers, so it is a flag
        // rather than a second count: two keys that both carried notches could
        // ask for a zoom and a pan on the same frame, which no hand can do.
        "wheel-shift" => probe.wheel_shift = parse_probe_flag(value)?,
        "middle-drag" => probe.middle_drag = Some(parse_point(value)?),
        // Through `--scene`'s own resolver, aliases included, so there is one
        // spelling of a scene on this command line rather than two.
        "scene-pick" => probe.scene_pick = Some(scene_from_cli_name(value)?),
        "tune-seed" => probe.tune_seed = Some(value.parse().ok()?),
        "tune-explore" => {
            // Validated here rather than in the panel, so a typo fails the
            // command line instead of quietly photographing an unexplored scene.
            for action in value.split('+') {
                if !matches!(action, "nudge" | "surprise" | "compare" | "revert" | "keep") {
                    return None;
                }
            }
            probe.tune_explore = Some(value.to_string());
        }
        "tune-type" => {
            // `KEY:VALUE`, colon-separated because the spec is already split on
            // `,` and `=`. The key is resolved against the descriptor tables now
            // for the same reason `route=` is.
            let (key, typed) = value.split_once(':')?;
            let key = if key.starts_with("settings.") {
                key.to_string()
            } else {
                format!("settings.{key}")
            };
            settings::descriptor_by_key(&key)?;
            if typed.is_empty() {
                return None;
            }
            probe.tune_type = Some(format!("{key}:{typed}"));
        }
        // Not validated for extension here: the render path refuses anything
        // but `.mp4` and the still path writes a PNG, and a probe that aimed
        // the wrong one must fail *there*, where a real picker's answer would.
        "save-to" => probe.save_to = Some(PathBuf::from(value)),
        "protocol-answer" => {
            // `ID:CHOICE` pairs joined by `+`, colon-separated like `tune-type`
            // because the spec is already split on `,` and `=`. Items are named
            // by id, never by pixel (GX-1): the item's marker can move with any
            // layout change and the probe still lands. Ids cannot be resolved
            // here — the protocol file loads later — so only the shape is
            // checked, and the runner reports an unknown id by name.
            for pair in value.split('+') {
                let (id, choice) = pair.split_once(':')?;
                let digit: u8 = choice.parse().ok()?;
                if id.is_empty() || !(1..=4).contains(&digit) {
                    return None;
                }
            }
            probe.protocol_answer = Some(value.to_string());
        }
        // One flip before the answer, so a probe run can put both looks of an
        // A/B item on record and the gate can check the recorded order.
        "protocol-flip" => {
            if value.is_empty() {
                return None;
            }
            probe.protocol_flip = Some(value.to_string());
        }
        "audio-stall" => {
            let milliseconds: u64 = value.parse().ok()?;
            if !(1..=5_000).contains(&milliseconds) {
                return None;
            }
            probe.audio_stall_ms = Some(milliseconds);
        }
        _ => return None,
    }
    Some(())
}

fn parse_split_position(text: &str) -> Option<f32> {
    let value: f32 = text.parse().ok()?;
    (value.is_finite() && (80.0..=4096.0).contains(&value)).then_some(value)
}

/// `XxY` for `hover=`. Floats, because a control's centre rarely lands on an
/// integer and a capture script should be able to say where it aimed.
///
/// Separated by `x` rather than a comma, matching `size=WxH`: the spec itself is
/// comma-separated, so `hover=1121,449` parses as two broken pairs — which is what
/// it did on the first run, and the warning said only "invalid spec".
fn parse_point(text: &str) -> Option<(f32, f32)> {
    let (x, y) = text.split_once('x')?;
    let x: f32 = x.trim().parse().ok()?;
    let y: f32 = y.trim().parse().ok()?;
    (x.is_finite() && y.is_finite()).then_some((x, y))
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
/// `plug.c:4293`).
///
/// Settled at the parity gate: only the third is a field in a file, so only the
/// third is held byte-exact (`project::APPLICATION_VERSION`). These two are prose,
/// and this build names the parity target instead of claiming to be it — see the
/// `Outcome::Version` arm in `main.rs` for the reasoning.
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
  --protocol FILE         Run a *.protocol.json listening session: the file
                          names its audio (path + sha256), questions land on
                          keys 1-4, answers append beside it as
                          *.answers.jsonl
  --auto-scenes           Enable imported scene suggestions

Export:
  --render FILE           Render MP4 and exit
  --render-window S D     Render only D seconds starting at S.
                          Frames match the same span of a full render
  --resolution WIDTHxHEIGHT
  --fps N
  --quality NAME          balanced, high, or master
  --encoder NAME          x264 (default), nvenc, or nvenc-hevc. nvenc uses the
                          GPU: far faster, larger file at equal quality.

Diagnostics:
  --mute                  Start with the output volume at zero
  --ui-scale VALUE        Shell scale: auto, 100, 125, 150, 175, or 200
                          (Ctrl+- / Ctrl+0 / Ctrl++ also adjust it)
  --hud[=0|1]             Draw the diagnostic readout over the preview (also H)
  --reload-once           Exercise one hot-reload handoff (unsupported)
  --ui-probe SPEC         Open a workspace panel and park the transport
                          for reproducible headless UI capture. SPEC is
                          comma-separated key=value pairs:
                          panel=none|tune|export|lyrics|assist,
                          fullscreen=0|1, time=SECONDS, size=WIDTHxHEIGHT,
                          sidebar=PX, inspector=PX, timeline-height=PX
                          set logical split positions for capture,
                          assist=confirm|candidate|running|failed puts the
                          Assist panel in that state, lyric=N selects the
                          nth lyric cue (needs panel=assist),
                          zoom=FACTOR zooms the timeline strip about the
                          playhead (1 = whole track),
                          style=caption shows the caption typography
                          pane (needs panel=lyrics),
                          lyrics-file=PATH selects an authored lyric
                          sheet for the next Assist lyrics run,
                          route=KEY opens the Tune inspector's route
                          editor on that setting, e.g. route=loom.weight
                          (needs panel=tune, and the key's scene has to
                          be the one being drawn),
                          fonts=consent shows the face browser's network
                          consent panel; fonts=PATH loads a family list
                          from disk instead, so a capture never opens a
                          network connection (needs panel=lyrics),
                          drop=PATH synthesizes one file drop of PATH,
                          through the same typed dispatch a real drop
                          takes (.musi opens, PNG/JPEG/BMP imports as
                          ASCII, anything else is tried as audio),
                          play=0|1. The transport is parked unless
                          play=1; audio-reactive scenes need play=1 but
                          then capture a frame that is not reproducible.
                          Every panel except none needs a loaded track,
                          hover=XxY parks the pointer and zeroes the
                          tooltip dwell; click=XxY presses there over
                          three frames; wheel=NOTCHES turns the wheel
                          once wherever hover= parked it,
                          share-frame=playhead seeds the Export panel's
                          first-frame choice from time= before click=,
                          scene-pick=NAME presses that scene tile,
                          picker=ink|plate|glow and tune=pulse|hue open
                          the caption pane's disclosures,
                          lyric-tap=N arms a tap run at time= and presses
                          the tap key N times; lyric-undo=1 then presses
                          Ctrl+Z once (needs panel=lyrics),
                          protocol-answer=ID:CHOICE[+ID:CHOICE] answers
                          protocol items by id; protocol-flip=ID plays
                          the item's other look first (need --protocol),
                          audio-stall=MS stalls the audio callback
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
    /// A wheel probe is a signed notch count, and zero is refused.
    ///
    /// Zero is the refusal worth having: `wheel=0` reads as "send no wheel",
    /// which is what leaving the key out already means — accepting it would let
    /// a capture assert a zoom that the run never asked for and never got.
    #[test]
    fn a_wheel_probe_takes_a_signed_notch_count_and_refuses_a_no_op() {
        assert_eq!(
            parse_ui_probe("hover=100x200,wheel=1")
                .expect("valid spec")
                .wheel,
            Some(1.0)
        );
        assert_eq!(
            parse_ui_probe("wheel=-2.5").expect("valid spec").wheel,
            Some(-2.5)
        );
        assert_eq!(parse_ui_probe("wheel=0"), None);
        assert_eq!(parse_ui_probe("wheel=9"), None);
        assert_eq!(parse_ui_probe("wheel=nan"), None);
        assert_eq!(parse_ui_probe("wheel=up"), None);
    }

    /// Protocol probes address items by id, never by pixel (HX-5, GX-1), and
    /// the pair shape is checked at the command line so a typo fails there
    /// rather than photographing an unanswered session.
    #[test]
    fn protocol_probes_take_id_choice_pairs_and_refuse_malformed_ones() {
        let probe = parse_ui_probe("protocol-answer=atlas-p1:2+free-1:1,protocol-flip=atlas-p1")
            .expect("valid spec");
        assert_eq!(
            probe.protocol_answer.as_deref(),
            Some("atlas-p1:2+free-1:1")
        );
        assert_eq!(probe.protocol_flip.as_deref(), Some("atlas-p1"));

        assert_eq!(parse_ui_probe("protocol-answer=atlas-p1"), None);
        assert_eq!(parse_ui_probe("protocol-answer=:2"), None);
        assert_eq!(parse_ui_probe("protocol-answer=atlas-p1:0"), None);
        assert_eq!(parse_ui_probe("protocol-answer=atlas-p1:5"), None);
        assert_eq!(parse_ui_probe("protocol-answer=a:1+b"), None);
        assert_eq!(parse_ui_probe("protocol-flip="), None);
    }

    /// `--protocol` is additive: it takes one path, and its value can never be
    /// re-scanned as a positional track.
    #[test]
    fn the_protocol_flag_takes_one_path() {
        let Outcome::Parsed(cli) = parse(["--protocol", "cx4.protocol.json"]) else {
            panic!("--protocol should run");
        };
        assert_eq!(
            cli.protocol.as_deref(),
            Some(std::path::Path::new("cx4.protocol.json"))
        );
        assert!(cli.actions.is_empty(), "the path leaked into the actions");
        assert!(!cli.error);
        assert!(takes_one_value("--protocol"));

        let Outcome::Parsed(cli) = parse(["--protocol"]) else {
            panic!("a missing value still runs, with a warning");
        };
        assert!(cli.error);
    }

    /// The tap probe refuses a no-op for the same reason `wheel=0` does
    /// (UX0-C03).
    ///
    /// `lyric-tap=0` would arm a run, stamp nothing, and leave a frame that is
    /// indistinguishable from one where the whole feature failed — which is
    /// exactly the picture a probe exists to rule out.
    #[test]
    fn a_lyric_tap_probe_takes_a_count_and_refuses_a_no_op() {
        assert_eq!(
            parse_ui_probe("panel=lyrics,lyric-tap=4")
                .expect("valid spec")
                .lyric_taps,
            Some(4)
        );
        assert_eq!(parse_ui_probe("lyric-tap=0"), None);
        assert_eq!(parse_ui_probe("lyric-tap=65"), None);
        assert_eq!(parse_ui_probe("lyric-tap=-1"), None);
        assert_eq!(parse_ui_probe("lyric-tap=all"), None);
    }

    #[test]
    fn a_lyric_undo_probe_is_a_flag_like_every_other_flag_in_this_grammar() {
        assert!(
            parse_ui_probe("panel=lyrics,lyric-tap=2,lyric-undo=1")
                .expect("valid spec")
                .lyric_undo
        );
        assert!(
            !parse_ui_probe("lyric-undo=0")
                .expect("valid spec")
                .lyric_undo
        );
        assert_eq!(parse_ui_probe("lyric-undo=yes"), None);
        assert_eq!(parse_ui_probe("lyric-undo"), None);
    }

    #[test]
    fn a_scene_pick_probe_resolves_through_the_same_names_as_the_scene_flag() {
        assert_eq!(
            parse_ui_probe("scene-pick=pentagram")
                .expect("valid spec")
                .scene_pick,
            Some(SceneId::Pentagram)
        );
        // `--scene`'s aliases resolve here too, because it is `--scene`'s own
        // resolver: two spellings of one scene on one command line would be a
        // grammar to keep in step forever.
        assert_eq!(
            parse_ui_probe("scene-pick=pulse")
                .expect("valid spec")
                .scene_pick,
            scene_from_cli_name("pulse")
        );
        assert_eq!(parse_ui_probe("scene-pick=nosuchscene"), None);
        assert_eq!(parse_ui_probe("scene-pick="), None);
    }

    /// `hover=` is separated by `x`, not by a comma.
    ///
    /// The spec itself is comma-separated, so `hover=1121,449` splits into two
    /// broken pairs and the whole probe is rejected with a message that names no
    /// key. That cost a capture round-trip to diagnose, which is exactly the sort
    /// of thing a two-line test stops the next person paying for.
    #[test]
    fn a_hover_point_uses_the_same_separator_as_a_size() {
        let probe = parse_ui_probe("play=1,hover=1121x449").expect("valid spec");
        assert_eq!(probe.hover, Some((1121.0, 449.0)));
        assert!(probe.playing);
        assert_eq!(parse_ui_probe("hover=1121,449"), None);
        assert_eq!(parse_ui_probe("hover=nonsense"), None);
        assert_eq!(parse_ui_probe("hover=1x"), None);
    }

    /// `click=` reads through the same point parser as `hover=` (EX1).
    ///
    /// One parser rather than two, so the separator lesson above is learned once:
    /// a `click=116,542` would otherwise fail exactly the way `hover=1121,449`
    /// did, in a probe whose whole purpose is to tell "the control refused the
    /// press" from "the press never happened".
    #[test]
    fn a_click_point_reads_through_the_same_parser_as_a_hover() {
        let probe = parse_ui_probe("panel=export,click=116x542").expect("valid spec");
        assert_eq!(probe.click, Some((116.0, 542.0)));
        assert_eq!(probe.hover, None, "click= does not imply a hover= as well");
        assert_eq!(parse_ui_probe("click=116,542"), None);
        assert_eq!(parse_ui_probe("click=nonsense"), None);
        assert_eq!(parse_ui_probe("click=1x"), None);
    }

    use super::*;
    use musializer_core::scene::routes::{AnalysisSource, Interpolation};

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
            ("phosphor", SceneId::PhosphorDream),
            ("phosphor-dream", SceneId::PhosphorDream),
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
    fn inputs_keep_argv_order_for_runtime_append_and_project_selection() {
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
             time=12.5,zoom=4,size=960x640,sidebar=400,inspector=440,\
             timeline-height=330,lyrics-file=/tmp/a.txt,drop=/tmp/b.musi,\
             share-frame=playhead",
        )
        .unwrap();
        assert_eq!(probe.panel, UiPanel::Assist);
        assert_eq!(probe.assist, Some(AssistProbe::Confirm));
        assert_eq!(probe.lyric_selection, Some(3));
        assert!(probe.playing);
        assert!(!probe.fullscreen);
        assert_eq!(probe.seek_seconds, Some(12.5));
        assert_eq!(probe.timeline_zoom, Some(4.0));
        assert_eq!(probe.size, Some((960, 640)));
        assert_eq!(probe.sidebar_width, Some(400.0));
        assert_eq!(probe.inspector_width, Some(440.0));
        assert_eq!(probe.timeline_height, Some(330.0));
        assert_eq!(
            probe.lyrics_reference_path,
            Some(PathBuf::from("/tmp/a.txt"))
        );
        assert_eq!(probe.drop_file, Some(PathBuf::from("/tmp/b.musi")));
        assert!(probe.share_frame_playhead);

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

        // The free colour picker, which a click is otherwise the only way to
        // open. Two named colours rather than a flag: a capture that
        // photographed the wrong one would still exit 0.
        let probe = parse_ui_probe("panel=lyrics,picker=ink").unwrap();
        assert_eq!(probe.caption_picker, Some(CaptionPickerProbe::Ink));
        let probe = parse_ui_probe("panel=lyrics,style=caption,picker=plate").unwrap();
        assert_eq!(probe.caption_picker, Some(CaptionPickerProbe::Plate));
        assert!(parse_ui_probe("panel=lyrics,picker=1").is_none());
        assert!(parse_ui_probe("panel=lyrics,picker=text").is_none());
    }

    #[test]
    fn ui_probe_names_the_tune_editor_by_drive() {
        // UX0-C14: the tuning editor is a disclosure behind a button, so it
        // needs a probe for the same reason the picker does.
        let probe = parse_ui_probe("panel=lyrics,tune=pulse").unwrap();
        assert_eq!(probe.caption_tune, Some(CaptionTuneProbe::Pulse));
        let probe = parse_ui_probe("panel=lyrics,style=effects,tune=hue").unwrap();
        assert_eq!(probe.caption_tune, Some(CaptionTuneProbe::Hue));
        assert!(parse_ui_probe("panel=lyrics,tune=glow").is_none());
        assert!(parse_ui_probe("panel=lyrics,tune=").is_none());
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
    fn the_assist_probe_names_all_four_states_and_confirm_still_means_confirm() {
        // Review 4.2: Candidate, Running and Failed are the panel's consequential
        // bodies and had never been in a frame, because the grammar was one word
        // wide. `confirm` keeps its exact old meaning, including the flag
        // `main.rs` reads today.
        for (word, state) in [
            ("confirm", AssistProbe::Confirm),
            ("candidate", AssistProbe::Candidate),
            ("running", AssistProbe::Running),
            ("failed", AssistProbe::Failed),
        ] {
            let probe = parse_ui_probe(&format!("panel=assist,assist={word}"))
                .unwrap_or_else(|| panic!("assist={word} should parse"));
            assert_eq!(probe.assist, Some(state));
        }
        // Near-misses are refused rather than rounded to a neighbour: a capture
        // that photographs Ready while claiming Candidate is worse than a failure.
        for word in ["cancel", "Candidate", "candidates", "run", "fail", "1"] {
            assert!(
                parse_ui_probe(&format!("assist={word}")).is_none(),
                "assist={word} must not parse"
            );
        }
    }

    #[test]
    fn a_refused_probe_pair_is_reported_with_its_key() {
        // The warning that said only "invalid spec" cost a capture round-trip
        // when `hover=1121,449` silently failed. Every refusal now names what it
        // refused.
        let message = parse_ui_probe_spec("panel=assist,assist=cancel").unwrap_err();
        assert!(
            message.contains("assist=cancel"),
            "the message must name the pair, got: {message}"
        );
        assert!(parse_ui_probe_spec("panel=tune,panel=export")
            .unwrap_err()
            .contains("panel"));
        // The comma-separated spec eats this one at the first pair, which is
        // still the pair a reader has to fix: `hover=` wants `XxY`.
        assert!(parse_ui_probe_spec("hover=1121,449")
            .unwrap_err()
            .contains("hover=1121"));
        assert!(parse_ui_probe_spec("pannel=tune")
            .unwrap_err()
            .contains("pannel=tune"));

        // And the message reaches the command line rather than being swallowed.
        let cli = parsed(&["--ui-probe", "panel=assist,assist=cancel"]);
        assert!(cli.error);
        assert!(
            cli.warnings
                .iter()
                .any(|line| line.contains("assist=cancel")),
            "the CLI warning must name the pair, got: {:?}",
            cli.warnings
        );
    }

    #[test]
    fn the_route_probe_resolves_its_key_and_rejects_a_typo() {
        // Resolved at parse time on purpose: a capture script that names a
        // setting that does not exist has to fail the command line, not
        // photograph an unexpanded row and look like a working editor.
        let probe = parse_ui_probe("panel=tune,route=loom.weight").unwrap();
        assert_eq!(
            probe.route_editor.as_deref(),
            Some("settings.loom.weight"),
            "the settings. prefix is optional, as it is for --route"
        );
        let prefixed = parse_ui_probe("panel=tune,route=settings.loom.weight").unwrap();
        assert_eq!(prefixed.route_editor, probe.route_editor);
        assert!(parse_ui_probe("panel=tune,route=loom.wieght").is_none());
        assert!(parse_ui_probe("panel=tune,route=nope.nothing").is_none());
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
        // style= accepts exactly one word; assist= accepts exactly four.
        assert!(parse_ui_probe("style=body").is_none());
        assert!(parse_ui_probe("assist=cancel").is_none());
        assert!(parse_ui_probe("assist=candidate").is_some());
        // time= is parse_seconds: finite and non-negative.
        assert!(parse_ui_probe("time=-1").is_none());
        assert!(parse_ui_probe("share-frame=playhead").is_some());
        assert!(parse_ui_probe("share-frame=normal").is_none());
        assert!(parse_ui_probe("sidebar=79").is_none());
        assert!(parse_ui_probe("sidebar=80").is_some());
        assert!(parse_ui_probe("timeline-height=4096").is_some());
        assert!(parse_ui_probe("inspector=4097").is_none());
        assert!(parse_ui_probe("sidebar=nan").is_none());
        assert_eq!(
            parse_ui_probe("audio-stall=750").unwrap().audio_stall_ms,
            Some(750)
        );
        assert!(parse_ui_probe("audio-stall=0").is_none());
        assert!(parse_ui_probe("audio-stall=5001").is_none());
    }

    #[test]
    fn ui_scale_accepts_only_the_supported_rungs() {
        for value in ["auto", "100", "125", "150", "175", "200"] {
            let cli = parsed(&["--ui-scale", value]);
            assert!(cli.ui_scale.is_some(), "{value}");
            assert!(!cli.error, "{value}");
        }
        for value in ["0", "110", "225", "wide"] {
            assert!(parsed(&["--ui-scale", value]).error, "{value}");
        }
    }

    #[test]
    fn a_too_long_ui_probe_spec_is_rejected() {
        let spec = format!("lyrics-file={}", "a".repeat(UI_PROBE_SPEC_CAPACITY));
        assert!(parse_ui_probe(&spec).is_none());
    }

    #[test]
    fn an_unknown_flag_is_an_audio_input_action_like_the_c() {
        // There is no unknown-option arm: the runtime attempts to load this path
        // and reports the ordinary track failure (`musializer.c:546-550`).
        let cli = parsed(&["--typo"]);
        assert!(!cli.error);
        assert_eq!(
            cli.actions,
            vec![Action::LoadTrack(PathBuf::from("--typo"))]
        );
        assert!(cli.warnings.is_empty());
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
