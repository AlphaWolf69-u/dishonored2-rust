//! Transport the existing decoded image formats without colour conversion.
use bevy::render::render_resource::TextureFormat;

pub fn gpu_image_format(format: crate::assets::texture::ImageFormat) -> TextureFormat {
    use crate::assets::texture::ImageFormat::*;
    match format {
        Rgba8 => TextureFormat::Rgba8Unorm,
        Rgba8Srgb => TextureFormat::Rgba8UnormSrgb,
        Rg8 => TextureFormat::Rg8Unorm,
        R16Float => TextureFormat::R16Float,
        R16Unorm => TextureFormat::R16Unorm,
        Rg32Float => TextureFormat::Rg32Float,
        Bgra8 => TextureFormat::Bgra8Unorm,
        Bc1 => TextureFormat::Bc1RgbaUnorm,
        Bc1Srgb => TextureFormat::Bc1RgbaUnormSrgb,
        Bc3 => TextureFormat::Bc3RgbaUnorm,
        Bc3Srgb => TextureFormat::Bc3RgbaUnormSrgb,
        Bc4 => TextureFormat::Bc4RUnorm,
        Bc4Snorm => TextureFormat::Bc4RSnorm,
        Bc5 => TextureFormat::Bc5RgUnorm,
        Bc5Snorm => TextureFormat::Bc5RgSnorm,
        Bc6Unsigned => TextureFormat::Bc6hRgbUfloat,
        Bc6Signed => TextureFormat::Bc6hRgbFloat,
        Bc7 => TextureFormat::Bc7RgbaUnorm,
        Bc7Srgb => TextureFormat::Bc7RgbaUnormSrgb,
    }
}
