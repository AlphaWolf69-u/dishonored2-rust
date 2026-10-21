//! Persistent GPU portion of 1403030f0's water-height evaluations.
//! Query handles and the source four-frame readback ordering are separate.
use super::{noise_textures::NoiseTextures, ocean_surface::OceanSurfaceParameters};
use crate::rendering::ocean::OceanQueryQueue;
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};
mod readback;
pub use readback::OceanQueryReadbacks;

/// 140302960 allocates 8192 input bytes and 16384 output bytes.
pub use crate::rendering::ocean::OCEAN_QUERY_CAPACITY;

/// Actual source images, not substitutes for unfinished wave/mask production.
pub struct OceanHeightTextures<'a> {
    pub noise: &'a NoiseTextures,
    pub displacement: [&'a TextureView; 2],
    pub collision_height: &'a TextureView,
    pub foam: &'a TextureView,
    pub shore_distance: &'a TextureView,
    pub amplitude_mask: &'a TextureView,
}

pub struct OceanHeightQueries {
    positions: Buffer,
    evaluations: Buffer,
    surface: Buffer,
    controls: Buffer,
    pipeline: ComputePipeline,
    bindings: [BindGroup; 2],
    position_bytes: Vec<u8>,
    prepared_count: u32,
}

impl OceanHeightQueries {
    /// 3030f0's dispatch/copy/count transition. Ring selection is supplied by
    /// the renderer; this does not rotate a ring or manufacture CPU results.
    pub fn record_pending(
        &mut self,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        requests: &mut OceanQueryQueue,
        readbacks: &mut OceanQueryReadbacks,
        parameters: &OceanSurfaceParameters,
        water_level: f32,
        current_wave_slot: usize,
        current_ring: usize,
    ) -> Result<(), String> {
        if current_ring >= 4 || current_wave_slot >= 2 {
            return Err("ocean query ring/wave slot is out of range".into());
        }
        self.prepare(queue, parameters, water_level, requests.pending_points())?;
        self.record(encoder, current_wave_slot);
        readbacks.record_copy(
            encoder,
            self.evaluations(),
            self.prepared_count,
            current_ring,
        )?;
        requests.record_submission(current_ring)?;
        Ok(())
    }

    pub fn new(device: &RenderDevice, textures: OceanHeightTextures<'_>) -> Self {
        let buffer = |label, size, usage| {
            device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let positions = buffer(
            "ocean query positions",
            8192,
            BufferUsages::STORAGE | BufferUsages::COPY_DST,
        );
        let evaluations = buffer(
            "ocean query evaluations",
            16384,
            BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
        );
        let surface = buffer(
            "ocean query surface",
            112,
            BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        );
        let controls = buffer(
            "ocean query count and water level",
            16,
            BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        );
        let source = format!(
            "{}\n{}",
            super::ocean_surface::SHADER_SOURCE,
            include_str!("../../../assets/shaders/dh2_ocean_height_query.wgsl")
        );
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("ocean height query"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: Some("ocean height query"),
            layout: None,
            module: &module,
            entry_point: Some("ocean_height_query"),
            compilation_options: Default::default(),
            cache: None,
        });
        let linear_wrap = super::shadow_pipeline::shadow_alpha_sampler(device);
        let linear_clamp = device.create_sampler(&SamplerDescriptor {
            label: Some("ocean query bilinearClamp"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Nearest,
            lod_max_clamp: 13.0,
            ..Default::default()
        });
        let layout = pipeline.get_bind_group_layout(0).into();
        let bindings = textures.displacement.map(|displacement| {
            device.create_bind_group(
                "ocean query inputs",
                &layout,
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: surface.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: controls.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(textures.noise.permutation()),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::TextureView(textures.noise.gradients()),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: BindingResource::Sampler(textures.noise.point_wrap()),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: BindingResource::Sampler(&linear_wrap),
                    },
                    BindGroupEntry {
                        binding: 6,
                        resource: BindingResource::Sampler(&linear_clamp),
                    },
                    BindGroupEntry {
                        binding: 7,
                        resource: BindingResource::TextureView(textures.shore_distance),
                    },
                    BindGroupEntry {
                        binding: 8,
                        resource: BindingResource::TextureView(displacement),
                    },
                    BindGroupEntry {
                        binding: 9,
                        resource: BindingResource::TextureView(textures.collision_height),
                    },
                    BindGroupEntry {
                        binding: 10,
                        resource: BindingResource::TextureView(textures.foam),
                    },
                    BindGroupEntry {
                        binding: 11,
                        resource: BindingResource::TextureView(textures.amplitude_mask),
                    },
                    BindGroupEntry {
                        binding: 12,
                        resource: positions.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 13,
                        resource: evaluations.as_entire_binding(),
                    },
                ],
            )
        });
        Self {
            positions,
            evaluations,
            surface,
            controls,
            pipeline,
            bindings,
            position_bytes: Vec::with_capacity(8192),
            prepared_count: 0,
        }
    }

    /// Prepare before recording/submitting this dispatch. Calling this again
    /// before submission replaces its queued uniform/input values.
    pub fn prepare(
        &mut self,
        queue: &RenderQueue,
        parameters: &OceanSurfaceParameters,
        water_level: f32,
        positions: &[[f32; 2]],
    ) -> Result<(), String> {
        if positions.len() > OCEAN_QUERY_CAPACITY {
            return Err("ocean height query exceeds 1024 evaluations".into());
        }
        let count = positions.len() as u32;
        if count > 0 {
            self.position_bytes.clear();
            self.position_bytes.extend(
                positions
                    .iter()
                    .flatten()
                    .flat_map(|value| value.to_le_bytes()),
            );
            queue.write_buffer(&self.positions, 0, &self.position_bytes);
            queue.write_buffer(&self.surface, 0, &parameters.uniform_bytes());
            let mut controls = [0; 16];
            controls[..4].copy_from_slice(&count.to_le_bytes());
            controls[4..8].copy_from_slice(&water_level.to_le_bytes());
            queue.write_buffer(&self.controls, 0, &controls);
        }
        self.prepared_count = count;
        Ok(())
    }

    /// Supply the wave owner's real current double-buffer slot. No readback
    /// readiness is implied by recording GPU commands.
    pub fn record(&self, encoder: &mut CommandEncoder, current_slot: usize) {
        if self.prepared_count == 0 {
            return;
        }
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("ocean height queries"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bindings[current_slot], &[]);
        pass.dispatch_workgroups(self.prepared_count.div_ceil(64), 1, 1);
    }

    pub fn prepared_count(&self) -> u32 {
        self.prepared_count
    }
    /// Packed float4(height, normal.xyz), only valid after the dispatch completes.
    pub fn evaluations(&self) -> &Buffer {
        &self.evaluations
    }
    pub fn buffer_bytes(&self) -> u64 {
        self.positions.size() + self.evaluations.size() + self.surface.size() + self.controls.size()
    }
}

#[cfg(test)]
mod tests;
