//! Native 64x64 R32G32 adaptive coarse-depth production and asynchronous readback.
//!
//! It ports FUN_1402fbe40's four-mip pyramid plus FUN_1402fc6a0's separate
//! 64x64 grid. FUN_1402fc130's 16x16-group 120x68 tile map is a different
//! resource and is not implemented here.
//!
//! FUN_140351780 tries four staging textures newest-to-oldest with nonblocking
//! Map (FUN_1402eac80). Its later opaque task copies to (current+1)&3 unless
//! the owner's offscreen flag is set. This module ports the source reduction,
//! coarse-depth dispatch, persistent staging ring and mapped-frame mailbox; it
//! never waits synchronously for a GPU copy.

use bevy::{
    app::{App, Plugin},
    core_pipeline::{Core3d, Core3dSystems},
    ecs::prelude::*,
    render::{
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_resource::{
            BindGroup, BindGroupDescriptor, BindGroupEntry, BindingResource, Buffer,
            BufferDescriptor, BufferUsages, CommandEncoder, ComputePassDescriptor, Extent3d,
            MapMode, TexelCopyBufferInfo, TexelCopyBufferLayout, Texture, TextureDescriptor,
            TextureDimension, TextureFormat, TextureId, TextureUsages, TextureView,
            TextureViewDescriptor,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        view::{ExtractedView, ViewDepthTexture},
        GpuResourceAppExt, Render, RenderApp, RenderSystems,
    },
};
use std::sync::{Arc, Mutex};

pub const COARSE_DEPTH_DIAGNOSTICS_ENV: &str = "DH2_COARSE_DEPTH_DIAGNOSTICS";
pub const COARSE_DEPTH_SIZE: u32 = 64;
pub const COARSE_DEPTH_BYTES: u64 = 64 * 64 * 8;

pub fn diagnostics_enabled() -> bool {
    std::env::var(COARSE_DEPTH_DIAGNOSTICS_ENV).as_deref() == Ok("1")
}
/// FUN_14035a0c0 initializes `WndCtx+0x80` to this sentinel before the first
/// copy. The original first staging destination is `(sentinel + 1) & 3`.
pub const INITIAL_COARSE_DEPTH_OWNER_INDEX: u32 = 4;
pub type CoarseDepthPixels = [[f32; 2]; 64 * 64];

/// Mark the one original gameplay view whose completed main-pass depth owns
/// the directional coarse-depth producer. This is not attached to shadow views.
#[derive(Component, ExtractComponent, Clone, Copy, Debug, Default)]
pub struct CoarseDepthSource;

/// Matrices/extent from the exact main-view depth submission copied into a
/// native four-slot readback entry. Arrays are row-major native matrices.
#[derive(Clone, Debug)]
pub struct CoarseDepthFrameMetadata {
    pub sequence: u64,
    pub source_extent: [u32; 2],
    pub projection: crate::render_shadow::ShadowMatrix,
    pub inverse_projection: crate::render_shadow::ShadowMatrix,
    pub view_from_world: crate::render_shadow::ShadowMatrix,
}

/// One mapped native 64×64 grid and the view matrices that produced it.
#[derive(Clone)]
pub struct CoarseDepthSnapshot {
    pub metadata: CoarseDepthFrameMetadata,
    pub pixels: Arc<CoarseDepthPixels>,
}

/// Same-process mailbox shared with the render world through extraction. It
/// carries only an actually mapped GPU result; absent data remains absent.
#[derive(Resource, Clone, ExtractResource, Default)]
pub struct CoarseDepthMailbox(Arc<Mutex<Option<Arc<CoarseDepthSnapshot>>>>);

impl CoarseDepthMailbox {
    pub fn latest(&self) -> Result<Option<Arc<CoarseDepthSnapshot>>, String> {
        self.0
            .lock()
            .map(|snapshot| snapshot.clone())
            .map_err(|_| "coarse-depth mailbox is poisoned".into())
    }

    fn publish(&self, snapshot: CoarseDepthSnapshot) -> Result<(), String> {
        let mut latest = self
            .0
            .lock()
            .map_err(|_| "coarse-depth mailbox is poisoned".to_owned())?;
        if latest
            .as_ref()
            .is_none_or(|old| old.metadata.sequence < snapshot.metadata.sequence)
        {
            *latest = Some(Arc::new(snapshot));
        }
        Ok(())
    }
}

/// FUN_14036d0c0's downsampled image: clamp the framebuffer to at least eight,
/// then round its half-size up to eight. Exactly four RGBA16F mips are owned.
/// Other images allocated by that function are not represented here.
pub fn depth_pyramid_extent(framebuffer: [u32; 2]) -> Result<[u32; 2], String> {
    if framebuffer
        .iter()
        .any(|v| *v == 0 || *v > i32::MAX as u32 - 15)
    {
        return Err("depth pyramid has an invalid framebuffer extent".into());
    }
    Ok(framebuffer.map(|v| ((v.max(8) + 15) >> 1) & !7))
}

/// Coverage actually written by one dispatch. A smaller active viewport can
/// leave old pixels outside these extents in the persistent texture. The
/// native consumer must retain its own viewport scale; never sample those
/// pixels by assuming the full allocated mip was refreshed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepthReductionCoverage {
    pub allocated_mips: [[u32; 2]; 4],
    pub written_mips: [[u32; 2]; 4],
}

/// Persistent image and bindings for FUN_1402fbe40, not four independent
/// downscale dispatches or hardware-generated average-only mips.
/// The real source depth owner is mandatory; no constant/captured substitute.
pub struct DepthPyramid {
    texture: Texture,
    views: [bevy::render::render_resource::TextureView; 4],
    constants: Buffer,
    binding: bevy::render::render_resource::BindGroup,
    source_extent: [u32; 2],
}

impl DepthPyramid {
    pub fn new(
        device: &RenderDevice,
        pipelines: &crate::tiled_lighting_gpu::TlfPipelines,
        source: &Texture,
    ) -> Result<Self, String> {
        use bevy::render::render_resource::*;
        if source.dimension() != TextureDimension::D2
            || source.depth_or_array_layers() != 1
            || source.sample_count() != 1
            || !source.format().has_depth_aspect()
            || !source.usage().contains(TextureUsages::TEXTURE_BINDING)
        {
            return Err(
                "depth reduction requires the actual single-sample 2D depth input".into(),
            );
        }
        let source_extent = [source.width(), source.height()];
        let [width, height] = depth_pyramid_extent(source_extent)?;
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("four-mip DownsampledLinearDepth"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 4,
            sample_count: 1,
            dimension: TextureDimension::D2,
            // Packed format 0x5cb resolves through FUN_1402c2ef0 to DXGI 10.
            format: TextureFormat::Rgba16Float,
            usage: TextureUsages::STORAGE_BINDING
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let views = std::array::from_fn(|mip| {
            texture.create_view(&TextureViewDescriptor {
                base_mip_level: mip as u32,
                mip_level_count: Some(1),
                ..Default::default()
            })
        });
        let constants = device.create_buffer(&BufferDescriptor {
            label: Some("depth reduction owner constants"),
            size: 32,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let source_view = source.create_view(&TextureViewDescriptor {
            aspect: TextureAspect::DepthOnly,
            ..Default::default()
        });
        let binding = device
            .wgpu_device()
            .create_bind_group(&BindGroupDescriptor {
                label: Some("four-mip depth bindings"),
                layout: &pipelines.depth_reduction.get_bind_group_layout(0),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&source_view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(&views[0]),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(&views[1]),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::TextureView(&views[2]),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: BindingResource::TextureView(&views[3]),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: constants.as_entire_binding(),
                    },
                ],
            })
            .into();
        Ok(Self {
            texture,
            views,
            constants,
            binding,
            source_extent,
        })
    }

    pub fn texture(&self) -> &Texture {
        &self.texture
    }
    pub fn last_mip(&self) -> &bevy::render::render_resource::TextureView {
        &self.views[3]
    }
    pub fn last_mip_extent(&self) -> [u32; 2] {
        [self.texture.width() >> 3, self.texture.height() >> 3]
    }

    /// The caller supplies the same source extent selected by the original
    /// normal/offscreen branch. Preserve its ceil(extent/8) dispatch literally.
    /// Recording uses the caller's encoder; no extra queue submission or wait.
    pub fn record(
        &self,
        queue: &bevy::render::renderer::RenderQueue,
        encoder: &mut CommandEncoder,
        pipelines: &crate::tiled_lighting_gpu::TlfPipelines,
        active_source_extent: [u32; 2],
        inverse_projection: crate::render_shadow::ShadowMatrix,
    ) -> Result<DepthReductionCoverage, String> {
        use bevy::render::render_resource::ComputePassDescriptor;
        if (0..2).any(|axis| {
            active_source_extent[axis] == 0 || active_source_extent[axis] > self.source_extent[axis]
        }) {
            return Err("depth reduction extent exceeds its source owner".into());
        }
        let words = [
            active_source_extent[0],
            active_source_extent[1],
            0,
            0,
            inverse_projection[2][3].to_bits(),
            inverse_projection[3][2].to_bits(),
            inverse_projection[3][3].to_bits(),
            0,
        ];
        let bytes: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
        queue.write_buffer(&self.constants, 0, &bytes);
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("simultaneous four-level depth reduction"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipelines.depth_reduction);
        pass.set_bind_group(0, &self.binding, &[]);
        let groups = active_source_extent.map(|v| (v + 7) >> 3);
        pass.dispatch_workgroups(groups[0], groups[1], 1);
        let allocated_mips =
            std::array::from_fn(|mip| [self.texture.width() >> mip, self.texture.height() >> mip]);
        let written_mips = std::array::from_fn(|mip| {
            std::array::from_fn(|axis| (groups[axis] * (8 >> mip)).min(allocated_mips[mip][axis]))
        });
        Ok(DepthReductionCoverage {
            allocated_mips,
            written_mips,
        })
    }
}

/// FUN_1402fc6a0's coarseDepthGen constants (average/max channels X/Z).
/// Installed coarsedepthgeneration.decl indexes the logical matrix at [2].w,
/// [3][3], [3][2]. DXBC's column-major CB register indices reverse the two
/// off-diagonal slots; they are not row indices into our CPU matrix.
/// Dimensions belong to the last
/// downsampled-depth mip, not to the full camera target. No capture constants
/// or assumed full-size viewport replace the owner's scale/inverse projection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoarseDepthParameters {
    pub source_max: [u32; 2],
    pub source_per_destination: [f32; 2],
    pub inverse_projection: [f32; 4],
}

impl CoarseDepthParameters {
    /// Retain FUN_1402fc6a0's actual viewport scale, and verify that it cannot
    /// address an unwritten part of the persistent reduction image. No derived
    /// crop scale, clearing, or substitute depth fills the missing region.
    pub fn from_reduction(
        coverage: DepthReductionCoverage,
        viewport_scale: [f32; 2],
        inverse_projection: crate::render_shadow::ShadowMatrix,
    ) -> Result<Self, String> {
        let parameters = Self::from_owner(
            coverage.allocated_mips[3],
            viewport_scale,
            inverse_projection,
        )?;
        if (0..2).any(|axis| parameters.source_max[axis] >= coverage.written_mips[3][axis]) {
            return Err("coarseDepth viewport addresses unwritten reduction pixels".into());
        }
        Ok(parameters)
    }

    pub fn from_owner(
        source_mip_size: [u32; 2],
        viewport_scale: [f32; 2],
        inverse_projection: crate::render_shadow::ShadowMatrix,
    ) -> Result<Self, String> {
        let scaled = std::array::from_fn::<_, 2, _>(|axis| {
            source_mip_size[axis] as f32 * viewport_scale[axis]
        });
        // The original converts these positive camera extents to signed integer
        // maximum indices. Reject malformed ownership, rather than synthesizing
        // a full viewport or a zero-depth grid.
        if scaled
            .iter()
            .any(|v| !v.is_finite() || *v < 1.0 || *v >= 2147483648.0)
        {
            return Err("coarseDepth owner has an invalid scaled source extent".into());
        }
        Ok(Self {
            source_max: scaled.map(|v| (v as i32 - 1) as u32),
            source_per_destination: scaled.map(|v| v / COARSE_DEPTH_SIZE as f32),
            inverse_projection: [
                inverse_projection[2][3],
                inverse_projection[3][3],
                inverse_projection[3][2],
                0.0,
            ],
        })
    }

    pub fn to_bytes(self) -> [u8; 32] {
        let words = [
            self.source_max[0],
            self.source_max[1],
            self.source_per_destination[0].to_bits(),
            self.source_per_destination[1].to_bits(),
            self.inverse_projection[0].to_bits(),
            self.inverse_projection[1].to_bits(),
            self.inverse_projection[2].to_bits(),
            self.inverse_projection[3].to_bits(),
        ];
        std::array::from_fn(|i| words[i / 4].to_le_bytes()[i % 4])
    }
}

struct CpuSlot {
    generation: u64,
    mapping: bool,
    ready: bool,
    pixels: Box<CoarseDepthPixels>,
    error: Option<String>,
}

struct StagingSlot {
    buffer: Buffer,
    cpu: Arc<Mutex<CpuSlot>>,
    metadata: Option<CoarseDepthFrameMetadata>,
    recorded: bool,
}

pub struct CoarseDepthReadback {
    current: u32,
    slots: [StagingSlot; 4],
}

/// Native read order, including wraparound. No oldest-completed or fixed-lag
/// shortcut replaces the actual current, current-1, current-2, current-3 order.
pub fn coarse_depth_read_order(current: u32) -> [usize; 4] {
    std::array::from_fn(|age| (current.wrapping_sub(age as u32) & 3) as usize)
}

impl CoarseDepthReadback {
    pub fn new(device: &RenderDevice, initial_owner_index: u32) -> Result<Self, String> {
        if initial_owner_index > INITIAL_COARSE_DEPTH_OWNER_INDEX {
            return Err(
                "coarse-depth staging owner index is outside four-slot ring/sentinel".into(),
            );
        }
        Ok(Self {
            current: initial_owner_index,
            slots: std::array::from_fn(|_| StagingSlot {
                buffer: device.create_buffer(&BufferDescriptor {
                    label: Some("coarseDepth CPU staging"),
                    size: COARSE_DEPTH_BYTES,
                    usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                cpu: Arc::new(Mutex::new(CpuSlot {
                    generation: 0,
                    mapping: false,
                    ready: false,
                    pixels: Box::new([[0.0; 2]; 4096]),
                    error: None,
                })),
                metadata: None,
                recorded: false,
            }),
        })
    }

    /// Invoke the CPU consumer against the first successfully completed staging
    /// slot, just as the original Map loop does. No device poll/wait occurs here.
    /// None means no real grid is available; the frame owner must not publish a
    /// ready shadow frame with a zero/constant substitute. The original's stall
    /// handling beyond this four-attempt cycle is deliberately not claimed.
    pub fn try_read<R>(
        &self,
        consume: impl FnOnce(&CoarseDepthPixels) -> R,
    ) -> Result<Option<R>, String> {
        for index in coarse_depth_read_order(self.current) {
            let cpu = self.slots[index]
                .cpu
                .lock()
                .map_err(|_| "coarse-depth CPU slot poisoned")?;
            if cpu.ready {
                return Ok(Some(consume(&cpu.pixels)));
            }
        }
        for slot in &self.slots {
            let cpu = slot
                .cpu
                .lock()
                .map_err(|_| "coarse-depth CPU slot poisoned")?;
            if let Some(error) = &cpu.error {
                return Err(error.clone());
            }
        }
        Ok(None)
    }

    /// Return one real completed slot plus the exact view matrices attached
    /// when its GPU copy was submitted. The caller tracks `(slot,generation)`
    /// and sequence to avoid republishing stale asynchronous mappings.
    pub fn try_read_frame<R>(
        &self,
        consume: impl FnOnce(usize, u64, &CoarseDepthFrameMetadata, &CoarseDepthPixels) -> R,
    ) -> Result<Option<R>, String> {
        for index in coarse_depth_read_order(self.current) {
            let cpu = self.slots[index]
                .cpu
                .lock()
                .map_err(|_| "coarse-depth CPU slot poisoned")?;
            if cpu.ready {
                if let Some(metadata) = &self.slots[index].metadata {
                    return Ok(Some(consume(index, cpu.generation, metadata, &cpu.pixels)));
                }
            }
        }
        for slot in &self.slots {
            let cpu = slot
                .cpu
                .lock()
                .map_err(|_| "coarse-depth CPU slot poisoned")?;
            if let Some(error) = &cpu.error {
                return Err(error.clone());
            }
        }
        Ok(None)
    }

    /// Record the original task's single staging copy in the caller's frame
    /// command encoder. No additional queue submit is introduced. The native
    /// offscreen branch records no copy and does not advance the owner index.
    pub fn record_copy(
        &mut self,
        encoder: &mut CommandEncoder,
        source: &Texture,
        offscreen: bool,
    ) -> Result<bool, String> {
        self.record_copy_with_metadata(encoder, source, offscreen, None)
    }

    pub fn record_copy_with_metadata(
        &mut self,
        encoder: &mut CommandEncoder,
        source: &Texture,
        offscreen: bool,
        metadata: Option<CoarseDepthFrameMetadata>,
    ) -> Result<bool, String> {
        if offscreen {
            return Ok(false);
        }
        if source.format() != TextureFormat::Rg32Float
            || source.size()
                != (Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                })
            || source.sample_count() != 1
            || !source
                .usage()
                .contains(bevy::render::render_resource::TextureUsages::COPY_SRC)
        {
            return Err("coarseDepth copy needs the owner's 64x64 single-sample R32G32 COPY_SRC texture".into());
        }
        let next = (self.current + 1) & 3;
        let slot = &mut self.slots[next as usize];
        // Cancel any unfinished map before reusing this exact native ring slot.
        // Generation prevents the cancelled callback from publishing stale data.
        let mapping = {
            let mut cpu = slot
                .cpu
                .lock()
                .map_err(|_| "coarse-depth CPU slot poisoned")?;
            cpu.generation = cpu.generation.wrapping_add(1);
            cpu.ready = false;
            cpu.error = None;
            std::mem::replace(&mut cpu.mapping, false)
        };
        // Completed callbacks already unmap. WGPU rejects a second unmap;
        // only a still-pending generation needs cancellation before this copy.
        if mapping {
            slot.buffer.unmap();
        }
        encoder.copy_texture_to_buffer(
            source.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &slot.buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(512),
                    rows_per_image: Some(64),
                },
            },
            source.size(),
        );
        slot.metadata = metadata;
        slot.recorded = true;
        self.current = next;
        Ok(true)
    }

    /// RenderSystems::Cleanup equivalent, AFTER the frame queue submission.
    /// Mirrors Bevy's installed gpu_readback.rs lifecycle while retaining the
    /// original four persistent resources and their exact selection order.
    pub fn map_submitted(&mut self) -> Result<(), String> {
        for slot in &mut self.slots {
            if !slot.recorded {
                continue;
            }
            slot.recorded = false;
            let cpu = Arc::clone(&slot.cpu);
            let generation = {
                let mut state = cpu.lock().map_err(|_| "coarse-depth CPU slot poisoned")?;
                state.mapping = true;
                state.generation
            };
            let buffer = slot.buffer.clone();
            slot.buffer
                .slice(..)
                .map_async(MapMode::Read, move |result| {
                    let Ok(mut cpu) = cpu.lock() else {
                        return;
                    };
                    if cpu.generation != generation {
                        return;
                    }
                    cpu.mapping = false;
                    if let Err(error) = result {
                        cpu.error =
                            Some(format!("coarse-depth staging map failed: {error}"));
                        return;
                    }
                    let mapped = buffer.slice(..).get_mapped_range();
                    // The validated fixed-size map and boxed f32 array are distinct;
                    // every 32-bit float representation is valid. Windows DH2 and
                    // its R32G32 buffer use the host's little-endian IEEE layout.
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            mapped.as_ptr(),
                            cpu.pixels.as_mut_ptr().cast::<u8>(),
                            COARSE_DEPTH_BYTES as usize,
                        );
                    }
                    drop(mapped);
                    buffer.unmap();
                    cpu.ready = true;
                });
        }
        Ok(())
    }
}

struct CoarseDepthOutput {
    texture: Texture,
    _view: TextureView,
    constants: Buffer,
    binding: BindGroup,
}

impl CoarseDepthOutput {
    fn new(
        device: &RenderDevice,
        pipelines: &crate::tiled_lighting_gpu::TlfPipelines,
        source: &TextureView,
    ) -> Self {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("native 64x64 coarseDepth owner"),
            size: Extent3d {
                width: COARSE_DEPTH_SIZE,
                height: COARSE_DEPTH_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rg32Float,
            usage: TextureUsages::STORAGE_BINDING | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        let constants = device.create_buffer(&BufferDescriptor {
            label: Some("coarseDepth owner constants"),
            size: 32,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = device
            .wgpu_device()
            .create_bind_group(&BindGroupDescriptor {
                label: Some("coarseDepth generator bindings"),
                layout: &pipelines.coarse_depth.get_bind_group_layout(0),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(source),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(&view),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: constants.as_entire_binding(),
                    },
                ],
            })
            .into();
        Self {
            texture,
            _view: view,
            constants,
            binding,
        }
    }

    fn record(
        &self,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        pipelines: &crate::tiled_lighting_gpu::TlfPipelines,
        parameters: CoarseDepthParameters,
    ) {
        queue.write_buffer(&self.constants, 0, &parameters.to_bytes());
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("coarseDepthGen 64x64 dispatch"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipelines.coarse_depth);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.dispatch_workgroups(COARSE_DEPTH_SIZE, COARSE_DEPTH_SIZE, 1);
    }
}

#[derive(Resource, Default)]
struct CoarseDepthGpu {
    view_entity: Option<Entity>,
    source_texture_id: Option<TextureId>,
    source_extent: Option<[u32; 2]>,
    pyramid: Option<DepthPyramid>,
    output: Option<CoarseDepthOutput>,
    readback: Option<CoarseDepthReadback>,
    next_sequence: u64,
    last_published_sequence: u64,
}

impl CoarseDepthGpu {
    fn ensure(
        &mut self,
        entity: Entity,
        source_texture_id: TextureId,
        source: &Texture,
        device: &RenderDevice,
        pipelines: &crate::tiled_lighting_gpu::TlfPipelines,
    ) -> Result<(), String> {
        let extent = [source.width(), source.height()];
        let rebuild = self.view_entity != Some(entity)
            || self.source_texture_id != Some(source_texture_id)
            || self.source_extent != Some(extent)
            || self.pyramid.is_none();
        if !rebuild {
            return Ok(());
        }
        let pyramid = DepthPyramid::new(device, pipelines, source)?;
        let output = CoarseDepthOutput::new(device, pipelines, pyramid.last_mip());
        let readback = CoarseDepthReadback::new(device, INITIAL_COARSE_DEPTH_OWNER_INDEX)?;
        self.view_entity = Some(entity);
        self.source_texture_id = Some(source_texture_id);
        self.source_extent = Some(extent);
        self.pyramid = Some(pyramid);
        self.output = Some(output);
        self.readback = Some(readback);
        self.last_published_sequence = 0;
        Ok(())
    }
}

/// Gameplay-camera producer for FUN_1402fc6a0's adaptive grid. It runs on
/// cameras marked CoarseDepthSource, including the live cascade owner
/// when r_tightenParallelShadows is enabled. Captured
/// `decompressedDepth` is a single-sample R24G8 full-view input;
/// FUN_1402fbe40 builds its four-mip pyramid and FUN_1402fc6a0 dispatches the
/// 64x64 RG32 grid. FUN_1402fc130's 16x16-group 120x68 tile map is separate.
/// This binds Bevy's same-view main-pass `ViewDepthTexture` only when the full
/// target/viewport and single-sample conditions hold. The CPU cascade owner
/// consumes the mapped max-depth channel through the original adaptive-far
/// and persistent prepare sequence, separately from ordinary Tome depth.
pub struct CoarseDepthPlugin;

impl Plugin for CoarseDepthPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CoarseDepthMailbox>()
            .add_plugins(ExtractResourcePlugin::<CoarseDepthMailbox>::default())
            .add_plugins(ExtractComponentPlugin::<CoarseDepthSource>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<CoarseDepthGpu>()
            .init_gpu_resource::<crate::tiled_lighting_gpu::TlfPipelines>()
            .add_systems(
                Render,
                publish_ready_coarse_depth.in_set(RenderSystems::PrepareResources),
            )
            .add_systems(
                Core3d,
                render_coarse_depth
                    .after(Core3dSystems::MainPass)
                    .before(Core3dSystems::EarlyPostProcess),
            )
            .add_systems(
                Render,
                map_coarse_depth.in_set(RenderSystems::Cleanup),
            );
    }
}

fn publish_ready_coarse_depth(
    mut gpu: ResMut<CoarseDepthGpu>,
    mailbox: Res<CoarseDepthMailbox>,
    mut last_error: Local<Option<String>>,
) {
    let Some(readback) = gpu.readback.as_ref() else {
        return;
    };
    let attempt = readback.try_read_frame(|_slot, _generation, metadata, pixels| {
        (metadata.clone(), Arc::new(*pixels))
    });
    match attempt {
        Ok(Some((metadata, pixels))) if metadata.sequence > gpu.last_published_sequence => {
            let sequence = metadata.sequence;
            if diagnostics_enabled() && sequence % 300 == 0 {
                let range = |channel: usize| {
                    pixels
                        .iter()
                        .fold([f32::INFINITY, f32::NEG_INFINITY], |v, p| {
                            [v[0].min(p[channel]), v[1].max(p[channel])]
                        })
                };
                eprintln!("Coarse mapped sequence={sequence} ranges={:?}/{:?} samples={:?} inverse={:?}",range(0),range(1),&pixels[..8],metadata.inverse_projection);
            }
            let source_extent = metadata.source_extent;
            let first_mapped_frame = gpu.last_published_sequence == 0;
            match mailbox.publish(CoarseDepthSnapshot { metadata, pixels }) {
                Ok(()) => {
                    if first_mapped_frame {
                        bevy::log::info!(
                            target: "dh2::coarse_depth",
                            "Coarse-depth readback online: sequence={sequence}, source={}x{}, grid=64x64 RG32",
                            source_extent[0], source_extent[1]
                        );
                    }
                    gpu.last_published_sequence = sequence;
                    *last_error = None;
                }
                Err(error) => report_coarse_depth_error(&mut last_error, error),
            }
        }
        Ok(_) => {}
        Err(error) => report_coarse_depth_error(&mut last_error, error),
    }
}

fn render_coarse_depth(
    views: Query<
        (
            Entity,
            &ViewDepthTexture,
            &ExtractedView,
            &bevy::render::camera::ExtractedCamera,
        ),
        With<CoarseDepthSource>,
    >,
    pipelines: Res<crate::tiled_lighting_gpu::TlfPipelines>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    mut gpu: ResMut<CoarseDepthGpu>,
    mut context: RenderContext,
    mut last_error: Local<Option<String>>,
    readiness: Option<Res<crate::shadow_pass::ShadowCaptureReadiness>>,
    profile: Option<Res<crate::frame_profile::CpuFrameProfile>>,
) {
    let _span = profile.as_deref().map(|profile| profile.measure(43));
    // The original has resident shaders before drawing gameplay depth. An
    // asynchronously compiling forward pass supplies an incomplete depth image
    // and must not seed the native persistent cascade/movement cache.
    if readiness
        .as_ref()
        .is_some_and(|r| r.0.load(std::sync::atomic::Ordering::Acquire) == 0)
    {
        return;
    }
    let Ok((entity, depth, view, camera)) = views.single() else {
        return;
    };
    let source_extent = [depth.texture.width(), depth.texture.height()];
    if depth.texture.sample_count() != 1 {
        report_coarse_depth_error(
            &mut last_error,
            format!(
                "coarse-depth shader requires the original single-sample depth input; received {} samples",
                depth.texture.sample_count()
            ),
        );
        return;
    }
    let full_target = camera.physical_target_size
        == Some(bevy::math::UVec2::new(source_extent[0], source_extent[1]))
        && camera.physical_viewport_size == camera.physical_target_size;
    let full_rect = camera.viewport.as_ref().is_none_or(|viewport| {
        viewport.physical_position == bevy::math::UVec2::ZERO
            && viewport.physical_size == bevy::math::UVec2::new(source_extent[0], source_extent[1])
    });
    if !full_target || !full_rect {
        report_coarse_depth_error(
            &mut last_error,
            "decompressedDepth producer does not match the full gameplay target extent"
                .into(),
        );
        return;
    }
    let projection = mat4_to_shadow_rows(view.clip_from_view);
    let mut inverse_projection = [[0.0; 4]; 4];
    if !crate::render_shadow::invert_rows(&projection, &mut inverse_projection) {
        report_coarse_depth_error(
            &mut last_error,
            "depth projection is singular".into(),
        );
        return;
    }
    let view_from_world = mat4_to_shadow_rows(view.world_from_view.to_matrix().inverse());
    let result = (|| -> Result<(), String> {
        gpu.ensure(
            entity,
            depth.texture.id(),
            &depth.texture,
            &render_device,
            &pipelines,
        )?;
        let pyramid = gpu
            .pyramid
            .as_ref()
            .ok_or("depth pyramid was not initialized")?;
        let coverage = pyramid.record(
            &render_queue,
            context.command_encoder(),
            &pipelines,
            source_extent,
            inverse_projection,
        )?;
        let parameters =
            CoarseDepthParameters::from_reduction(coverage, [1.0, 1.0], inverse_projection)?;
        gpu.output
            .as_ref()
            .ok_or("coarseDepth output was not initialized")?
            .record(
                &render_queue,
                context.command_encoder(),
                &pipelines,
                parameters,
            );
        let sequence = gpu
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| "coarse-depth frame sequence overflow".to_owned())?;
        let metadata = CoarseDepthFrameMetadata {
            sequence,
            source_extent,
            projection,
            inverse_projection,
            view_from_world,
        };
        let CoarseDepthGpu {
            output, readback, ..
        } = &mut *gpu;
        let output_texture = &output
            .as_ref()
            .ok_or("coarseDepth output was not initialized")?
            .texture;
        readback
            .as_mut()
            .ok_or("coarseDepth staging ring was not initialized")?
            .record_copy_with_metadata(
                context.command_encoder(),
                output_texture,
                false,
                Some(metadata),
            )?;
        gpu.next_sequence = sequence;
        Ok(())
    })();
    match result {
        Ok(()) => {
            if gpu.next_sequence == 1 {
                bevy::log::info!(
                    target: "dh2::coarse_depth",
                    "Coarse-depth producer submitted: view={entity:?}, texture={:?}, extent={}x{}, depth_samples=1",
                    depth.texture.id(), source_extent[0], source_extent[1]
                );
            }
            *last_error = None;
        }
        Err(error) => report_coarse_depth_error(&mut last_error, error),
    }
}

fn map_coarse_depth(mut gpu: ResMut<CoarseDepthGpu>) {
    let Some(readback) = gpu.readback.as_mut() else {
        return;
    };
    if let Err(error) = readback.map_submitted() {
        bevy::log::warn!(target: "dh2::coarse_depth", "Coarse-depth map unavailable: {error}");
    }
}

fn report_coarse_depth_error(last_error: &mut Option<String>, error: String) {
    if last_error.as_ref() != Some(&error) {
        bevy::log::warn!(target: "dh2::coarse_depth", "Coarse-depth producer unavailable: {error}");
        *last_error = Some(error);
    }
}

fn mat4_to_shadow_rows(matrix: bevy::math::Mat4) -> crate::render_shadow::ShadowMatrix {
    let columns = matrix.to_cols_array_2d();
    std::array::from_fn(|row| std::array::from_fn(|column| columns[column][row]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a graphics adapter; validates captured D24S8 sampled-depth bindings and the native reduction/generator pipelines"]
    fn d24s8_view_depth_runs_coarse_depth_gpu() {
        use bevy::{
            app::PluginGroup,
            prelude::{default, DefaultPlugins},
            render::render_resource::{TextureDescriptor, TextureUsages},
            window::{ExitCondition, WindowPlugin},
        };

        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
        );
        app.finish();
        app.cleanup();
        let render_world = app.sub_app_mut(RenderApp).world_mut();
        render_world.init_resource::<crate::tiled_lighting_gpu::TlfPipelines>();
        let device = render_world.resource::<RenderDevice>();
        let queue = render_world.resource::<RenderQueue>();
        let pipelines = render_world.resource::<crate::tiled_lighting_gpu::TlfPipelines>();
        let source = device.create_texture(&TextureDescriptor {
            label: Some("captured original decompressedDepth view fixture"),
            size: Extent3d {
                width: 1920,
                height: 1080,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth24PlusStencil8,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let pyramid = DepthPyramid::new(device, pipelines, &source)
            .expect("captured D24S8 source must bind as texture_depth_2d");
        let output = CoarseDepthOutput::new(device, pipelines, pyramid.last_mip());
        let mut encoder = device.create_command_encoder(&Default::default());
        let inverse = bevy::math::Mat4::IDENTITY.to_cols_array_2d();
        let inverse = std::array::from_fn(|row| std::array::from_fn(|column| inverse[column][row]));
        let coverage = pyramid
            .record(queue, &mut encoder, pipelines, [1920, 1080], inverse)
            .expect("captured full-resolution source must reduce");
        let parameters = CoarseDepthParameters::from_reduction(coverage, [1.0, 1.0], inverse)
            .expect("full captured viewport must fit the reduction mip");
        output.record(queue, &mut encoder, pipelines, parameters);
        queue.submit([encoder.finish()]);
    }

    #[test]
    fn coarse_constants_use_scaled_last_mip_and_matrix_slots() {
        let rows = std::array::from_fn(|r| std::array::from_fn(|c| (r * 4 + c) as f32));
        let p = CoarseDepthParameters::from_owner([117, 69], [0.85, 0.93], rows).unwrap();
        assert_eq!(p.source_max, [98, 63]);
        assert_eq!(
            p.source_per_destination.map(f32::to_bits),
            [(117.0f32 * 0.85) / 64.0, (69.0f32 * 0.93) / 64.0].map(f32::to_bits)
        );
        assert_eq!(p.inverse_projection, [11.0, 15.0, 14.0, 0.0]);
        let bytes = p.to_bytes();
        let words: [u32; 8] = std::array::from_fn(|i| {
            u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
        });
        assert_eq!(
            words,
            [
                98,
                63,
                p.source_per_destination[0].to_bits(),
                p.source_per_destination[1].to_bits(),
                11.0f32.to_bits(),
                15.0f32.to_bits(),
                14.0f32.to_bits(),
                0
            ]
        );
    }

    #[test]
    fn finite_gameplay_depth_keeps_the_shadow_receiver_distance() {
        // Full Steam-game depth-source capture, 2026-10-17. Use asymmetric
        // off-diagonal entries: identity fixtures cannot catch a CB transpose.
        let projection = [
            [0.7002075910568237, 0.0, 0.0, 0.0],
            [0.0, 1.2448134422302246, 0.0, 0.0],
            [0.0, 0.0, 1.4285733413998969e-6, 0.010000013746321201],
            [0.0, 0.0, -1.0, 0.0],
        ];
        let mut inverse = [[0.0; 4]; 4];
        assert!(crate::render_shadow::invert_rows(&projection, &mut inverse));
        let parameters =
            CoarseDepthParameters::from_owner([120, 68], [1.0; 2], inverse).unwrap();
        let [a, b, c, _] = parameters.inverse_projection;
        // Installed coarsedepthgeneration.decl's de-linearization, retaining
        // its native plus sign. A 50-metre receiver must not collapse to 2m.
        let depth = (-a / 50.0 + b) / c;
        assert!((0.0..1.0).contains(&depth));
        let far = crate::render_shadow::adaptive_directional_far(
            true,
            &projection,
            &inverse,
            [30.0, 75.0],
            Some(&[depth; 4096]),
        )
        .unwrap();
        assert_eq!(far, 50.0);
    }

    #[test]
    fn cropped_reduction_requires_the_consumers_real_viewport_scale() {
        let coverage = DepthReductionCoverage {
            allocated_mips: [[32, 32], [16, 16], [8, 8], [4, 4]],
            written_mips: [[16, 8], [8, 4], [4, 2], [2, 1]],
        };
        let inverse = [[0.0; 4]; 4];
        assert!(
            CoarseDepthParameters::from_reduction(coverage, [1.0, 1.0], inverse).is_err()
        );
        let p =
            CoarseDepthParameters::from_reduction(coverage, [0.5, 0.25], inverse).unwrap();
        assert_eq!(p.source_max, [1, 0]);
        assert!(
            CoarseDepthParameters::from_reduction(coverage, [0.75, 0.25], inverse).is_err()
        );
    }

    #[test]
    fn malformed_coarse_extent_does_not_invent_a_viewport() {
        let rows = [[0.0; 4]; 4];
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 33554432.0] {
            assert!(CoarseDepthParameters::from_owner([64, 64], [scale, 1.0], rows).is_err());
        }
        assert!(CoarseDepthParameters::from_owner([0, 64], [1.0, 1.0], rows).is_err());
    }

    #[test]
    fn pyramid_allocation_matches_minimum_and_half_alignment() {
        assert_eq!(depth_pyramid_extent([1920, 1080]).unwrap(), [960, 544]);
        assert_eq!(depth_pyramid_extent([27, 19]).unwrap(), [16, 16]);
        assert_eq!(depth_pyramid_extent([1, 1]).unwrap(), [8, 8]);
        for width in 1..4097 {
            let output = depth_pyramid_extent([width, width]).unwrap()[0];
            assert_eq!(output % 8, 0);
            assert!(output * 2 >= width.max(8));
            assert!(output * 2 - width.max(8) < 16);
        }
        assert!(depth_pyramid_extent([0, 1080]).is_err());
        assert!(depth_pyramid_extent([u32::MAX, 1]).is_err());
    }

    #[test]
    fn staging_read_order_matches_four_attempt_loop() {
        assert_eq!(
            coarse_depth_read_order(INITIAL_COARSE_DEPTH_OWNER_INDEX),
            [0, 3, 2, 1]
        );
        assert_eq!(coarse_depth_read_order(0), [0, 3, 2, 1]);
        assert_eq!(coarse_depth_read_order(1), [1, 0, 3, 2]);
        assert_eq!(coarse_depth_read_order(2), [2, 1, 0, 3]);
        assert_eq!(coarse_depth_read_order(3), [3, 2, 1, 0]);
    }
}
