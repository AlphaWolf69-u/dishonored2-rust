//! Reusable native TLF GPU buffers; pass ownership and dispatch remain external.
use bevy::{
    prelude::{FromWorld, Resource, World},
    render::{
        render_resource::{Buffer, BufferDescriptor, BufferUsages, CommandEncoder,
            ComputePipeline, RawComputePipelineDescriptor, ShaderModule, ShaderModuleDescriptor, ShaderSource},
        renderer::{RenderDevice, RenderQueue},
    },
};

use crate::render_resource::{resource_name_key, RenderTaskProperties};
use crate::tiled_lighting::{
    prepare_light_cull_packets, LIGHT_BUFFER_CAPACITY, LIGHT_NODE_COUNTER_BYTES, LIGHT_NODE_POOL_CAPACITY,
};

/// The shared render-world pipelines execute the original-backed shaders used
/// by hardware parity tests and the eventual native pass; no per-frame compilation.
#[derive(Resource)]
pub struct NativeTlfPipelines {
    pub depth: ComputePipeline,
    pub cull: ComputePipeline,
    /// Original depthcopy.decl; frame-graph destination/state remain external.
    pub depth_copy: ShaderModule,
    /// Captured 2D identity/downscale shaders used by the original depth inputs.
    pub depth_copy_2d: ShaderModule,
}

impl NativeTlfPipelines {
    /// Capture82270 events25931/25946: point-clamp 2D depth copy/min-gather,
    /// full-screen triangle, no colour outputs, depth write with Always test.
    /// Build once for the caller-owned depth format and vertex layout.
    /// The capture's vertex stride is 32; compact fixture vertices are not the
    /// native buffer ABI. Never compile per frame.
    pub fn depth_copy_pipeline(
        &self, device: &RenderDevice,
        format: bevy::render::render_resource::TextureFormat, downscale: bool,
        buffers: &[bevy::render::render_resource::RawVertexBufferLayout<'_>],
    ) -> bevy::render::render_resource::RenderPipeline {
        use bevy::render::render_resource::*;
        device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("native 2D depth copy/downscale"), layout: None,
            vertex: RawVertexState {module: &self.depth_copy_2d, entry_point: Some("vertex"),
                compilation_options: Default::default(), buffers},
            fragment: Some(RawFragmentState {module: &self.depth_copy_2d,
                entry_point: Some(if downscale {"downscale"} else {"identity"}),
                compilation_options: Default::default(), targets: &[]}),
            primitive: PrimitiveState {front_face: FrontFace::Cw, ..Default::default()},
            multisample: Default::default(), multiview_mask: None, cache: None,
            depth_stencil: Some(DepthStencilState {format, depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::Always), stencil: Default::default(), bias: Default::default()}),
        })
    }
}

impl FromWorld for NativeTlfPipelines {
    fn from_world(world: &mut World) -> Self {
        let device = world.resource::<RenderDevice>();
        let compile = |source: &'static str, entry: &'static str| {
            let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
                label: Some(entry), source: ShaderSource::Wgsl(source.into()),
            });
            device.create_compute_pipeline(&RawComputePipelineDescriptor {
                label: Some(entry), layout: None, module: &module,
                entry_point: Some(entry), compilation_options: Default::default(), cache: None,
            })
        };
        Self {
            depth: compile(include_str!("../assets/shaders/dh2_tlf_depth.wgsl"), "gather_depth"),
            cull: compile(include_str!("../assets/shaders/dh2_tlf_cull.wgsl"), "cull"),
            depth_copy: device.create_and_validate_shader_module(ShaderModuleDescriptor {
                label: Some("native depthcopy"),
                source: ShaderSource::Wgsl(include_str!("../assets/shaders/depth_copy.wgsl").into()),
            }),
            depth_copy_2d: device.create_and_validate_shader_module(ShaderModuleDescriptor {
                label: Some("native 2D depth identity/downscale"),
                source: ShaderSource::Wgsl(include_str!("../assets/shaders/depth_copy_2d.wgsl").into()),
            }),
        }
    }
}

/// Native FUN_140371750 allocations: 448 × 48-byte cull packets, 1,152,000
/// four-byte pool links, and a separate four-byte counter. FUN_140351780
/// clears the counter once per frame before normal and scatter passes.
/// The caller must separately initialize each pass's heads to END_OF_LIGHT_LIST;
/// cull_lights exchanges/appends heads, it does not clear them. Stale pool links
/// are unreachable only after both counter and head initialization.
/// Light/shadow payload owners and material/shader pipeline integration remain
/// outside this resource.
pub struct NativeTlfBuffers {
    pub packets: Buffer,
    pub counter: Buffer,
    pub pool: Buffer,
    inputs: RenderTaskProperties<Buffer>,
}

impl NativeTlfBuffers {
    pub fn new(device: &RenderDevice) -> Self {
        let mut buffers = Self {
            packets: device.create_buffer(&BufferDescriptor {
                label: Some("native TLF cull packets"),
                size: (LIGHT_BUFFER_CAPACITY * 48) as u64,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            counter: device.create_buffer(&BufferDescriptor {
                label: Some("native TLF shared node counter"),
                size: LIGHT_NODE_COUNTER_BYTES,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            pool: device.create_buffer(&BufferDescriptor {
                label: Some("native TLF light-node pool"),
                size: (LIGHT_NODE_POOL_CAPACITY * 4) as u64,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            inputs: RenderTaskProperties::default(),
        };
        for (name, buffer) in [
            ("lightToPutInTiles", &buffers.packets),
            ("lightNodePoolNextFreeEntry", &buffers.counter),
            ("lightNodePool", &buffers.pool),
        ] {
            buffers.inputs.register(resource_name_key(name), buffer.clone());
        }
        buffers
    }

    /// Resolve the three named buffer inputs used by FUN_140351780's
    /// TLF callback, including the original terminal-NUL resource hash.
    pub fn resource(&self, key: u32) -> Option<&Buffer> {
        self.inputs.get(key)
    }

    /// Upload whole 48-byte packet records; inputs outside the native capacity
    /// or packet stride are rejected rather than truncated.
    pub fn upload_cull_packets(&self, queue: &RenderQueue, words: &[u32]) -> Result<(), String> {
        if words.len() % 12 != 0 {
            return Err("native TLF cull packets must contain 12 words each".into());
        }
        if words.len() / 12 > LIGHT_BUFFER_CAPACITY {
            return Err("native TLF cull packet count exceeds 448".into());
        }
        let bytes = words
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect::<Vec<_>>();
        if !bytes.is_empty() {
            queue.write_buffer(&self.packets, 0, &bytes);
        }
        Ok(())
    }

    /// Shared producer path from the prepared native lighting records. Preserve
    /// their index/order, use the original affine arithmetic and initialize all
    /// padding lanes before publishing the GPU buffer. Return the rounded count.
    pub fn upload_lighting_packets(
        &self, queue: &RenderQueue, view: &[[f32; 4]; 3], lighting: &[[u32; 28]],
    ) -> Result<usize, String> {
        let mut packets = [[0; 12]; LIGHT_BUFFER_CAPACITY];
        let count = prepare_light_cull_packets(view, lighting, &mut packets)?;
        let words = packets[..count].iter().flatten().copied().collect::<Vec<_>>();
        self.upload_cull_packets(queue, &words)?;
        Ok(count)
    }

    /// Match FUN_140351780: reset the separate allocation counter once before
    /// both normal and scattering culls. Do not clear the pool or reset between passes.
    pub fn begin_frame(&self, encoder: &mut CommandEncoder) {
        encoder.clear_buffer(&self.counter, 0, None);
    }
}
