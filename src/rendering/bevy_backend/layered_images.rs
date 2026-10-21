//! Persistent GPU arrays and cubes from resident mip/layer records.
use crate::assets::texture::{BimageInfo, InlineArrayChain, InlineCubeChain};
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};

pub const SHADER_SOURCE: &str =
    include_str!("../../../assets/shaders/dh2_texture_array_sampling.wgsl");

pub struct LayeredImage {
    texture: Texture,
    view: TextureView,
    payload_bytes: u64,
}
impl LayeredImage {
    pub fn array(
        device: &RenderDevice,
        queue: &RenderQueue,
        chain: &InlineArrayChain,
    ) -> Result<Self, String> {
        Self::new(
            device,
            queue,
            chain.info,
            chain.layers,
            &chain.payload,
            TextureViewDimension::D2Array,
        )
    }
    pub fn cube(
        device: &RenderDevice,
        queue: &RenderQueue,
        chain: &InlineCubeChain,
    ) -> Result<Self, String> {
        if chain.info.width != chain.info.height {
            return Err("cube faces must be square".into());
        }
        Self::new(
            device,
            queue,
            chain.info,
            6,
            &chain.payload,
            TextureViewDimension::Cube,
        )
    }
    fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        info: BimageInfo,
        layers: u32,
        payload: &[u8],
        dimension: TextureViewDimension,
    ) -> Result<Self, String> {
        let limits = device.limits();
        if info.width == 0
            || info.height == 0
            || layers == 0
            || info.width.max(info.height) > limits.max_texture_dimension_2d
            || layers > limits.max_texture_array_layers
            || info.mip_count == 0
            || info.mip_count > 32 - info.width.max(info.height).leading_zeros()
        {
            return Err("texture-array dimensions exceed the actual GPU allocation limits".into());
        }
        let format = super::image_formats::gpu_image_format(info.format);
        if !device
            .wgpu_device()
            .features()
            .contains(format.required_features())
        {
            return Err(format!(
                "texture-array format {format:?} requires disabled GPU features"
            ));
        }
        let expected = (0..info.mip_count)
            .map(|level| {
                info.format
                    .mip_bytes((info.width >> level).max(1), (info.height >> level).max(1))
                    * layers as usize
            })
            .sum::<usize>();
        if expected != payload.len() {
            return Err("texture-array payload differs from its mip/layer allocation".into());
        }
        let texture = device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("resident image array"),
                size: Extent3d {
                    width: info.width,
                    height: info.height,
                    depth_or_array_layers: layers,
                },
                mip_level_count: info.mip_count,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
                view_formats: &[],
            },
            TextureDataOrder::MipMajor,
            payload,
        );
        let view = texture.create_view(&TextureViewDescriptor {
            label: Some("resident image array"),
            dimension: Some(dimension),
            ..Default::default()
        });
        Ok(Self {
            texture,
            view,
            payload_bytes: payload.len() as u64,
        })
    }
    pub fn texture(&self) -> &Texture {
        &self.texture
    }
    pub fn view(&self) -> &TextureView {
        &self.view
    }
    pub fn payload_bytes(&self) -> u64 {
        self.payload_bytes
    }
}

#[cfg(test)]
#[path = "layered_images/tests.rs"]
mod tests;
