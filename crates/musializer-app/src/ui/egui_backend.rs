//! Egui's frame/input/texture bridge into the application's existing raylib window.
//!
//! Adapted from egui-raylib 0.1.3 (MIT, copyright 2026 Yoppez); see
//! `egui_backend/LICENSE`. We keep ownership of the integration because the app
//! shares input with its legacy panels and must release textures *after* drawing.
//! Call `begin_frame`, build widgets, `end_frame`, then `draw` last on the default
//! framebuffer, outside every shader/3D/texture/scissor scope. That guarantees the
//! incoming and outgoing raylib state is its ordinary 2D state. Never use this
//! overlay for scene exports. Drop the backend before closing the raylib window.
#[path = "egui_backend/input.rs"]
mod input;
#[path = "egui_backend/output.rs"]
mod output;
#[path = "egui_backend/renderer.rs"]
mod renderer;
#[path = "egui_backend/textures.rs"]
mod textures;

use egui::{ClippedPrimitive, Context, TextureId, TexturesDelta};
use raylib::prelude::*;
use std::collections::HashMap;

#[derive(Default)]
pub struct EguiBackend {
    ctx: Context,
    textures: HashMap<TextureId, Texture2D>,
    pixels_per_point: f32,
    textures_delta: TexturesDelta,
    primitives: Vec<ClippedPrimitive>,
}

impl EguiBackend {
    pub fn ctx(&self) -> &Context {
        &self.ctx
    }

    /// `scale` is logical raylib window units per egui point. Character input is
    /// consumed only when `keyboard_enabled`; the legacy editor owns it otherwise.
    pub fn begin_frame(&mut self, rl: &mut RaylibHandle, scale: f32, keyboard_enabled: bool) {
        self.pixels_per_point = if scale.is_finite() {
            scale.clamp(0.5, 4.0)
        } else {
            1.0
        };
        let input = input::gather_input(rl, self.pixels_per_point, keyboard_enabled);
        self.ctx.set_pixels_per_point(self.pixels_per_point);
        self.ctx.begin_pass(input);
    }

    pub fn end_frame(&mut self, rl: &mut RaylibHandle) {
        let output = self.ctx.end_pass();
        if self.wants_pointer_input() || self.wants_keyboard_input() {
            output::handle_platform_output(rl, output.platform_output);
        }
        self.pixels_per_point = output.pixels_per_point;
        self.primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.textures_delta.append(output.textures_delta);
    }

    pub fn wants_pointer_input(&self) -> bool {
        self.ctx.egui_wants_pointer_input()
    }
    pub fn wants_keyboard_input(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }

    /// Draw last on the main framebuffer. No caller-owned texture is accepted;
    /// scene render targets remain wholly owned by the existing scene host.
    pub fn draw(&mut self, d: &mut RaylibDrawHandle, thread: &RaylibThread) -> Result<(), String> {
        // Texture uploads can change GL bindings behind an already queued raylib
        // batch. Submit the legacy UI before uploading or replacing any atlas.
        flush_batch();
        textures::handle_textures(&mut self.textures, d, thread, &self.textures_delta)
            .map_err(|err| format!("egui texture upload: {err}"))?;
        renderer::render(d, self.pixels_per_point, &self.primitives, &self.textures);
        flush_batch();
        // Egui can paint a texture in the same frame that it asks us to release it.
        for id in self.textures_delta.free.drain(..) {
            self.textures.remove(&id);
        }
        self.textures_delta.set.clear();
        self.primitives.clear();
        Ok(())
    }
}

fn flush_batch() {
    // SAFETY: called only by draw with a live RaylibDrawHandle, outside any rlgl
    // immediate-mode scope. This submits the active batch and retains no pointer.
    unsafe { raylib_sys::rlDrawRenderBatchActive() }
}
