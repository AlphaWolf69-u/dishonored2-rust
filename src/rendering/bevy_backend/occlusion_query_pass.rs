//! GPU side of the renderer's occlusion query manager (1402926d0's draw and
//! 140292d10's counts). Each frame the issued queries draw the
//! `renderprog/occlusionquery` quad against the scene depth (depth test
//! GEQUAL, no writes, no colour) inside counting occlusion queries.
//!
//! bevy_backend: the original reads each slot's count back from the D3D
//! query the next frame. Here the counts are resolved into a buffer and read
//! asynchronously; a slot keeps its last arrived count until a newer one
//! lands, which can add a frame or two of latency to the visibility.
use crate::rendering::occlusion_query::{OcclusionQueries, QueryViewport, SLOTS};
use bevy::render::{render_resource::*, renderer::{RenderDevice, RenderQueue}};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

const UNIFORM_STRIDE: u64 = 256;
const IDLE: u8 = 0;
const COPIED: u8 = 1;
const MAPPING: u8 = 2;
const READY: u8 = 3;

struct Staging {
    buffer: Buffer,
    /// The slot drawn by each query index of the copied frame.
    slots: Vec<usize>,
    state: Arc<AtomicU8>,
}

pub struct OcclusionQueryPass {
    depth_format: TextureFormat,
    pipeline: RenderPipeline,
    uniforms: Buffer,
    binding: BindGroup,
    queries: QuerySet,
    resolve: Buffer,
    staging: Vec<Staging>,
}

impl OcclusionQueryPass {
    pub fn new(device: &RenderDevice, depth_format: TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(Some("occlusion query"), &[BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::VERTEX,
            count: None,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: BufferSize::new(32),
            },
        }]);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("occlusion query"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("occlusion query"),
            source: ShaderSource::Wgsl(include_str!("../../../assets/shaders/dh2_occlusion_query.wgsl").into()),
        });
        // renderprog/occlusionquery state: blend ONE ZERO, depthWrite 0,
        // depthEnable 1, depthfunc GEQUAL, cull none.
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("occlusion query"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &module,
                entry_point: Some("occlusion_query_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: None,
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleStrip,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&BufferDescriptor {
            label: Some("occlusion query transforms"),
            size: UNIFORM_STRIDE * SLOTS as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = device.create_bind_group(Some("occlusion query"), &layout, &[BindGroupEntry {
            binding: 0,
            resource: BindingResource::Buffer(BufferBinding { buffer: &uniforms, offset: 0, size: BufferSize::new(32) }),
        }]);
        let queries = device.wgpu_device().create_query_set(&QuerySetDescriptor {
            label: Some("occlusion queries"),
            ty: QueryType::Occlusion,
            count: SLOTS as u32,
        });
        let resolve = device.create_buffer(&BufferDescriptor {
            label: Some("occlusion query resolve"),
            size: 8 * SLOTS as u64,
            usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        Self { depth_format, pipeline, uniforms, binding, queries, resolve, staging: Vec::new() }
    }

    pub fn depth_format(&self) -> TextureFormat {
        self.depth_format
    }

    /// Map the copies submitted with earlier frames and hand arrived
    /// counts to the manager.
    fn harvest(&mut self, manager: &mut OcclusionQueries) {
        for staging in &mut self.staging {
            match staging.state.load(Ordering::Acquire) {
                COPIED => {
                    staging.state.store(MAPPING, Ordering::Release);
                    let state = staging.state.clone();
                    staging.buffer.slice(..).map_async(MapMode::Read, move |result| {
                        state.store(if result.is_ok() { READY } else { IDLE }, Ordering::Release);
                    });
                }
                READY => {
                    {
                        let data = staging.buffer.slice(..).get_mapped_range();
                        for (index, &slot) in staging.slots.iter().enumerate() {
                            let count = u64::from_le_bytes(data[index * 8..index * 8 + 8].try_into().unwrap());
                            manager.record_count(slot, count);
                        }
                    }
                    staging.buffer.unmap();
                    staging.slots.clear();
                    staging.state.store(IDLE, Ordering::Release);
                }
                _ => {}
            }
        }
    }

    /// 1402926d0 for one view. `depth` is the scene depth attachment.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        depth: &TextureView,
        matrices: &crate::rendering::view_history::ViewMatrices,
        extent: [u32; 2],
        manager: &mut OcclusionQueries,
    ) {
        self.harvest(manager);
        let draws = manager.prepare(
            &matrices.view,
            &matrices.projection,
            QueryViewport { width: extent[0], height: extent[1], scale: [1., 1.] },
        );
        if draws.is_empty() {
            return;
        }
        let mut bytes = vec![0u8; UNIFORM_STRIDE as usize * draws.len()];
        for (index, draw) in draws.iter().enumerate() {
            let at = index * UNIFORM_STRIDE as usize;
            for (k, value) in draw.transform1.iter().chain(draw.transform2.iter()).enumerate() {
                bytes[at + k * 4..at + k * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        queue.write_buffer(&self.uniforms, 0, &bytes);
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("occlusion queries"),
                color_attachments: &[],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: None,
                    stencil_ops: self.depth_format.has_stencil_aspect().then_some(Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: Some(&self.queries),
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            for (index, _) in draws.iter().enumerate() {
                pass.set_bind_group(0, &self.binding, &[(index as u64 * UNIFORM_STRIDE) as u32]);
                pass.begin_occlusion_query(index as u32);
                pass.draw(0..4, 0..1);
                pass.end_occlusion_query();
            }
        }
        let count = draws.len() as u32;
        encoder.resolve_query_set(&self.queries, 0..count, &self.resolve, 0);
        let staging = match self.staging.iter().position(|s| s.state.load(Ordering::Acquire) == IDLE) {
            Some(index) => index,
            None => {
                self.staging.push(Staging {
                    buffer: device.create_buffer(&BufferDescriptor {
                        label: Some("occlusion query readback"),
                        size: 8 * SLOTS as u64,
                        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    }),
                    slots: Vec::new(),
                    state: Arc::new(AtomicU8::new(IDLE)),
                });
                self.staging.len() - 1
            }
        };
        let staging = &mut self.staging[staging];
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &staging.buffer, 0, 8 * count as u64);
        staging.slots = draws.iter().map(|draw| draw.slot).collect();
        staging.state.store(COPIED, Ordering::Release);
    }
}
