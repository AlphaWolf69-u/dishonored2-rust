//! World-water colour vertex inputs, including per-vertex reflection probes.
use bevy::render::{render_resource::*, renderer::RenderDevice};

#[path = "water_world_colour/pass.rs"]
mod pass;
pub use pass::{WorldProbeColourDraw, WorldProbeColourParameters, WorldProbeColourProgram};

/// Select raw game-space positions, including when the host forward mesh has
/// converted or baked positions. The retained corner channel is a scalar.
pub fn vertex_buffer_layout(
    mesh: &bevy::mesh::MeshVertexBufferLayoutRef,
) -> Result<bevy::mesh::VertexBufferLayout, bevy::mesh::MissingVertexAttributeError> {
    mesh.0.get_layout(&[
        super::shadow_pass::VEGETATION_POSITION.at_shader_location(0),
        NORMAL_W.at_shader_location(1),
    ])
}

/// Vertex half of the verified WORLD/IBL colour specialization (VS14494).
/// Fragment options/resources and actual draw admission belong to the owner.
pub struct WorldProbeVertexStage {
    pub parameters: BindGroupLayout,
    pub inputs: BindGroupLayout,
    pub module: ShaderModule,
}

pub struct WorldProbeVertexBuffers<'a> {
    pub instances: &'a Buffer,
    pub matrices: &'a Buffer,
    pub references: &'a Buffer,
    pub probe_info: &'a Buffer,
    pub cube_indices: &'a Buffer,
}

impl WorldProbeVertexStage {
    pub fn new(device: &RenderDevice) -> Self {
        let layout = |name, sizes: &[u64], uniform| {
            device.create_bind_group_layout(
                Some(name),
                &sizes
                    .iter()
                    .enumerate()
                    .map(|(binding, &size)| BindGroupLayoutEntry {
                        binding: binding as u32,
                        visibility: ShaderStages::VERTEX,
                        count: None,
                        ty: BindingType::Buffer {
                            ty: if uniform {
                                BufferBindingType::Uniform
                            } else {
                                BufferBindingType::Storage { read_only: true }
                            },
                            has_dynamic_offset: uniform,
                            min_binding_size: BufferSize::new(size),
                        },
                    })
                    .collect::<Vec<_>>(),
            )
        };
        Self {
            parameters: layout("world water vertex parameters", &[80], true),
            inputs: layout("world water matrices and probes", &[4, 48, 32, 8, 4], false),
            module: device.create_and_validate_shader_module(ShaderModuleDescriptor {
                label: Some("arkWater world probe vertex"),
                source: ShaderSource::Wgsl(SHADER_SOURCE.into()),
            }),
        }
    }

    pub fn bind_parameters(&self, device: &RenderDevice, buffer: &Buffer) -> BindGroup {
        device.create_bind_group(
            Some("world water vertex parameters"),
            &self.parameters,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer,
                    offset: 0,
                    size: BufferSize::new(80),
                }),
            }],
        )
    }

    pub fn bind_inputs(
        &self,
        device: &RenderDevice,
        input: WorldProbeVertexBuffers<'_>,
    ) -> BindGroup {
        let buffers = [
            input.instances,
            input.matrices,
            input.references,
            input.probe_info,
            input.cube_indices,
        ];
        device.create_bind_group(
            Some("world water vertex sources"),
            &self.inputs,
            &buffers
                .into_iter()
                .enumerate()
                .map(|(binding, buffer)| BindGroupEntry {
                    binding: binding as u32,
                    resource: buffer.as_entire_binding(),
                })
                .collect::<Vec<_>>(),
        )
    }
}
/// SNORM normal W carries the authored water-probe triangle corner. World
/// forward/shadow geometry already retains raw POSITION, but not this channel.
pub const NORMAL_W: bevy::mesh::MeshVertexAttribute = bevy::mesh::MeshVertexAttribute::new(
    "Water_NormalW",
    0x4448_3204,
    bevy::render::render_resource::VertexFormat::Float32,
);

pub fn retain_normal_w<const N: usize>(
    mesh: &mut bevy::mesh::Mesh,
    packed: &[[u8; N]],
) -> Result<(), String> {
    if N != 12 && N != 16 {
        return Err(format!("unsupported world water channel stride {N}"));
    }
    if mesh.count_vertices() != packed.len() {
        return Err("world water source/mesh vertex counts disagree".into());
    }
    if !mesh.contains_attribute(NORMAL_W) {
        let normal_w: Vec<f32> = packed
            .iter()
            .map(|v| ((v[7] as i8 as f32) / 127.0).max(-1.0))
            .collect();
        mesh.insert_attribute(NORMAL_W, normal_w);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct WaterWorldColourParameters {
    /// Logical rows in game coordinates, before the vertex shader's projection.
    pub clip_from_world: [[f32; 4]; 4],
    pub instance_buffer_offset: u32,
    /// Base vertex from the actual shared mesh allocation.
    pub first_vertex: u32,
    /// WGPU draw start, removed to recover D3D11's draw-relative instance ID.
    pub first_instance: u32,
}

impl WaterWorldColourParameters {
    pub fn uniform_bytes(&self) -> [u8; 80] {
        let mut bytes = [0; 80];
        let words = self
            .clip_from_world
            .into_iter()
            .flatten()
            .map(f32::to_bits)
            .chain([
                self.instance_buffer_offset,
                self.first_vertex,
                self.first_instance,
                0,
            ]);
        for (i, word) in words.enumerate() {
            bytes[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
        }
        bytes
    }
}

pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_probe_weights.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_shadow.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_probes.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_shadow_world_project.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_world_colour.wgsl"),
);

#[cfg(test)]
#[path = "water_world_colour/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "water_world_colour/raster_tests.rs"]
mod raster_tests;

#[cfg(test)]
#[path = "water_world_colour/exposure_tests.rs"]
mod exposure_tests;
