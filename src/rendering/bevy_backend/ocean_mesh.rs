//! Shared once-uploaded ocean grid and source strip/edge submission.
use bevy::render::{render_resource::*, renderer::RenderDevice};

/// Depth and colour share the exact grid and source 16-bit strip/list indices.
pub struct OceanMeshBuffers {
    pub vertices: Buffer,
    pub indices: Buffer,
    pub records: Vec<crate::rendering::ocean::OceanGeometryRecord>,
}
impl OceanMeshBuffers {
    pub fn new(device: &RenderDevice, geometry: crate::rendering::ocean::OceanGeometry) -> Self {
        let vertices = geometry
            .vertices
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        let indices = geometry
            .indices
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        Self {
            vertices: device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("ocean grid vertices"),
                contents: &vertices,
                usage: BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("ocean grid indices"),
                contents: &indices,
                usage: BufferUsages::INDEX,
            }),
            records: geometry.records,
        }
    }
    pub fn buffer_bytes(&self) -> u64 {
        self.vertices.size() + self.indices.size()
    }
}

pub struct OceanPatchDraw<'a> {
    pub record: usize,
    pub parameters: &'a BindGroup,
    pub parameter_offset: u32,
}
