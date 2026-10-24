//! Iggy UI composition: draws a Flash draw list over the display output,
//! after the post-processing set, as Iggy's GDraw draws over the backbuffer.
//! Colours are written in the display encoding the display pass produces.
//! Masks use a stencil attachment as GDraw D3D11 does: a batch's stencil
//! test bits must all be set (read mask = test, EQUAL 0xff), a mask batch
//! writes its set bits with REPLACE and no colour, a clear batch writes 0
//! (`FUN_1402e81b0`, `FUN_1402e7050`); every render starts cleared
//! (`FUN_1402e5b80`).
use crate::flash::player::CharRef;
use crate::flash::filter_passes::{blur_axis, blur_pixels, FilterRecord, TapPass};
use crate::flash::render::{Batch, DrawList, Layer, Texture as FillTexture, Vertex};
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
pub const FILTER_SHADER_SOURCE: &str = include_str!("../../../assets/shaders/dh2_flash_filters.wgsl");

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

/// The filter passes' pipelines (tap blur and colour matrix).
struct FilterPrograms {
    layout: BindGroupLayout,
    blur: RenderPipeline,
    matrix: RenderPipeline,
    sampler: Sampler,
}

/// The filter shader's uniform block (`Pass` in dh2_flash_filters.wgsl).
const PASS_BYTES: u64 = 64 + 18 * 16 + 5 * 16;

impl FilterPrograms {
    fn new(device: &RenderDevice, format: TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(
            Some("flash filter"),
            &[
                BindGroupLayoutEntry { binding: 0, visibility: ShaderStages::VERTEX_FRAGMENT, count: None, ty: BindingType::Buffer { ty: BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: BufferSize::new(PASS_BYTES) } },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    count: None,
                    ty: BindingType::Texture { sample_type: TextureSampleType::Float { filterable: true }, view_dimension: TextureViewDimension::D2, multisampled: false },
                },
                BindGroupLayoutEntry { binding: 2, visibility: ShaderStages::FRAGMENT, count: None, ty: BindingType::Sampler(SamplerBindingType::Filtering) },
            ],
        );
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor { label: Some("flash filter"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor { label: Some("flash filter"), source: ShaderSource::Wgsl(FILTER_SHADER_SOURCE.into()) });
        let pipeline = |entry: &'static str| {
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: RawVertexState { module: &module, entry_point: Some("filter_vertex"), compilation_options: Default::default(), buffers: &[] },
                fragment: Some(RawFragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL })],
                }),
                primitive: PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("flash filter"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            ..Default::default()
        });
        FilterPrograms { layout, blur: pipeline("blur_fragment"), matrix: pipeline("matrix_fragment"), sampler }
    }
}

struct Programs {
    format: TextureFormat,
    filters: FilterPrograms,
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
            filters: FilterPrograms::new(device, format),
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
    fn layer_targets_map_stage_points_to_their_pixels() {
        use crate::flash::filter_passes::FilterRecord;
        let layer = crate::flash::render::Layer {
            first: 0,
            end: 1,
            bounds: [100.0, 200.0, 300.0, 260.0],
            filters: vec![FilterRecord::Blur { blur: [25.0, 25.0], passes: 2 }],
            mul: [1.0; 4],
            add: [0.0; 4],
            blend: 1,
            stencil_test: 0,
        };
        // A 1920x1080 stage on a 1280x720 target.
        let affine = [2.0 / 1920.0, -2.0 / 1080.0, -1.0, 1.0];
        let target = (1280, 720);
        let plan = super::plan_layer(&layer, affine, target);
        let size = ((plan.rect[2] - plan.rect[0]) as f32, (plan.rect[3] - plan.rect[1]) as f32);
        // A stage point lands where the display target has it, relative to the layer.
        let p = [100.0f32, 200.0];
        let display = super::to_pixels(p, affine, target);
        let ndc = [p[0] * plan.affine[0] + plan.affine[2], p[1] * plan.affine[1] + plan.affine[3]];
        let local = [(ndc[0] + 1.0) * 0.5 * size.0, (1.0 - ndc[1]) * 0.5 * size.1];
        assert!((local[0] - (display[0] - plan.rect[0] as f32)).abs() < 1e-3, "{local:?} {display:?} {:?}", plan.rect);
        assert!((local[1] - (display[1] - plan.rect[1] as f32)).abs() < 1e-3);
        // The blur grew the target beyond the content.
        assert!(plan.rect[0] < display[0] as i32 - 10);
        assert!(!plan.passes.is_empty());
        // The composite quad covers the target rectangle.
        let quad = super::composite_quad(plan.rect, affine, target, &layer);
        let corner = super::to_pixels(quad[0].position, affine, target);
        assert!((corner[0] - plan.rect[0] as f32).abs() < 1e-3 && (corner[1] - plan.rect[1] as f32).abs() < 1e-3);
    }

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
    for f in [2.0 / frame.list.stage.0, -2.0 / frame.list.stage.1, -1.0, 1.0] {
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
    // The layers' render targets and composite quads.
    let target_size = (size.width as i32, size.height as i32);
    let main_affine = [2.0 / frame.list.stage.0, -2.0 / frame.list.stage.1, -1.0, 1.0];
    let plans: Vec<LayerPlan> = frame.list.layers.iter().map(|l| plan_layer(l, main_affine, target_size)).collect();
    let mut all_vertices = frame.list.vertices.clone();
    let composite_first: Vec<u32> = plans
        .iter()
        .zip(&frame.list.layers)
        .map(|(plan, layer)| {
            let first = all_vertices.len() as u32;
            all_vertices.extend(composite_quad(plan.rect, main_affine, target_size, layer));
            first
        })
        .collect();
    if !frame.list.layers.is_empty() {
        let bytes = vertex_bytes(&all_vertices);
        let needed = bytes.len() as u64;
        if gpu.vertices.as_ref().is_none_or(|(_, size)| *size < needed) {
            let size = needed.next_power_of_two().max(4096);
            let buffer = device.create_buffer(&BufferDescriptor { label: Some("flash vertices"), size, usage: BufferUsages::VERTEX | BufferUsages::COPY_DST, mapped_at_creation: false });
            gpu.vertices = Some((buffer, size));
        }
        queue.write_buffer(&gpu.vertices.as_ref().unwrap().0, 0, &bytes);
    }
    let mut keys_by_batch: Vec<PipelineKey> = frame
        .list
        .batches
        .iter()
        .map(|b| PipelineKey { add: b.blend == 7, test: b.stencil_test, set: b.stencil_set, clear: b.stencil_clear })
        .collect();
    let composite_keys: Vec<PipelineKey> = frame.list.layers.iter().map(|l| PipelineKey { add: l.blend == 7, test: l.stencil_test, set: 0, clear: 0 }).collect();
    {
        let programs = gpu.programs.as_mut().unwrap();
        for key in keys_by_batch.iter().chain(&composite_keys) {
            programs.pipeline(&device, *key);
        }
    }
    keys_by_batch.shrink_to_fit();

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
    let draw = Draw {
        device: &device,
        programs,
        vertices: &gpu.vertices.as_ref().unwrap().0,
        batches: &frame.list.batches,
        groups: &groups,
        keys: &keys_by_batch,
        layers: &frame.list.layers,
        plans: &plans,
        composite_first: &composite_first,
        composite_keys: &composite_keys,
        format,
    };
    let encoder = context.command_encoder();
    let main = TargetViews { color: target.main_texture_view(), stencil: &gpu.stencil.as_ref().unwrap().1, stage: &stage_group, clear: false };
    draw.range(encoder, &main, 0, frame.list.batches.len() as u32, None);
}

/// A layer's render target: its rectangle in display-target pixels, the
/// stage mapping into it, its content rectangle and its filter passes.
struct LayerPlan {
    rect: [i32; 4],
    affine: [f32; 4],
    passes: Vec<FilterPass>,
}

enum FilterPass {
    Taps(TapPass),
    Matrix([f32; 20]),
}

/// Display-target pixels of a stage point.
fn to_pixels(p: [f32; 2], affine: [f32; 4], target: (i32, i32)) -> [f32; 2] {
    let ndc = [p[0] * affine[0] + affine[2], p[1] * affine[1] + affine[3]];
    [(ndc[0] + 1.0) * 0.5 * target.0 as f32, (1.0 - ndc[1]) * 0.5 * target.1 as f32]
}

/// The blur passes of `filters` over `content` in a target of `size`.
fn filter_passes(filters: &[FilterRecord], scale: [f32; 2], size: (i32, i32), mut content: [i32; 4]) -> Vec<FilterPass> {
    let mut clamp = [(content[0] - 1).max(0), (content[1] - 1).max(0), (content[2] + 1).min(size.0), (content[3] + 1).min(size.1)];
    let mut out = Vec::new();
    for filter in filters {
        match filter {
            FilterRecord::Blur { blur, passes } => {
                let (bx, by) = (blur_pixels(blur[0], scale[0]), blur_pixels(blur[1], scale[1]));
                // `FUN_1402e48c0`: per pass, the horizontal then the vertical axis.
                for _ in 0..*passes {
                    out.extend(blur_axis(0, bx, size, &mut content, &mut clamp).into_iter().map(FilterPass::Taps));
                    out.extend(blur_axis(1, by, size, &mut content, &mut clamp).into_iter().map(FilterPass::Taps));
                }
            }
            FilterRecord::ColourMatrix(m) => out.push(FilterPass::Matrix(*m)),
            FilterRecord::Unsupported => {}
        }
    }
    out
}

fn plan_layer(layer: &Layer, affine: [f32; 4], target: (i32, i32)) -> LayerPlan {
    let a = to_pixels([layer.bounds[0], layer.bounds[1]], affine, target);
    let b = to_pixels([layer.bounds[2], layer.bounds[3]], affine, target);
    let content = [a[0].min(b[0]).floor() as i32, a[1].min(b[1]).floor() as i32, a[0].max(b[0]).ceil() as i32, a[1].max(b[1]).ceil() as i32];
    // Stage pixels per display pixel (`P+0x1f54` / `P+0x1f58`).
    let scale = [affine[0].abs() * 0.5 * target.0 as f32, affine[1].abs() * 0.5 * target.1 as f32];
    // How far the passes grow the content: plan them in an unbounded target.
    let big = (1 << 20, 1 << 20);
    let mid = [big.0 / 2, big.1 / 2, big.0 / 2 + (content[2] - content[0]), big.1 / 2 + (content[3] - content[1])];
    let grown = filter_passes(&layer.filters, scale, big, mid).iter().fold(mid, |r, p| match p {
        FilterPass::Taps(t) => [r[0].min(t.destination[0]), r[1].min(t.destination[1]), r[2].max(t.destination[2]), r[3].max(t.destination[3])],
        FilterPass::Matrix(_) => r,
    });
    let margin = [mid[0] - grown[0] + 2, mid[1] - grown[1] + 2, grown[2] - mid[2] + 2, grown[3] - mid[3] + 2];
    let rect = [
        (content[0] - margin[0]).max(0),
        (content[1] - margin[1]).max(0),
        (content[2] + margin[2]).min(target.0).max(content[0].max(0) + 1),
        (content[3] + margin[3]).min(target.1).max(content[1].max(0) + 1),
    ];
    let size = (rect[2] - rect[0], rect[3] - rect[1]);
    let local = [content[0] - rect[0], content[1] - rect[1], content[2] - rect[0], content[3] - rect[1]];
    // ndc_rt = (ndc_main + 1)·W/w − 2·x0/w − 1 (and mirrored for y).
    let (w, h, tw, th) = (size.0 as f32, size.1 as f32, target.0 as f32, target.1 as f32);
    let layer_affine = [
        affine[0] * tw / w,
        affine[1] * th / h,
        (affine[2] + 1.0) * tw / w - 2.0 * rect[0] as f32 / w - 1.0,
        1.0 - (1.0 - affine[3]) * th / h + 2.0 * rect[1] as f32 / h,
    ];
    LayerPlan { rect, affine: layer_affine, passes: filter_passes(&layer.filters, scale, size, local) }
}

/// The composite quad of a layer in stage coordinates: the render target's
/// premultiplied texture (sampling mode 3) with the layer's colour transform.
fn composite_quad(rect: [i32; 4], affine: [f32; 4], target: (i32, i32), layer: &Layer) -> [Vertex; 6] {
    let stage = |px: f32, py: f32| {
        let ndc = [px / target.0 as f32 * 2.0 - 1.0, 1.0 - py / target.1 as f32 * 2.0];
        [(ndc[0] - affine[2]) / affine[0], (ndc[1] - affine[3]) / affine[1]]
    };
    let corner = |x: i32, y: i32, u: f32, v: f32| Vertex {
        position: stage(x as f32, y as f32),
        uv: [u, v],
        mul: layer.mul,
        add: layer.add,
        params: [crate::flash::render::mode::LINEAR_RAMP as f32, 0.0],
        w: 1.0,
    };
    let (a, b, c, d) = (corner(rect[0], rect[1], 0.0, 0.0), corner(rect[2], rect[1], 1.0, 0.0), corner(rect[2], rect[3], 1.0, 1.0), corner(rect[0], rect[3], 0.0, 1.0));
    [a, b, c, a, c, d]
}

struct TargetViews<'a> {
    color: &'a TextureView,
    stencil: &'a TextureView,
    stage: &'a BindGroup,
    /// A fresh layer target: the first pass clears it.
    clear: bool,
}

struct Draw<'a> {
    device: &'a RenderDevice,
    programs: &'a Programs,
    vertices: &'a Buffer,
    batches: &'a [Batch],
    groups: &'a [Option<BindGroup>],
    keys: &'a [PipelineKey],
    layers: &'a [Layer],
    plans: &'a [LayerPlan],
    composite_first: &'a [u32],
    composite_keys: &'a [PipelineKey],
    format: TextureFormat,
}

impl Draw<'_> {
    fn render_target(&self, size: (i32, i32), format: TextureFormat, label: &'static str) -> (Texture, TextureView) {
        let texture = self.device.create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d { width: size.0.max(1) as u32, height: size.1.max(1) as u32, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        (texture, view)
    }

    /// Draw batches `first..end` into `target`, layers included; `inside` is
    /// the layer being drawn (its nested layers only).
    fn range(&self, encoder: &mut CommandEncoder, target: &TargetViews, first: u32, end: u32, inside: Option<usize>) {
        let mut at = first;
        let mut clear = target.clear;
        let mut index = inside.map_or(0, |i| i + 1);
        while index < self.layers.len() {
            let layer = &self.layers[index];
            if layer.first >= end {
                break;
            }
            if layer.first < at {
                index += 1;
                continue;
            }
            self.batches_pass(encoder, target, at, layer.first, &mut clear);
            self.layer(encoder, target, index, &mut clear);
            at = layer.end;
            // Skip the layers nested in this one.
            index += 1;
            while index < self.layers.len() && self.layers[index].first < layer.end {
                index += 1;
            }
        }
        self.batches_pass(encoder, target, at, end, &mut clear);
    }


    fn batches_pass(&self, encoder: &mut CommandEncoder, target: &TargetViews, first: u32, end: u32, clear: &mut bool) {
        if first >= end && !*clear {
            return;
        }
        let (colour_load, stencil_load) = if *clear { (LoadOp::Clear(LinearRgba::NONE.into()), LoadOp::Clear(0)) } else { (LoadOp::Load, LoadOp::Load) };
        *clear = false;
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("flash ui"),
            color_attachments: &[Some(RenderPassColorAttachment { view: target.color, depth_slice: None, resolve_target: None, ops: Operations { load: colour_load, store: StoreOp::Store } })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment { view: target.stencil, depth_ops: None, stencil_ops: Some(Operations { load: stencil_load, store: StoreOp::Store }) }),
            ..Default::default()
        });
        pass.set_vertex_buffer(0, *self.vertices.slice(..));
        pass.set_bind_group(0, target.stage, &[]);
        for i in first as usize..end as usize {
            let (batch, Some(group)) = (&self.batches[i], &self.groups[i]) else { continue };
            pass.set_pipeline(&self.programs.pipelines[&self.keys[i]]);
            // GDraw draws with reference 0xff; its clear draw with 0.
            pass.set_stencil_reference(if batch.stencil_clear != 0 { 0 } else { 0xff });
            pass.set_bind_group(1, group, &[]);
            pass.draw(batch.first..batch.first + batch.count, 0..1);
        }
    }

    fn layer(&self, encoder: &mut CommandEncoder, target: &TargetViews, index: usize, clear: &mut bool) {
        let (layer, plan) = (&self.layers[index], &self.plans[index]);
        let size = (plan.rect[2] - plan.rect[0], plan.rect[3] - plan.rect[1]);
        if size.0 <= 0 || size.1 <= 0 {
            return;
        }
        let (_colour_texture, colour) = self.render_target(size, self.format, "flash layer");
        let (_stencil_texture, stencil) = self.render_target_stencil(size);
        let stage_buffer = self.device.create_buffer_with_data(&BufferInitDescriptor { label: Some("flash layer stage"), contents: &plan.affine.map(f32::to_le_bytes).concat(), usage: BufferUsages::UNIFORM });
        let stage = self.device.create_bind_group(Some("flash layer stage"), &self.programs.stage_layout, &BindGroupEntries::single(stage_buffer.as_entire_binding()));
        self.range(encoder, &TargetViews { color: &colour, stencil: &stencil, stage: &stage, clear: true }, layer.first, layer.end, Some(index));
        // The filter passes, each into a fresh target.
        let mut source = (_colour_texture, colour);
        for pass in &plan.passes {
            let destination = self.render_target(size, self.format, "flash filter");
            self.filter_pass(encoder, pass, &source.1, &destination.1, size);
            source = destination;
        }
        // Draw the result back.
        let group = self.device.create_bind_group(Some("flash layer composite"), &self.programs.texture_layout, &BindGroupEntries::sequential((&source.1, &self.programs.samplers[1])));
        let (colour_load, stencil_load) = if *clear { (LoadOp::Clear(LinearRgba::NONE.into()), LoadOp::Clear(0)) } else { (LoadOp::Load, LoadOp::Load) };
        *clear = false;
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("flash ui"),
            color_attachments: &[Some(RenderPassColorAttachment { view: target.color, depth_slice: None, resolve_target: None, ops: Operations { load: colour_load, store: StoreOp::Store } })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment { view: target.stencil, depth_ops: None, stencil_ops: Some(Operations { load: stencil_load, store: StoreOp::Store }) }),
            ..Default::default()
        });
        pass.set_vertex_buffer(0, *self.vertices.slice(..));
        pass.set_bind_group(0, target.stage, &[]);
        pass.set_pipeline(&self.programs.pipelines[&self.composite_keys[index]]);
        pass.set_stencil_reference(0xff);
        pass.set_bind_group(1, &group, &[]);
        let first = self.composite_first[index];
        pass.draw(first..first + 6, 0..1);
    }

    fn render_target_stencil(&self, size: (i32, i32)) -> (Texture, TextureView) {
        let texture = self.device.create_texture(&TextureDescriptor {
            label: Some("flash layer stencil"),
            size: Extent3d { width: size.0.max(1) as u32, height: size.1.max(1) as u32, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: STENCIL_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        (texture, view)
    }

    fn filter_pass(&self, encoder: &mut CommandEncoder, pass: &FilterPass, source: &TextureView, destination: &TextureView, size: (i32, i32)) {
        let ndc = |r: [i32; 4]| [r[0] as f32 / size.0 as f32 * 2.0 - 1.0, 1.0 - r[1] as f32 / size.1 as f32 * 2.0, r[2] as f32 / size.0 as f32 * 2.0 - 1.0, 1.0 - r[3] as f32 / size.1 as f32 * 2.0];
        let mut data = Vec::with_capacity(PASS_BYTES as usize);
        let mut floats = |v: &[f32]| {
            for f in v {
                data.extend_from_slice(&f.to_le_bytes());
            }
        };
        let (pipeline, count) = match pass {
            FilterPass::Taps(t) => {
                floats(&ndc(t.destination));
                floats(&t.uv);
                floats(&t.clamp);
                (&self.programs.filters.blur, t.taps.len().min(18) as u32)
            }
            FilterPass::Matrix(_) => {
                floats(&ndc([0, 0, size.0, size.1]));
                floats(&[0.0, 0.0, 1.0, 1.0]);
                floats(&[0.0, 0.0, 1.0, 1.0]);
                (&self.programs.filters.matrix, 0)
            }
        };
        for v in [count, 0, 0, 0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let mut taps = [0.0f32; 18 * 4];
        let mut rows = [0.0f32; 20];
        match pass {
            FilterPass::Taps(t) => {
                for (i, tap) in t.taps.iter().take(18).enumerate() {
                    taps[i * 4..i * 4 + 3].copy_from_slice(tap);
                }
            }
            FilterPass::Matrix(m) => rows = *m,
        }
        for f in taps.iter().chain(&rows) {
            data.extend_from_slice(&f.to_le_bytes());
        }
        let buffer = self.device.create_buffer_with_data(&BufferInitDescriptor { label: Some("flash filter pass"), contents: &data, usage: BufferUsages::UNIFORM });
        let group = self.device.create_bind_group(Some("flash filter pass"), &self.programs.filters.layout, &BindGroupEntries::sequential((buffer.as_entire_binding(), source, &self.programs.filters.sampler)));
        let mut render = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("flash filter"),
            color_attachments: &[Some(RenderPassColorAttachment { view: destination, depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Clear(LinearRgba::NONE.into()), store: StoreOp::Store } })],
            ..Default::default()
        });
        render.set_pipeline(pipeline);
        render.set_bind_group(0, &group, &[]);
        render.draw(0..6, 0..1);
    }
}
