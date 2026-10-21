//! Position-only mesh passes share current scene allocations and transforms.
use bevy::{
    mesh::MeshVertexBufferLayoutRef,
    platform::collections::HashMap,
    prelude::*,
    render::{
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

struct Record {
    pipeline: usize,
    vertex: Buffer,
    index: Option<(Buffer, IndexFormat, std::ops::Range<u32>, i32)>,
    vertices: std::ops::Range<u32>,
    instance: u32,
}

pub struct PositionInstance {
    pub mesh: AssetId<Mesh>,
    pub world_from_local: Mat4,
}

pub struct PositionDraw<'a> {
    pub pipeline: &'a RenderPipeline,
    pub models: &'a BindGroup,
    pub positions: &'a Buffer,
    pub vertices: std::ops::Range<u32>,
    pub indices: Option<(&'a Buffer, IndexFormat, std::ops::Range<u32>, i32)>,
    pub instances: std::ops::Range<u32>,
}

/// Each owner supplies one invariant model layout and pipeline factory. Reuse
/// the owner across frames; recreate it when its program/target state changes.
#[derive(Default)]
pub struct PositionMeshes {
    ready: bool,
    records: Vec<Record>,
    layouts: HashMap<MeshVertexBufferLayoutRef, usize>,
    pipelines: Vec<RenderPipeline>,
    transforms: StorageBuffer<Vec<Mat4>>,
    models: Option<(BufferId, BindGroup)>,
}

impl PositionMeshes {
    pub fn prepare(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        instances: impl IntoIterator<Item = PositionInstance>,
        allocator: &MeshAllocator,
        meshes: &RenderAssets<RenderMesh>,
        model_layout: &BindGroupLayout,
        mut pipeline: impl FnMut(&bevy::mesh::VertexBufferLayout) -> RenderPipeline,
    ) -> Result<(), String> {
        self.records.clear();
        self.ready = false;
        self.transforms.get_mut().clear();
        for instance in instances {
            let mesh = meshes.get(instance.mesh).ok_or_else(|| {
                format!(
                    "position-only mesh {:?} awaits GPU preparation",
                    instance.mesh
                )
            })?;
            let vertex = allocator
                .mesh_vertex_slice(&instance.mesh)
                .ok_or("position-only mesh has no current vertex allocation")?;
            let pipeline = if let Some(index) = self.layouts.get(&mesh.layout) {
                *index
            } else {
                let vertex_layout = mesh
                    .layout
                    .0
                    .get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])
                    .map_err(|e| e.to_string())?;
                let index = self.pipelines.len();
                self.pipelines.push(pipeline(&vertex_layout));
                self.layouts.insert(mesh.layout.clone(), index);
                index
            };
            let index = if let Some(format) = mesh.index_format() {
                let slice = allocator
                    .mesh_index_slice(&instance.mesh)
                    .ok_or("position-only mesh has no current index allocation")?;
                Some((
                    slice.buffer.clone(),
                    format,
                    slice.range.clone(),
                    i32::try_from(vertex.range.start)
                        .map_err(|_| "position-only base vertex exceeds GPU index range")?,
                ))
            } else {
                None
            };
            let slot = u32::try_from(self.transforms.get().len())
                .map_err(|_| "too many position-only instances")?;
            self.transforms.get_mut().push(instance.world_from_local);
            self.records.push(Record {
                pipeline,
                vertex: vertex.buffer.clone(),
                index,
                vertices: vertex.range.clone(),
                instance: slot,
            });
        }
        if self.records.is_empty() {
            self.ready = true;
            return Ok(());
        }
        self.transforms.write_buffer(device, queue);
        let buffer = self
            .transforms
            .buffer()
            .ok_or("position-only transform buffer was not prepared")?;
        if self
            .models
            .as_ref()
            .is_none_or(|(id, _)| *id != buffer.id())
        {
            self.models = Some((
                buffer.id(),
                device.create_bind_group(
                    Some("position-only instance transforms"),
                    model_layout,
                    &[BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                ),
            ));
        }
        self.ready = true;
        Ok(())
    }

    pub fn draws(&self) -> Vec<PositionDraw<'_>> {
        assert!(
            self.ready,
            "position-only draws require a completed mesh preparation"
        );
        self.records
            .iter()
            .map(|record| PositionDraw {
                pipeline: &self.pipelines[record.pipeline],
                models: &self
                    .models
                    .as_ref()
                    .expect("prepared position-only transforms")
                    .1,
                positions: &record.vertex,
                vertices: record.vertices.clone(),
                indices: record
                    .index
                    .as_ref()
                    .map(|(buffer, format, range, base)| (buffer, *format, range.clone(), *base)),
                instances: record.instance..record.instance + 1,
            })
            .collect()
    }
}
