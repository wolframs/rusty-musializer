//! Egui-owned texture allocation, patching and sampler updates.

use std::collections::HashMap;

use egui::{ImageData, TextureId, TexturesDelta};
use raylib::{
    error::{LoadTextureError, UpdateTextureError},
    prelude::*,
};

#[derive(Debug)]
pub enum HandleTexturesErrors {
    LoadTextureError(LoadTextureError),
    UpdateTextureError(UpdateTextureError),
}

impl std::fmt::Display for HandleTexturesErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LoadTextureError(error) => write!(f, "load: {error}"),
            Self::UpdateTextureError(error) => write!(f, "update: {error}"),
        }
    }
}

impl From<LoadTextureError> for HandleTexturesErrors {
    fn from(value: LoadTextureError) -> Self {
        HandleTexturesErrors::LoadTextureError(value)
    }
}

impl From<UpdateTextureError> for HandleTexturesErrors {
    fn from(value: UpdateTextureError) -> Self {
        HandleTexturesErrors::UpdateTextureError(value)
    }
}

fn map_texture_filter(filter: egui::TextureFilter) -> TextureFilter {
    match filter {
        egui::TextureFilter::Nearest => TextureFilter::TEXTURE_FILTER_POINT,
        egui::TextureFilter::Linear => TextureFilter::TEXTURE_FILTER_BILINEAR,
    }
}

fn map_texture_wrap_mode(wrap: egui::TextureWrapMode) -> TextureWrap {
    match wrap {
        egui::TextureWrapMode::ClampToEdge => TextureWrap::TEXTURE_WRAP_CLAMP,
        egui::TextureWrapMode::Repeat => TextureWrap::TEXTURE_WRAP_REPEAT,
        egui::TextureWrapMode::MirroredRepeat => TextureWrap::TEXTURE_WRAP_MIRROR_REPEAT,
    }
}

pub(crate) fn handle_textures(
    map: &mut HashMap<TextureId, Texture2D>,
    d: &mut RaylibDrawHandle,
    thread: &RaylibThread,
    textures_delta: &TexturesDelta,
) -> Result<(), HandleTexturesErrors> {
    for (id, image_delta) in textures_delta.set.iter() {
        let image_width = image_delta.image.width();
        let image_height = image_delta.image.height();

        let options = image_delta.options;

        match &image_delta.image {
            ImageData::Color(color_image) => {
                let bytes = color_image
                    .pixels
                    .iter()
                    .flat_map(|x| [x.r(), x.g(), x.b(), x.a()])
                    .collect::<Vec<u8>>();

                if let Some(texture) = map.get_mut(id) {
                    texture.set_texture_filter(thread, map_texture_filter(options.minification));
                    texture.set_texture_wrap(thread, map_texture_wrap_mode(options.wrap_mode));
                    if let Some(pos) = image_delta.pos {
                        texture.update_texture_rec(
                            Rectangle::new(
                                pos[0] as f32,
                                pos[1] as f32,
                                image_width as f32,
                                image_height as f32,
                            ),
                            &bytes,
                        )?;
                    } else {
                        if image_width as i32 == texture.width
                            && image_height as i32 == texture.height
                        {
                            texture.set_texture_filter(
                                thread,
                                map_texture_filter(options.minification),
                            );
                            texture
                                .set_texture_wrap(thread, map_texture_wrap_mode(options.wrap_mode));
                            texture.update_texture(&bytes)?;
                        } else {
                            let image = Image::gen_image_color(
                                image_width as i32,
                                image_height as i32,
                                Color::BLACK,
                            );
                            *texture = d.load_texture_from_image(thread, &image)?;
                            texture.set_texture_filter(
                                thread,
                                map_texture_filter(options.minification),
                            );
                            texture
                                .set_texture_wrap(thread, map_texture_wrap_mode(options.wrap_mode));
                            texture.update_texture(&bytes)?;
                        }
                    }
                } else {
                    let rl_image = Image::gen_image_color(
                        image_width as i32,
                        image_height as i32,
                        Color::BLACK,
                    );
                    let mut new_texture = d.load_texture_from_image(thread, &rl_image)?;
                    new_texture
                        .set_texture_filter(thread, map_texture_filter(options.minification));
                    new_texture.set_texture_wrap(thread, map_texture_wrap_mode(options.wrap_mode));
                    new_texture.update_texture(&bytes)?;

                    map.insert(*id, new_texture);
                }
            }
        }
    }

    Ok(())
}
