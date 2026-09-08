//! Egui triangle and clipping submission into raylib’s normal 2D pass.

use std::collections::HashMap;

use egui::{epaint::Primitive, ClippedPrimitive, TextureId};
use raylib::prelude::*;

pub(crate) fn render(
    d: &mut RaylibDrawHandle,
    pixels_per_point: f32,
    primitives: &[ClippedPrimitive],
    texture_map: &HashMap<TextureId, Texture2D>,
) {
    d.rl_disable_depth_test();
    d.rl_disable_backface_culling();
    d.draw_blend_mode(BlendMode::BLEND_ALPHA_PREMULTIPLY, |mut d| {
        for primitive in primitives {
            match &primitive.primitive {
                Primitive::Mesh(mesh) => {
                    if let Some(texture) = texture_map.get(&mesh.texture_id) {
                        let vertices = &mesh.vertices;
                        let indices = &mesh.indices;
                        let clip = primitive.clip_rect * pixels_per_point;
                        let left = clip.min.x.floor().max(0.0) as i32;
                        let top = clip.min.y.floor().max(0.0) as i32;
                        let right = clip.max.x.ceil().min(d.get_screen_width() as f32) as i32;
                        let bottom = clip.max.y.ceil().min(d.get_screen_height() as f32) as i32;
                        if right <= left || bottom <= top {
                            continue;
                        }
                        d.draw_scissor_mode(left, top, right - left, bottom - top, |mut s| {
                            s.rl_set_texture(texture);
                            s.rl_draw(DrawMode::Triangles, |v| {
                                for indices_triple in indices.chunks_exact(3) {
                                    for index in indices_triple {
                                        let vertex = vertices[*index as usize];

                                        v.color4ub((
                                            vertex.color.r(),
                                            vertex.color.g(),
                                            vertex.color.b(),
                                            vertex.color.a(),
                                        ));
                                        v.texcoord2f(vertex.uv.x, vertex.uv.y);
                                        v.vertex2f(
                                            vertex.pos.x * pixels_per_point,
                                            vertex.pos.y * pixels_per_point,
                                        );
                                    }
                                }
                            });
                            s.rl_disable_texture();
                        });
                    }
                }
                Primitive::Callback(_) => (),
            };
        }
    });
    d.rl_enable_backface_culling();
}
