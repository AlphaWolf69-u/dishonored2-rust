//! Native shadow depth-array storage and comparison state.
//!
//! Gameplay capture dh2_frame1145447.rdc records D24S8 arrays and a
//! linear/comparison/Less/clamp sampler. The shader declaration
//! `smp_shadowhardwarepcfsampler` in dh2_frame400867.shadow-buffers.json selects
//! sampler 16599; the point sampler belongs to the separate depth-copy draw.
//! Enums are resolved in artifacts/renderdoc/shadow-enums.json. Sizes and layer counts belong
//! to the original shadow owner, not defaults in this storage helper. Allocation
//! happens when that owner changes; this does not clear or redraw static cache
//! entries every frame, nor does it enable Bevy's shadow rendering.
use bevy::render::{
    render_resource::{
        AddressMode, CompareFunction, Extent3d, FilterMode, MipmapFilterMode, Sampler,
        SamplerDescriptor, Texture, TextureAspect, TextureDescriptor, TextureDimension,
        TextureFormat, TextureUsages, TextureView, TextureViewDescriptor, TextureViewDimension,
    },
    renderer::RenderDevice,
};
use bevy::{
    asset::RenderAssetUsages,
    image::{Image, ImageSampler},
};

pub struct ShadowDepthArray {
    pub texture: Texture,
    pub sampled: TextureView,
    pub layers: Vec<TextureView>,
    pub size: Extent3d,
}

impl ShadowDepthArray {
    /// Allocate from the actual native quality/CVar owner, not a capture.
    /// Native false/true image templates resolve to D16 / D24S8 respectively;
    /// the "32 bits" CVar includes stencil and does not request D32F.
    pub fn parallel(
        device: &RenderDevice,
        slice_size: u32,
        divider: u32,
        cascade_count: u32,
        depth_32_bits: bool,
        label: &str,
    ) -> Result<Self, String> {
        let target = crate::render_shadow::ParallelShadowTargetDimensions::from_settings(
            slice_size,
            divider,
            cascade_count,
        )?;
        let format = match crate::render_shadow::parallel_shadow_depth_format(depth_32_bits)? {
            crate::render_shadow::ParallelShadowDepthFormat::Depth16 => TextureFormat::Depth16Unorm,
            crate::render_shadow::ParallelShadowDepthFormat::Depth24Stencil8 => {
                TextureFormat::Depth24PlusStencil8
            }
        };
        Self::new_with_format(
            device,
            Extent3d {
                width: target.side,
                height: target.side,
                depth_or_array_layers: target.layers,
            },
            format,
            label,
        )
    }

    pub fn new(device: &RenderDevice, size: Extent3d, label: &str) -> Result<Self, String> {
        Self::new_with_format(device, size, TextureFormat::Depth24PlusStencil8, label)
    }

    pub fn new_with_format(
        device: &RenderDevice,
        size: Extent3d,
        format: TextureFormat,
        label: &str,
    ) -> Result<Self, String> {
        if !matches!(
            format,
            TextureFormat::Depth16Unorm | TextureFormat::Depth24PlusStencil8
        ) {
            return Err("unsupported parallel shadow depth-array format".into());
        }
        let limits = device.limits();
        if size.width == 0
            || size.height == 0
            || size.depth_or_array_layers == 0
            || size.width > limits.max_texture_dimension_2d
            || size.height > limits.max_texture_dimension_2d
            || size.depth_or_array_layers > limits.max_texture_array_layers
        {
            return Err("invalid shadow depth-array dimensions".into());
        }
        // Native shadow descriptors explicitly request one mip. The shared
        // original finalizer therefore preserves all four fields (2e98a0).
        let mut mip_counts = [1; 4];
        let family = if format == TextureFormat::Depth16Unorm {
            2
        } else {
            22
        };
        crate::texture::initialize_image_mip_counts(
            family,
            [size.width as i32, size.height as i32, 1],
            &mut mip_counts,
        );
        let texture = device.create_texture(&TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: mip_counts[0] as u32,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC
                | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        Self::from_owned_texture(texture, label)
    }

    /// Whole native cache-resource transfer (2bb270 -> D3D11 CopyResource).
    /// Match the actual owner's complete descriptor; this is not the separate
    /// per-slice copy or depth-copy shader, and must not silently select layer 0.
    pub fn copy_resource_from(
        &self,
        encoder: &mut CommandEncoder,
        source: &Self,
    ) -> Result<(), String> {
        if self.texture.id() == source.texture.id() {
            return Err("shadow cache cannot copy a resource onto itself".into());
        }
        if self.size != source.size || self.texture.format() != source.texture.format() {
            return Err(
                "whole-resource shadow copy requires matching format and dimensions".into(),
            );
        }
        if !source.texture.usage().contains(TextureUsages::COPY_SRC)
            || !self.texture.usage().contains(TextureUsages::COPY_DST)
        {
            return Err(
                "shadow cache owner did not allocate copy source/destination usage".into(),
            );
        }
        encoder.copy_texture_to_texture(
            source.texture.as_image_copy(),
            self.texture.as_image_copy(),
            self.size,
        );
        Ok(())
    }

    /// Full-rectangle subresource transfer used by FUN_14032db10's native
    /// fast copy branch (+0x170 = ID3D11DeviceContext::CopySubresourceRegion).
    /// Only equal-size, equal-format layers are supported here; cropped or
    /// shader-transformed transfers must use their original depth-copy draw.
    pub fn copy_layer_from(
        &self,
        encoder: &mut CommandEncoder,
        destination_layer: u32,
        source: &Self,
        source_layer: u32,
    ) -> Result<(), String> {
        if destination_layer >= self.size.depth_or_array_layers
            || source_layer >= source.size.depth_or_array_layers
            || self.size.width != source.size.width
            || self.size.height != source.size.height
            || self.texture.format() != source.texture.format()
        {
            return Err("full-layer copy requires matching in-range subresources".into());
        }
        if self.texture.id() == source.texture.id() {
            return Err("layer copy requires distinct owner resources".into());
        }
        if !source.texture.usage().contains(TextureUsages::COPY_SRC)
            || !self.texture.usage().contains(TextureUsages::COPY_DST)
        {
            return Err("layer copy owner did not allocate copy usage".into());
        }
        let mut input = source.texture.as_image_copy();
        input.origin.z = source_layer;
        let mut output = self.texture.as_image_copy();
        output.origin.z = destination_layer;
        encoder.copy_texture_to_texture(
            input,
            output,
            Extent3d {
                width: self.size.width,
                height: self.size.height,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    /// Attach views to the renderer's existing image allocation. Rendering and
    /// material sampling must alias this SAME texture, not two lookalike arrays.
    pub fn from_owned_texture(texture: Texture, label: &str) -> Result<Self, String> {
        let size = texture.size();
        if !matches!(
            texture.format(),
            TextureFormat::Depth16Unorm | TextureFormat::Depth24PlusStencil8
        ) || texture.dimension() != TextureDimension::D2
            || texture.sample_count() != 1
            || texture.mip_level_count() != 1
            || !texture
                .usage()
                .contains(TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING)
        {
            return Err("invalid shadow image allocation".into());
        }
        let sampled = texture.create_view(&TextureViewDescriptor {
            label: Some(label),
            dimension: Some(TextureViewDimension::D2Array),
            aspect: TextureAspect::DepthOnly,
            ..Default::default()
        });
        let layers = (0..size.depth_or_array_layers)
            .map(|layer| {
                texture.create_view(&TextureViewDescriptor {
                    label: Some(label),
                    dimension: Some(TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        Ok(Self {
            texture,
            sampled,
            layers,
            size,
        })
    }
}

/// The original hardware-PCF sampler. The optimized 3x3/5x5 shader coefficients
/// rely on bilinear comparison filtering; a point sampler gives the wrong kernel.
pub fn shadow_comparison_sampler(device: &RenderDevice) -> Sampler {
    device.create_sampler(&shadow_comparison_descriptor())
}

pub fn shadow_comparison_descriptor() -> SamplerDescriptor<'static> {
    SamplerDescriptor {
        label: Some("shadow linear/Less/clamp comparison"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Nearest,
        lod_max_clamp: 13.0,
        compare: Some(CompareFunction::Less),
        ..Default::default()
    }
}

/// A renderer-owned depth asset with the SAME sampled view and hardware-PCF
/// sampler as the raster target. It contains no captured or CPU-uploaded pixels.
/// The 1x1 unavailable binding uses this too; sampling is disabled by the shared
/// receiver header, rather than relying on Bevy's noncomparison fallback image.
pub fn shadow_depth_image(size: Extent3d, format: TextureFormat) -> Result<Image, String> {
    if size.width == 0
        || size.height == 0
        || size.depth_or_array_layers == 0
        || !matches!(
            format,
            TextureFormat::Depth16Unorm | TextureFormat::Depth24PlusStencil8
        )
    {
        return Err("invalid shadow depth-image descriptor".into());
    }
    let mut image = Image::new_uninit(
        size,
        TextureDimension::D2,
        format,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage = TextureUsages::RENDER_ATTACHMENT
        | TextureUsages::TEXTURE_BINDING
        | TextureUsages::COPY_SRC
        | TextureUsages::COPY_DST;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        aspect: TextureAspect::DepthOnly,
        ..Default::default()
    });
    image.sampler = ImageSampler::Descriptor(shadow_comparison_descriptor().into());
    Ok(image)
}

/// Original `bilinearsampler.decl`: linear min/mag, point mip, wrap U/V/W.
/// RenderDoc smp_bilinearsampler descriptor confirms LOD 0..13, no bias or
/// comparison. Shadow alpha must not reuse the forward anisotropic sampler.
pub fn shadow_alpha_sampler(device: &RenderDevice) -> Sampler {
    device.create_sampler(&SamplerDescriptor {
        label: Some("shadow diffuse bilinear/wrap sampler"),
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        address_mode_w: AddressMode::Repeat,
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Nearest,
        lod_max_clamp: 13.0,
        ..Default::default()
    })
}

/// Installed renderparm/anisotropicsampler.decl explicitly requests all three
/// filters ANISO, WRAP U/V/W and MaxAnisotropy 4. FUN_140378280 uses that own
/// value when mip-filter code is 2; it is NOT inferred from a capture or the
/// user's image_anisotropy. Original default LOD range is 0..13, no compare.
pub fn shadow_vista_sampler(device: &RenderDevice) -> Sampler {
    device.create_sampler(&SamplerDescriptor {
        label: Some("Vista diffuse explicit anisotropicsampler"),
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        address_mode_w: AddressMode::Repeat,
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        anisotropy_clamp: 4,
        lod_max_clamp: 13.0,
        ..Default::default()
    })
}

use bevy::render::{
    render_resource::{
        BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayoutEntry, BindingResource,
        BindingType, Buffer, BufferBinding, BufferBindingType, BufferUsages, CommandEncoder,
        DepthBiasState, DepthStencilState, DownlevelFlags, Face, FrontFace, IndexFormat, LoadOp,
        MultisampleState, Operations, PipelineLayout, PipelineLayoutDescriptor, PolygonMode,
        PrimitiveState, RawFragmentState, RawRenderPipelineDescriptor, RawVertexBufferLayout,
        RawVertexState, RenderPassDepthStencilAttachment, RenderPassDescriptor, RenderPipeline,
        SamplerBindingType, ShaderModuleDescriptor, ShaderSource, ShaderStages,
        StencilState, StoreOp, TextureSampleType, VertexAttribute, VertexFormat,
        VertexStepMode,
    },
    renderer::RenderAdapter,
};
use std::{num::NonZeroU64, ops::Range};

/// Stride of the captured fixture, not a shader requirement. The position-only
/// native program can use the existing rewrite mesh owner's layout unchanged.
pub const SHADOW_OPAQUE_VERTEX_STRIDE: u64 = 32;
const SHADOW_PER_INSTANCE_BYTES: u64 = 80;

pub use super::pipeline_state::{depth_stencil_gpu, mesh_raster as shadow_mesh_raster};

/// A native depth draw with caller-prepared model state and vertex streams.
/// Index format, ranges and scissor are explicit. Rigid opaque/cutout and the
/// verified vegetation program are supported; skin/paraboloid remain unmapped.
pub struct ShadowDepthDraw<'a> {
    pub model_state: &'a BindGroup,
    /// Byte offset in the shared per-frame constant ring. Static bindings use 0.
    pub model_state_offset: u32,
    /// Program-specific group 1: original diffuse/sampler/UV for cutouts;
    /// remapped instance IDs and 48-byte matrix records for WorldOpaque.
    pub program_resources: Option<&'a BindGroup>,
    pub vertex_buffer: &'a Buffer,
    pub index_buffer: &'a Buffer,
    pub index_format: IndexFormat,
    pub indices: Range<u32>,
    pub base_vertex: i32,
    pub instances: Range<u32>,
    pub scissor: ShadowScissor,
}

/// Scissor-on raster state captured for the native model-shadow draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShadowScissor {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Verified native vertex/fragment pairs; layout is part of each program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowDepthProgram {
    Skinned {
        influences: u8,
        alpha: bool,
        dither: bool,
    },
    Opaque,
    /// vertex.decl useMultiInstancing=1, no skinning/look-at/wind/alpha branch.
    WorldOpaque,
    WorldAlpha,
    WorldVistaAlpha,
    /// Installed dfe9d6cb: real per-instance current wind, remap and matrix rows.
    WorldVegetation,
    WorldVegetationFixed,
    RigidAlpha,
    /// Installed VS14200/PS14201: raw color.xyz/normal.xy and 208-byte CB.
    Vegetation,
}

impl ShadowDepthProgram {
    pub fn model_uniform_bytes(self) -> u64 {
        match self {
            Self::Skinned { .. } => 112,
            Self::Opaque | Self::RigidAlpha => SHADOW_PER_INSTANCE_BYTES,
            Self::WorldOpaque | Self::WorldAlpha | Self::WorldVistaAlpha => 96,
            Self::Vegetation | Self::WorldVegetation | Self::WorldVegetationFixed => 208,
        }
    }
}

/// Keep the original per-draw CB bytes, selecting their ring slice dynamically
/// instead of allocating a descriptor for every draw/program combination.
fn shadow_pipeline_layout(
    device: &RenderDevice,
    program: ShadowDepthProgram,
) -> PipelineLayout {
    let uniform = |binding, bytes, dynamic| BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::VERTEX_FRAGMENT,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: NonZeroU64::new(bytes),
        },
        count: None,
    };
    let storage = |binding| BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::VERTEX,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let texture = |binding| BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty: BindingType::Texture {
            sample_type: TextureSampleType::Float { filterable: true },
            view_dimension: TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let sampler = |binding| BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty: BindingType::Sampler(SamplerBindingType::Filtering),
        count: None,
    };
    let model = device.create_bind_group_layout(
        "shadow constant ring slice",
        &[uniform(0, program.model_uniform_bytes(), true)],
    );
    let entries = match program {
        ShadowDepthProgram::Opaque => Vec::new(),
        ShadowDepthProgram::RigidAlpha => vec![texture(0), sampler(1), uniform(2, 16, false)],
        ShadowDepthProgram::Vegetation => vec![texture(0), sampler(1)],
        ShadowDepthProgram::WorldOpaque => vec![storage(0), storage(1)],
        ShadowDepthProgram::WorldAlpha
        | ShadowDepthProgram::WorldVistaAlpha
        | ShadowDepthProgram::WorldVegetationFixed => {
            vec![storage(0), storage(1), texture(2), sampler(3)]
        }
        ShadowDepthProgram::WorldVegetation => {
            vec![storage(0), storage(1), texture(2), sampler(3), storage(4)]
        }
        ShadowDepthProgram::Skinned { alpha: true, .. } => {
            vec![storage(0), texture(1), sampler(2)]
        }
        ShadowDepthProgram::Skinned { alpha: false, .. } => vec![storage(0)],
    };
    let resources = (!entries.is_empty())
        .then(|| device.create_bind_group_layout("shadow program resources", &entries));
    let mut layouts = vec![Some(&*model)];
    if let Some(resources) = &resources {
        layouts.push(Some(&**resources));
    }
    device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("shadow program with dynamic constant ring"),
        bind_group_layouts: &layouts,
        immediate_size: 0,
    })
}

/// Cached shadow depth pipeline. The program selects vertex streams and uniform layout.
pub struct ShadowDepthPipeline {
    pipeline: RenderPipeline,
    format: TextureFormat,
    alpha_test: bool,
    program: ShadowDepthProgram,
    stencil_reference: u32,
}

impl ShadowDepthPipeline {
    /// Build the original-backed opaque vertex pipeline. Bias and the complete
    /// owner vertex layout are explicit inputs, not defaults. The capture used
    /// `DepthBiasState { constant: 80, slope_scale: 5.0, clamp: 1000.0 }`; this
    /// is not assumed for other instances. Nonzero clamp is accepted only when
    /// the active adapter reports `DEPTH_BIAS_CLAMP` support.
    pub fn new(
        device: &RenderDevice,
        adapter: &RenderAdapter,
        vertex_layout: &RawVertexBufferLayout<'_>,
        bias: DepthBiasState,
    ) -> Result<Self, String> {
        Self::new_with_format(
            device,
            adapter,
            vertex_layout,
            bias,
            TextureFormat::Depth24PlusStencil8,
        )
    }

    /// Match the actual original image template, including the D16 branch.
    pub fn new_with_format(
        device: &RenderDevice,
        adapter: &RenderAdapter,
        vertex_layout: &RawVertexBufferLayout<'_>,
        bias: DepthBiasState,
        format: TextureFormat,
    ) -> Result<Self, String> {
        Self::new_with_raster_state(
            device,
            adapter,
            vertex_layout,
            bias,
            format,
            FrontFace::Cw,
            Some(Face::Back),
        )
    }

    /// Use the owner's resolved culling and mesh representation. Native DH2
    /// indices have CW fronts; game.rs reverses them once for Bevy's CCW
    /// representation (including reflected-instance variants). Reusing those
    /// GPU allocations must therefore use CCW, not the capture's CW state.
    pub fn new_with_raster_state(
        device: &RenderDevice,
        adapter: &RenderAdapter,
        vertex_layout: &RawVertexBufferLayout<'_>,
        bias: DepthBiasState,
        format: TextureFormat,
        front_face: FrontFace,
        cull_mode: Option<Face>,
    ) -> Result<Self, String> {
        Self::with_program(
            device,
            adapter,
            vertex_layout,
            bias,
            format,
            front_face,
            cull_mode,
            ShadowDepthProgram::Opaque,
            None,
        )
    }

    /// Original world instancing: the owner provides original VP rows, remap
    /// offset, visible-ID buffer and the MatrixInstanceType array. Host positions
    /// use game.rs's exact game_to_view signed-axis permutation.
    pub fn new_world_with_raster_state(
        device: &RenderDevice,
        adapter: &RenderAdapter,
        vertex_layout: &RawVertexBufferLayout<'_>,
        bias: DepthBiasState,
        format: TextureFormat,
        front_face: FrontFace,
        cull_mode: Option<Face>,
    ) -> Result<Self, String> {
        Self::with_program(
            device,
            adapter,
            vertex_layout,
            bias,
            format,
            front_face,
            cull_mode,
            ShadowDepthProgram::WorldOpaque,
            None,
        )
    }

    /// Installed world cutout VS/PS: original instanced positions and raw UV,
    /// followed by layer0 scale/bias and the raw diffuse alpha test.
    pub fn new_world_alpha_with_raster_state(
        device: &RenderDevice,
        adapter: &RenderAdapter,
        vertex_layout: &RawVertexBufferLayout<'_>,
        bias: DepthBiasState,
        format: TextureFormat,
        front_face: FrontFace,
        cull_mode: Option<Face>,
    ) -> Result<Self, String> {
        Self::with_program(
            device,
            adapter,
            vertex_layout,
            bias,
            format,
            front_face,
            cull_mode,
            ShadowDepthProgram::WorldAlpha,
            None,
        )
    }

    /// Installed dff3e1d3/1a8eaf20 pair: raw UV (no Layer0 MAD),
    /// anisotropicsampler, strict alpha < 0.5 discard.
    pub fn new_world_vista_with_raster_state(
        device: &RenderDevice,
        adapter: &RenderAdapter,
        vertex_layout: &RawVertexBufferLayout<'_>,
        bias: DepthBiasState,
        format: TextureFormat,
        front_face: FrontFace,
        cull_mode: Option<Face>,
    ) -> Result<Self, String> {
        Self::with_program(
            device,
            adapter,
            vertex_layout,
            bias,
            format,
            front_face,
            cull_mode,
            ShadowDepthProgram::WorldVistaAlpha,
            None,
        )
    }

    pub fn new_world_vegetation_with_raster_state(
        device: &RenderDevice,
        adapter: &RenderAdapter,
        vertex_layout: &RawVertexBufferLayout<'_>,
        bias: DepthBiasState,
        format: TextureFormat,
        front_face: FrontFace,
        cull_mode: Option<Face>,
    ) -> Result<Self, String> {
        Self::with_program(
            device,
            adapter,
            vertex_layout,
            bias,
            format,
            front_face,
            cull_mode,
            ShadowDepthProgram::WorldVegetation,
            None,
        )
    }

    pub fn new_world_fixed_vegetation_with_raster_state(
        device: &RenderDevice,
        adapter: &RenderAdapter,
        vertex_layout: &RawVertexBufferLayout<'_>,
        bias: DepthBiasState,
        format: TextureFormat,
        front_face: FrontFace,
        cull_mode: Option<Face>,
    ) -> Result<Self, String> {
        Self::with_program(
            device,
            adapter,
            vertex_layout,
            bias,
            format,
            front_face,
            cull_mode,
            ShadowDepthProgram::WorldVegetationFixed,
            None,
        )
    }

    /// arkDefault's rigid alpha-test shadow path. The layout's location 1 is
    /// the owner's selected layer-zero UV stream; it is not always UV0.
    pub fn new_alpha_with_raster_state(
        device: &RenderDevice,
        adapter: &RenderAdapter,
        vertex_layout: &RawVertexBufferLayout<'_>,
        bias: DepthBiasState,
        format: TextureFormat,
        front_face: FrontFace,
        cull_mode: Option<Face>,
    ) -> Result<Self, String> {
        Self::with_program(
            device,
            adapter,
            vertex_layout,
            bias,
            format,
            front_face,
            cull_mode,
            ShadowDepthProgram::RigidAlpha,
            None,
        )
    }

    /// The installed vegetation shadow program. The owner must supply the
    /// original color.xyz and normal.xy stream values, not forward albedo or
    /// a guessed decoded normal, and the complete 208-byte PerInstanceCB.
    pub fn new_vegetation_with_raster_state(
        device: &RenderDevice,
        adapter: &RenderAdapter,
        vertex_layout: &RawVertexBufferLayout<'_>,
        bias: DepthBiasState,
        format: TextureFormat,
        front_face: FrontFace,
        cull_mode: Option<Face>,
    ) -> Result<Self, String> {
        Self::with_program(
            device,
            adapter,
            vertex_layout,
            bias,
            format,
            front_face,
            cull_mode,
            ShadowDepthProgram::Vegetation,
            None,
        )
    }

    /// The same evidenced rigid position program, with the main-Z owner's
    /// actual prepared state/reference and D32S8 target. Instance CB bytes are
    /// still caller-owned: a model's stencil flag is NOT its shader multiplier.
    /// This covers rigid opaque geometry only, not a guessed alpha/skin variant.
    pub fn new_main_opaque_depth(
        device: &RenderDevice,
        adapter: &RenderAdapter,
        vertex_layout: &RawVertexBufferLayout<'_>,
        state: &crate::render_state::RenderState,
        stencil_reference: u32,
        front_face: FrontFace,
        cull_mode: Option<Face>,
    ) -> Result<Self, String> {
        Self::new_prepared_shadow_depth(
            device,
            adapter,
            vertex_layout,
            state,
            stencil_reference,
            TextureFormat::Depth32FloatStencil8,
            front_face,
            cull_mode,
            ShadowDepthProgram::Opaque,
        )
    }

    /// Submit the owner's complete native prepared state, not a reconstructed
    /// LESS/write-enabled shadow preset. FUN_1402996f0 retains material+0x2c8
    /// depth/stencil fields; FUN_140293470 also controls depth clipping.
    /// Winding/culling remain explicit because imported mesh representation
    /// can differ from native index winding. Unsupported platform state fails.
    pub fn new_prepared_shadow_depth(
        device: &RenderDevice,
        adapter: &RenderAdapter,
        vertex_layout: &RawVertexBufferLayout<'_>,
        state: &crate::render_state::RenderState,
        stencil_reference: u32,
        format: TextureFormat,
        front_face: FrontFace,
        cull_mode: Option<Face>,
        program: ShadowDepthProgram,
    ) -> Result<Self, String> {
        use bevy::render::settings::WgpuFeatures;
        let features = device.wgpu_device().features();
        if format == TextureFormat::Depth32FloatStencil8
            && !features.contains(WgpuFeatures::DEPTH32FLOAT_STENCIL8)
        {
            return Err("depth requires enabled DEPTH32FLOAT_STENCIL8".into());
        }
        let raster = crate::render_state::raster_state(state);
        let polygon_mode = if raster.fill_mode == 2 {
            PolygonMode::Line
        } else {
            PolygonMode::Fill
        };
        if polygon_mode == PolygonMode::Line && !features.contains(WgpuFeatures::POLYGON_MODE_LINE)
        {
            return Err("wireframe depth requires enabled POLYGON_MODE_LINE".into());
        }
        if !raster.depth_clip && !features.contains(WgpuFeatures::DEPTH_CLIP_CONTROL) {
            return Err("unclipped depth requires enabled DEPTH_CLIP_CONTROL".into());
        }
        if raster.antialiased_line {
            return Err("antialiased-line depth rasterization is not mapped".into());
        }
        let primitive = PrimitiveState {
            front_face,
            cull_mode,
            polygon_mode,
            unclipped_depth: !raster.depth_clip,
            ..Default::default()
        };
        let depth = depth_stencil_gpu(state, format)?;
        let mut pipeline = Self::with_program(
            device,
            adapter,
            vertex_layout,
            depth.bias,
            depth.format,
            front_face,
            cull_mode,
            program,
            Some((depth, primitive)),
        )?;
        pipeline.stencil_reference = stencil_reference;
        Ok(pipeline)
    }

    fn with_program(
        device: &RenderDevice,
        adapter: &RenderAdapter,
        vertex_layout: &RawVertexBufferLayout<'_>,
        bias: DepthBiasState,
        format: TextureFormat,
        front_face: FrontFace,
        cull_mode: Option<Face>,
        program: ShadowDepthProgram,
        resolved_state: Option<(DepthStencilState, PrimitiveState)>,
    ) -> Result<Self, String> {
        if !matches!(
            format,
            TextureFormat::Depth16Unorm | TextureFormat::Depth24PlusStencil8
        ) && !(resolved_state.is_some() && format == TextureFormat::Depth32FloatStencil8)
        {
            return Err("unsupported rigid depth format".into());
        }
        let alpha_test = matches!(
            program,
            ShadowDepthProgram::Skinned { alpha: true, .. }
                | ShadowDepthProgram::RigidAlpha
                | ShadowDepthProgram::Vegetation
                | ShadowDepthProgram::WorldAlpha
                | ShadowDepthProgram::WorldVegetation
                | ShadowDepthProgram::WorldVegetationFixed
                | ShadowDepthProgram::WorldVistaAlpha
        );
        if matches!(
            program,
            ShadowDepthProgram::Vegetation
                | ShadowDepthProgram::WorldVegetation
                | ShadowDepthProgram::WorldVegetationFixed
        ) {
            validate_vegetation_vertex_layout(vertex_layout)?;
        } else {
            validate_opaque_vertex_layout(vertex_layout)?;
            if alpha_test {
                validate_alpha_vertex_layout(vertex_layout)?;
            }
        }
        if let ShadowDepthProgram::Skinned {
            influences,
            alpha,
            dither,
        } = program
        {
            if !matches!(influences, 4 | 8) || (dither && !alpha) {
                return Err("unsupported skin shader inputs".into());
            }
            for (location, format) in [
                (2, VertexFormat::Uint8x4),
                (3, VertexFormat::Unorm8x4),
                (4, VertexFormat::Uint8x4),
                (5, VertexFormat::Unorm8x4),
            ] {
                if !vertex_layout
                    .attributes
                    .iter()
                    .any(|a| a.shader_location == location && a.format == format)
                {
                    return Err(
                        "skin shader requires original UINT8 joints and UNORM8 weights"
                            .into(),
                    );
                }
            }
        }
        validate_opaque_depth_bias(bias, adapter.get_downlevel_capabilities().flags)?;
        let (depth_state, primitive) = resolved_state.unwrap_or((
            DepthStencilState {
                format,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::Less),
                stencil: StencilState::default(),
                bias,
            },
            PrimitiveState {
                front_face,
                cull_mode,
                ..Default::default()
            },
        ));
        let world_wind_helpers = concat!(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/shaders/dh2_animate_vegetation.wgsl"
            )),
            "\n",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/shaders/dh2_shadow_world_project.wgsl"
            ))
        );
        let helpers = match program {
            ShadowDepthProgram::WorldVegetation
            | ShadowDepthProgram::WorldVegetationFixed => world_wind_helpers,
            ShadowDepthProgram::WorldOpaque
            | ShadowDepthProgram::WorldAlpha
            | ShadowDepthProgram::WorldVistaAlpha => include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/shaders/dh2_shadow_world_project.wgsl"
            )),
            ShadowDepthProgram::Vegetation => include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/shaders/dh2_animate_vegetation.wgsl"
            )),
            _ => "",
        };
        let skin_defines = if let ShadowDepthProgram::Skinned {
            influences, dither, ..
        } = program
        {
            format!("const SKIN_INFLUENCES:u32={influences}u;\nconst SKIN_DITHER:bool={dither};\n")
        } else {
            String::new()
        };
        let shader_source = format!(
            "{skin_defines}{helpers}\n{}",
            match program {
                ShadowDepthProgram::Skinned { .. } => include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/assets/shaders/dh2_shadow_skin.wgsl"
                )),
                ShadowDepthProgram::Opaque => {
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/assets/shaders/dh2_shadow_opaque.wgsl"
                    ))
                }
                ShadowDepthProgram::WorldOpaque
                | ShadowDepthProgram::WorldAlpha
                | ShadowDepthProgram::WorldVistaAlpha => {
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/assets/shaders/dh2_shadow_world.wgsl"
                    ))
                }
                ShadowDepthProgram::WorldVegetation
                | ShadowDepthProgram::WorldVegetationFixed => {
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/assets/shaders/dh2_shadow_world_vegetation.wgsl"
                    ))
                }
                ShadowDepthProgram::RigidAlpha => {
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/assets/shaders/dh2_shadow_alpha.wgsl"
                    ))
                }
                ShadowDepthProgram::Vegetation => {
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/assets/shaders/dh2_shadow_vegetation.wgsl"
                    ))
                }
            }
        );
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("resolved shadow vertex program"),
            source: ShaderSource::Wgsl(shader_source.into()),
        });
        let layout = shadow_pipeline_layout(device, program);
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("resolved rigid depth pipeline"),
            layout: Some(&layout),
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some(
                    if matches!(
                        program,
                        ShadowDepthProgram::WorldAlpha
                            | ShadowDepthProgram::WorldVistaAlpha
                    ) {
                        "vertex_alpha"
                    } else if program == ShadowDepthProgram::WorldVegetationFixed {
                        "vertex_fixed"
                    } else {
                        "vertex"
                    },
                ),
                compilation_options: Default::default(),
                buffers: std::slice::from_ref(vertex_layout),
            },
            fragment: alpha_test.then_some(RawFragmentState {
                module: &shader,
                entry_point: Some(if program == ShadowDepthProgram::WorldVistaAlpha {
                    "fragment_vista"
                } else {
                    "fragment"
                }),
                compilation_options: Default::default(),
                targets: &[],
            }),
            primitive,
            multisample: MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
            depth_stencil: Some(depth_state),
        });
        Ok(Self {
            pipeline,
            format,
            alpha_test,
            program,
            stencil_reference: 0,
        })
    }

    /// Full installed world-wind ABI, including CURRENT wind in each 32-byte
    /// previous/current record. No zero/fixed-wind fallback is permitted.
    pub fn bind_skinned_instances(
        &self,
        device: &RenderDevice,
        palette: &Buffer,
        diffuse: Option<(&TextureView, &Sampler)>,
    ) -> Result<BindGroup, String> {
        let ShadowDepthProgram::Skinned { alpha, .. } = self.program else {
            return Err("skin palette needs a skin program".into());
        };
        if alpha != diffuse.is_some()
            || palette.size() == 0
            || palette.size() % 48 != 0
            || !palette.usage().contains(BufferUsages::STORAGE)
        {
            return Err("skin palette/material layout mismatch".into());
        }
        let mut entries = vec![BindGroupEntry {
            binding: 0,
            resource: palette.as_entire_binding(),
        }];
        if let Some((image, sampler)) = diffuse {
            entries.push(BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(image),
            });
            entries.push(BindGroupEntry {
                binding: 2,
                resource: BindingResource::Sampler(sampler),
            });
        }
        Ok(BindGroup::from(device.wgpu_device().create_bind_group(
            &BindGroupDescriptor {
                label: Some("skeletal shadow palette/material"),
                layout: &self.pipeline.get_bind_group_layout(1),
                entries: &entries,
            },
        )))
    }

    pub fn bind_world_vegetation_instances(
        &self,
        device: &RenderDevice,
        visible_indices: &Buffer,
        matrices: &Buffer,
        winds: &Buffer,
        diffuse: &TextureView,
        sampler: &Sampler,
    ) -> Result<BindGroup, String> {
        if self.program != ShadowDepthProgram::WorldVegetation {
            return Err("world wind buffers require the installed world vegetation program".into());
        }
        for (buffer, stride) in [(visible_indices, 4), (matrices, 48), (winds, 32)] {
            if buffer.size() == 0
                || buffer.size() % stride != 0
                || !buffer.usage().contains(BufferUsages::STORAGE)
            {
                return Err("world vegetation buffer size/usage differs from native ABI".into());
            }
        }
        Ok(BindGroup::from(device.wgpu_device().create_bind_group(
            &BindGroupDescriptor {
                label: Some("world vegetation remap/matrices/current-wind/diffuse"),
                layout: &self.pipeline.get_bind_group_layout(1),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: visible_indices.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: matrices.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(diffuse),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::Sampler(sampler),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: winds.as_entire_binding(),
                    },
                ],
            },
        )))
    }

    pub fn bind_world_instances(
        &self,
        device: &RenderDevice,
        visible_indices: &Buffer,
        matrices: &Buffer,
    ) -> Result<BindGroup, String> {
        if self.program != ShadowDepthProgram::WorldOpaque {
            return Err(
                "world instance buffers require the native multi-instancing program".into(),
            );
        }
        if visible_indices.size() == 0
            || visible_indices.size() % 4 != 0
            || matrices.size() == 0
            || matrices.size() % 48 != 0
            || !visible_indices.usage().contains(BufferUsages::STORAGE)
            || !matrices.usage().contains(BufferUsages::STORAGE)
        {
            return Err("world shadows need uint32 IDs and 48-byte MatrixInstanceType storage records".into());
        }
        Ok(BindGroup::from(device.wgpu_device().create_bind_group(
            &BindGroupDescriptor {
                label: Some("world shadow visible IDs and matrices"),
                layout: &self.pipeline.get_bind_group_layout(1),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: visible_indices.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: matrices.as_entire_binding(),
                    },
                ],
            },
        )))
    }

    /// The same actual world buffers plus the installed cutout material.
    pub fn bind_world_alpha_instances(
        &self,
        device: &RenderDevice,
        visible_indices: &Buffer,
        matrices: &Buffer,
        diffuse: &TextureView,
        sampler: &Sampler,
    ) -> Result<BindGroup, String> {
        if !matches!(
            self.program,
            ShadowDepthProgram::WorldAlpha
                | ShadowDepthProgram::WorldVegetationFixed
                | ShadowDepthProgram::WorldVistaAlpha
        ) {
            return Err(
                "world alpha resources require a compatible installed world program".into(),
            );
        }
        if visible_indices.size() == 0
            || visible_indices.size() % 4 != 0
            || matrices.size() == 0
            || matrices.size() % 48 != 0
            || !visible_indices.usage().contains(BufferUsages::STORAGE)
            || !matrices.usage().contains(BufferUsages::STORAGE)
        {
            return Err("invalid world cutout instance/UV buffers".into());
        }
        Ok(BindGroup::from(device.wgpu_device().create_bind_group(
            &BindGroupDescriptor {
                label: Some("world cutout IDs/matrices/diffuse/UV"),
                layout: &self.pipeline.get_bind_group_layout(1),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: visible_indices.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: matrices.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(diffuse),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::Sampler(sampler),
                    },
                ],
            },
        )))
    }

    /// The caller supplies the resolved original $diffusemap/$bilinearSampler
    /// and a 16-byte layer-zero scale/bias buffer. No forward cutoff, tint,
    /// fallback texture or guessed addressing mode enters the shadow pass.
    pub fn bind_alpha_material(
        &self,
        device: &RenderDevice,
        diffuse: &TextureView,
        sampler: &Sampler,
        uv_scale_bias: &Buffer,
    ) -> Result<BindGroup, String> {
        if self.program != ShadowDepthProgram::RigidAlpha {
            return Err(
                "separate layer-zero UV binding requires the rigid alpha shadow program".into(),
            );
        }
        if uv_scale_bias.size() < 16 || !uv_scale_bias.usage().contains(BufferUsages::UNIFORM) {
            return Err("shadow layer-zero scale/bias needs 16 uniform bytes".into());
        }
        Ok(BindGroup::from(device.wgpu_device().create_bind_group(
            &BindGroupDescriptor {
                label: Some("rigid shadow diffuse-alpha material"),
                layout: &self.pipeline.get_bind_group_layout(1),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(diffuse),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Sampler(sampler),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::Buffer(BufferBinding {
                            buffer: uv_scale_bias,
                            offset: 0,
                            size: NonZeroU64::new(16),
                        }),
                    },
                ],
            },
        )))
    }

    /// Bind the vegetation program's diffuse/bilinear resources. Its UV
    /// scale/bias is in the owner's model CB at byte 0, not a third material
    /// binding. The supplied sampler must be the resolved original sampler.
    pub fn bind_vegetation_material(
        &self,
        device: &RenderDevice,
        diffuse: &TextureView,
        sampler: &Sampler,
    ) -> Result<BindGroup, String> {
        if self.program != ShadowDepthProgram::Vegetation {
            return Err(
                "vegetation material binding requires the vegetation shadow program".into(),
            );
        }
        Ok(BindGroup::from(device.wgpu_device().create_bind_group(
            &BindGroupDescriptor {
                label: Some("vegetation shadow diffuse/bilinear material"),
                layout: &self.pipeline.get_bind_group_layout(1),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(diffuse),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Sampler(sampler),
                    },
                ],
            },
        )))
    }

    /// Make a cached binding for the selected program's caller-prepared CB.
    /// Rigid: 80 bytes, W/Z/Y/X rows at 0/16/32/48, depthhack at 64.
    /// Vegetation: 208 bytes, wind fields/UV followed by Z/Y/X/W rows and time.
    pub fn bind_prepared_model_state(
        &self,
        device: &RenderDevice,
        prepared_model_state: &Buffer,
    ) -> Result<BindGroup, String> {
        self.bind_prepared_model_state_at(device, prepared_model_state, 0)
    }

    /// Bind one unchanged native record from a persistent uniform pool. A
    /// layer owner can upload many matrices in one write rather than allocate
    /// a buffer per surface or recreate bindings during every shadow redraw.
    /// The offset obeys the active backend's uniform-binding alignment.
    pub fn bind_prepared_model_state_at(
        &self,
        device: &RenderDevice,
        prepared_model_state: &Buffer,
        offset: u64,
    ) -> Result<BindGroup, String> {
        let required = self.program.model_uniform_bytes();
        if offset
            .checked_add(required)
            .is_none_or(|end| end > prepared_model_state.size())
        {
            return Err(format!(
                "native {:?} shadow PerInstanceCB needs {required} bytes at offset {offset}, buffer has {}",
                self.program,
                prepared_model_state.size()
            ));
        }
        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        if offset % alignment != 0 {
            return Err(format!(
                "opaque shadow PerInstanceCB offset {offset} is not aligned to {alignment} bytes"
            ));
        }
        if !prepared_model_state.usage().contains(BufferUsages::UNIFORM) {
            return Err("opaque shadow PerInstanceCB buffer lacks UNIFORM usage".into());
        }
        Ok(BindGroup::from(device.wgpu_device().create_bind_group(
            &BindGroupDescriptor {
                label: Some("opaque shadow prepared PerInstanceCB"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: prepared_model_state,
                        offset,
                        size: NonZeroU64::new(required),
                    }),
                }],
            },
        )))
    }

    /// Record indexed opaque model draws into a cached depth-array layer view.
    /// The caller supplies load/store operations. Empty cached reuse is a no-op;
    /// explicit clears/discards still execute even when no caster was selected.
    /// Otherwise an empty dirty layer retains stale occluders instead of the
    /// original owner's clear depth. Each draw uses existing mesh buffers,
    /// a prepared state binding, an explicit index type/range, and a scissor.
    pub fn draw_layer(
        &self,
        encoder: &mut CommandEncoder,
        depths: &ShadowDepthArray,
        layer: u32,
        depth_ops: Operations<f32>,
        stencil_ops: Option<Operations<u32>>,
        draws: &[ShadowDepthDraw<'_>],
    ) -> Result<(), String> {
        if depths.texture.format() != self.format {
            return Err("shadow pipeline and depth-array formats differ".into());
        }
        if self.format == TextureFormat::Depth16Unorm && stencil_ops.is_some() {
            return Err("D16 parallel shadow target has no stencil aspect".into());
        }
        let Some(view) = depths.layers.get(layer as usize) else {
            return Err(format!(
                "opaque shadow layer {layer} is outside {} cached layers",
                depths.layers.len()
            ));
        };
        self.draw_depth_view(
            encoder,
            view,
            [depths.size.width, depths.size.height],
            depth_ops,
            stencil_ops,
            draws,
        )
    }

    /// Submit resolved rigid model draws to an owner's actual main-Z view or
    /// cached shadow layer. No geometry copying or depth-only stencil fallback.
    pub fn draw_depth_view(
        &self,
        encoder: &mut CommandEncoder,
        view: &TextureView,
        dimensions: [u32; 2],
        depth_ops: Operations<f32>,
        stencil_ops: Option<Operations<u32>>,
        draws: &[ShadowDepthDraw<'_>],
    ) -> Result<(), String> {
        let sequence: Vec<_> = draws.iter().map(|draw| (self, draw)).collect();
        Self::draw_depth_sequence(
            encoder,
            view,
            dimensions,
            None,
            depth_ops,
            stencil_ops,
            &sequence,
        )
    }

    /// D3D's ordered pipeline changes do not require separate attachment
    /// passes. Keep one attachment open until the native cache/copy boundary.
    pub fn draw_layer_sequence(
        encoder: &mut CommandEncoder,
        depths: &ShadowDepthArray,
        layer: u32,
        depth_ops: Operations<f32>,
        stencil_ops: Option<Operations<u32>>,
        draws: &[(&Self, &ShadowDepthDraw<'_>)],
    ) -> Result<(), String> {
        if draws
            .iter()
            .any(|(pipeline, _)| pipeline.format != depths.texture.format())
        {
            return Err("shadow pipeline and depth-array formats differ".into());
        }
        if depths.texture.format() == TextureFormat::Depth16Unorm && stencil_ops.is_some() {
            return Err("D16 parallel shadow target has no stencil aspect".into());
        }
        let view = depths
            .layers
            .get(layer as usize)
            .ok_or("shadow sequence layer is absent")?;
        Self::draw_depth_sequence(
            encoder,
            view,
            [depths.size.width, depths.size.height],
            None,
            depth_ops,
            stencil_ops,
            draws,
        )
    }

    ///32cea0 installs the allocated inclusive rectangle as both viewport
    ///and scissor before drawing a local side. Attachment clearing belongs
    ///to32d6e0's whole-atlas boundary; a region draw must retain its peers.
    pub fn draw_layer_region_sequence(
        encoder: &mut CommandEncoder,
        depths: &ShadowDepthArray,
        layer: u32,
        viewport: ShadowScissor,
        draws: &[(&Self, &ShadowDepthDraw<'_>)],
    ) -> Result<(), String> {
        if draws
            .iter()
            .any(|(pipeline, _)| pipeline.format != depths.texture.format())
        {
            return Err("local shadow pipeline and atlas formats differ".into());
        }
        let view = depths
            .layers
            .get(layer as usize)
            .ok_or("local shadow atlas layer is absent")?;
        Self::draw_depth_sequence(
            encoder,
            view,
            [depths.size.width, depths.size.height],
            Some(viewport),
            Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
            (depths.texture.format() == TextureFormat::Depth24PlusStencil8).then_some(Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            }),
            draws,
        )
    }

    fn draw_depth_sequence(
        encoder: &mut CommandEncoder,
        view: &TextureView,
        dimensions: [u32; 2],
        viewport: Option<ShadowScissor>,
        depth_ops: Operations<f32>,
        stencil_ops: Option<Operations<u32>>,
        draws: &[(&Self, &ShadowDepthDraw<'_>)],
    ) -> Result<(), String> {
        if let Some(region) = viewport {
            if region.width == 0
                || region.height == 0
                || region
                    .x
                    .checked_add(region.width)
                    .is_none_or(|v| v > dimensions[0])
                || region
                    .y
                    .checked_add(region.height)
                    .is_none_or(|v| v > dimensions[1])
            {
                return Err("local shadow viewport is outside its atlas layer".into());
            }
        }
        for (pipeline, draw) in draws {
            let needs_resources = pipeline.alpha_test
                || pipeline.program == ShadowDepthProgram::WorldOpaque
                || matches!(pipeline.program, ShadowDepthProgram::Skinned { .. });
            if needs_resources != draw.program_resources.is_some() {
                return Err("shadow draw and program resource bindings differ".into());
            }
            let scissor = draw.scissor;
            let right = scissor.x.checked_add(scissor.width);
            let bottom = scissor.y.checked_add(scissor.height);
            if scissor.width == 0
                || scissor.height == 0
                || right.is_none_or(|right| right > dimensions[0])
                || bottom.is_none_or(|bottom| bottom > dimensions[1])
            {
                return Err(format!(
                    "opaque shadow scissor is outside layer: {scissor:?}"
                ));
            }
            if draw.indices.start >= draw.indices.end
                || (draw.indices.end - draw.indices.start) % 3 != 0
                || draw.instances.start >= draw.instances.end
            {
                return Err(
                    "opaque shadow requires nonempty indexed triangle and instance ranges"
                        .into(),
                );
            }
        }
        if draws.is_empty()
            && matches!(depth_ops.load, LoadOp::Load)
            && depth_ops.store == StoreOp::Store
            && stencil_ops
                .as_ref()
                .is_none_or(|ops| matches!(ops.load, LoadOp::Load) && ops.store == StoreOp::Store)
        {
            return Ok(());
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("opaque model-shadow layer draws"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(depth_ops),
                stencil_ops,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let mut previous: Option<&Self> = None;
        let mut previous_scissor = None;
        let mut previous_vertex = None;
        let mut previous_index = None;
        let mut previous_resources = None;
        let mut previous_uniform = None;
        if let Some(region) = viewport {
            pass.set_viewport(
                region.x as f32,
                region.y as f32,
                region.width as f32,
                region.height as f32,
                0.0,
                1.0,
            );
        }
        for &(pipeline, draw) in draws {
            if previous.is_none_or(|old| !std::ptr::eq(old, pipeline)) {
                pass.set_pipeline(&pipeline.pipeline);
                pass.set_stencil_reference(pipeline.stencil_reference);
                previous = Some(pipeline);
                previous_resources = None;
                previous_uniform = None;
            }
            if previous_scissor != Some(draw.scissor) {
                pass.set_scissor_rect(
                    draw.scissor.x,
                    draw.scissor.y,
                    draw.scissor.width,
                    draw.scissor.height,
                );
                previous_scissor = Some(draw.scissor);
            }
            let uniform = (draw.model_state.id(), draw.model_state_offset);
            if previous_uniform != Some(uniform) {
                pass.set_bind_group(0, draw.model_state, &[draw.model_state_offset]);
                previous_uniform = Some(uniform);
            }
            if let Some(resources) = draw.program_resources {
                if previous_resources != Some(resources.id()) {
                    pass.set_bind_group(1, resources, &[]);
                    previous_resources = Some(resources.id());
                }
            }
            if previous_vertex != Some(draw.vertex_buffer.id()) {
                pass.set_vertex_buffer(0, *draw.vertex_buffer.slice(..));
                previous_vertex = Some(draw.vertex_buffer.id());
            }
            let index = (draw.index_buffer.id(), draw.index_format);
            if previous_index != Some(index) {
                pass.set_index_buffer(*draw.index_buffer.slice(..), draw.index_format);
                previous_index = Some(index);
            }
            pass.draw_indexed(
                draw.indices.clone(),
                draw.base_vertex,
                draw.instances.clone(),
            );
        }
        Ok(())
    }
}

fn validate_opaque_depth_bias(
    bias: DepthBiasState,
    adapter_flags: DownlevelFlags,
) -> Result<(), String> {
    if !bias.slope_scale.is_finite() || !bias.clamp.is_finite() {
        return Err("opaque shadow depth bias must be finite".into());
    }
    if bias.clamp != 0.0 && !adapter_flags.contains(DownlevelFlags::DEPTH_BIAS_CLAMP) {
        return Err(format!(
            "opaque shadow depth-bias clamp {} is unsupported: adapter lacks DownlevelFlags::DEPTH_BIAS_CLAMP",
            bias.clamp
        ));
    }
    Ok(())
}

fn validate_opaque_vertex_layout(layout: &RawVertexBufferLayout<'_>) -> Result<(), String> {
    if layout.array_stride == 0 || layout.array_stride % 4 != 0 {
        return Err(
            "opaque shadow vertex stride must be nonzero and four-byte aligned".into(),
        );
    }
    if layout.step_mode != VertexStepMode::Vertex {
        return Err("opaque shadow position stream must use per-vertex stepping".into());
    }
    let positions: Vec<&VertexAttribute> = layout
        .attributes
        .iter()
        .filter(|attribute| attribute.shader_location == 0)
        .collect();
    if positions.len() != 1
        || positions[0].format != VertexFormat::Float32x3
        || positions[0].offset % 4 != 0
        || positions[0]
            .offset
            .checked_add(12)
            .is_none_or(|end| end > layout.array_stride)
    {
        return Err(
            "opaque shadow requires one aligned Float32x3 position at location 0 within the owner's stride"
                .into(),
        );
    }
    Ok(())
}

fn validate_alpha_vertex_layout(layout: &RawVertexBufferLayout<'_>) -> Result<(), String> {
    let mut uv = layout
        .attributes
        .iter()
        .filter(|attribute| attribute.shader_location == 1);
    let Some(attribute) = uv.next() else {
        return Err("alpha shadow mesh lacks layer-zero UV stream".into());
    };
    if uv.next().is_some()
        || attribute.format != VertexFormat::Float32x2
        || attribute.offset % 4 != 0
        || attribute
            .offset
            .checked_add(8)
            .is_none_or(|end| end > layout.array_stride)
    {
        return Err("alpha shadow needs one aligned Float32x2 UV at location 1".into());
    }
    Ok(())
}

fn validate_vegetation_vertex_layout(layout: &RawVertexBufferLayout<'_>) -> Result<(), String> {
    validate_opaque_vertex_layout(layout)?;
    validate_alpha_vertex_layout(layout)?;
    for (location, format, bytes, name) in [
        (2, VertexFormat::Float32x3, 12, "raw color.xyz"),
        (3, VertexFormat::Float32x2, 8, "original normal.xy"),
    ] {
        let mut fields = layout
            .attributes
            .iter()
            .filter(|field| field.shader_location == location);
        let Some(field) = fields.next() else {
            return Err(format!(
                "vegetation shadow mesh lacks {name} at location {location}"
            ));
        };
        if fields.next().is_some()
            || field.format != format
            || field.offset % 4 != 0
            || field
                .offset
                .checked_add(bytes)
                .is_none_or(|end| end > layout.array_stride)
        {
            return Err(format!(
                "vegetation shadow needs one aligned {format:?} {name} at location {location}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "shadow_pipeline_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "shadow_draw_state_tests.rs"]
mod prepared_state_tests;

#[cfg(test)]
#[path = "vegetation_shadow_tests.rs"]
mod vegetation_tests;

#[cfg(test)]
#[path = "world_shadow_tests.rs"]
mod world_tests;

#[cfg(test)]
#[path = "world_wind_shadow_tests.rs"]
mod world_wind_tests;

#[cfg(test)]
#[path = "vista_shadow_tests.rs"]
mod vista_tests;

#[cfg(test)]
#[path = "skinned_shadow_tests.rs"]
mod skin_tests;
