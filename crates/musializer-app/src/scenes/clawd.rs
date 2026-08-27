//! Clawd: the drawing half.
//!
//! **No oracle.** The character's state — envelopes, the expression machine,
//! cat choreography, the petal show, the lyric typist — lives in
//! [`musializer_core::scenes::clawd`]; only the raylib calls are here, which is
//! the same split every other scene uses. See that module for what this scene
//! is and who it is an homage to (thebes, <https://x.com/voooooogel>).
//!
//! ## How it is drawn, and why this way
//!
//! Everything is procedural geometry: petals are rotated ellipses filled with a
//! triangle fan and outlined segment-by-segment, the face is stroked polylines
//! with round joints, and the cats are ASCII kaomoji through the built-in
//! monospace face. No bitmap is imported —
//! partly because thebes' artwork is thebes', and partly for the reason the
//! caption work already paid for: a bitmap magnified under export supersampling
//! is a blur, while an ellipse is exact at 12 px and at 1200.
//!
//! - **One ink colour, derived from `daylight`.** The outline that reads on the
//!   tiki-cream backdrop is near-black and would vanish on the dark one, so the
//!   ink lerps between the two with the backdrop. Every stroke, glyph and
//!   outline uses it; there is no second "almost the ink" colour to drift.
//! - **The face never inherits the petal ring's rotation.** Core keeps the
//!   angle out of the face's geometry entirely; the drawing half honours that by
//!   building the features in axis-aligned head coordinates.
//! - **Cats are text, not paths.** The kaomoji *is* the joke — a cat that is
//!   visibly made of parentheses is the character thebes draws next to the
//!   flower. Every string is pure ASCII because the built-in face is Latin-1
//!   and stops; a test walks the strings so a fancier whisker fails loudly
//!   instead of drawing an empty box.
//! - **The sing-along mouth is Happy-only.** Every other expression's mouth is
//!   part of that expression's read — Boom already sings an `O`, and an open
//!   mouth inside a scrunch would blur two signals into one — so only the
//!   resting smile has the slack to animate. The smile flattens as the mouth
//!   opens: lips part instead of grinning through the note.
//! - **A blink swaps strokes rather than scaling them.** Squashing an eye's
//!   own geometry vertically works for a caret and collapses a scaled spiral
//!   polyline into a scribble; one curved lid stroke per eye at the same
//!   anchor is uniform across expressions and always reads as an eyelid.
//!   Scrunch and Boom are never blinked — core holds `blink()` at 0 there,
//!   and the drawing half draws no lids for them regardless.
//! - **Warmth is a tint, not a costume.** Petal saturation and the smile's
//!   depth lean gently with the Assist lane's valence; nothing else reads it
//!   yet. The report line prints `warmth`/`semantic` even when the lane is
//!   off, so a run with an Assist analysis attached is distinguishable from
//!   one without.
//! - **The petal show recolours only the petals.** At full level the twelve
//!   petals take the hue wheel — 30° apart, drifting on the sway clock, each
//!   brightened by its own band — while the face, ink, ground and cats stay
//!   exactly themselves, so Clawd in show mode is still unmistakably Clawd.
//!   The blend is an RGB lerp from the terracotta fill, which makes level 0
//!   *equal* to the resting palette rather than close to it. (Replaced the
//!   smoke ribbon, 2026-08-26: translucent grey discs with none of the
//!   reference image's framing read as anonymous fog, not a joke — a control
//!   slot at the 12-descriptor ceiling has to buy something legible.)
//!
//! ## The trail, and what deliberately does not have one (2026-08-27)
//!
//! The petals leave light behind them, through a
//! [`FeedbackBuffer`](musializer_runtime::feedback::FeedbackBuffer): last
//! frame's trail, faded on a half-life and crept outward from the head, plus
//! this frame's petal fills, composited **under** the flower. That "under" is
//! structural rather than a setting — the face, its ink and the cats are drawn
//! after the composite and never deposit into the buffer, so smeared light is
//! reachable and a smeared face is not statable. The one thing the flower does
//! that the old frame could not show is *move*: the ring spins, the petals
//! breathe with their bands, and a boom throws them outward at speed. A ghost
//! is what makes that motion visible in a still frame, which is what a poster
//! or a paused video is.
//!
//! The composite mode follows the backdrop for the reason the bass glow already
//! documents: additive light over the cream daylight ground saturates toward
//! white and vanishes. So a light ground takes the trail as premultiplied paint
//! *over* it (a colour smear, which is what a ghost on paper looks like) and a
//! dark ground takes it as added light.
//!
//! **The strengths are constants, not controls.** The descriptor table is full
//! at `MAX_CONTROLS` (twelve), and no slot here is worth less than what it
//! already buys. The tuning that matters is coupled to the music anyway: the
//! trail's half-life, its outward creep, its swirl and its strength all lerp up
//! with the petal show's level and with a boom's first moments, so it is a
//! whisper in ordinary playback and streamers when the track earns them. The
//! `.musi` surface does not change.

use musializer_core::scene::settings::index::clawd as setting;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::clawd::{CatFace, ClawdState, Expression, PETAL_COUNT};
use musializer_runtime::draw;
use musializer_runtime::feedback::{self, Carry, FeedbackBuffer};
use raylib::prelude::{
    BlendMode, Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, Rectangle, Vector2,
};
use raylib::text::RaylibFont;

use super::ascii_field::DefaultFont;

const TAU: f32 = std::f32::consts::TAU;

/// How long the trail keeps half its light, at rest and at a full petal show.
///
/// At rest it is short enough to read as motion blur on a spinning ring rather
/// than as a stain; at show level the streamers have to survive a whole turn of
/// the hue wheel or the effect reads as smudging rather than as light.
const TRAIL_HALF_LIFE_REST: f32 = 0.30;
const TRAIL_HALF_LIFE_FLARE: f32 = 0.85;

/// How fast the trail creeps outward from the head, as a scale per second.
///
/// Outward rather than inward: a flower's light belongs radiating away from it,
/// and an inward zoom pulls the ghosts into the face — the one place this scene
/// is not allowed to smear.
const TRAIL_ZOOM_REST: f32 = 1.09;
const TRAIL_ZOOM_FLARE: f32 = 1.16;

/// Degrees per second the trail turns about the head at a full show, for swirl.
/// Zero at rest: a resting flower that quietly rotates its own aura reads as a
/// rendering fault, not as an effect.
const TRAIL_SWIRL_DEGREES: f32 = 16.0;

/// How much light a petal lays down per **second** — turned into this frame's
/// alpha by [`feedback::deposit`], never used as a per-frame constant.
///
/// It rises with the flare like everything else here, and that is what the two
/// values are for rather than one: a resting deposit high enough to make a show
/// blaze piles up behind a slow-turning ring as a muddy drop shadow, and a
/// deposit low enough to keep the resting flower clean leaves the show looking
/// like a smudge instead of a light.
const TRAIL_DEPOSIT_REST: f32 = 5.0;
const TRAIL_DEPOSIT_FLARE: f32 = 9.0;

/// How strongly the accumulated trail is composited, at rest and at flare.
const TRAIL_STRENGTH_REST: f32 = 0.40;
const TRAIL_STRENGTH_FLARE: f32 = 0.92;

/// The trail buffer's longest edge in texels.
///
/// A trail has no high frequencies by construction — it is the blurred history
/// of big filled shapes — so it is accumulated at a fixed working size and
/// upscaled by the composite. That also makes it *the same buffer* at 1x and
/// under a 2x supersampled export, which is what keeps a still and its video
/// frame comparable.
const TRAIL_BUFFER_EDGE: i32 = 1024;

/// Points along half an ellipse outline. 18 segments keeps a fist-sized petal
/// visually round and costs nothing that matters.
const PETAL_SEGMENTS: usize = 18;

/// The petal base hue before the `hue` setting rotates it — thebes' terracotta.
const PETAL_HUE: f32 = 14.0;

/// The two cat rows, per face. ASCII only — see the module docs and the test.
fn cat_rows(face: CatFace) -> [&'static str; 2] {
    match face {
        CatFace::Content => ["/\\_/\\", "( ^w^ )"],
        CatFace::Excited => ["/\\_/\\", "( >w< )"],
        CatFace::Sleepy => ["/\\_/\\", "( -w- )"],
        CatFace::Curious => ["/\\_/\\", "( o.o )"],
    }
}

/// Everything the draw needs that is not the frame: the trail buffer, the
/// frame's resolved flower, and the report string.
///
/// This scene draws a plausible frame in several wrong states — a dead
/// expression machine is a permanent smile, petals uncoupled from their bands
/// are a symmetric flower, cats that never spawn are an empty floor, and every
/// one of them photographs as "a cute picture, looks fine". The report line is
/// where those states become distinguishable, so the renderer owns it.
pub struct ClawdResources {
    /// The accumulated light behind the flower.
    ///
    /// **No `load(rl, thread)`**, unlike `PhosphorResources`: the pair of render
    /// textures is sized from the scene boundary, which is one thing under the
    /// preview panel and another under a supersampled export target, so there is
    /// nothing to allocate before the first frame. Constructing it eagerly would
    /// mean allocating a guess and throwing it away.
    pub trail: FeedbackBuffer,
    /// This frame's flower, resolved by [`prepare`] and consumed by [`draw`].
    ///
    /// Carried rather than recomputed because the two must agree exactly: the
    /// trail is a ghost of the petals, and half a degree of disagreement between
    /// the deposit and the drawn petal reads as a badly registered effect. The
    /// boundary travels with it so a stale resolve can never be used against a
    /// different rectangle.
    flower: Option<(Rectangle, Flower)>,
    /// What the trail did on the last frame, for the report line.
    trail_status: String,
    /// The last drawn frame's report, or `"none"` before the scene ever drew.
    pub last: String,
}

impl ClawdResources {
    #[must_use]
    pub fn new() -> Self {
        Self {
            trail: FeedbackBuffer::new(),
            flower: None,
            trail_status: "off".to_string(),
            last: "none".to_string(),
        }
    }

    /// One line for the slice report.
    #[must_use]
    pub fn describe(&self) -> String {
        self.last.clone()
    }
}

impl Default for ClawdResources {
    fn default() -> Self {
        Self::new()
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::new(
        lerp(a.r as f32, b.r as f32, t) as u8,
        lerp(a.g as f32, b.g as f32, t) as u8,
        lerp(a.b as f32, b.b as f32, t) as u8,
        lerp(a.a as f32, b.a as f32, t) as u8,
    )
}

/// The single ink colour — see the module docs.
fn ink_color(daylight: f32) -> Color {
    lerp_color(
        Color::new(230, 226, 216, 255),
        Color::new(44, 30, 24, 255),
        daylight,
    )
}

/// A stroked polyline with round joints, the primitive every facial feature is
/// built from. Bare `draw_line_ex` butts leave notches at direction changes at
/// export scale; a disc at each vertex is the cheap round join.
fn stroke_polyline<D: RaylibDraw>(d: &mut D, points: &[Vector2], thickness: f32, color: Color) {
    for pair in points.windows(2) {
        d.draw_line_ex(pair[0], pair[1], thickness, color);
    }
    for point in points {
        d.draw_circle_v(*point, thickness * 0.5, color);
    }
}

/// The outline of one petal: a rotated ellipse.
///
/// `origin` is the flower centre, `angle` the petal's axis, `inner`/`length`
/// where it starts and how far it reaches, `width` the full minor axis.
fn petal_outline(origin: Vector2, angle: f32, inner: f32, length: f32, width: f32) -> Vec<Vector2> {
    let dir = Vector2::new(angle.cos(), angle.sin());
    let normal = Vector2::new(-dir.y, dir.x);
    let centre = Vector2::new(
        origin.x + dir.x * (inner + length * 0.5),
        origin.y + dir.y * (inner + length * 0.5),
    );
    let a = length * 0.5;
    let b = width * 0.5;
    (0..PETAL_SEGMENTS)
        .map(|k| {
            // Negated so the sweep runs counter-clockwise *in screen
            // coordinates* (y grows downward). raylib culls back faces, and the
            // first capture proved it: a clockwise fan draws nothing, and the
            // flower photographed as a colouring-book page of outlines with the
            // terracotta silently missing.
            let theta = -(k as f32) / PETAL_SEGMENTS as f32 * TAU;
            Vector2::new(
                centre.x + dir.x * a * theta.cos() + normal.x * b * theta.sin(),
                centre.y + dir.y * a * theta.cos() + normal.y * b * theta.sin(),
            )
        })
        .collect()
}

/// The boom petal scatter envelope: `(radial_offset_fraction, alpha)` over the
/// 1.6 s playout's 0..1 progress.
///
/// Shape: the offset blasts outward on an ease-out quadratic, reaching `PEAK`
/// head-radii by progress 0.3; it holds there while the alpha smooth-steps
/// down to `ALPHA_FLOOR`; the last 25 % eases both back (smoothstep) so the
/// petals regrow into place. Exactly `(0.0, 1.0)` at 0 and 1 — a non-boom
/// frame (`boom_progress()` is 0 there) must be byte-identical to a build
/// without this feature, so the endpoints are guards, not math that happens
/// to land on them.
fn scatter(progress: f32) -> (f32, f32) {
    // How far the petals fly, as a fraction of `head_radius`.
    const PEAK: f32 = 1.6;
    // How faint they get once blown out.
    const ALPHA_FLOOR: f32 = 0.35;
    // Progress where the outward blast completes.
    const RISE_END: f32 = 0.3;
    // Progress where the regrow begins — the last 25 %.
    const RECOVER_START: f32 = 0.75;

    if progress <= 0.0 || progress >= 1.0 {
        return (0.0, 1.0);
    }
    if progress < RISE_END {
        // Ease-out: fast off the mark, arriving gently at the peak.
        let t = progress / RISE_END;
        let ease = (1.0 - t).mul_add(-(1.0 - t), 1.0);
        (PEAK * ease, 1.0)
    } else if progress < RECOVER_START {
        // Hold at the peak while the petals fade toward the floor.
        let t = (progress - RISE_END) / (RECOVER_START - RISE_END);
        let ease = t * t * (3.0 - 2.0 * t);
        (PEAK, 1.0 - (1.0 - ALPHA_FLOOR) * ease)
    } else {
        // Regrow: ease-in-out back to no offset and full alpha. Written as
        // `1 - k * (1 - ease)` so `ease == 1` lands on 0.0 and 1.0 exactly.
        let t = (progress - RECOVER_START) / (1.0 - RECOVER_START);
        let ease = t * t * (3.0 - 2.0 * t);
        (
            PEAK * (1.0 - ease),
            1.0 - (1.0 - ALPHA_FLOOR) * (1.0 - ease),
        )
    }
}

/// Happy's resting `w` mouth in face coordinates, reshaped by semantic warmth
/// and the sing-along open fraction.
///
/// The baseline is the corners' y (0.24); only the bounce below it scales, so
/// the mouth flattens toward a line where it sits rather than migrating up
/// the face. Warmth scales the bounce by `0.55 + 0.45 * warmth` — a
/// melancholy track gets a gentler smile — and the sing-along open composes
/// multiplicatively (`1 - open`), flattening the lips as the mouth parts.
fn happy_mouth_points(warmth: f32, open: f32) -> [(f32, f32); 5] {
    #[rustfmt::skip]
    const REST: [(f32, f32); 5] = [
        (-0.30, 0.24), (-0.15, 0.40), (0.0, 0.26), (0.15, 0.40), (0.30, 0.24),
    ];
    let scale = (0.55 + 0.45 * warmth) * (1.0 - open);
    REST.map(|(x, y)| (x, 0.24 + (y - 0.24) * scale))
}

/// A closed eyelid in face coordinates: one gently-curved horizontal stroke.
/// The ends sit 0.02 face-radii *above* the middle (screen y grows downward),
/// so the lid bows softly down — a relaxed closed eye, not a flat dash.
fn closed_eye_points(cx: f32, cy: f32, half_width: f32) -> [(f32, f32); 3] {
    [
        (cx - half_width, cy - 0.02),
        (cx, cy),
        (cx + half_width, cy - 0.02),
    ]
}

fn fill_convex<D: RaylibDraw>(d: &mut D, outline: &[Vector2], color: Color) {
    if outline.len() < 3 {
        return;
    }
    let centre = outline.iter().fold(Vector2::zero(), |acc, p| {
        Vector2::new(acc.x + p.x, acc.y + p.y)
    });
    let centre = Vector2::new(
        centre.x / outline.len() as f32,
        centre.y / outline.len() as f32,
    );
    // raylib's fan wants the pivot first and the ring closed.
    let mut fan = Vec::with_capacity(outline.len() + 2);
    fan.push(centre);
    fan.extend_from_slice(outline);
    fan.push(outline[0]);
    d.draw_triangle_fan(&fan, color);
}

fn outline_loop<D: RaylibDraw>(d: &mut D, outline: &[Vector2], thickness: f32, color: Color) {
    if outline.len() < 2 {
        return;
    }
    for k in 0..outline.len() {
        let next = (k + 1) % outline.len();
        d.draw_line_ex(outline[k], outline[next], thickness, color);
    }
    for point in outline {
        d.draw_circle_v(*point, thickness * 0.5, color);
    }
}

/// One petal, resolved: where it is and what colour it is this frame.
struct Petal {
    outline: Vec<Vector2>,
    fill: Color,
    energy: f32,
}

/// The frame's flower, resolved once for both the trail deposit and the draw.
///
/// One resolve rather than two, because the trail is a *ghost of these exact
/// petals*: a second computation that drifted by a degree would read as a badly
/// registered effect rather than as a bug, which is the failure mode this
/// repository keeps paying for.
struct Flower {
    centre: Vector2,
    head_radius: f32,
    face_radius: f32,
    stroke: f32,
    ink: Color,
    petals: Vec<Petal>,
    petal_peak: f32,
    petal_peak_index: usize,
    /// Fades a blown-out petal during a boom; exactly 1.0 otherwise.
    scatter_alpha: f32,
    /// The petal show's level after the strength control, `0..=1`.
    show: f32,
    /// `boom_progress()`, carried so the trail and the face agree about it.
    boom: f32,
    /// How hard the trail should be pushed this frame, `0..=1`: the show's
    /// level, or a boom's opening moments, whichever is higher. One number so
    /// half-life, creep, swirl and composite strength cannot disagree about
    /// whether something is happening.
    flare: f32,
}

impl Flower {
    fn resolve(
        state: &ClawdState,
        frame: &SceneFrame<'_>,
        boundary: Rectangle,
        pixel_scale: f32,
    ) -> Self {
        let scene = SceneId::Clawd;
        let hue_shift = frame.setting(scene, setting::HUE);
        let daylight = frame.setting(scene, setting::DAYLIGHT);
        let ink_weight = frame.setting(scene, setting::INK);
        let bounce_depth = frame.setting(scene, setting::BOUNCE);
        let wiggle = frame.setting(scene, setting::WIGGLE);
        let show_setting = frame.setting(scene, setting::SHOW);

        let min_dim = boundary.width.min(boundary.height);
        let ink = ink_color(daylight);
        let petal_hue = (PETAL_HUE + hue_shift).rem_euclid(360.0);

        // The petal show: core's eased state-machine level, scaled by the
        // strength control. Above 1.0 the control cannot push the lerp past the
        // full wheel — it just gets there at a lower machine level.
        let show = (state.show_level() * show_setting).clamp(0.0, 1.0);
        // The colour wave's drift, on the sway clock like every other idle
        // motion, so it is deterministic in an export.
        let show_drift = state.sway_phase() * 40.0;
        // Petal saturation leans with the semantic lane's valence. Subtle on
        // purpose: warmth is a tint, and 0.48..0.66 brackets the old constant.
        let petal_saturation = lerp(0.48, 0.66, state.warmth());

        let sway = state.sway_phase();
        let bounce = state.bounce() * bounce_depth;
        let head_radius = min_dim * 0.150;
        let centre = Vector2::new(
            boundary.x + boundary.width * 0.5 + (sway * 0.7).sin() * min_dim * 0.012 * wiggle,
            boundary.y
                + boundary.height * 0.42
                + (sway * 0.53).sin() * min_dim * 0.008 * wiggle
                + bounce * head_radius * 0.16,
        );
        // Squash on the beat: wider and shorter, recovering as the impulse decays.
        let squash_x = 1.0 + bounce * 0.10;
        let squash_y = 1.0 - bounce * 0.12;
        let stroke = (ink_weight * head_radius * 0.055).max(pixel_scale);

        // During a boom the whole flower trembles, and the petals blow outward
        // and regrow through the scatter envelope. Both are `(0.0, 1.0)`-neutral
        // on a non-boom frame.
        let boom = state.boom_progress();
        let (scatter_offset, scatter_alpha) = scatter(boom);
        let tremble = if boom > 0.0 {
            (state.expression_age() * 43.0).sin() * 0.05 * (1.0 - boom)
        } else {
            0.0
        };

        let mut petal_peak = 0.0f32;
        let mut petal_peak_index = 0usize;
        let mut petals = Vec::with_capacity(PETAL_COUNT);
        for (index, &energy) in state.petals().iter().enumerate() {
            if energy > petal_peak {
                petal_peak = energy;
                petal_peak_index = index;
            }
            // Hand-drawn irregularity: thebes' petals are all slightly
            // different, and a mathematically perfect ring reads as clip-art.
            // Pure functions of the index, so the jitter is stable across frames
            // and identical in preview and export.
            let organic = |salt: u32| {
                let hash = (index as u32)
                    .wrapping_mul(2_654_435_761)
                    .wrapping_add(salt.wrapping_mul(0x9E37_79B9));
                let hash = (hash ^ (hash >> 15)).wrapping_mul(0x85EB_CA6B);
                (hash >> 8) as f32 / ((1u32 << 24) as f32) * 2.0 - 1.0
            };
            let angle = state.petal_angle()
                + tremble
                + index as f32 / PETAL_COUNT as f32 * TAU
                + organic(1) * 0.03
                - TAU * 0.25;
            // The boom scatter pushes the whole petal outward from the head; on
            // a non-boom frame the offset is exactly 0.0 and `x + 0.0 == x`.
            let inner = head_radius * 0.55 + head_radius * scatter_offset;
            let length = head_radius
                * (0.95 + 0.85 * energy)
                * (1.0 + organic(2) * 0.07)
                * squash_y.mul_add(0.5, 0.5);
            let width = head_radius * (0.36 + 0.16 * energy) * (1.0 + organic(3) * 0.08) * squash_x;
            let outline = petal_outline(centre, angle, inner, length, width);
            let mut fill = draw::color_from_hsv(petal_hue, petal_saturation, 0.70 + 0.22 * energy);
            if show > 0.0 {
                // The light show: this petal's own slice of the hue wheel,
                // phase-shifted 30° per petal and drifting, brilliance from its
                // own band. RGB-lerped from the terracotta fill so `show == 0.0`
                // returns the resting palette as an equality.
                let wheel_hue =
                    (petal_hue + index as f32 * (360.0 / PETAL_COUNT as f32) + show_drift)
                        .rem_euclid(360.0);
                let brilliance = 0.55 + 0.45 * energy;
                let lit = draw::color_from_hsv(wheel_hue, 0.85, brilliance);
                fill = lerp_color(fill, lit, show);
            }
            petals.push(Petal {
                outline,
                fill,
                energy,
            });
        }

        // A boom's flare is its opening, not its whole playout: the shockwave is
        // the moment the petals leave, and holding the trail wide open for the
        // full 1.6 s would still be smearing while they are calmly regrowing.
        let boom_flare = if boom > 0.0 {
            (1.0 - boom * 2.0).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Self {
            centre,
            head_radius,
            face_radius: head_radius * squash_x.mul_add(0.5, 0.5),
            stroke,
            ink,
            petals,
            petal_peak,
            petal_peak_index,
            scatter_alpha,
            show,
            boom,
            flare: show.max(boom_flare),
        }
    }

    /// How strongly the accumulated trail is painted this frame.
    fn trail_strength(&self) -> f32 {
        lerp(TRAIL_STRENGTH_REST, TRAIL_STRENGTH_FLARE, self.flare)
    }
}

/// Whether a carried resolve belongs to the rectangle being drawn.
///
/// Exact equality, deliberately: the boundary is computed the same way twice in
/// one frame, so anything but equality means a *different* rectangle and the
/// carried flower must be thrown away rather than stretched onto it.
fn same_rect(a: Rectangle, b: Rectangle) -> bool {
    a.x == b.x && a.y == b.y && a.width == b.width && a.height == b.height
}

/// A premultiplied colour: what the feedback buffer's blending expects.
fn premultiplied(color: Color, alpha: f32) -> Color {
    let alpha = alpha.clamp(0.0, 1.0);
    Color::new(
        (f32::from(color.r) * alpha) as u8,
        (f32::from(color.g) * alpha) as u8,
        (f32::from(color.b) * alpha) as u8,
        (alpha * 255.0) as u8,
    )
}

/// The trail buffer's texel dimensions for a boundary, aspect preserved.
fn trail_buffer_size(boundary: Rectangle) -> (i32, i32) {
    let aspect = boundary.width / boundary.height.max(1.0);
    if aspect >= 1.0 {
        (
            TRAIL_BUFFER_EDGE,
            (TRAIL_BUFFER_EDGE as f32 / aspect)
                .round()
                .clamp(16.0, 8192.0) as i32,
        )
    } else {
        (
            (TRAIL_BUFFER_EDGE as f32 * aspect)
                .round()
                .clamp(16.0, 8192.0) as i32,
            TRAIL_BUFFER_EDGE,
        )
    }
}

/// Resolves the flower and lays this frame's light into the trail buffer.
///
/// Called from `scene_host` **before** the scene's clip is opened, for the same
/// reason Phosphor Dream's `prepare` is: the accumulation redirects the
/// framebuffer, and a scissor rect is global GL state in framebuffer
/// coordinates that would crop the offscreen pass with a rectangle meant for
/// the preview panel.
pub fn prepare(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut ClawdResources,
    state: &ClawdState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
    pixel_scale: f32,
) {
    resources.flower = None;
    if boundary.width < 16.0 || boundary.height < 16.0 {
        resources.trail_status = "off".to_string();
        return;
    }
    let flower = Flower::resolve(state, frame, boundary, pixel_scale);
    let (buffer_width, buffer_height) = trail_buffer_size(boundary);
    let scale_x = buffer_width as f32 / boundary.width;
    let scale_y = buffer_height as f32 / boundary.height;
    let to_buffer = |point: Vector2| {
        Vector2::new(
            (point.x - boundary.x) * scale_x,
            (point.y - boundary.y) * scale_y,
        )
    };

    // Every one of these is per second, raised to this frame's delta. A
    // per-frame constant here is exactly the freewheeling-clock bug the cats
    // were fixed for: a 30 fps export would smear half as far as the 60 fps
    // preview it was authored against.
    let delta = frame.delta_seconds;
    let half_life = lerp(TRAIL_HALF_LIFE_REST, TRAIL_HALF_LIFE_FLARE, flower.flare);
    let zoom_rate = lerp(TRAIL_ZOOM_REST, TRAIL_ZOOM_FLARE, flower.flare);
    let carry = Carry {
        retain: feedback::retention(half_life, delta),
        zoom: zoom_rate.powf(delta.clamp(0.0, 0.5)),
        rotation_degrees: TRAIL_SWIRL_DEGREES * flower.flare * delta,
        centre: to_buffer(flower.centre),
    };

    let built = resources
        .trail
        .accumulate(d, buffer_width, buffer_height, carry, |target| {
            // Petals only. The face, its ink and the cats are drawn after the
            // composite and deposit nothing, which is what makes "smeared light,
            // crisp character" structural rather than a value somebody has to
            // keep tuned.
            let rate = lerp(TRAIL_DEPOSIT_REST, TRAIL_DEPOSIT_FLARE, flower.flare);
            let mut pass = target.begin_blend_mode(BlendMode::BLEND_ALPHA_PREMULTIPLY);
            for petal in &flower.petals {
                let alpha = feedback::deposit(
                    rate * (0.45 + 0.55 * petal.energy) * flower.scatter_alpha,
                    delta,
                );
                if alpha <= 0.0 {
                    continue;
                }
                let outline: Vec<Vector2> = petal.outline.iter().map(|p| to_buffer(*p)).collect();
                fill_convex(&mut pass, &outline, premultiplied(petal.fill, alpha));
            }
        });
    resources.trail_status = if built {
        format!("on@{:.2}", flower.trail_strength())
    } else if resources.trail.refused() {
        "unavailable".to_string()
    } else {
        "off".to_string()
    };
    resources.flower = Some((boundary, flower));
}

/// Draws one frame. Sets `resources.last` on every path, including refusals,
/// so the report line always states what actually happened.
#[allow(clippy::too_many_lines)]
pub fn draw(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut ClawdResources,
    state: &ClawdState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
    pixel_scale: f32,
) {
    if boundary.width < 16.0 || boundary.height < 16.0 {
        resources.last = "boundary=none".to_string();
        return;
    }
    let scene = SceneId::Clawd;
    let daylight = frame.setting(scene, setting::DAYLIGHT);
    let glow_setting = frame.setting(scene, setting::GLOW);
    let terminal_on = frame.setting(scene, setting::TERMINAL) >= 0.5;

    let min_dim = boundary.width.min(boundary.height);
    let petal_hue = (PETAL_HUE + frame.setting(scene, setting::HUE)).rem_euclid(360.0);

    // The flower [`prepare`] resolved and deposited into the trail, so the
    // ghost and the petal it belongs to cannot part ways. The fallback resolve
    // is not dead code: it is what keeps the scene drawable from a call site
    // that has not run `prepare` — a probe, or a future host — rather than
    // silently drawing nothing.
    let flower = match resources.flower.take() {
        Some((rect, flower)) if same_rect(rect, boundary) => flower,
        _ => Flower::resolve(state, frame, boundary, pixel_scale),
    };
    let ink = flower.ink;
    let stroke = flower.stroke;
    let centre = flower.centre;
    let head_radius = flower.head_radius;
    let show = flower.show;

    // -- the senses ------------------------------------------------------------
    let warmth = state.warmth();
    // Below 0.05 the sing-along oscillator is noise off the flux floor; treat
    // it as shut so the resting smile does not tremble between cues.
    let mouth_open = state.mouth_open();
    let open = if mouth_open > 0.05 { mouth_open } else { 0.0 };
    let eyelids_down = state.blink() > 0.5;

    // -- backdrop --------------------------------------------------------------
    let top = lerp_color(
        Color::new(40, 42, 48, 255),
        Color::new(243, 234, 202, 255),
        daylight,
    );
    let bottom = lerp_color(
        Color::new(14, 15, 18, 255),
        Color::new(252, 250, 244, 255),
        daylight,
    );
    d.draw_rectangle_gradient_v(
        boundary.x as i32,
        boundary.y as i32,
        boundary.width as i32,
        boundary.height as i32,
        top,
        bottom,
    );

    // -- the flower ------------------------------------------------------------
    let sway = state.sway_phase();

    // Bass glow behind everything of the flower's own. Two composites, one
    // choice: additive light saturates toward white, so over the near-white
    // daylight backdrop it vanishes and the `glow` control silently did
    // nothing a user could see — the defect class this repository documents.
    // On a light ground the glow is instead a normal-blend wash of a deepened
    // petal colour; the dark path is the original additive pass, unchanged.
    let glow_strength = (state.bass() * glow_setting).clamp(0.0, 2.0);
    if glow_strength > 0.01 {
        let alpha = (0.16 * glow_strength).min(0.4);
        let radius = head_radius * (2.3 + 0.9 * state.bass());
        if daylight >= 0.5 {
            // Same hue machinery as the petals, pushed darker and more
            // saturated so the wash reads *against* cream rather than into it.
            let glow = draw::color_from_hsv(petal_hue, 0.72, 0.52);
            d.draw_circle_gradient(
                centre.x as i32,
                centre.y as i32,
                radius,
                draw::color_alpha(glow, alpha * 0.7),
                draw::color_alpha(glow, 0.0),
            );
        } else {
            let glow = draw::color_from_hsv(petal_hue, 0.55, 0.95);
            let mut blend = d.begin_blend_mode(BlendMode::BLEND_ADDITIVE);
            blend.draw_circle_gradient(
                centre.x as i32,
                centre.y as i32,
                radius,
                draw::color_alpha(glow, alpha),
                draw::color_alpha(glow, 0.0),
            );
        }
    }

    // -- the trail -------------------------------------------------------------
    // Under the flower, over the backdrop and the bass glow. The composite mode
    // follows the ground for the same reason the glow's does: additive light on
    // cream saturates toward white and disappears, so a light ground takes the
    // trail as premultiplied paint *over* it — a colour ghost, which is what a
    // smear on paper looks like — and a dark one takes it as added light.
    if let Some(texture) = resources.trail.result() {
        let strength = (flower.trail_strength().clamp(0.0, 1.0) * 255.0) as u8;
        let tint = Color::new(strength, strength, strength, strength);
        // A render texture's rows are stored bottom-up.
        let source = Rectangle::new(0.0, 0.0, texture.width as f32, -(texture.height as f32));
        let mode = if daylight >= 0.5 {
            BlendMode::BLEND_ALPHA_PREMULTIPLY
        } else {
            // `ADD_COLORS`, not `ADDITIVE`: the buffer is premultiplied, so its
            // colour already carries its own alpha and multiplying by `srcA` a
            // second time would square the coverage.
            BlendMode::BLEND_ADD_COLORS
        };
        let mut blend = d.begin_blend_mode(mode);
        draw::draw_texture_pro(
            &mut blend,
            texture,
            source,
            boundary,
            Vector2::zero(),
            0.0,
            tint,
        );
    }

    // Petals, painted far-to-near is meaningless for a flat ring; order by index.
    for petal in &flower.petals {
        // Scatter alpha fades a blown-out petal; at 1.0 `ColorAlpha` returns
        // the colour byte-for-byte, so this is free on a non-boom frame.
        fill_convex(
            d,
            &petal.outline,
            draw::color_alpha(petal.fill, flower.scatter_alpha),
        );
        outline_loop(
            d,
            &petal.outline,
            stroke,
            draw::color_alpha(ink, flower.scatter_alpha),
        );
    }
    let (petal_peak, petal_peak_index) = (flower.petal_peak, flower.petal_peak_index);
    let boom = flower.boom;

    // The face disc. Slightly cream rather than pure white, like the art.
    let face_radius = flower.face_radius;
    d.draw_circle_v(centre, face_radius, Color::new(250, 248, 242, 255));
    d.draw_ring(
        centre,
        face_radius - stroke * 0.5,
        face_radius + stroke * 0.5,
        0.0,
        360.0,
        48,
        ink,
    );

    // Facial features, in axis-aligned head coordinates: the face stays upright.
    let f = |x: f32, y: f32| Vector2::new(centre.x + x * face_radius, centre.y + y * face_radius);
    let expression = state.expression();
    match expression {
        Expression::Happy => {
            for side in [-1.0f32, 1.0] {
                if eyelids_down {
                    // A blink swaps the caret for a lid stroke at the same
                    // anchor and width — see the module docs for why swap,
                    // not squash.
                    let lid: Vec<Vector2> = closed_eye_points(side * 0.42, -0.18, 0.14)
                        .iter()
                        .map(|&(x, y)| f(x, y))
                        .collect();
                    stroke_polyline(d, &lid, stroke, ink);
                } else {
                    stroke_polyline(
                        d,
                        &[
                            f(side * 0.56, -0.10),
                            f(side * 0.42, -0.26),
                            f(side * 0.28, -0.10),
                        ],
                        stroke,
                        ink,
                    );
                }
            }
            let mouth: Vec<Vector2> = happy_mouth_points(warmth, open)
                .iter()
                .map(|&(x, y)| f(x, y))
                .collect();
            stroke_polyline(d, &mouth, stroke, ink);
            if open > 0.0 {
                // The open mouth: a vertical capsule from two overlapping ink
                // discs — taller than wide reads as singing. Both radii scale
                // with `open` (a fixed-width slit at a tiny open fraction
                // would read as a moustache), and the capsule beats a single
                // circle because a round hole at full width reads as Boom's
                // shock `O` rather than a note being held.
                let ry = open * 0.16 * face_radius;
                let rx = open * 0.11 * face_radius;
                let mouth_centre = f(0.0, 0.32);
                let spread = (ry - rx).max(0.0);
                d.draw_circle_v(
                    Vector2::new(mouth_centre.x, mouth_centre.y - spread),
                    rx,
                    ink,
                );
                d.draw_circle_v(
                    Vector2::new(mouth_centre.x, mouth_centre.y + spread),
                    rx,
                    ink,
                );
            }
        }
        Expression::Scrunch => {
            // Never blinked: core holds `blink()` at 0 during a scrunch, and
            // the `>` `<` strokes *are* the closed-eye idea — no lid branch
            // here regardless, belt and braces.
            for side in [-1.0f32, 1.0] {
                // `>` `<`: both point inward.
                stroke_polyline(
                    d,
                    &[
                        f(side * 0.56, -0.30),
                        f(side * 0.30, -0.18),
                        f(side * 0.56, -0.06),
                    ],
                    stroke,
                    ink,
                );
            }
            stroke_polyline(
                d,
                &[
                    f(-0.30, 0.30),
                    f(-0.15, 0.24),
                    f(0.0, 0.34),
                    f(0.15, 0.24),
                    f(0.30, 0.30),
                ],
                stroke,
                ink,
            );
        }
        Expression::Dizzy => {
            for side in [-1.0f32, 1.0] {
                let eye = f(side * 0.42, -0.18);
                if eyelids_down {
                    // Lid at the spiral's own anchor, reach and stroke weight.
                    // Swapped, not scaled: a vertically squashed spiral
                    // polyline collapses into a scribble.
                    let lid: Vec<Vector2> = closed_eye_points(side * 0.42, -0.18, 0.19)
                        .iter()
                        .map(|&(x, y)| f(x, y))
                        .collect();
                    stroke_polyline(d, &lid, stroke * 0.8, ink);
                } else {
                    let spiral: Vec<Vector2> = (0..=20)
                        .map(|k| {
                            let s = k as f32 / 20.0;
                            let theta = s * TAU * 2.2 * side;
                            let radius = s * face_radius * 0.19;
                            Vector2::new(eye.x + radius * theta.cos(), eye.y + radius * theta.sin())
                        })
                        .collect();
                    stroke_polyline(d, &spiral, stroke * 0.8, ink);
                }
            }
            stroke_polyline(
                d,
                &[
                    f(-0.26, 0.30),
                    f(-0.13, 0.26),
                    f(0.0, 0.32),
                    f(0.13, 0.26),
                    f(0.26, 0.30),
                ],
                stroke,
                ink,
            );
        }
        Expression::Pleading => {
            for side in [-1.0f32, 1.0] {
                if eyelids_down {
                    // Lid at the wet circle's centre and width — the
                    // highlight goes with the eye, or the blink reads as the
                    // eyeball turning white.
                    let lid: Vec<Vector2> = closed_eye_points(side * 0.42, -0.16, 0.17)
                        .iter()
                        .map(|&(x, y)| f(x, y))
                        .collect();
                    stroke_polyline(d, &lid, stroke, ink);
                } else {
                    let eye = f(side * 0.42, -0.16);
                    d.draw_circle_v(eye, face_radius * 0.17, ink);
                    d.draw_circle_v(
                        Vector2::new(eye.x - face_radius * 0.055, eye.y - face_radius * 0.06),
                        face_radius * 0.055,
                        Color::new(250, 248, 242, 255),
                    );
                }
            }
            // A small worried frown, curving down at the ends.
            stroke_polyline(
                d,
                &[f(-0.18, 0.34), f(-0.06, 0.28), f(0.06, 0.28), f(0.18, 0.34)],
                stroke,
                ink,
            );
        }
        Expression::Boom => {
            // Never blinked, like Scrunch: an eyelid over a head-explode
            // would defuse the one expression built on wide-open shock.
            // `>` `<`-adjacent shock eyes and an open mouth...
            for side in [-1.0f32, 1.0] {
                let eye = f(side * 0.42, -0.14);
                d.draw_ring(
                    eye,
                    face_radius * 0.10,
                    face_radius * 0.10 + stroke,
                    0.0,
                    360.0,
                    24,
                    ink,
                );
            }
            d.draw_circle_v(f(0.0, 0.30), face_radius * 0.13, ink);
            // ...and the mushroom cloud, rising and swelling through the playout.
            let rise = 1.0 - (1.0 - boom) * (1.0 - boom);
            let cloud = Color::new(226, 218, 184, 255);
            let stem_width = face_radius * 0.34 * (0.6 + 0.4 * rise);
            let stem_top = centre.y - face_radius * (1.1 + 0.9 * rise);
            d.draw_rectangle_rec(
                Rectangle::new(
                    centre.x - stem_width * 0.5,
                    stem_top,
                    stem_width,
                    centre.y - face_radius * 0.6 - stem_top,
                ),
                cloud,
            );
            let cap = Vector2::new(centre.x, stem_top);
            let cap_radius = face_radius * (0.30 + 0.35 * rise);
            for (dx, dy, r) in [
                (0.0f32, 0.0f32, 1.0f32),
                (-0.9, 0.25, 0.7),
                (0.9, 0.25, 0.7),
                (-0.45, -0.4, 0.65),
                (0.45, -0.4, 0.65),
            ] {
                let at = Vector2::new(cap.x + dx * cap_radius, cap.y + dy * cap_radius);
                d.draw_circle_v(at, cap_radius * r, cloud);
                d.draw_circle_lines(at.x as i32, at.y as i32, cap_radius * r, ink);
            }
        }
    }

    // -- cats ------------------------------------------------------------------
    let font = DefaultFont::get();
    let cat_size = (min_dim * 0.045).max(10.0 * pixel_scale);
    let spacing = cat_size * 0.1;
    let floor = boundary.y + boundary.height - cat_size * 0.6;
    let drawn_cats = state.cat_count();
    for index in 0..drawn_cats {
        let Some(cat) = state.cat(index) else { break };
        let rows = cat_rows(cat.face);
        let size = cat_size * cat.scale;
        // The hop is a ballistic arc from core (`CatPose::hop` is the
        // half-sine); the idle bob under it keeps the floor alive through
        // sections with no bass transients at all — a row of statues waiting
        // for a kick reads as broken, a row of gently breathing cats reads as
        // listening. Amplitude scales the bob so silence stills them.
        let bob = (sway * 1.15 + index as f32 * 2.1).sin()
            * size
            * 0.05
            * (0.3 + 0.7 * state.amplitude());
        let lift = cat.hop * size * 1.1 + bob;
        let x = boundary.x + cat.lane * boundary.width;
        let tail = if cat.flip { "~ " } else { " ~" };
        let face_row = if cat.flip {
            format!("{}{}", tail, rows[1])
        } else {
            format!("{}{}", rows[1], tail)
        };
        let ears_width = font.measure_text(rows[0], size, spacing).x;
        let face_width = font.measure_text(&face_row, size, spacing).x;
        let colour = draw::color_alpha(ink, 0.88);
        d.draw_text_ex(
            &font,
            rows[0],
            Vector2::new(x - ears_width * 0.5, floor - lift - size * 1.9),
            size,
            spacing,
            colour,
        );
        d.draw_text_ex(
            &font,
            &face_row,
            Vector2::new(x - face_width * 0.5, floor - lift - size),
            size,
            spacing,
            colour,
        );
    }

    // -- the lyric terminal ----------------------------------------------------
    let mut terminal_status = if terminal_on { "idle" } else { "off" };
    let mut typed_report = String::new();
    if terminal_on {
        if let Some(cue) = frame.lyric {
            let typed = state.typed_chars();
            let text: String = cue.text.chars().take(typed).collect();
            let line = format!("> {text}");
            let size = (min_dim * 0.032).max(9.0 * pixel_scale);
            let spacing = size * 0.1;
            let at = Vector2::new(
                boundary.x + boundary.width * 0.04,
                boundary.y + boundary.height * 0.05,
            );
            let colour = draw::color_alpha(ink, 0.66);
            d.draw_text_ex(&font, &line, at, size, spacing, colour);
            // The block cursor, blinking on the sway clock so it is
            // deterministic in an export.
            if (sway * 1.8).sin() > -0.2 {
                let width = font.measure_text(&line, size, spacing).x;
                d.draw_rectangle_rec(
                    Rectangle::new(at.x + width + spacing + size * 0.1, at.y, size * 0.5, size),
                    colour,
                );
            }
            terminal_status = "typing";
            typed_report = format!("({typed}/{})", cue.text.chars().count());
        }
    }

    // A touch of vignette as the light goes down, like the reference's clouds.
    if daylight < 0.6 {
        draw::vignette(d, boundary, (0.6 - daylight) * 0.5);
    }

    // -- the report ------------------------------------------------------------
    // Every claim a capture cannot make on its own: which face (a dead machine
    // is a permanent "happy" — with music playing and beats>0 that combination
    // is readable), whether the petals coupled (peak and where), whether the
    // beat reached us, how many cats actually drew, and what the terminal did.
    // `warmth`/`semantic` print even when the lane is off — a run with an
    // Assist analysis attached shows `semantic=on` and a moving warmth, one
    // without shows `off` at the 0.60 neutral, so the plumbing is checkable
    // either way. `mouth` proves the sing-along oscillator coupled to a live
    // cue. `dyn` names which loudness gates were live — `track` (profiled) or
    // `none` (absolute fallback) — because the two draw the same picture on a
    // loud track, and a scene quietly running fallback gates on real material
    // is the unwired-feature trap. `show` prints phase *and* level: `on` at
    // 0.00 and a dead machine both photograph as terracotta. Blink is
    // deliberately not reported: a 0.24 s event sampled at one frame is noise.
    // `trail` is the same argument one layer down: a frame drawn with no
    // accumulated light and a frame whose buffer was refused by the driver are
    // the same picture, and `on@` carries the composite strength so a show's
    // streamers are a number rather than an impression.
    resources.last = format!(
        "face={} amp={:.2} bass={:.2} energy={:.2} dyn={} petal-peak={:.2}@{} beats={} kicks={} bounce={:.2} cats={} show={}@{:.2} trail={} terminal={}{} daylight={:.2} warmth={:.2} mouth={:.2} semantic={}",
        expression.name(),
        state.amplitude(),
        state.bass(),
        state.energy(),
        if state.dynamics_present() { "track" } else { "none" },
        petal_peak,
        petal_peak_index,
        state.beat_count(),
        state.kick_count(),
        state.bounce(),
        drawn_cats,
        state.show_phase().name(),
        show,
        resources.trail_status,
        terminal_status,
        typed_report,
        daylight,
        warmth,
        mouth_open,
        if state.semantic_active() { "on" } else { "off" },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every drawn string must be ASCII: the built-in face is Latin-1 and an
    /// absent glyph draws as an empty box — a plausible picture, which is the
    /// failure mode this repository keeps paying for. Same rule as phosphor's
    /// alphabet test.
    #[test]
    fn every_cat_string_is_ascii() {
        for face in [
            CatFace::Content,
            CatFace::Excited,
            CatFace::Sleepy,
            CatFace::Curious,
        ] {
            for row in cat_rows(face) {
                assert!(row.is_ascii(), "{row:?} would draw empty boxes");
            }
        }
        assert!("> ~".is_ascii());
    }

    #[test]
    fn petal_outlines_are_closed_and_finite() {
        let outline = petal_outline(Vector2::new(100.0, 100.0), 1.2, 20.0, 60.0, 24.0);
        assert_eq!(outline.len(), PETAL_SEGMENTS);
        for p in &outline {
            assert!(p.x.is_finite() && p.y.is_finite());
            // Everything stays within the reach the parameters describe.
            let dx = p.x - 100.0;
            let dy = p.y - 100.0;
            assert!((dx * dx + dy * dy).sqrt() <= 20.0 + 60.0 + 1.0e-3);
        }
    }

    /// The endpoints are the byte-identity contract: `boom_progress()` is 0 on
    /// every non-boom frame, and `(0.0, 1.0)` there is what keeps those frames
    /// pixel-equal to a build without the scatter. Exact, not approximate.
    #[test]
    fn scatter_endpoints_are_exact() {
        assert_eq!(scatter(0.0), (0.0, 1.0));
        assert_eq!(scatter(1.0), (0.0, 1.0));
        // Out-of-range progress (a clamp upstream slipping) stays neutral too.
        assert_eq!(scatter(-0.25), (0.0, 1.0));
        assert_eq!(scatter(1.25), (0.0, 1.0));
    }

    #[test]
    fn scatter_offset_peaks_near_1_6_and_holds() {
        let (peak, _) = scatter(0.3);
        assert!((1.5..=1.7).contains(&peak), "peak {peak} out of range");
        // The hold: the offset does not move between the rise and the regrow.
        for p in [0.35, 0.5, 0.65, 0.74] {
            let (offset, _) = scatter(p);
            assert!((offset - peak).abs() < 1.0e-6, "offset moved at {p}");
        }
    }

    #[test]
    fn scatter_alpha_stays_on_the_floor() {
        // Never below the floor, never above full, anywhere in the playout.
        for k in 0..=200 {
            let p = k as f32 / 200.0;
            let (_, alpha) = scatter(p);
            assert!(
                (0.35 - 1.0e-6..=1.0).contains(&alpha),
                "alpha {alpha} at {p}"
            );
        }
        // And it actually reaches the floor before the regrow starts.
        let (_, faded) = scatter(0.75);
        assert!((faded - 0.35).abs() < 1.0e-6, "fade never landed: {faded}");
    }

    /// The last 25 % is the regrow: offset strictly shrinking, alpha strictly
    /// rising, no bounce on the way back to normal.
    #[test]
    fn scatter_recovery_is_monotone() {
        let mut prev = scatter(0.75);
        for k in 1..=50 {
            let p = 0.75 + k as f32 / 50.0 * 0.25;
            let cur = scatter(p);
            assert!(cur.0 <= prev.0 + 1.0e-6, "offset rose at {p}");
            assert!(cur.1 >= prev.1 - 1.0e-6, "alpha fell at {p}");
            prev = cur;
        }
    }

    /// The mouth's baseline is the corners' y: flattening must converge on
    /// that line, not drift the mouth up or down the face.
    #[test]
    fn happy_mouth_flattens_onto_its_baseline() {
        // Fully open: every point exactly on the 0.24 baseline.
        for (_, y) in happy_mouth_points(0.6, 1.0) {
            assert_eq!(y, 0.24);
        }
        // Max warmth, shut: byte-equal to the resting shape.
        let rest = happy_mouth_points(1.0, 0.0);
        assert_eq!(rest[0], (-0.30, 0.24));
        assert_eq!(rest[1], (-0.15, 0.40));
        assert_eq!(rest[2], (0.0, 0.26));
        // x never moves, whatever the drives do.
        for (warmth, open) in [(0.0, 0.0), (0.6, 0.5), (1.0, 0.9)] {
            for (rest_point, moved) in rest.iter().zip(happy_mouth_points(warmth, open)) {
                assert_eq!(rest_point.0, moved.0);
            }
        }
    }

    /// A warmer track smiles deeper, a more open mouth smiles flatter — both
    /// monotone, so the two drives cannot fight into a wobble.
    #[test]
    fn happy_mouth_bounce_is_monotone_in_both_drives() {
        let depth = |warmth: f32, open: f32| {
            happy_mouth_points(warmth, open)
                .iter()
                .map(|&(_, y)| y - 0.24)
                .fold(0.0f32, f32::max)
        };
        let mut prev = depth(0.0, 0.0);
        for k in 1..=10 {
            let cur = depth(k as f32 / 10.0, 0.0);
            assert!(cur >= prev, "warmth {k} shallowed the smile");
            prev = cur;
        }
        let mut prev = depth(0.6, 0.0);
        for k in 1..=10 {
            let cur = depth(0.6, k as f32 / 10.0);
            assert!(cur <= prev, "open {k} deepened the smile");
            prev = cur;
        }
    }

    /// The lid stroke's contract: ends exactly 0.02 face-radii above the
    /// middle (screen y grows downward), symmetric about the eye centre.
    #[test]
    fn closed_eye_is_a_shallow_symmetric_curve() {
        let lid = closed_eye_points(0.42, -0.18, 0.14);
        assert_eq!(lid[1], (0.42, -0.18));
        for end in [lid[0], lid[2]] {
            assert_eq!(end.1, lid[1].1 - 0.02, "ends must sit 0.02 higher");
        }
        assert_eq!(lid[1].0 - lid[0].0, lid[2].0 - lid[1].0);
    }

    /// The buffer keeps the boundary's aspect and its long edge, both ways
    /// round. A stretched trail is a ghost that does not line up with the petal
    /// it came from, and a 9:16 export is exactly where a hard-coded landscape
    /// assumption would show up.
    #[test]
    fn the_trail_buffer_keeps_its_boundarys_aspect() {
        let (w, h) = trail_buffer_size(Rectangle::new(0.0, 0.0, 1920.0, 1080.0));
        assert_eq!((w, h), (TRAIL_BUFFER_EDGE, 576));
        let (w, h) = trail_buffer_size(Rectangle::new(0.0, 0.0, 1080.0, 1920.0));
        assert_eq!((w, h), (576, TRAIL_BUFFER_EDGE));
        // Square, and a degenerate sliver that must still be allocatable.
        assert_eq!(
            trail_buffer_size(Rectangle::new(0.0, 0.0, 700.0, 700.0)),
            (TRAIL_BUFFER_EDGE, TRAIL_BUFFER_EDGE)
        );
        let (w, h) = trail_buffer_size(Rectangle::new(0.0, 0.0, 4000.0, 20.0));
        assert!(w >= 16 && h >= 16, "{w}x{h} cannot be allocated");
        // Supersampling must not change the grid: the composite scales it, so a
        // 2x export accumulates in the same buffer a 1x preview does, which is
        // what lets a still and its video frame be compared at all.
        assert_eq!(
            trail_buffer_size(Rectangle::new(0.0, 0.0, 3840.0, 2160.0)),
            trail_buffer_size(Rectangle::new(0.0, 0.0, 1920.0, 1080.0))
        );
    }

    /// Premultiplication is the buffer's contract, and its two endpoints are
    /// the ones a blend can be wrong about: nothing at all, and full paint.
    #[test]
    fn premultiplied_endpoints_are_exact() {
        let terracotta = Color::new(200, 100, 60, 255);
        assert_eq!(premultiplied(terracotta, 0.0), Color::new(0, 0, 0, 0));
        assert_eq!(
            premultiplied(terracotta, 1.0),
            Color::new(200, 100, 60, 255)
        );
        // And in between, every channel carries the same factor — the property
        // the fade's single-tint multiply depends on.
        let half = premultiplied(terracotta, 0.5);
        assert_eq!(half.r, 100);
        assert_eq!(half.g, 50);
        assert_eq!(half.b, 30);
        assert_eq!(half.a, 127);
        // Out-of-range alpha is clamped rather than wrapped: a `as u8` cast of
        // 300.0 is 255 on some paths and a surprise on others.
        assert_eq!(
            premultiplied(terracotta, 2.0),
            Color::new(200, 100, 60, 255)
        );
        assert_eq!(premultiplied(terracotta, -1.0), Color::new(0, 0, 0, 0));
    }

    #[test]
    fn a_carried_flower_only_matches_its_own_rectangle() {
        let a = Rectangle::new(10.0, 20.0, 300.0, 200.0);
        assert!(same_rect(a, a));
        for other in [
            Rectangle::new(10.5, 20.0, 300.0, 200.0),
            Rectangle::new(10.0, 20.0, 300.0, 200.5),
        ] {
            assert!(!same_rect(a, other), "{other:?} matched");
        }
    }

    #[test]
    fn ink_reads_on_both_backdrops() {
        let dark_bg_ink = ink_color(0.0);
        let cream_bg_ink = ink_color(1.0);
        assert!(dark_bg_ink.r > 200, "dark backdrop needs light ink");
        assert!(cream_bg_ink.r < 80, "cream backdrop needs dark ink");
    }
}
