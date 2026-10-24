//! Iggy UI composition: draws a Flash draw list over the display output,
//! after the post-processing set, as Iggy's GDraw draws over the backbuffer.
//! Colours are written in the display encoding the display pass produces.
//! Masks use a stencil attachment as GDraw D3D11 does: a batch's stencil
//! test bits must all be set (read mask = test, EQUAL 0xff), a mask batch
//! writes its set bits with REPLACE and no colour, a clear batch writes 0
//! (`FUN_1402e81b0`, `FUN_1402e7050`); every render starts cleared
//! (`FUN_1402e5b80`).
use crate::flash::player::CharRef;
use crate::flash::render::{Batch, DrawList, Texture as FillTexture, Vertex};
use bevy::{
    core_pipeline::{Core3d, Core3dSystems},
    prelude::*,
    render::{
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_resource::*,
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        view::ViewTarget,
        RenderApp,
    },
};
use std::collections::HashMap;
use std::sync::Arc;

pub const SHADER_SOURCE: &str = include_str!("../../../assets/shaders/dh2_flash.wgsl");

/// A decoded RGBA8 bitmap.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// One frame of UI to draw. `generation` identifies the player whose
/// library bitmaps `images` holds.
#[derive(Clone)]
pub struct FlashFrame {
    pub list: Arc<DrawList>,
    pub images: Arc<HashMap<CharRef, Arc<Image>>>,
    pub generation: u64,
}

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct FlashUiFrame(pub Option<FlashFrame>);

pub struct FlashUiPlugin;

impl Plugin for FlashUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FlashUiFrame>().add_plugins(ExtractResourcePlugin::<FlashUiFrame>::default());
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render
                .init_resource::<FlashUiGpu>()
                .add_systems(Core3d, render_flash_ui.after(Core3dSystems::PostProcess));
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum TextureKey {
    White,
    Bitmap(u64, CharRef),
    Ramp(Vec<u8>),
}

#[derive(Default, Resource)]
struct FlashUiGpu {
    programs: Option<Programs>,
    textures: HashMap<TextureKey, (Texture, TextureView)>,
    vertices: Option<(Buffer, u64)>,
    stage: Option<Buffer>,
    /// The stencil attachment and its size.
    stencil: Option<(Texture, TextureView, (u32, u32))>,
}

/// What selects a pipeline: additive blend, stencil test, set and clear bits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineKey {
    add: bool,
    test: u8,
    set: u8,
    clear: u8,
}

const STENCIL_FORMAT: TextureFormat = TextureFormat::Stencil8;

struct Programs {
    format: TextureFormat,
    stage_layout: BindGroupLayout,
    texture_layout: BindGroupLayout,
    layout: PipelineLayout,
    module: ShaderModule,
    /// Created when first used (GDraw also caches its stencil states).
    pipelines: HashMap<PipelineKey, RenderPipeline>,
    /// Samplers by (repeat, smooth).
    samplers: [Sampler; 4],
}

fn vertex_layout() -> RawVertexBufferLayout<'static> {
    const ATTRIBUTES: [VertexAttribute; 6] = [
        VertexAttribute { format: VertexFormat::Float32x2, offset: 0, shader_location: 0 },
        VertexAttribute { format: VertexFormat::Float32x2, offset: 8, shader_location: 1 },
        VertexAttribute { format: VertexFormat::Float32x4, offset: 16, shader_location: 2 },
        VertexAttribute { format: VertexFormat::Float32x4, offset: 32, shader_location: 3 },
        VertexAttribute { format: VertexFormat::Float32x2, offset: 48, shader_location: 4 },
        VertexAttribute { format: VertexFormat::Float32, offset: 56, shader_location: 5 },
    ];
    RawVertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: VertexStepMode::Vertex, attributes: &ATTRIBUTES }
}

impl Programs {
    fn new(device: &RenderDevice, format: TextureFormat) -> Self {
        let stage_layout = device.create_bind_group_layout(
            Some("flash stage"),
            &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                count: None,
                ty: BindingType::Buffer { ty: BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: BufferSize::new(16) },
            }],
        );
        let texture_layout = device.create_bind_group_layout(
            Some("flash fill texture"),
            &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    count: None,
                    ty: BindingType::Texture { sample_type: TextureSampleType::Float { filterable: true }, view_dimension: TextureViewDimension::D2, multisampled: false },
                },
                BindGroupLayoutEntry { binding: 1, visibility: ShaderStages::FRAGMENT, count: None, ty: BindingType::Sampler(SamplerBindingType::Filtering) },
            ],
        );
        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("flash ui"),
            bind_group_layouts: &[Some(&stage_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor { label: Some("flash ui"), source: ShaderSource::Wgsl(SHADER_SOURCE.into()) });
        let sampler = |repeat: bool, smooth: bool| {
            let address = if repeat { AddressMode::Repeat } else { AddressMode::ClampToEdge };
            let filter = if smooth { FilterMode::Linear } else { FilterMode::Nearest };
            device.create_sampler(&SamplerDescriptor {
                label: Some("flash fill"),
                address_mode_u: address,
                address_mode_v: address,
                min_filter: filter,
                mag_filter: filter,
                ..Default::default()
            })
        };
        Programs {
            format,
            stage_layout,
            texture_layout,
            layout,
            module,
            pipelines: HashMap::new(),
            samplers: [sampler(false, false), sampler(false, true), sampler(true, false), sampler(true, true)],
        }
    }

    fn pipeline(&mut self, device: &RenderDevice, key: PipelineKey) -> &RenderPipeline {
        let (format, layout, module) = (self.format, &self.layout, &self.module);
        self.pipelines.entry(key).or_insert_with(|| {
            let blend = if key.add {
                BlendState {
                    color: BlendComponent { src_factor: BlendFactor::SrcAlpha, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                    alpha: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                }
            } else {
                BlendState {
                    color: BlendComponent { src_factor: BlendFactor::SrcAlpha, dst_factor: BlendFactor::OneMinusSrcAlpha, operation: BlendOperation::Add },
                    alpha: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::OneMinusSrcAlpha, operation: BlendOperation::Add },
                }
            };
            // Mask and clear draws write no colour (the zeroed blend state).
            let write_mask = if key.set != 0 || key.clear != 0 { ColorWrites::empty() } else { ColorWrites::ALL };
            let face = StencilFaceState { compare: CompareFunction::Equal, fail_op: StencilOperation::Keep, depth_fail_op: StencilOperation::Keep, pass_op: StencilOperation::Replace };
            let stencil = if key.clear != 0 {
                StencilState { front: face, back: face, read_mask: 0, write_mask: key.clear as u32 }
            } else if key.test != 0 || key.set != 0 {
                StencilState { front: face, back: face, read_mask: key.test as u32, write_mask: key.set as u32 }
            } else {
                StencilState::default()
            };
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some("flash ui"),
                layout: Some(layout),
                vertex: RawVertexState { module, entry_point: Some("flash_vertex"), compilation_options: Default::default(), buffers: &[vertex_layout()] },
                fragment: Some(RawFragmentState {
                    module,
                    entry_point: Some("flash_fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(ColorTargetState { format, blend: Some(blend), write_mask })],
                }),
                primitive: PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(DepthStencilState { format: STENCIL_FORMAT, depth_write_enabled: None, depth_compare: None, stencil, bias: Default::default() }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        })
    }
}

fn upload(device: &RenderDevice, queue: &RenderQueue, width: u32, height: u32, rgba: &[u8]) -> (Texture, TextureView) {
    let texture = device.create_texture_with_data(
        queue,
        &TextureDescriptor {
            label: Some("flash bitmap"),
            size: Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        TextureDataOrder::MipMajor,
        rgba,
    );
    let view = texture.create_view(&TextureViewDescriptor::default());
    (texture, view)
}

fn vertex_bytes(vertices: &[Vertex]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vertices.len() * std::mem::size_of::<Vertex>());
    for v in vertices {
        for f in v.position.iter().chain(&v.uv).chain(&v.mul).chain(&v.add).chain(&v.params).chain(std::iter::once(&v.w)) {
            out.extend_from_slice(&f.to_le_bytes());
        }
    }
    debug_assert_eq!(out.len(), vertices.len() * std::mem::size_of::<Vertex>());
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn packed_vertices_match_the_layout_stride() {
        let v = crate::flash::render::Vertex { position: [1.0, 2.0], uv: [0.0; 2], mul: [1.0; 4], add: [0.0; 4], params: [0.0; 2], w: 1.0 };
        assert_eq!(super::vertex_bytes(&[v, v]).len(), 2 * super::vertex_layout().array_stride as usize);
    }
}

fn render_flash_ui(
    frame: Res<FlashUiFrame>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut gpu: ResMut<FlashUiGpu>,
    mut context: RenderContext,
    view: ViewQuery<&ViewTarget>,
) {
    let Some(frame) = &frame.0 else { return };
    if frame.list.batches.is_empty() {
        return;
    }
    let target = view.into_inner();
    let format = target.main_texture_format();
    if gpu.programs.as_ref().is_none_or(|p| p.format != format) {
        gpu.programs = Some(Programs::new(&device, format));
    }
    // Textures: library bitmaps per player generation, gradient ramps by content.
    let keys: Vec<TextureKey> = frame
        .list
        .batches
        .iter()
        .map(|b| match &b.texture {
            FillTexture::White => TextureKey::White,
            FillTexture::Bitmap(c) => TextureKey::Bitmap(frame.generation, *c),
            FillTexture::Ramp(i) => TextureKey::Ramp(frame.list.ramps[*i].to_vec()),
        })
        .collect();
    gpu.textures.retain(|k, _| !matches!(k, TextureKey::Bitmap(g, _) if *g != frame.generation));
    for key in &keys {
        if gpu.textures.contains_key(key) {
            continue;
        }
        let uploaded = match key {
            TextureKey::White => upload(&device, &queue, 1, 1, &[255; 4]),
            TextureKey::Ramp(bytes) => upload(&device, &queue, 64, 1, bytes),
            TextureKey::Bitmap(_, c) => match frame.images.get(c) {
                Some(image) => upload(&device, &queue, image.width, image.height, &image.rgba),
                None => continue,
            },
        };
        gpu.textures.insert(key.clone(), uploaded);
    }
    let bytes = vertex_bytes(&frame.list.vertices);
    let needed = bytes.len() as u64;
    if gpu.vertices.as_ref().is_none_or(|(_, size)| *size < needed) {
        let size = needed.next_power_of_two().max(4096);
        let buffer = device.create_buffer(&BufferDescriptor { label: Some("flash vertices"), size, usage: BufferUsages::VERTEX | BufferUsages::COPY_DST, mapped_at_creation: false });
        gpu.vertices = Some((buffer, size));
    }
    queue.write_buffer(&gpu.vertices.as_ref().unwrap().0, 0, &bytes);
    if gpu.stage.is_none() {
        gpu.stage = Some(device.create_buffer(&BufferDescriptor { label: Some("flash stage"), size: 16, usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST, mapped_at_creation: false }));
    }
    let mut stage_bytes = Vec::with_capacity(16);
    for f in [frame.list.stage.0, frame.list.stage.1, 1.0, 1.0] {
        stage_bytes.extend_from_slice(&f.to_le_bytes());
    }
    queue.write_buffer(gpu.stage.as_ref().unwrap(), 0, &stage_bytes);
    let size = target.main_texture().size();
    if gpu.stencil.as_ref().is_none_or(|s| s.2 != (size.width, size.height)) {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("flash stencil"),
            size: Extent3d { width: size.width, height: size.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: STENCIL_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        gpu.stencil = Some((texture, view, (size.width, size.height)));
    }
    let keys_by_batch: Vec<PipelineKey> = frame
        .list
        .batches
        .iter()
        .map(|b| PipelineKey { add: b.blend == 7, test: b.stencil_test, set: b.stencil_set, clear: b.stencil_clear })
        .collect();
    {
        let programs = gpu.programs.as_mut().unwrap();
        for key in &keys_by_batch {
            programs.pipeline(&device, *key);
        }
    }

    let gpu = &*gpu;
    let programs = gpu.programs.as_ref().unwrap();
    let stage_group = device.create_bind_group(Some("flash stage"), &programs.stage_layout, &BindGroupEntries::single(gpu.stage.as_ref().unwrap().as_entire_binding()));
    let groups: Vec<Option<BindGroup>> = frame
        .list
        .batches
        .iter()
        .zip(&keys)
        .map(|(b, key): (&Batch, &TextureKey)| {
            let (_, view) = gpu.textures.get(key)?;
            let sampler = &programs.samplers[(b.repeat as usize) * 2 + b.smooth as usize];
            Some(device.create_bind_group(Some("flash fill texture"), &programs.texture_layout, &BindGroupEntries::sequential((view, sampler))))
        })
        .collect();
    let encoder = context.command_encoder();
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("flash ui"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: target.main_texture_view(),
            depth_slice: None,
            resolve_target: None,
            ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
        })],
        depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
            view: &gpu.stencil.as_ref().unwrap().1,
            depth_ops: None,
            stencil_ops: Some(Operations { load: LoadOp::Clear(0), store: StoreOp::Discard }),
        }),
        ..Default::default()
    });
    pass.set_vertex_buffer(0, *gpu.vertices.as_ref().unwrap().0.slice(..));
    pass.set_bind_group(0, &stage_group, &[]);
    for ((batch, group), key) in frame.list.batches.iter().zip(&groups).zip(&keys_by_batch) {
        let Some(group) = group else { continue };
        pass.set_pipeline(&programs.pipelines[key]);
        // GDraw draws with reference 0xff; its clear draw with 0.
        pass.set_stencil_reference(if batch.stencil_clear != 0 { 0 } else { 0xff });
        pass.set_bind_group(1, group, &[]);
        pass.draw(batch.first..batch.first + batch.count, 0..1);
    }
}
