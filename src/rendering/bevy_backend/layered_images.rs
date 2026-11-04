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
    cube_array: std::sync::OnceLock<TextureView>,
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
            cube_array: Default::default(),
        })
    }
    pub fn texture(&self) -> &Texture {
        &self.texture
    }
    pub fn view(&self) -> &TextureView {
        &self.view
    }
    /// Alternate sampling descriptor over the same six faces (1402bd380).
    pub(super) fn cube_array_view(&self) -> &TextureView {
        self.cube_array.get_or_init(|| self.texture.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::CubeArray), ..Default::default()
        }))
    }
    pub fn payload_bytes(&self) -> u64 {
        self.payload_bytes
    }
}

/// Host lifetime for one acquired image (1400607f0 / 14005feb0). Authored
/// aliases share this source. All consumers lease its immutable GPU result;
/// compressed preparation bytes retire after the synchronous upload copy.
pub struct LayeredImageSource {
    name: String,
    info: BimageInfo,
    layers: u32,
    dimension: TextureViewDimension,
    payload: std::sync::Mutex<Option<Vec<u8>>>,
    image: std::sync::OnceLock<Result<std::sync::Arc<LayeredImage>, String>>,
    requests: std::sync::atomic::AtomicU64,
    uploads: std::sync::atomic::AtomicU64,
    uploaded_bytes: std::sync::atomic::AtomicU64,
    retired_bytes: std::sync::atomic::AtomicU64,
    signature: u32,
}
fn source_mode() -> (bool, bool) {
    static MODE: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| (
        std::env::var("DH2_DEBUG_SEPARATE_LAYERED_IMAGES").as_deref() == Ok("1"),
        std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1"),
    ))
}
impl LayeredImageSource {
    pub(crate) fn separate() -> bool { source_mode().0 }
    pub(crate) fn is_cube(&self) -> bool { self.dimension == TextureViewDimension::Cube }
    pub fn cube(name: String, chain: InlineCubeChain) -> Self {
        Self::new(name, chain.info, 6, chain.payload, TextureViewDimension::Cube)
    }
    pub fn array(name: String, chain: InlineArrayChain) -> Self {
        Self::new(name, chain.info, chain.layers, chain.payload, TextureViewDimension::D2Array)
    }
    fn new(name: String, info: BimageInfo, layers: u32, payload: Vec<u8>, dimension: TextureViewDimension) -> Self {
        let signature = if source_mode().1 {
            let mut crc = flate2::Crc::new(); crc.update(&payload); crc.sum()
        } else { 0 };
        Self { name, info, layers, dimension, payload: std::sync::Mutex::new(Some(payload)),
            image: Default::default(), requests: Default::default(), uploads: Default::default(),
            uploaded_bytes: Default::default(), retired_bytes: Default::default(), signature }
    }
    pub fn acquire(&self, device: &RenderDevice, queue: &RenderQueue) -> Result<std::sync::Arc<LayeredImage>, String> {
        use std::sync::atomic::Ordering::Relaxed;
        let (separate, trace) = source_mode();
        if trace { self.requests.fetch_add(1, Relaxed); }
        let upload = || {
            if self.dimension == TextureViewDimension::Cube && self.info.width != self.info.height {
                return Err("cube faces must be square".into());
            }
            let mut staging = self.payload.lock().unwrap();
            let payload = staging.as_ref().ok_or("layered image staging already consumed without a GPU owner")?;
            let image = std::sync::Arc::new(LayeredImage::new(device, queue, self.info,
                self.layers, payload, self.dimension)?);
            if trace {
                self.uploads.fetch_add(1, Relaxed);
                self.uploaded_bytes.fetch_add(payload.len() as u64, Relaxed);
                eprintln!("MEMORY layered image upload: path={} dimension={:?} extent={}x{} layers={} mips={} format={:?} bytes={} signature={:08x}",
                    self.name, self.dimension, self.info.width, self.info.height, self.layers,
                    self.info.mip_count, self.info.format, payload.len(), self.signature);
            }
            if !separate {
                let retired = staging.take().unwrap();
                if trace { self.retired_bytes.fetch_add(retired.capacity() as u64, Relaxed); }
                // The queue has copied the upload source; only GPU/frame leases remain.
                drop(retired);
            }
            Ok(image)
        };
        if separate { upload() } else { self.image.get_or_init(upload).clone() }
    }
}
impl Drop for LayeredImageSource {
    fn drop(&mut self) {
        if !source_mode().1 { return; }
        use std::sync::atomic::Ordering::Relaxed;
        eprintln!("MEMORY layered image release: path={} requests={} uploads={} uploaded_bytes={} retired_staging_bytes={} remaining_staging_bytes={}",
            self.name, self.requests.load(Relaxed), self.uploads.load(Relaxed), self.uploaded_bytes.load(Relaxed),
            self.retired_bytes.load(Relaxed), self.payload.get_mut().unwrap().as_ref().map_or(0, Vec::capacity));
    }
}

#[cfg(test)]
#[path = "layered_images/tests.rs"]
mod tests;
