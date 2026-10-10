//! Pearlescent ribbons suspended in an audio-lit aurora. All animation reads
//! scene time/state; the highlight bloom has no framebuffer history.
use musializer_core::scene::settings::index::knot as control;
use musializer_core::scene::{SceneFrame, SceneId};
use musializer_core::scenes::openai_knot::{
    breathe, centerline, depth, KnotState, HALF_WIDTH, SEGMENTS, STRANDS,
};
use musializer_runtime::draw;
use musializer_runtime::halo::HaloBlur;
use raylib::prelude::{
    BlendMode, Color, RaylibBlendModeExt, RaylibDraw, RaylibDrawHandle, RaylibShader,
    RaylibShaderModeExt, Rectangle, Shader, Vector2, Vector4,
};
use std::f32::consts::{PI, TAU};

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
    state: &KnotState,
    frame: &SceneFrame<'_>,
    boundary: Rectangle,
) {
    if frame.setting(SceneId::OpenAiKnot, control::BLOOM) <= 0.0
        || boundary.width <= 0.0
        || boundary.height <= 0.0
    {
        resources.bloom_status = "off";
        return;
    }
    let ratio = 384.0 / boundary.width.max(boundary.height);
    let w = (boundary.width * ratio).round().max(16.0) as i32;
    let h = (boundary.height * ratio).round().max(16.0) as i32;
    let vertices = &mut resources.vertices;
    let built = resources.bloom.render(d, w, h, 3.2, |target| {
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
    state: &KnotState,
    resources: &mut KnotResources,
    boundary: Rectangle,
) {
    let mut report = draw_base(d, frame, state, &mut resources.vertices, boundary, false);
    let setting = |i| frame.setting(SceneId::OpenAiKnot, i);
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

fn rgb(c: [f32; 3]) -> Color {
    Color::new(
        c[0].clamp(0.0, 255.0) as u8,
        c[1].clamp(0.0, 255.0) as u8,
        c[2].clamp(0.0, 255.0) as u8,
        255,
    )
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

fn draw_base(
    d: &mut impl RaylibDraw,
    frame: &SceneFrame<'_>,
    state: &KnotState,
    out: &mut Vec<Vertex>,
    boundary: Rectangle,
    emission: bool,
) -> String {
    if boundary.width <= 0.0 || boundary.height <= 0.0 {
        return String::new();
    }
    let setting = |i| frame.setting(SceneId::OpenAiKnot, i);
    let time = state.time;
    let response = setting(control::RESPONSE);
    let beat = state.beat * setting(control::PULSE);
    let energy = state.energy * response;
    let atmosphere = setting(control::ATMOSPHERE);
    let polish = setting(control::POLISH);
    let hue = setting(control::HUE);
    let phase = state.phase;
    let rotation = setting(control::DRIFT)
        * (0.09 * (time * 0.16 + phase).sin() + 0.022 * beat * (time * 0.9).sin());
    let (sin, cos) = rotation.sin_cos();
    // Reserve room for the strongest local breathing at every control extreme.
    // Raising Size must never clip a lobe in portrait or a short preview panel.
    let expansion = 1.0 + 0.075 * response * state.bands.iter().copied().fold(0.0_f32, f32::max);
    let radius = (0.405 * setting(control::SIZE) * (1.0 + 0.10 * energy + 0.04 * beat))
        .min(0.45 / expansion);
    let scale = boundary.width.min(boundary.height) * radius;
    let center = Vector2::new(
        boundary.x + boundary.width * 0.5,
        boundary.y + boundary.height * 0.5,
    );
    let project = |p: [f32; 2]| {
        let p = breathe(p, &state.bands, response);
        Vector2::new(
            center.x + scale * (p[0] * cos - p[1] * sin),
            center.y + scale * (p[0] * sin + p[1] * cos),
        )
    };
    out.clear();
    // A low-frequency mesh rather than circles behind a logo: the light field
    // moves across the entire canvas and respects any export aspect ratio.
    let cols = 40;
    let rows = 28;
    let background = |x: usize, y: usize| {
        let u = x as f32 / cols as f32;
        let v = y as f32 / rows as f32;
        let px = (u - 0.5) * boundary.width / boundary.height;
        let py = v - 0.5;
        let wave = (px * 3.7 + py * 2.4 + time * 0.09 + phase).sin() * 0.12;
        let mist = (-((py - wave) * 4.0).powi(2) - (px * 1.1).powi(2)).exp() * atmosphere;
        let core = (-((px * 2.0).powi(2) + (py * 2.0).powi(2))).exp();
        let light = 0.12 + 0.10 * core + mist * (0.13 + 0.16 * energy);
        let col = draw::color_from_hsv(
            (263.0 + hue + 35.0 * (px + time * 0.018).sin()).rem_euclid(360.0),
            0.74,
            light.min(0.65),
        );
        (
            Vector2::new(
                boundary.x + u * boundary.width,
                boundary.y + v * boundary.height,
            ),
            col,
        )
    };
    if !emission {
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
        // The soft underside gives the object weight without a blur buffer or an
        // export-only path. Nested translucent contours are confined to the haze.
        for ring in (0..22).rev() {
            let r = scale * (0.95 + ring as f32 * 0.013);
            let alpha = ((1.0 - ring as f32 / 22.0) * 1.5 * atmosphere) as u8;
            d.draw_ellipse(
                center.x as i32,
                (center.y + scale * 0.27) as i32,
                r,
                r * 0.74,
                Color::new(93, 52, 208, alpha),
            );
        }
    }
    // Fixed segment order keeps all six receiving strands above their tucked
    // ends. The order never depends on amplitude or phase.
    // The blur buffer cannot resolve the full bevel mesh. Fewer samples there
    // preserve the light field while keeping the extra pass inexpensive.
    let segments = if emission { SEGMENTS / 2 } else { SEGMENTS };
    let mut order: Vec<_> = (0..segments).collect();
    order.sort_by(|a, b| {
        depth((*a as f32 + 0.5) / segments as f32)
            .total_cmp(&depth((*b as f32 + 0.5) / segments as f32))
    });
    let cross_steps = if emission { 4 } else { 12 };
    // Each vertex belongs to four adjacent quads. Shade it once, then retain
    // the original depth-sorted triangle order when submitting those quads.
    let stride = cross_steps + 1;
    let strand_size = (segments + 1) * stride;
    let mut mesh = Vec::with_capacity(STRANDS * strand_size);
    for strand in 0..STRANDS {
        let amplitude = state.bands[strand] * response;
        let vertex = |j: usize, k: usize| {
            let t = j as f32 / segments as f32;
            let p = centerline(strand, t);
            let a = centerline(strand, (t - 0.0003).max(0.0));
            let b = centerline(strand, (t + 0.0003).min(1.0));
            let len = (b[0] - a[0]).hypot(b[1] - a[1]).max(0.00001);
            let n = [-(b[1] - a[1]) / len, (b[0] - a[0]) / len];
            let across = -1.0 + 2.0 * k as f32 / cross_steps as f32;
            let crown = (1.0 - across * across).max(0.0).sqrt();
            let position = project([
                p[0] + n[0] * across * HALF_WIDTH,
                p[1] + n[1] * across * HALF_WIDTH,
            ]);
            let nx = (n[0] * cos - n[1] * sin) * across;
            let ny = (n[0] * sin + n[1] * cos) * across;
            let lambert = (-nx * 0.35 - ny * 0.45 + crown * 0.82).max(0.0);
            let spec = (-nx * 0.24 - ny * 0.34 + crown * 0.91).max(0.0).powi(24) * polish;
            let travel =
                (t * TAU * 1.7 - time * setting(control::FLOW) * 1.3 + strand as f32 * 0.8 + phase)
                    .sin();
            let beam =
                ((travel - 0.4) / 0.6).max(0.0).powi(3) * (0.18 + amplitude * 1.8 + beat * 0.4);
            let tint = (259.0 + hue + 54.0 * (t * PI + strand as f32 * 0.7 + time * 0.07).sin()
                - 40.0 * beam)
                .rem_euclid(360.0);
            let base = draw::color_from_hsv(tint, 0.42 + 0.24 * (1.0 - crown), 1.0);
            let tuck = (depth(t) * 5.0).clamp(0.0, 1.0);
            let light = (0.18 + 0.62 * lambert + 0.15 * amplitude) * (0.50 + 0.50 * tuck);
            let white = (spec * 0.48 + beam * (0.24 + 0.24 * polish)) * 255.0;
            let mut color = rgb([
                base.r as f32 * light + white,
                base.g as f32 * light + white,
                base.b as f32 * light + white,
            ]);
            if emission {
                // Threshold the lit material, not a duplicate full silhouette:
                // only travelling highlights and pearly edges scatter light.
                let luma =
                    color.r as f32 * 0.2126 + color.g as f32 * 0.7152 + color.b as f32 * 0.0722;
                let gain = ((luma - 95.0) / 160.0).clamp(0.0, 1.0);
                color = rgb([
                    color.r as f32 * gain,
                    color.g as f32 * gain,
                    color.b as f32 * gain,
                ]);
            }
            (position, color)
        };
        for j in 0..=segments {
            for k in 0..=cross_steps {
                mesh.push(vertex(j, k));
            }
        }
    }
    for segment in order {
        for strand in 0..STRANDS {
            let vertex = |j: usize, k: usize| mesh[strand * strand_size + j * stride + k];
            for k in 0..cross_steps {
                quad(
                    out,
                    vertex(segment, k),
                    vertex(segment + 1, k),
                    vertex(segment + 1, k + 1),
                    vertex(segment, k + 1),
                );
            }
        }
    }
    let triangles = out.len() / 3;
    draw::shaded_triangles(d, out);
    // Sparse motes use absolute time and seed phase; they never accumulate or
    // flash on a beat. The long diagonal drift echoes the aurora's direction.
    if !emission {
        for i in 0..42 {
            let h = i as f32;
            let u = (h * 0.618_034 + time * 0.003 + phase).fract();
            let v = (h * 0.381_966 - time * 0.006).rem_euclid(1.0);
            let a =
                ((0.35 + 0.65 * (h + time * 0.4).sin().abs()) * 36.0 * atmosphere).min(100.0) as u8;
            d.draw_circle_v(
                Vector2::new(
                    boundary.x + u * boundary.width,
                    boundary.y + v * boundary.height,
                ),
                boundary.height * 0.0012,
                Color::new(170, 181, 255, a),
            );
        }
    }
    format!("strands=6 segments={SEGMENTS} triangles={triangles} weave=fixed energy={:.4} beat={:.4} bands={} clock={:.3}",state.energy,state.beat,state.bands.iter().map(|b|format!("{b:.3}")).collect::<Vec<_>>().join(","),time)
}
