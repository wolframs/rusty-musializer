//! Track-specific poster frames, one per plan-lane run (WX-5b).
//!
//! **This file is the seam between the plan lane and the poster service**
//! (`build/redesign/wx5b-design.md` in the main checkout, §3). The read side
//! below — [`PosterRequest`], [`PosterView`], [`PosterBoard::view`] and
//! [`PosterBoard::describe`] — is what the lane is written against; the service
//! fills in everything behind it (the recipe and key in
//! `musializer_core::render::poster`, the replay, the textures, the cache) and
//! owns this file from the seam commit on.
//!
//! The rules the seam commits both sides to:
//!
//! - **Requests travel without interior mutability.** The lane pushes one
//!   [`PosterRequest`] per drawn run into `Shell`'s own list; the composition
//!   root drains it after the frame ([`PosterBoard::enqueue`]). `view` takes
//!   `&self`, and the board is a `main.rs` local rather than part of `Shell`,
//!   so the borrow `ShellInput` holds while the shell draws is a plain shared
//!   one.
//! - **[`PosterBoard::step`] is the last thing in the workspace drawing pair**,
//!   after the egui pass: nothing then holds the board, no texture mode or
//!   scissor is active, and the default framebuffer is bound. Every offscreen
//!   draw restores it the way `runtime::offscreen::Canvas` does — never a bare
//!   `begin_texture_mode`, because `EndTextureMode` does not restore rlgl's
//!   cached framebuffer size (AGENTS.md). A poster drawn in frame N shows from
//!   frame N+1.
//! - **The service never touches the preview's state**: not `app.scene`, the
//!   analyzer, the preview's `SceneRenderer`, the music stream or the plan's
//!   switch state. A poster is close to the export frame, not bit-identical
//!   with it, and is never written as a still.
//! - **Invalidation is a key changing.** The view is recomputed from the track
//!   and the run every frame; an edit that changes a poster's inputs changes
//!   its key, and the old picture is offered only as [`PosterView::Stale`].

use musializer_core::ui::plan_runs::RunSpan;
use raylib::prelude::{RaylibDrawHandle, RaylibThread, Texture2D};

use crate::workspace::Track;

/// One run the lane drew this frame and wants a poster for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PosterRequest {
    pub track_slot: usize,
    pub run: RunSpan,
    /// Inside the lane's visible span this frame. Visible runs are served
    /// first, then by distance from the playhead.
    pub visible: bool,
}

/// What the lane draws for a run.
#[allow(
    dead_code,
    reason = "the seam: the WX-5b service constructs Ready and Stale, and the lane reads the reason into its tooltip"
)]
pub enum PosterView<'a> {
    /// The run's poster for its current inputs.
    Ready { texture: &'a Texture2D, aspect: f32 },
    /// The run's last poster, whose inputs have since changed. Drawn at 45 %
    /// over the run's fill until the new one is ready — never at full strength,
    /// because a stale preview confidently lies (CX-5).
    Stale { texture: &'a Texture2D, aspect: f32 },
    /// Never drawn yet: the run draws as it would without posters.
    Pending,
    /// Refused, with the reason for the run's tooltip: no audio, a surface the
    /// driver would not allocate, posters switched off.
    Unavailable(&'static str),
}

/// `--ui-probe posters=`: let a capture finish every requested poster before
/// it is taken, or switch the service off so the lane's fallback is what is
/// photographed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosterProbe {
    Settle,
    Off,
}

/// The poster service's read side, and (after the service lands) its owner.
///
/// The seam commit's board answers [`PosterView::Pending`] for everything and
/// does no work, so the lane can be built and gated against it before a single
/// poster exists.
#[derive(Default)]
pub struct PosterBoard {
    probe: Option<PosterProbe>,
    dump: Option<std::path::PathBuf>,
    /// Requests the last drained frame carried.
    requested: usize,
}

impl PosterBoard {
    /// Applies `--ui-probe posters=` and `poster-dump=`.
    pub fn configure(&mut self, probe: Option<PosterProbe>, dump: Option<std::path::PathBuf>) {
        self.probe = probe;
        self.dump = dump;
    }

    /// What the lane should draw for `run` of the track in `track_slot`.
    pub fn view(&self, _track: &Track, _track_slot: usize, _run: &RunSpan) -> PosterView<'_> {
        if self.probe == Some(PosterProbe::Off) {
            return PosterView::Unavailable("posters are switched off for this run");
        }
        PosterView::Pending
    }

    /// Hands the frame's requests to the service.
    pub fn enqueue(&mut self, requests: Vec<PosterRequest>) {
        self.requested = requests.len();
    }

    /// Does up to one frame's budget of poster work. See the module comment
    /// for where it is called and what it may not touch.
    pub fn step(&mut self, _d: &mut RaylibDrawHandle<'_>, _thread: &RaylibThread, _playing: bool) {}

    /// The `posters:` report line.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "requested={} ready=0 stale=0 pending={} unavailable=0 service=none probe={} dump={}",
            self.requested,
            self.requested,
            match self.probe {
                None => "none",
                Some(PosterProbe::Settle) => "settle",
                Some(PosterProbe::Off) => "off",
            },
            self.dump
                .as_deref()
                .map_or_else(|| "none".to_owned(), |path| path.display().to_string()),
        )
    }
}
