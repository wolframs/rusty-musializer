//! Lumen Knot's drawing half: one glass strand tied as a torus knot, lit from
//! within by comets that run over and under its own crossings, in a night-blue
//! haze. All animation reads scene time and state; the bloom has no
//! framebuffer history.
use musializer_core::scene::settings::index::knot as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::lumen_knot::{
    breathe, point, LumenKnotState, BANDS, BREATH, COMET_SECONDS, EXTENT, FORMS, SEGMENTS,
};
use musializer_runtime::draw;
use musializer_runtime::halo::HaloBlur;
use raylib::prelude::{
    BlendMode, Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, RaylibShader,
    RaylibShaderModeExt, Rectangle, Shader, Vector2, Vector4,
};
use std::f32::consts::TAU;

const ID: SceneId = SceneId::LumenKnot;
/// Perspective focal distance in knot units: enough to swell the near petals,
/// little enough that the fit's allowance for it stays a few percent.
const FOCAL: f32 = 4.0;
/// The woven gap: the soft shadow each pass lays either side of itself
/// reaches this many tube radii from its centre line.
const GAP: f32 = 1.6;
/// How far behind its own pass a shadow sorts, in tube radii. Less than the
/// depth between two crossing passes (the core pins more than 2.6), more than
/// the depth between a pass and its own neighbours on a bend, so the shadow
/// falls on the strand beneath and never on the strand that cast it.
const SHADOW_SINK: f32 = 2.2;
/// Cross-section steps of the tube at full resolution and in the bloom pass.
const CROSS: usize = 10;
const CROSS_BLOOM: usize = 4;

type Vertex = (Vector2, Color);
pub struct KnotResources {
    vertices: Vec<Vertex>,
    bloom: HaloBlur,
    bloom_status: &'static str,
    finish: Option<Shader>,
    finish_locations: [i32; 4],
    pub report: String,
}
impl KnotResources {
    pub fn load(rl: &mut raylib::RaylibHandle, thread: &raylib::RaylibThread) -> Self {
        let finish = rl.load_shader_from_memory(
            thread,
            Some(include_str!(
                "../../../../resources/shaders/glsl330/knot_finish.vs"
            )),
            Some(include_str!(
                "../../../../resources/shaders/glsl330/knot_finish.fs"
            )),
        );
        let finish_locations =
            ["boundary", "grain", "tick", "seed"].map(|name| finish.get_shader_location(name));
        let valid = finish.is_shader_valid() && finish_locations.iter().all(|loc| *loc >= 0);
        Self {
            finish: valid.then_some(finish),
            finish_locations,
            vertices: Vec::new(),
            bloom: HaloBlur::load(rl, thread),
            bloom_status: "none",
            report: String::new(),
        }
    }
}

/// Before the host's scissor: HaloBlur temporarily redirects the framebuffer,
/// including when the caller is already inside a supersampled export target.
pub fn prepare(
    d: &mut RaylibDrawHandle<'_>,
    resources: &mut KnotResources,
    state: &LumenKnotState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
) {
    if frame.setting(ID, control::BLOOM) <= 0.0 || boundary.width <= 0.0 || boundary.height <= 0.0 {
        resources.bloom_status = "off";
        return;
    }
    let ratio = 384.0 / boundary.width.max(boundary.height);
    let w = (boundary.width * ratio).round().max(16.0) as i32;
    let h = (boundary.height * ratio).round().max(16.0) as i32;
    let vertices = &mut resources.vertices;
    let built = resources.bloom.render(d, w, h, 3.0, |target| {
        draw_base(
            target,
            frame,
            state,
            vertices,
            Rectangle::new(0.0, 0.0, w as f32, h as f32),
            true,
        );
    });
    resources.bloom_status = if built { "blurred" } else { "unavailable" };
}

pub fn draw(
    d: &mut impl RaylibDraw,
    frame: &SceneFrame<'_>,
    state: &LumenKnotState,
    resources: &mut KnotResources,
    boundary: Rectangle,
) {
    let mut report = draw_base(d, frame, state, &mut resources.vertices, boundary, false);
    let setting = |i| frame.setting(ID, i);
    let strength = setting(control::BLOOM);
    let fringe = setting(control::FRINGE);
    if strength > 0.0 && resources.bloom_status == "blurred" {
        if let Some(texture) = resources.bloom.result() {
            let source = Rectangle::new(0.0, 0.0, texture.width as f32, -(texture.height as f32));
            let level = |v: f32| (255.0 * (strength * v).min(1.0)) as u8;
            let mut blend = d.begin_blend_mode(BlendMode::BLEND_ADDITIVE);
            draw::draw_texture_pro(
                &mut blend,
                texture,
                source,
                boundary,
                Vector2::zero(),
                0.0,
                Color::new(255, 255, 255, level(0.85)),
            );
            if fringe > 0.0 {
                let split = boundary.width.min(boundary.height) * 0.012 * fringe;
                for (direction, color) in [
                    (-1.0, Color::new(0, 225, 255, level(0.55))),
                    (1.0, Color::new(255, 0, 125, level(0.55))),
                ] {
                    let shifted = Rectangle::new(
                        boundary.x + split * direction,
                        boundary.y - split * direction * 0.3,
                        boundary.width,
                        boundary.height,
                    );
                    draw::draw_texture_pro(
                        &mut blend,
                        texture,
                        source,
                        shifted,
                        Vector2::zero(),
                        0.0,
                        color,
                    );
                }
            }
        }
    }
    let grain = setting(control::GRAIN);
    let finish_status = if let Some(shader) = resources.finish.as_mut() {
        let [rect, amount, tick, seed] = resources.finish_locations;
        shader.set_shader_value(
            rect,
            Vector4::new(boundary.x, boundary.y, boundary.width, boundary.height),
        );
        shader.set_shader_value(amount, grain);
        shader.set_shader_value(tick, (state.time * 24.0).floor());
        shader.set_shader_value(seed, state.phase * 65536.0);
        let mut mode = d.begin_shader_mode(shader);
        mode.draw_rectangle_rec(boundary, Color::WHITE);
        "ready"
    } else {
        "unavailable"
    };
    report.push_str(&format!(
        " bloom={} fringe={fringe:.2} grain={grain:.2} finish={finish_status}",
        resources.bloom_status
    ));
    resources.report = report;
}

fn rgb(c: [f32; 3], alpha: u8) -> Color {
    Color::new(
        c[0].clamp(0.0, 255.0) as u8,
        c[1].clamp(0.0, 255.0) as u8,
        c[2].clamp(0.0, 255.0) as u8,
        alpha,
    )
}
fn hsv(hue: f32, saturation: f32, value: f32) -> [f32; 3] {
    let c = draw::color_from_hsv(hue.rem_euclid(360.0), saturation, value.clamp(0.0, 1.0));
    [f32::from(c.r), f32::from(c.g), f32::from(c.b)]
}
fn triangle(out: &mut Vec<Vertex>, a: Vertex, b: Vertex, c: Vertex) {
    let cross = (b.0.x - a.0.x) * (c.0.y - a.0.y) - (b.0.y - a.0.y) * (c.0.x - a.0.x);
    if cross < 0.0 {
        out.extend([a, b, c]);
    } else {
        out.extend([a, c, b]);
    }
}
fn quad(out: &mut Vec<Vertex>, a: Vertex, b: Vertex, c: Vertex, d: Vertex) {
    triangle(out, a, b, c);
    triangle(out, a, c, d);
}

type V3 = [f32; 3];
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add_scaled(a: V3, b: V3, k: f32) -> V3 {
    [a[0] + b[0] * k, a[1] + b[1] * k, a[2] + b[2] * k]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalized(a: V3) -> Option<V3> {
    let len = dot(a, a).sqrt();
    (len > 1e-7).then(|| [a[0] / len, a[1] / len, a[2] / len])
}

/// One sample of the strand after breathing and turning: its centre, the
/// screen-across direction and the toward-the-viewer direction of its tube.
#[derive(Clone, Copy)]
struct Sample {
    p: V3,
    across: V3,
    toward: V3,
    /// Angle in the knot's own frame: colour and band glow are fixed to the
    /// knot, so the passes that meet in a change's ring agree.
    angle: f32,
    /// How far the shadow reaches, in tube radii: [`GAP`], or less where the
    /// strand turns tighter on screen than the shadow is wide, whose quads
    /// would fold into a dark spike inside the corner.
    reach: f32,
}

#[allow(clippy::too_many_lines)] // One frame's composition, read top to bottom.
fn draw_base(
    d: &mut impl RaylibDraw,
    frame: &SceneFrame<'_>,
    state: &LumenKnotState,
    out: &mut Vec<Vertex>,
    boundary: Rectangle,
    emission: bool,
) -> String {
    if boundary.width <= 0.0 || boundary.height <= 0.0 {
        return String::new();
    }
    let setting = |i| frame.setting(ID, i);
    let time = state.time;
    let response = setting(control::RESPONSE);
    // Beat lift scales both what onsets and what kicks do to the strand.
    let beat = (state.beat + state.punch) * setting(control::PULSE);
    let energy = state.energy * response;
    let atmosphere = setting(control::ATMOSPHERE);
    let polish = setting(control::POLISH);
    let hue = setting(control::HUE);
    let drift = setting(control::DRIFT).min(1.0);
    let flow = setting(control::FLOW);
    let phase = state.phase;
    let pose = state.pose();
    let form = &FORMS[pose.form];
    let tube = pose.tube;

    // Reserve room for the strongest local breathing and the near side's
    // perspective at every control extreme: raising Size must never clip a
    // petal in portrait or a short preview panel.
    let expansion =
        1.0 + BREATH * response.clamp(0.0, 2.0) * state.bands.iter().copied().fold(0.0, f32::max);
    let reach = EXTENT * expansion;
    let perspective = 1.0 / (1.0 - (reach / FOCAL).powi(2)).sqrt();
    let radius = (0.47 * setting(control::SIZE) * (1.0 + 0.06 * energy + 0.03 * beat))
        .min(0.47 / (reach * perspective));
    let short = boundary.width.min(boundary.height);
    let scale = short * radius;
    let center = Vector2::new(
        boundary.x + boundary.width * 0.5,
        boundary.y + boundary.height * 0.5,
    );
    let yaw = (state.spin.rem_euclid(std::f64::consts::TAU)) as f32 + phase;
    let lean = 0.52 + 0.10 * drift * (time * 0.21 + phase).sin();
    let sway = 0.16 * drift * (time * 0.13 + phase * 1.7).sin();
    let (sy, cy) = yaw.sin_cos();
    let (sl, cl) = lean.sin_cos();
    let (ss, cs) = sway.sin_cos();
    let turn = |p: V3| -> V3 {
        let p = [p[0] * cy - p[1] * sy, p[0] * sy + p[1] * cy, p[2]];
        let p = [p[0], p[1] * cl - p[2] * sl, p[1] * sl + p[2] * cl];
        [p[0] * cs + p[2] * ss, p[1], -p[0] * ss + p[2] * cs]
    };
    let project = |p: V3| {
        let k = FOCAL / (FOCAL - p[2]);
        Vector2::new(center.x + scale * k * p[0], center.y - scale * k * p[1])
    };

    out.clear();
    if !emission {
        // A low-frequency mesh rather than circles behind an object: the haze
        // moves across the entire canvas and respects any export aspect.
        let cols = 40;
        let rows = 28;
        let aspect = boundary.width / boundary.height;
        let background = |x: usize, y: usize| {
            let u = x as f32 / cols as f32;
            let v = y as f32 / rows as f32;
            let px = (u - 0.5) * aspect;
            let py = v - 0.5;
            let wave = (px * 3.1 + py * 2.2 + time * 0.07 + phase).sin() * 0.14;
            let mist = (-((py - wave) * 3.6).powi(2) - (px * 0.9).powi(2)).exp() * atmosphere;
            let r = (px * px + py * py).sqrt() / (short / boundary.height);
            // The knot lights the haze behind its strands, not its opening:
            // a ring of glow round the knot's own ring keeps the centre a dark
            // window the weave reads against.
            let halo =
                0.45 * (-(r * 2.4).powi(2)).exp() + 0.75 * (-((r - 0.27) / 0.15).powi(2)).exp();
            let ground = hsv(229.0 + hue, 0.78, 0.075 + 0.05 * v);
            let glow = hsv(248.0 + hue, 0.62, 0.20 + 0.14 * energy + 0.08 * beat);
            let veil = hsv(
                188.0 + hue + 70.0 * (px * 0.8 + time * 0.02).sin(),
                0.70,
                0.11 + 0.13 * energy,
            );
            let c: [f32; 3] =
                std::array::from_fn(|i| ground[i] + glow[i] * halo + veil[i] * mist.min(1.4));
            (
                Vector2::new(
                    boundary.x + u * boundary.width,
                    boundary.y + v * boundary.height,
                ),
                rgb(c, 255),
            )
        };
        for y in 0..rows {
            for x in 0..cols {
                quad(
                    out,
                    background(x, y),
                    background(x + 1, y),
                    background(x + 1, y + 1),
                    background(x, y + 1),
                );
            }
        }
        draw::shaded_triangles(d, out);
        out.clear();
        // Light the knot throws on an unseen floor below it: weight without a
        // blur buffer or an export-only path.
        for ring in (0..20).rev() {
            let r = scale * (0.80 + ring as f32 * 0.016);
            let alpha = ((1.0 - ring as f32 / 20.0) * (1.4 + 1.6 * energy) * atmosphere) as u8;
            d.draw_ellipse(
                center.x as i32,
                (center.y + scale * 0.98) as i32,
                r,
                r * 0.16,
                Color::new(70, 150, 230, alpha),
            );
        }
        // Out-of-focus lights far behind the knot: depth for the eye, drifting
        // on absolute time, a little brighter when the music is.
        let mut blend = d.begin_blend_mode(BlendMode::BLEND_ADDITIVE);
        for i in 0..14 {
            let h = i as f32;
            let u = (h * 0.754_878 + 0.13 * (time * 0.011 + h).sin() + phase * 0.1).fract();
            let v =
                (h * 0.569_840 - time * (0.004 + 0.002 * (h * 1.3).sin().abs())).rem_euclid(1.0);
            let size = short * (0.025 + 0.045 * (h * 2.1).sin().abs());
            let tint = hsv([196.0, 262.0, 32.0][i % 3] + hue, 0.55, 1.0);
            let alpha = (0.035 + 0.05 * energy) * atmosphere.min(1.5);
            let inner = rgb(tint, (255.0 * alpha.min(1.0)) as u8);
            blend.draw_circle_gradient(
                (boundary.x + u * boundary.width) as i32,
                (boundary.y + v * boundary.height) as i32,
                size,
                inner,
                rgb(tint, 0),
            );
        }
    }

    // The strand: one sample per segment boundary, closed.
    let n = if emission { SEGMENTS / 2 } else { SEGMENTS };
    let mut samples: Vec<Sample> = Vec::with_capacity(n);
    let mut centres: Vec<V3> = Vec::with_capacity(n);
    let mut angles: Vec<f32> = Vec::with_capacity(n);
    for j in 0..n {
        let base = point(&pose, j as f32 / n as f32);
        angles.push(base[1].atan2(base[0]));
        centres.push(turn(breathe(base, &state.bands, response)));
    }
    let view = [0.0, 0.0, 1.0];
    let mut last_across = [1.0, 0.0, 0.0];
    for j in 0..n {
        let tangent = sub(centres[(j + 1) % n], centres[(j + n - 1) % n]);
        // A pass seen end-on has no screen direction of its own; it keeps its
        // neighbour's rather than snapping.
        let across = normalized(cross(tangent, view)).unwrap_or(last_across);
        last_across = across;
        let mut toward = normalized(cross(across, tangent)).unwrap_or(view);
        if toward[2] < 0.0 {
            toward = [-toward[0], -toward[1], -toward[2]];
        }
        let (back_step, fore_step) = (
            sub(centres[j], centres[(j + n - 1) % n]),
            sub(centres[(j + 1) % n], centres[j]),
        );
        let (a, b) = (
            back_step[0].hypot(back_step[1]),
            fore_step[0].hypot(fore_step[1]),
        );
        let turn = if a > 1e-7 && b > 1e-7 {
            ((back_step[0] * fore_step[0] + back_step[1] * fore_step[1]) / (a * b))
                .clamp(-1.0, 1.0)
                .acos()
        } else {
            0.0
        };
        let bend_radius = if turn > 1e-6 {
            0.5 * (a + b) / turn
        } else {
            f32::INFINITY
        };
        samples.push(Sample {
            p: centres[j],
            across,
            toward,
            angle: angles[j],
            reach: (0.9 * bend_radius / tube).clamp(1.0, GAP),
        });
    }

    // Lights, in the turned frame (x right, y up, z toward the viewer).
    let key = normalized([-0.45, 0.62, 0.64]).unwrap_or(view);
    let half = normalized([key[0], key[1], key[2] + 1.0]).unwrap_or(view);
    let back = normalized([0.55, -0.35, -0.75]).unwrap_or(view);
    let band_glow = |angle: f32| {
        let mut weight = 0.0;
        let mut drive = 0.0;
        for (i, band) in state.bands.iter().enumerate() {
            let w = (0.5 + 0.5 * (angle - i as f32 * TAU / BANDS as f32).cos()).powi(6);
            weight += w;
            drive += w * band;
        }
        drive / f32::max(weight, 0.001)
    };
    let comet_light = |u: f32| {
        let mut light = 0.0;
        for comet in &state.comets {
            let envelope = (comet.age / 0.12).min(1.0)
                * (1.0 - comet.age / COMET_SECONDS).max(0.0).powf(1.5)
                * (0.35 + 0.65 * comet.strength);
            let behind = (comet.head - u).rem_euclid(1.0);
            let ahead = (u - comet.head).rem_euclid(1.0);
            let tail = if behind < 0.3 {
                (-behind / 0.045).exp()
            } else {
                0.0
            };
            let spark = if ahead < 0.02 {
                (-ahead / 0.003).exp()
            } else {
                0.0
            };
            light += envelope * (tail + spark);
        }
        light
    };
    let depth_span = reach.max(0.01);
    let cross_steps = if emission { CROSS_BLOOM } else { CROSS };
    let stride = cross_steps + 1;
    let mut mesh: Vec<Vertex> = Vec::with_capacity(n * stride);
    let mut shadow: Vec<[Vertex; 4]> = Vec::with_capacity(n);
    for (j, sample) in samples.iter().enumerate() {
        let u = j as f32 / n as f32;
        let glow = band_glow(sample.angle) * response;
        let comet = comet_light(u);
        // A slow sheen travels the strand at Light travel, even in silence.
        let travel = (u * TAU * form.petals as f32 * 0.5 - time * flow * 1.4 + phase).sin();
        let shimmer = ((travel - 0.55) / 0.45).max(0.0).powi(2);
        let near = ((sample.p[2] + depth_span) / (2.0 * depth_span)).clamp(0.0, 1.0);
        // Sapphire glass whose hue drifts round the knot; the far side sinks
        // toward indigo, which is most of what reads as depth.
        let base_hue = 214.0
            + hue
            + 30.0 * (sample.angle * 2.0 + time * 0.11 + phase).sin()
            + 22.0 * (1.0 - near);
        let band_color = hsv(base_hue - 36.0, 0.70, 1.0);
        let carried = 1.5 * comet + 0.7 * pose.flare + 0.30 * beat + 0.22 * shimmer;
        for k in 0..=cross_steps {
            let s = -1.0 + 2.0 * k as f32 / cross_steps as f32;
            let crown = (1.0 - s * s).max(0.0).sqrt();
            let normal = add_scaled(
                [
                    sample.across[0] * s,
                    sample.across[1] * s,
                    sample.across[2] * s,
                ],
                sample.toward,
                crown,
            );
            let position = project(add_scaled(sample.p, sample.across, tube * s));
            let facing = dot(normal, view).max(0.0);
            let fresnel = (1.0 - facing).powi(2);
            let lambert = dot(normal, key).max(0.0);
            let backlit = dot(normal, back).max(0.0).powi(2);
            let spec = dot(normal, half).max(0.0).powi(56) * polish;
            let sheen = dot(normal, half).max(0.0).powi(7) * polish;
            // Thin-film iridescence: the crown stays sapphire, the edges turn
            // violet, then rose, as they face away.
            let film = base_hue + 100.0 * fresnel;
            let glass = hsv(film, 0.55 + 0.30 * fresnel, 1.0);
            let edge = hsv(film + 30.0, 0.72, 1.0);
            let depth = 0.35 + 0.65 * near;
            // Comet light: amber in the body of the tail, white-hot where it
            // is brightest.
            let heat = (carried - 0.6).clamp(0.0, 1.0);
            let warm = [255.0, 186.0 + 60.0 * heat, 96.0 + 140.0 * heat];
            let color: [f32; 3] = if emission {
                // Only light the strand carries scatters: comets, band glow,
                // the change's flare and the sharpest highlights. Black
                // elsewhere, so a near pass hides the glow of the one beneath.
                std::array::from_fn(|i| {
                    warm[i] * crown.powi(2) * carried * 1.6
                        + band_color[i] * crown * (glow - 0.12).max(0.0) * 0.9
                        + 255.0 * (spec - 0.4).max(0.0) * 0.7
                })
            } else {
                std::array::from_fn(|i| {
                    glass[i] * (0.05 + 0.42 * lambert) * depth
                        + edge[i]
                            * (0.48 * fresnel + 0.45 * backlit)
                            * depth
                            * (0.45 + 0.35 * polish)
                        + 255.0 * (spec + sheen * 0.08)
                        + warm[i] * crown.powi(4) * carried * 0.85
                        + band_color[i] * crown.powi(2) * glow * 0.85
                        + glass[i] * crown.powi(8) * 0.10
                })
            };
            mesh.push((position, rgb(color, 255)));
        }
        if !emission {
            let shade = Color::new(3, 4, 14, 0);
            let dark = Color::new(2, 3, 12, 235);
            shadow.push([
                (
                    project(add_scaled(sample.p, sample.across, -tube * sample.reach)),
                    shade,
                ),
                (project(add_scaled(sample.p, sample.across, -tube)), dark),
                (project(add_scaled(sample.p, sample.across, tube)), dark),
                (
                    project(add_scaled(sample.p, sample.across, tube * sample.reach)),
                    shade,
                ),
            ]);
        }
    }
    // Painter's order over the true depth. Each segment is a body and, a
    // little deeper, the shadow it lays on whatever passes beneath it.
    let mut order: Vec<(f32, usize, bool)> = Vec::with_capacity(2 * n);
    for j in 0..n {
        let z = 0.5 * (samples[j].p[2] + samples[(j + 1) % n].p[2]);
        order.push((z, j, true));
        if !emission {
            order.push((z - SHADOW_SINK * tube, j, false));
        }
    }
    order.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, j, body) in order {
        let next = (j + 1) % n;
        if body {
            let vertex = |i: usize, k: usize| mesh[i * stride + k];
            for k in 0..cross_steps {
                quad(
                    out,
                    vertex(j, k),
                    vertex(next, k),
                    vertex(next, k + 1),
                    vertex(j, k + 1),
                );
            }
        } else {
            let (a, b) = (shadow[j], shadow[next]);
            quad(out, a[0], b[0], b[1], a[1]);
            quad(out, a[2], b[2], b[3], a[3]);
        }
    }
    let triangles = out.len() / 3;
    draw::shaded_triangles(d, out);
    // Sparse motes use absolute time and seed phase; they never accumulate or
    // flash on a beat.
    if !emission {
        for i in 0..42 {
            let h = i as f32;
            let u = (h * 0.618_034 + time * 0.003 + phase).fract();
            let v = (h * 0.381_966 - time * 0.006).rem_euclid(1.0);
            let a =
                ((0.35 + 0.65 * (h + time * 0.4).sin().abs()) * 34.0 * atmosphere).min(100.0) as u8;
            d.draw_circle_v(
                Vector2::new(
                    boundary.x + u * boundary.width,
                    boundary.y + v * boundary.height,
                ),
                boundary.height * 0.0012,
                Color::new(182, 214, 255, a),
            );
        }
    }
    format!(
        "form={} petals={} winds={} swing={:.3} morph={:.3} segments={SEGMENTS} triangles={triangles} weave=sorted comets={} energy={:.4} beat={:.4} bands={} clock={:.3}",
        pose.form + 1,
        form.petals,
        form.winds,
        pose.swing,
        state.morph,
        state.comets.len(),
        state.energy,
        state.beat,
        state
            .bands
            .iter()
            .map(|b| format!("{b:.3}"))
            .collect::<Vec<_>>()
            .join(","),
        time
    )
}
