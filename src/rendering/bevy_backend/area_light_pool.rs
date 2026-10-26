//! 2f84d0/2f8c30: shared scene-light texture requests and persistent pool slots.
use bevy::{
    prelude::*,
    render::{
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        texture::GpuImage,
    },
};

const DIMENSION: u32 = 256;
const SLOTS: usize = 32;
const REQUESTS: usize = 128;

pub struct AreaLightPool {
    texture: Texture,
    pub view: TextureView,
    slots: [Option<Handle<Image>>; SLOTS],
    copy: ComputePipeline,
}

impl AreaLightPool {
    pub fn new(device: &RenderDevice, copy: &ComputePipeline) -> Self {
        // 36d0c0's _poolAreaLights descriptor: packed format0x54a (DXGI28),
        // 256x256x1, auto mip counts, 32 array elements, shader write access.
        let mut counts = [0; 4];
        crate::texture::initialize_image_mip_counts(10, [256, 256, 1], &mut counts);
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("scene area-light texture pool"),
            size: Extent3d {
                width: DIMENSION,
                height: DIMENSION,
                depth_or_array_layers: SLOTS as u32,
            },
            mip_level_count: counts[0] as u32,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..default()
        });
        Self {
            texture,
            view,
            slots: std::array::from_fn(|_| None),
            copy: copy.clone(),
        }
    }

    pub fn prepare(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        images: &RenderAssets<GpuImage>,
        requested: &[Option<Handle<Image>>],
        packets: &mut [[u32; 28]],
    ) -> Result<(), String> {
        if requested.len() != packets.len() {
            return Err("area-light images do not match scene light packets".into());
        }
        let mut selected: Vec<(Handle<Image>, usize)> = Vec::new();
        let mut count = 0;
        for image in requested.iter().flatten() {
            count += 1;
            if count > REQUESTS {
                return Err("Too many area lights requesting textures! Maximum is 128".into());
            }
            if let Some((_, refs)) = selected.iter_mut().find(|(old, _)| old == image) {
                *refs += 1;
            } else {
                selected.push((image.clone(), 1));
            }
        }
        if selected.len() > SLOTS {
            return Err("All 32 slots for area light textures are used".into());
        }
        selected.sort_by_key(|(_, references)| std::cmp::Reverse(*references));
        // Complete readiness checks before changing resident slot identities.
        for (image, _) in &selected {
            let gpu = images
                .get(image)
                .ok_or("area-light image awaits GPU preparation")?;
            if gpu.texture.dimension() != TextureDimension::D2
                || gpu.texture.size().depth_or_array_layers != 1
            {
                return Err("area-light source must be a two-dimensional image".into());
            }
        }
        for slot in &mut self.slots {
            if slot
                .as_ref()
                .is_some_and(|old| !selected.iter().any(|(image, _)| image == old))
            {
                *slot = None;
            }
        }
        for (image, _) in selected {
            if self.slots.iter().any(|slot| slot.as_ref() == Some(&image)) {
                continue;
            }
            let slot = self
                .slots
                .iter()
                .position(Option::is_none)
                .ok_or("area-light slot allocation failed")?;
            let source = images
                .get(&image)
                .ok_or("area-light image awaits GPU preparation")?;
            for mip in 0..source
                .texture
                .mip_level_count()
                .min(self.texture.mip_level_count())
            {
                let target = self.texture.create_view(&TextureViewDescriptor {
                    dimension: Some(TextureViewDimension::D2Array),
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    base_array_layer: slot as u32,
                    array_layer_count: Some(1),
                    ..default()
                });
                let mut parameters = UniformBuffer::from(UVec4::new(mip, 0, 0, 0));
                parameters.write_buffer(device, queue);
                let layout: BindGroupLayout = self.copy.get_bind_group_layout(0).into();
                let binding = device.create_bind_group(
                    Some("area-light image copy"),
                    &layout,
                    &[
                        BindGroupEntry {
                            binding: 0,
                            resource: BindingResource::TextureView(&source.texture_view),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: BindingResource::TextureView(&target),
                        },
                        BindGroupEntry {
                            binding: 2,
                            resource: parameters.binding().unwrap(),
                        },
                    ],
                );
                let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                    label: Some("area-light pool upload"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.copy);
                pass.set_bind_group(0, &binding, &[]);
                let groups = (DIMENSION >> mip).max(1).div_ceil(16);
                pass.dispatch_workgroups(groups, groups, 1);
            }
            self.slots[slot] = Some(image);
        }
        for (packet, image) in packets.iter_mut().zip(requested) {
            if (packet[25] >> 4) & 7 != 3 {
                continue;
            }
            let slice = image
                .as_ref()
                .map(|image| {
                    self.slots
                        .iter()
                        .position(|slot| slot.as_ref() == Some(image))
                        .ok_or("area-light packet image has no resident slot")
                })
                .transpose()?
                .map_or(255, |slot| slot as u32);
            packet[25] = (packet[25] & !0xff00) | (slice << 8);
        }
        Ok(())
    }
}
