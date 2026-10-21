//! The palette lock and the finish, as GPU passes over a drawn frame (MV9-a/b).
//!
//! Both work the same way: copy what the active framebuffer holds inside the
//! scene's rectangle ([`HeldFrame`] — one blit, the same copy a transition
//! holds), then draw that copy back over the rectangle through a shader. The
//! finish adds two blurred bloom buffers first, built with
//! [`crate::offscreen::Canvas`] and the caption halo's own blur shader.
//!
//! The data and its CPU reference live in `musializer-core::project::look`;
//! this module only moves pixels. Every pass writes alpha 1, because the
//! framebuffer's alpha reaches a saved still.
//!
//! Like the other offscreen passes, both entry points must run with **no
//! scissor active** (`halo.rs` has the reasoning).

use musializer_core::project::look::{self, Finish, Grade, MAX_STOPS};
use raylib::prelude::{
    BlendMode, Color, RaylibBlendModeExt, RaylibDrawHandle, RaylibShader, RaylibShaderModeExt,
    Rectangle, Vector2, Vector3, Vector4,
};
use raylib::shaders::Shader;
use raylib::{RaylibHandle, RaylibThread};

use crate::draw;
use crate::offscreen::Canvas;
use crate::transition::HeldFrame;

const GRADE_SOURCE: &str = include_str!("../../../resources/shaders/glsl330/post_grade.fs");
const BRIGHT_SOURCE: &str = include_str!("../../../resources/shaders/glsl330/post_bright.fs");
const FINISH_SOURCE: &str = include_str!("../../../resources/shaders/glsl330/post_finish.fs");
const BLUR_SOURCE: &str = include_str!("../../../resources/shaders/glsl330/halo_blur.fs");

/// The blur shader's comfortable sigma, in texels of the buffer it reads
/// (`halo.rs`'s `MAX_SIGMA_TEXELS`, same 17 taps).
const SIGMA_TEXELS: f32 = 2.5;

/// The wide bloom's radius as a multiple of the narrow one's.
const WIDE_BLOOM: f32 = 3.6;

/// What the last frame did, for the `look:` report line. A grade or a finish
/// that silently failed is the same picture as a project that asked for none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Outcome {
    #[default]
    Idle,
    Drew,
    /// A shader or a buffer was refused; the frame went out unfinished.
    Unavailable,
}

impl Outcome {
    fn name(self) -> &'static str {
        match self {
            Self::Idle => "off",
            Self::Drew => "drew",
            Self::Unavailable => "UNAVAILABLE",
        }
    }
}

struct Bloom {
    canvas: Canvas,
    scratch: Canvas,
}

/// The post passes' GPU resources: four shaders, one frame copy, two bloom
/// buffer pairs. Owned once by the scene renderer.
pub struct Post {
    grade_shader: Option<Shader>,
    bright_shader: Option<Shader>,
    finish_shader: Option<Shader>,
    blur_shader: Option<Shader>,
    grab: HeldFrame,
    narrow: Bloom,
    wide: Bloom,
    pub last_grade: Outcome,
    pub last_finish: Outcome,
    grade_frames: u64,
    finish_frames: u64,
}

fn compile(rl: &mut RaylibHandle, thread: &RaylibThread, source: &str) -> Option<Shader> {
    // raylib answers a failed compile with the default shader rather than an
    // error, so the check is `is_shader_valid` on what came back.
    let shader = rl.load_shader_from_memory(thread, None, Some(source));
    shader.is_shader_valid().then_some(shader)
}

impl Post {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Self {
        Self {
            grade_shader: compile(rl, thread, GRADE_SOURCE),
            bright_shader: compile(rl, thread, BRIGHT_SOURCE),
            finish_shader: compile(rl, thread, FINISH_SOURCE),
            blur_shader: compile(rl, thread, BLUR_SOURCE),
            grab: HeldFrame::new(),
            narrow: Bloom {
                canvas: Canvas::new(),
                scratch: Canvas::new(),
            },
            wide: Bloom {
                canvas: Canvas::new(),
                scratch: Canvas::new(),
            },
            last_grade: Outcome::Idle,
            last_finish: Outcome::Idle,
            grade_frames: 0,
            finish_frames: 0,
        }
    }

    /// One line for the report: which shaders compiled, what the last frame
    /// did, and how many frames each pass has drawn.
    #[must_use]
    pub fn describe(&self) -> String {
        let compiled = |shader: &Option<Shader>| if shader.is_some() { "ok" } else { "REFUSED" };
        format!(
            "grade={} ({} frames, shader {}), finish={} ({} frames, shaders {}/{}/{})",
            self.last_grade.name(),
            self.grade_frames,
            compiled(&self.grade_shader),
            self.last_finish.name(),
            self.finish_frames,
            compiled(&self.finish_shader),
            compiled(&self.bright_shader),
            compiled(&self.blur_shader),
        )
    }

    /// Marks a frame on which neither pass was asked for.
    pub fn idle(&mut self) {
        self.last_grade = Outcome::Idle;
        self.last_finish = Outcome::Idle;
    }

    /// Holds what the scene just drew inside `boundary` to `grade`.
    pub fn grade(&mut self, d: &mut RaylibDrawHandle<'_>, boundary: Rectangle, grade: &Grade) {
        self.last_grade = if self.grade_inner(d, boundary, grade) {
            self.grade_frames = self.grade_frames.saturating_add(1);
            Outcome::Drew
        } else {
            Outcome::Unavailable
        };
    }

    fn grade_inner(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        boundary: Rectangle,
        grade: &Grade,
    ) -> bool {
        if self.grade_shader.is_none() || !self.grab.capture(d, boundary, 0.0) {
            return false;
        }
        let (Some(texture), Some(shader)) = (self.grab.texture(), self.grade_shader.as_mut())
        else {
            return false;
        };
        let mut stops = [Vector3::zero(); MAX_STOPS];
        for (slot, stop) in stops.iter_mut().zip(grade.stops()) {
            let [r, g, b] = look::rgb(*stop);
            *slot = Vector3::new(r, g, b);
        }
        let location = shader.get_shader_location("stops");
        shader.set_shader_value_v(location, &stops);
        set(shader, "stopCount", f32::from(grade.stop_count));
        set(shader, "map", grade.map);
        set(shader, "shiftFrom", grade.shift_from);
        set(shader, "shiftBy", grade.shift_by);
        set(shader, "shiftWidth", grade.shift_width.max(1.0));
        let source = region_source(&self.grab, texture);
        d.draw_blend_mode(BlendMode::BLEND_ALPHA, |mut blend| {
            blend.draw_shader_mode(shader, |mut pass| {
                draw::draw_texture_pro(
                    &mut pass,
                    texture,
                    source,
                    boundary,
                    Vector2::zero(),
                    0.0,
                    Color::WHITE,
                );
            });
        });
        true
    }

    /// Finishes everything drawn inside `boundary` this frame.
    pub fn finish(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        boundary: Rectangle,
        finish: &Finish,
        time_seconds: f64,
    ) {
        self.last_finish = if self.finish_inner(d, boundary, finish, time_seconds) {
            self.finish_frames = self.finish_frames.saturating_add(1);
            Outcome::Drew
        } else {
            Outcome::Unavailable
        };
    }

    fn finish_inner(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        boundary: Rectangle,
        finish: &Finish,
        time_seconds: f64,
    ) -> bool {
        if self.finish_shader.is_none() || !self.grab.capture(d, boundary, time_seconds) {
            return false;
        }
        let Some(frame) = self.grab.texture() else {
            return false;
        };
        let source = region_source(&self.grab, frame);
        let short = boundary.width.min(boundary.height).max(1.0);
        let wants_bloom = finish.bloom > 0.0;
        if wants_bloom {
            let radius = finish.bloom_radius * short;
            let built = self.build_bloom(
                d,
                frame,
                source,
                boundary,
                finish.bloom_threshold,
                radius,
                false,
            ) && self.build_bloom(
                d,
                frame,
                source,
                boundary,
                finish.bloom_threshold,
                radius * WIDE_BLOOM,
                true,
            );
            if !built {
                return false;
            }
        }
        let (narrow, wide) = (self.narrow.canvas.texture(), self.wide.canvas.texture());
        let Some(shader) = self.finish_shader.as_mut() else {
            return false;
        };
        let region = self.grab.region();
        let location = shader.get_shader_location("region");
        shader.set_shader_value(
            location,
            Vector4::new(region[0], region[1], region[2], region[3]),
        );
        let location = shader.get_shader_location("size");
        shader.set_shader_value(location, Vector2::new(boundary.width, boundary.height));
        set(
            shader,
            "bloom",
            if wants_bloom { finish.bloom } else { 0.0 },
        );
        set(shader, "knee", finish.knee());
        set(shader, "tone", finish.tone);
        let [r, g, b] = look::rgb(finish.shadow_rgb);
        let location = shader.get_shader_location("shadowTone");
        shader.set_shader_value(location, Vector3::new(r, g, b));
        let [r, g, b] = look::rgb(finish.highlight_rgb);
        let location = shader.get_shader_location("highlightTone");
        shader.set_shader_value(location, Vector3::new(r, g, b));
        set(shader, "vignette", finish.vignette);
        set(shader, "grain", finish.grain);
        // Grain changes 24 times a second of *track time*, whatever the frame
        // rate: resampled per frame it crawls at 60 fps and strobes at 24.
        set(
            shader,
            "tick",
            (time_seconds.max(0.0) * 24.0).floor() as f32,
        );
        let narrow_location = shader.get_shader_location("texture1");
        let wide_location = shader.get_shader_location("texture2");
        let program = shader.id;
        d.draw_blend_mode(BlendMode::BLEND_ALPHA, |mut blend| {
            blend.draw_shader_mode(shader, |mut pass| {
                // Samplers are registered *inside* the shader mode: entering it
                // flushes the batch, and a flush forgets every registered unit
                // (`rlgl.h`, "Reset active texture units for next batch").
                if wants_bloom {
                    if let (Some(narrow), Some(wide)) = (narrow, wide) {
                        bind_sampler(&mut pass, program, narrow_location, narrow);
                        bind_sampler(&mut pass, program, wide_location, wide);
                    }
                }
                draw::draw_texture_pro(
                    &mut pass,
                    frame,
                    source,
                    boundary,
                    Vector2::zero(),
                    0.0,
                    Color::WHITE,
                );
            });
        });
        true
    }

    /// Bright-passes the frame into a small buffer and blurs it in place.
    #[allow(clippy::too_many_arguments, reason = "one pass of a fixed pipeline")]
    fn build_bloom(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: raylib_sys::Texture2D,
        source: Rectangle,
        boundary: Rectangle,
        threshold: f32,
        radius_px: f32,
        wide: bool,
    ) -> bool {
        // Downsample until the blur's sigma fits its taps; never below 16
        // texels across, never finer than a quarter.
        let factor = (radius_px / SIGMA_TEXELS).max(4.0);
        let width = ((boundary.width / factor).round() as i32).max(16);
        let height = ((boundary.height / factor).round() as i32).max(16);
        let sigma = (radius_px / (boundary.width / width as f32)).clamp(0.5, SIGMA_TEXELS);
        let (Some(bright), Some(blur)) = (self.bright_shader.as_mut(), self.blur_shader.as_mut())
        else {
            return false;
        };
        let bloom = if wide {
            &mut self.wide
        } else {
            &mut self.narrow
        };
        set(bright, "threshold", threshold);
        let target = Rectangle::new(0.0, 0.0, width as f32, height as f32);
        let drew = bloom.canvas.draw_into(d, width, height, Color::BLACK, |d| {
            d.draw_shader_mode(bright, |mut pass| {
                draw::draw_texture_pro(
                    &mut pass,
                    frame,
                    source,
                    target,
                    Vector2::zero(),
                    0.0,
                    Color::WHITE,
                );
            });
        });
        if !drew {
            return false;
        }
        let flipped = Rectangle::new(0.0, 0.0, width as f32, -(height as f32));
        let direction_location = blur.get_shader_location("direction");
        let sigma_location = blur.get_shader_location("sigmaTexels");
        blur.set_shader_value(sigma_location, sigma);
        // Horizontal into the scratch, vertical back.
        let Some(first) = bloom.canvas.texture() else {
            return false;
        };
        blur.set_shader_value(direction_location, Vector2::new(1.0 / width as f32, 0.0));
        if !bloom
            .scratch
            .draw_into(d, width, height, Color::BLACK, |d| {
                d.draw_shader_mode(blur, |mut pass| {
                    draw::draw_texture_pro(
                        &mut pass,
                        first,
                        flipped,
                        target,
                        Vector2::zero(),
                        0.0,
                        Color::WHITE,
                    );
                });
            })
        {
            return false;
        }
        let Some(second) = bloom.scratch.texture() else {
            return false;
        };
        blur.set_shader_value(direction_location, Vector2::new(0.0, 1.0 / height as f32));
        bloom.canvas.draw_into(d, width, height, Color::BLACK, |d| {
            d.draw_shader_mode(blur, |mut pass| {
                draw::draw_texture_pro(
                    &mut pass,
                    second,
                    flipped,
                    target,
                    Vector2::zero(),
                    0.0,
                    Color::WHITE,
                );
            });
        })
    }
}

fn set(shader: &mut Shader, name: &str, value: f32) {
    let location = shader.get_shader_location(name);
    shader.set_shader_value(location, value);
}

/// The scene's part of a [`HeldFrame`], flipped, because a render texture's
/// rows run bottom-up.
fn region_source(grab: &HeldFrame, texture: raylib_sys::Texture2D) -> Rectangle {
    let region = grab.region();
    let (width, height) = (texture.width as f32, texture.height as f32);
    Rectangle::new(
        region[0] * width,
        region[1] * height,
        region[2] * width,
        -(region[3] * height),
    )
}

/// Registers a second texture for the next draw of the active shader.
///
/// Must be called inside the shader mode, after it began: see the comment at
/// the call site.
fn bind_sampler<D>(_d: &mut D, program: u32, location: i32, texture: raylib_sys::Texture2D) {
    if location < 0 {
        return;
    }
    // SAFETY: `SetShaderValueTexture`'s two calls, by value: `rlEnableShader`
    // makes `program` current so the sampler uniform lands on it, and
    // `rlSetUniformSampler` records the texture id in rlgl's active-unit table
    // for the batch the enclosing shader mode will draw (`rcore.c:1403-1411`).
    // `program` is a live shader this module owns; the texture is owned by a
    // `Canvas` that outlives the draw call it is bound for. No pointer crosses.
    unsafe {
        raylib_sys::rlEnableShader(program);
        raylib_sys::rlSetUniformSampler(location, texture.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_name_themselves_for_the_report() {
        assert_eq!(Outcome::Idle.name(), "off");
        assert_eq!(Outcome::Drew.name(), "drew");
        assert_eq!(Outcome::Unavailable.name(), "UNAVAILABLE");
    }
}
