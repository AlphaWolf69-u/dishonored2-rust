//! Persistent per-view fog attachments. Formats and rounded extents follow
//! 307530, 30a310 and 36f8a0's dynamic-canvas owner; resize replaces the set.
use bevy::{
    math::UVec2,
    render::{render_resource::*, renderer::RenderDevice},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FogExtents {
    pub full: UVec2,
    pub world: UVec2,
    pub depth: UVec2,
    pub shadow: UVec2,
    pub canvas: UVec2,
}

impl FogExtents {
    pub fn new(full: UVec2, downsample: i32) -> Result<Self, String> {
        if full.min_element() == 0 || full.max_element() > i32::MAX as u32 - 15 {
            return Err(format!("invalid fog view extent {full:?}"));
        }
        // Keep the executable's shift semantics. Quality 0 is explicitly
        // described as unsupported by its registration, not silently clamped.
        if !(1..=2).contains(&downsample) {
            return Err(format!("unsupported r_downsampleQuality {downsample}"));
        }
        let shift = downsample as u32;
        // 36f8a0 starts at floor-reduced dimensions, then caps the longer
        // canvas axis at 640 using the full view's aspect ratio and floorf.
        let downsample = UVec2::from_array(crate::rendering::luminance::downsample_extent(
            full.to_array(),
            downsample,
        )?);
        let mut canvas = downsample;
        let aspect = full.x as f32 / full.y as f32;
        if aspect < 1. && canvas.y > 640 {
            canvas.y = 640;
            canvas.x = (aspect * 640.).floor() as u32;
        } else if aspect >= 1. && canvas.x > 640 {
            canvas.x = 640;
            canvas.y = (640. / aspect).floor() as u32;
        }
        if canvas.min_element() == 0 {
            return Err(format!("fog canvas collapses at view extent {full:?}"));
        }
        Ok(Self {
            full,
            depth: downsample,
            world: ((full + UVec2::ONE) >> shift).max(UVec2::splat(4)),
            shadow: ((full + UVec2::splat(3)) >> (shift + 1)).max(UVec2::splat(2)),
            canvas,
        })
    }
}

pub struct FogAttachment {
    pub texture: Texture,
    pub view: TextureView,
}

impl FogAttachment {
    pub(super) fn new(
        device: &RenderDevice,
        label: &'static str,
        size: UVec2,
        format: TextureFormat,
    ) -> Self {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        Self { texture, view }
    }
}

pub struct FogTargets {
    pub extents: FogExtents,
    pub interior: FogAttachment,
    pub depth: FogAttachment,
    pub canvas: FogAttachment,
    pub shadow: FogAttachment,
    pub world: FogAttachment,
    pub filtered: FogAttachment,
}

impl FogTargets {
    pub fn new(device: &RenderDevice, extents: FogExtents) -> Self {
        Self::allocate(device, extents, None)
    }

    pub fn shared(
        device: &RenderDevice,
        extents: FogExtents,
        world: &bevy::render::texture::GpuImage,
        canvas: &bevy::render::texture::GpuImage,
    ) -> Result<Self, String> {
        for (image, size, format) in [
            (world, extents.world, TextureFormat::Rgba16Float),
            (canvas, extents.canvas, TextureFormat::R8Unorm),
        ] {
            if image.texture.size().width != size.x
                || image.texture.size().height != size.y
                || image.texture.format() != format
                || image.texture.sample_count() != 1
            {
                return Err("shared fog outputs await matching camera dimensions/format".into());
            }
        }
        let attachment = |image: &bevy::render::texture::GpuImage| FogAttachment {
            texture: image.texture.clone(),
            view: image.texture_view.clone(),
        };
        Ok(Self::allocate(
            device,
            extents,
            Some((attachment(world), attachment(canvas))),
        ))
    }

    fn allocate(
        device: &RenderDevice,
        extents: FogExtents,
        outputs: Option<(FogAttachment, FogAttachment)>,
    ) -> Self {
        let (world, canvas) = outputs.unwrap_or_else(|| {
            (
                FogAttachment::new(
                    device,
                    "fog premultiplied radiance",
                    extents.world,
                    TextureFormat::Rgba16Float,
                ),
                FogAttachment::new(
                    device,
                    "fog turbulence canvas",
                    extents.canvas,
                    TextureFormat::R8Unorm,
                ),
            )
        });
        Self {
            extents,
            depth: FogAttachment::new(
                device,
                "fog linear depth min/max",
                extents.depth,
                TextureFormat::Rg16Float,
            ),
            interior: FogAttachment::new(
                device,
                "fog interior depth",
                extents.world,
                TextureFormat::R16Float,
            ),
            canvas,
            shadow: FogAttachment::new(
                device,
                "fog shadow march",
                extents.shadow,
                TextureFormat::R8Unorm,
            ),
            world,
            filtered: FogAttachment::new(
                device,
                "fog filter scratch",
                extents.world,
                TextureFormat::Rgba16Float,
            ),
        }
    }

    /// Retain allocations across ordinary frames; view resizing or a changed
    /// reduction setting is the only reason to replace these attachments.
    pub fn resize(&mut self, device: &RenderDevice, extents: FogExtents) -> bool {
        if self.extents == extents {
            return false;
        }
        *self = Self::new(device, extents);
        true
    }
}
