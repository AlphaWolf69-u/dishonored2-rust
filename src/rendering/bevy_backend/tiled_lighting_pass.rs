//! Persistent per-view TLF depth, payload and dual-list submission. The caller
//! supplies the actual decompressed opaque/alpha depth and admitted light order.
use super::tiled_lighting_gpu::{TlfBuffers, TlfPipelines};
use crate::tiled_lighting::{TileGrid, LIGHT_BUFFER_CAPACITY, SHADOW_BUFFER_CAPACITY};
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};
#[cfg(test)]
#[path = "tiled_lighting_pass_tests.rs"]
pub(super) mod tests;

#[derive(Clone, Copy, Default, ShaderType)]
pub struct TiledLightView {
    pub uv_to_ray: Vec4,
    pub inverse_tile_dimensions: Vec4,
    /// Original logical rows, not glam columns.
    pub view: [Vec4; 4],
    pub projection: [Vec4; 4],
    pub light_count: u32,
    pub gather_scattering: u32,
    pub padding: UVec2,
}
#[derive(Clone, Copy, Default, ShaderType)]
struct DepthView {
    unproject: Vec4,
    tile: UVec4,
}

pub struct TiledLightingPass {
    pub buffers: TlfBuffers,
    pub lights: Buffer,
    pub shadows: Buffer,
    pub normal_heads: Buffer,
    pub scattering_heads: Buffer,
    grid: TileGrid,
    tile_depth: Texture,
    tile_depth_view: TextureView,
    depth_view: UniformBuffer<DepthView>,
    normal_view: UniformBuffer<TiledLightView>,
    scattering_view: UniformBuffer<TiledLightView>,
    clamp: Sampler,
    depth_binding: Option<([TextureViewId; 2], BindGroup)>,
    cull_bindings: Option<[BindGroup; 2]>,
    clear_bindings: [BindGroup; 2],
    prepared: bool,
}
impl TiledLightingPass {
    pub fn grid(&self) -> TileGrid { self.grid }
    fn upload<const N: usize>(queue: &RenderQueue, buffer: &Buffer, records: &[[u32; N]]) {
        if !records.is_empty() {
            let bytes: Vec<u8> = records
                .iter()
                .flatten()
                .flat_map(|word| word.to_le_bytes())
                .collect();
            queue.write_buffer(buffer, 0, &bytes);
        }
    }
    fn heads(device: &RenderDevice, grid: TileGrid, types: u64) -> Buffer {
        device.create_buffer(&BufferDescriptor {
            label: Some("TLF per-type list heads"),
            size: u64::from(grid.dimensions[0]) * u64::from(grid.dimensions[1]) * types * 4,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }
    fn depth(device: &RenderDevice, grid: TileGrid) -> Texture {
        device.create_texture(&TextureDescriptor {
            label: Some("TLF tile depth/bins"),
            size: Extent3d {
                width: grid.dimensions[0],
                height: grid.dimensions[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba32Float,
            usage: TextureUsages::STORAGE_BINDING
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }
    fn clear_bindings(
        device: &RenderDevice,
        pipelines: &TlfPipelines,
        heads: [&Buffer; 2],
    ) -> [BindGroup; 2] {
        let layout = pipelines.clear_heads.get_bind_group_layout(0).into();
        heads.map(|buffer| {
            device.create_bind_group(
                Some("TLF head reset"),
                &layout,
                &[BindGroupEntry {
                    binding: 3,
                    resource: buffer.as_entire_binding(),
                }],
            )
        })
    }
    pub fn new(device: &RenderDevice, pipelines: &TlfPipelines, grid: TileGrid) -> Self {
        let normal_heads = Self::heads(device, grid, 4);
        let scattering_heads = Self::heads(device, grid, 3);
        let clear_bindings =
            Self::clear_bindings(device, pipelines, [&normal_heads, &scattering_heads]);
        let tile_depth = Self::depth(device, grid);
        let tile_depth_view = tile_depth.create_view(&default());
        let payload = |label, size| {
            device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let mut pass = Self {
            buffers: TlfBuffers::new(device),
            lights: payload("TLF lighting records", (LIGHT_BUFFER_CAPACITY * 112) as u64),
            shadows: payload("TLF shadow records", (SHADOW_BUFFER_CAPACITY * 144) as u64),
            normal_heads,
            scattering_heads,
            grid,
            tile_depth,
            tile_depth_view,
            depth_view: default(),
            normal_view: default(),
            scattering_view: default(),
            clamp: device.create_sampler(&SamplerDescriptor {
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                ..default()
            }),
            depth_binding: None,
            cull_bindings: None,
            clear_bindings,
            prepared: false,
        };
        pass.depth_view.set_label(Some("TLF depth parameters"));
        pass.normal_view.set_label(Some("TLF normal parameters"));
        pass.scattering_view
            .set_label(Some("TLF scattering parameters"));
        pass
    }
    /// Changing the viewport replaces only its depth/head attachments. Fixed
    /// light/shadow/pool/counter allocations retain their identities.
    pub fn resize(&mut self, device: &RenderDevice, pipelines: &TlfPipelines, grid: TileGrid) {
        if self.grid == grid {
            return;
        }
        self.normal_heads = Self::heads(device, grid, 4);
        self.scattering_heads = Self::heads(device, grid, 3);
        self.tile_depth = Self::depth(device, grid);
        self.tile_depth_view = self.tile_depth.create_view(&default());
        self.clear_bindings = Self::clear_bindings(
            device,
            pipelines,
            [&self.normal_heads, &self.scattering_heads],
        );
        self.grid = grid;
        self.depth_binding = None;
        self.cull_bindings = None;
        self.prepared = false;
    }
    pub fn prepare(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        pipelines: &TlfPipelines,
        mut view: TiledLightView,
        opaque: &TextureView,
        alpha: &TextureView,
        lighting: &[[u32; 28]],
        shadows: &[[u32; 36]],
    ) -> Result<(), String> {
        self.prepared = false;
        if shadows.len() > SHADOW_BUFFER_CAPACITY {
            return Err("TLF shadow records exceed 256".into());
        }
        for record in lighting {
            if record[24] == u32::MAX {
                continue;
            }
            let kind = (record[25] >> 4) & 7;
            let count = if kind < 2 {
                6
            } else {
                1 + usize::from(kind == 2 && record[25] & 128 != 0)
            };
            if record[24] as usize + count > shadows.len() {
                return Err("TLF light references unavailable shadow sides".into());
            }
        }
        let rows = std::array::from_fn(|i| view.view[i].to_array());
        let count = self
            .buffers
            .upload_lighting_packets(queue, &rows, lighting)?;
        Self::upload(queue, &self.lights, lighting);
        Self::upload(queue, &self.shadows, shadows);
        // Both native callbacks derive this pair from the SAME projection.
        // Do not accept the fog pass's different unprojection ABI here.
        let projection = view.projection.map(|row| row.to_array());
        let unproject = crate::tiled_lighting::depth_unprojection(&projection);
        self.depth_view.set(DepthView {
            unproject: Vec4::new(unproject[0], unproject[1], 0., 0.),
            tile: UVec4::new(self.grid.tile_dimension, 0, 0, 0),
        });
        self.depth_view.write_buffer(device, queue);
        view.inverse_tile_dimensions = Vec4::from_array(self.grid.inverse_dimensions);
        view.light_count = count as u32;
        view.gather_scattering = 0;
        self.normal_view.set(view);
        self.normal_view.write_buffer(device, queue);
        view.gather_scattering = 1;
        self.scattering_view.set(view);
        self.scattering_view.write_buffer(device, queue);
        let ids = [opaque.id(), alpha.id()];
        if self
            .depth_binding
            .as_ref()
            .is_none_or(|(old, _)| *old != ids)
        {
            let layout = pipelines.depth.get_bind_group_layout(0).into();
            self.depth_binding = Some((
                ids,
                device.create_bind_group(
                    Some("TLF depth inputs"),
                    &layout,
                    &[
                        BindGroupEntry {
                            binding: 0,
                            resource: BindingResource::TextureView(opaque),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: BindingResource::TextureView(alpha),
                        },
                        BindGroupEntry {
                            binding: 2,
                            resource: BindingResource::Sampler(&self.clamp),
                        },
                        BindGroupEntry {
                            binding: 3,
                            resource: self.depth_view.binding().unwrap(),
                        },
                        BindGroupEntry {
                            binding: 4,
                            resource: BindingResource::TextureView(&self.tile_depth_view),
                        },
                    ],
                ),
            ));
        }
        if self.cull_bindings.is_none() {
            let layout = pipelines.cull.get_bind_group_layout(0).into();
            self.cull_bindings = Some(
                [
                    (&self.normal_view, &self.normal_heads),
                    (&self.scattering_view, &self.scattering_heads),
                ]
                .map(|(uniform, heads)| {
                    device.create_bind_group(
                        Some("TLF cull inputs"),
                        &layout,
                        &[
                            BindGroupEntry {
                                binding: 0,
                                resource: uniform.binding().unwrap(),
                            },
                            BindGroupEntry {
                                binding: 1,
                                resource: BindingResource::TextureView(&self.tile_depth_view),
                            },
                            BindGroupEntry {
                                binding: 2,
                                resource: self.buffers.packets.as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 3,
                                resource: heads.as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 4,
                                resource: self.buffers.counter.as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 5,
                                resource: self.buffers.pool.as_entire_binding(),
                            },
                        ],
                    )
                }),
            );
        }
        self.prepared = true;
        Ok(())
    }
    pub fn record(
        &self,
        encoder: &mut CommandEncoder,
        pipelines: &TlfPipelines,
    ) -> Result<(), String> {
        if !self.prepared {
            return Err("TLF frame is not prepared".into());
        }
        self.buffers.begin_frame(encoder);
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("TLF head reset"),
                ..default()
            });
            pass.set_pipeline(&pipelines.clear_heads);
            let count = self.grid.dimensions[0] * self.grid.dimensions[1] * 4;
            for binding in &self.clear_bindings {
                pass.set_bind_group(0, binding, &[]);
                pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("TLF depth gather"),
                ..default()
            });
            pass.set_pipeline(&pipelines.depth);
            pass.set_bind_group(0, &self.depth_binding.as_ref().unwrap().1, &[]);
            pass.dispatch_workgroups(self.grid.dimensions[0], self.grid.dimensions[1], 1);
        }
        for binding in self.cull_bindings.as_ref().unwrap() {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("TLF normal/scattering cull"),
                ..default()
            });
            pass.set_pipeline(&pipelines.cull);
            pass.set_bind_group(0, binding, &[]);
            pass.dispatch_workgroups(self.grid.dimensions[0], self.grid.dimensions[1], 1);
        }
        Ok(())
    }
}
