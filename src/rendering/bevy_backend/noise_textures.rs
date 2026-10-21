//! Renderer-owned procedural textures from 1402f6450. No archive extraction.
use bevy::{
    prelude::Resource,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

/// Shared by all ocean users; initialize once for the renderer, not per frame.
#[derive(Resource)]
pub struct NoiseTextures {
    permutation: Texture,
    gradients: Texture,
    permutation_view: TextureView,
    gradients_view: TextureView,
    point_wrap: Sampler,
}

impl NoiseTextures {
    pub fn new(device: &RenderDevice, queue: &RenderQueue) -> Self {
        let pixels = crate::rendering::noise_textures::generate_noise_textures();
        let upload = |label, height, format, bytes: &[u8]| {
            let size = Extent3d {
                width: 256,
                height,
                depth_or_array_layers: 1,
            };
            let texture = device.create_texture(&TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                bytes,
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256 * 4),
                    rows_per_image: Some(height),
                },
                size,
            );
            texture
        };
        // Packed 0x54a -> DXGI28, 0x50a -> DXGI31. Signed gradients must not
        // pass through a colour/UNORM image upload or an sRGB conversion.
        let permutation = upload(
            "noise permutation",
            256,
            TextureFormat::Rgba8Unorm,
            &pixels.permutation,
        );
        let gradients = upload(
            "noise gradients",
            1,
            TextureFormat::Rgba8Snorm,
            &pixels.gradients,
        );
        let permutation_view = permutation.create_view(&Default::default());
        let gradients_view = gradients.create_view(&Default::default());
        let point_wrap = device.create_sampler(&SamplerDescriptor {
            label: Some("noise pointWrap"),
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::Repeat,
            address_mode_w: AddressMode::Repeat,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            mipmap_filter: MipmapFilterMode::Nearest,
            lod_max_clamp: 13.0,
            ..Default::default()
        });
        Self {
            permutation,
            gradients,
            permutation_view,
            gradients_view,
            point_wrap,
        }
    }
    pub fn permutation(&self) -> &TextureView {
        &self.permutation_view
    }
    pub fn gradients(&self) -> &TextureView {
        &self.gradients_view
    }
    pub fn point_wrap(&self) -> &Sampler {
        &self.point_wrap
    }
    /// Owned texture payload, excluding GPU allocation alignment and driver state.
    pub fn texture_bytes(&self) -> u64 {
        u64::from(self.permutation.width()) * u64::from(self.permutation.height()) * 4
            + u64::from(self.gradients.width()) * u64::from(self.gradients.height()) * 4
    }
}

#[cfg(test)]
#[path="noise_textures/tests.rs"]
mod tests;
