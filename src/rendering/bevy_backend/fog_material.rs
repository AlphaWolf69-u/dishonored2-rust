//! Shared gameplay-camera inputs for transparent material fog. Image handles
//! persist across frames; the render owner fills them before transparent draws.
use super::{
    fog::{FogFrame, FogParameters, FogView},
    fog_targets::FogExtents,
};
use bevy::{
    prelude::*,
    render::{render_resource::ShaderType, storage::ShaderBuffer},
};

const MATERIAL_FOG_SHADER: Handle<Shader> =
    bevy::asset::uuid_handle!("e25eaa98-661b-4d83-87c5-11e061079884");
const MATERIAL_SAMPLERS_SHADER: Handle<Shader> =
    bevy::asset::uuid_handle!("0893b7e4-b921-4c70-867c-39492ee81954");
pub fn register_shaders(app: &mut App) {
    bevy::asset::load_internal_asset!(
        app,
        MATERIAL_SAMPLERS_SHADER,
        "../../../assets/shaders/dh2_material_samplers.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        MATERIAL_FOG_SHADER,
        "../../../assets/shaders/dh2_fog_material.wgsl",
        |source: &str, path| Shader::from_wgsl(
            format!(
                "#define_import_path dh2_material_fog\n{}\n{}\n{source}",
                include_str!("../../../assets/shaders/dh2_fog_math.wgsl"),
                include_str!("../../../assets/shaders/dh2_fog_alpha.wgsl")
            ),
            path
        )
    );
}

#[derive(Clone, Debug)]
pub struct FogMaterialBindings {
    pub parameters: Handle<ShaderBuffer>,
    pub objects: Handle<ShaderBuffer>,
    pub world: Option<Handle<Image>>,
    pub canvas: Option<Handle<Image>>,
    pub curve: Option<Handle<Image>>,
    pub opaque_depth: Option<Handle<Image>>,
}
impl FogMaterialBindings {
    /// Valid disabled transport for renderers/tests without an atmosphere.
    pub fn disabled(images: &mut Assets<Image>, buffers: &mut Assets<ShaderBuffer>) -> Self {
        // AsBindGroup's absent-image fallback is a colour texture even for a
        // depth binding. Keep a real depth view while readiness stays zero.
        FogMaterialResources::new(images, buffers).bindings()
    }
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct FogMaterialParameters {
    /// Ready, near clip, game-camera height, world-tag table offset (-1 off).
    /// Render submission owns
    /// readiness so an unavailable/new camera never samples stale fog targets.
    pub control: Vec4,
    pub world: FogParameters,
    pub world_type: Vec4,
    pub unproject: Vec4,
}
impl FogMaterialParameters {
    pub fn disabled() -> Self {
        Self {
            control: Vec4::ZERO,
            world: FogParameters::new(Default::default(), Default::default()),
            world_type: Vec4::ZERO,
            unproject: Vec4::ZERO,
        }
    }
    pub fn ready(frame: &FogFrame, view: FogView) -> Self {
        Self {
            control: Vec4::new(1., frame.settings.near_clip, frame.eye.z, -1.),
            world: frame.parameters,
            world_type: crate::rendering::fog::world::WorldFog::alpha_world_type(
                frame.parameters.flags.x,
            ),
            unproject: view.unproject,
        }
    }
}

/// Host resource transport, distinct from authored fog defaults. The 1x1
/// attachments are initially disabled, then resized from the actual camera.
/// There is no CPU pixel upload or per-material copy of these frame targets.
#[derive(Resource, Clone, bevy::render::extract_resource::ExtractResource)]
pub struct FogMaterialResources {
    pub parameters: Handle<ShaderBuffer>,
    pub objects: Handle<ShaderBuffer>,
    pub object_frame: Option<std::sync::Arc<super::fog_objects::FogObjectFrame>>,
    pub world: Handle<Image>,
    pub canvas: Handle<Image>,
    pub opaque_depth: Handle<Image>,
    /// Three retained fog-curve layers shared with the atmosphere owner.
    pub curve: Handle<Image>,
    extents: Option<FogExtents>,
    bindings_dirty: bool,
}
impl FogMaterialResources {
    pub fn bindings(&self) -> FogMaterialBindings {
        FogMaterialBindings {
            parameters: self.parameters.clone(),
            objects: self.objects.clone(),
            world: Some(self.world.clone()),
            canvas: Some(self.canvas.clone()),
            curve: Some(self.curve.clone()),
            opaque_depth: Some(self.opaque_depth.clone()),
        }
    }
    /// Keep a resize pending across a failed frame publication. Only successful
    /// publication acknowledges it, so recovered materials cannot retain old views.
    pub fn publish_bindings(&mut self) -> Option<FogMaterialBindings> {
        std::mem::take(&mut self.bindings_dirty).then(|| self.bindings())
    }
    pub fn new(images: &mut Assets<Image>, buffers: &mut Assets<ShaderBuffer>) -> Self {
        use bevy::{
            image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
            render::render_resource::TextureFormat,
        };
        // new_target_texture also creates CPU zero pixels. These attachments
        // are produced on the GPU, including the non-uploadable depth target.
        let target = |format| {
            let mut image = Image::new_target_texture(1, 1, format, None);
            image.data = None;
            // Every attachment is regenerated for the current camera before
            // sampling. A resize must not copy the previous frame's contents.
            image.copy_on_resize = false;
            image
        };
        let world = images.add(target(TextureFormat::Rgba16Float));
        let mut curve = Image::new_fill(
            bevy::render::render_resource::Extent3d {
                width: 4096,
                height: 1,
                depth_or_array_layers: 3,
            },
            bevy::render::render_resource::TextureDimension::D2,
            &[0x00, 0x3c],
            TextureFormat::R16Float,
            bevy::asset::RenderAssetUsages::default(),
        );
        curve.texture_descriptor.usage |=
            bevy::render::render_resource::TextureUsages::RENDER_ATTACHMENT
                | bevy::render::render_resource::TextureUsages::COPY_SRC;
        curve.texture_view_descriptor =
            Some(bevy::render::render_resource::TextureViewDescriptor {
                dimension: Some(bevy::render::render_resource::TextureViewDimension::D2Array),
                ..default()
            });
        let mut canvas = target(TextureFormat::R8Unorm);
        // The actual bilinearSampler wraps; fog's precomputed colour/LUT/depth
        // use the separate linearClamp sampler supplied by the material.
        canvas.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            address_mode_w: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Nearest,
            ..Default::default()
        });
        Self {
            parameters: buffers.add(ShaderBuffer::from(FogMaterialParameters::disabled())),
            objects: buffers.add(ShaderBuffer::with_size(
                super::fog_objects::FogObjectParameters::SIZE,
                // Keep the small main-world descriptor for later growth.
                // Pixel/parameter contents are still produced only on the GPU.
                bevy::asset::RenderAssetUsages::default(),
            )),
            object_frame: None,
            world,
            canvas: images.add(canvas),
            opaque_depth: images.add(target(TextureFormat::Depth32Float)),
            curve: images.add(curve),
            extents: None,
            bindings_dirty: true,
        }
    }

    /// The caller invalidates material bindings only when this returns true.
    /// Floating fog/exposure changes update the shared buffer, never materials.
    pub fn resize(
        &mut self,
        images: &mut Assets<Image>,
        extents: FogExtents,
    ) -> Result<bool, String> {
        if self.extents == Some(extents) {
            return Ok(false);
        }
        let mut resized = false;
        for (handle, size) in [
            (&self.world, extents.world),
            (&self.canvas, extents.canvas),
            (&self.opaque_depth, extents.full),
        ] {
            let image = images
                .get(handle)
                .ok_or("shared fog attachment asset missing")?;
            if image.size() == size {
                continue;
            }
            let mut image = images
                .get_mut(handle)
                .ok_or("shared fog attachment asset missing")?;
            image.resize(bevy::render::render_resource::Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            });
            resized = true;
        }
        self.extents = Some(extents);
        self.bindings_dirty |= resized;
        Ok(resized)
    }
    pub fn extents(&self) -> Option<FogExtents> {
        self.extents
    }

    pub fn publish_objects(
        &mut self,
        frame: super::fog_objects::FogObjectFrame,
        buffers: &mut Assets<ShaderBuffer>,
    ) -> Result<(), String> {
        let size = (frame.capacity.max(1) * super::fog_objects::FogObjectParameters::SIZE) as u64;
        let buffer = buffers
            .get(&self.objects)
            .ok_or("alpha fog object asset unavailable")?;
        if buffer.buffer_description.size < size {
            let mut buffer = buffers.get_mut(&self.objects).unwrap();
            buffer.resize(size);
            buffer.copy_on_resize = false;
            self.bindings_dirty = true;
        }
        self.object_frame = Some(std::sync::Arc::new(frame));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::TextureFormat;
    #[test]
    fn shared_fog_outputs_retain_handles_and_skip_unchanged_frames() {
        let mut images = Assets::default();
        let mut buffers = Assets::default();
        let mut resources = FogMaterialResources::new(&mut images, &mut buffers);
        assert_eq!(
            buffers
                .get(&resources.parameters)
                .unwrap()
                .data
                .as_ref()
                .unwrap()
                .len(),
            256
        );
        let ids = [
            resources.world.id(),
            resources.canvas.id(),
            resources.opaque_depth.id(),
        ];
        let extents = FogExtents::new(UVec2::new(1920, 1080), 1).unwrap();
        assert!(resources.resize(&mut images, extents).unwrap());
        assert!(!resources.resize(&mut images, extents).unwrap());
        let curve = resources.curve.id();
        let lookup = images.get(curve).unwrap();
        assert_eq!(
            lookup.texture_descriptor.size,
            bevy::render::render_resource::Extent3d {
                width: 4096,
                height: 1,
                depth_or_array_layers: 3
            }
        );
        assert_eq!(lookup.texture_descriptor.format, TextureFormat::R16Float);
        assert_eq!(
            lookup.data.as_deref(),
            Some([0x00_u8, 0x3c].repeat(4096 * 3).as_slice())
        );
        assert!(resources.publish_bindings().is_some());
        assert!(resources.publish_bindings().is_none());
        assert_eq!(
            ids,
            [
                resources.world.id(),
                resources.canvas.id(),
                resources.opaque_depth.id()
            ]
        );
        for (id, size) in ids
            .into_iter()
            .zip([extents.world, extents.canvas, extents.full])
        {
            let image = images.get(id).unwrap();
            assert_eq!(image.size(), size);
            assert!(image.data.is_none());
            assert!(!image.copy_on_resize);
        }
        assert!(resources
            .resize(
                &mut images,
                FogExtents::new(UVec2::new(1280, 720), 2).unwrap()
            )
            .unwrap());
        assert!(resources.publish_bindings().is_some());
        let object_handle = resources.objects.id();
        let object_frame = super::super::fog_objects::FogObjectFrame {
            camera: Entity::PLACEHOLDER,
            revision: 1,
            world_offset: 8,
            capacity: 16,
            updates: Vec::new(),
        };
        resources
            .publish_objects(object_frame.clone(), &mut buffers)
            .unwrap();
        assert!(resources.publish_bindings().is_some());
        resources
            .publish_objects(object_frame, &mut buffers)
            .unwrap();
        assert!(resources.publish_bindings().is_none());
        assert_eq!(resources.objects.id(), object_handle);
        let objects = buffers.get(object_handle).unwrap();
        assert_eq!(objects.buffer_description.size, 16 * 128);
        assert!(objects.data.is_none() && !objects.copy_on_resize);
        assert!(objects
            .asset_usage
            .contains(bevy::asset::RenderAssetUsages::MAIN_WORLD));
        assert_eq!(resources.curve.id(), curve);
        assert_eq!(images.len(), 4);
        assert_eq!(buffers.len(), 2);
    }
}
