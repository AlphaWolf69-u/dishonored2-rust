//! Native 64x64 R32G32 coarse-depth CPU readback owner.
//!
//! FUN_140351780 tries four staging textures newest-to-oldest with nonblocking
//! Map (FUN_1402eac80). Its later opaque task copies to (current+1)&3 unless
//! the owner's offscreen flag is set. This module supplies that persistent
//! ring, not a fabricated depth grid or a synchronous per-frame GPU wait.
//! The caller must provide the real coarseDepth image and source owner index.

use bevy::render::{
    render_resource::{
        Buffer, BufferDescriptor, BufferUsages, CommandEncoder, Extent3d, MapMode,
        TexelCopyBufferInfo, TexelCopyBufferLayout, Texture, TextureFormat,
    },
    renderer::RenderDevice,
};
use std::sync::{Arc, Mutex};

pub const COARSE_DEPTH_SIZE: u32 = 64;
pub const COARSE_DEPTH_BYTES: u64 = 64 * 64 * 8;
pub type CoarseDepthPixels = [[f32; 2]; 64 * 64];

/// FUN_14036d0c0's downsampled image: clamp the framebuffer to at least eight,
/// then round its half-size up to eight. Exactly four RGBA16F mips are owned.
/// Other images allocated by that function are not represented here.
pub fn depth_pyramid_extent(framebuffer: [u32; 2]) -> Result<[u32; 2], String> {
    if framebuffer
        .iter()
        .any(|v| *v == 0 || *v > i32::MAX as u32 - 15)
    {
        return Err("native depth pyramid has an invalid framebuffer extent".into());
    }
    Ok(framebuffer.map(|v| ((v.max(8) + 15) >> 1) & !7))
}

/// Coverage actually written by one dispatch. A smaller active viewport can
/// leave old pixels outside these extents in the persistent texture. The
/// native consumer must retain its own viewport scale; never sample those
/// pixels by assuming the full allocated mip was refreshed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeDepthReductionCoverage {
    pub allocated_mips: [[u32; 2]; 4],
    pub written_mips: [[u32; 2]; 4],
}

/// Persistent image and bindings for FUN_1402fbe40, not four independent
/// downscale dispatches or hardware-generated average-only mips.
/// The real source depth owner is mandatory; no constant/captured substitute.
pub struct NativeDepthPyramid {
    texture: Texture,
    views: [bevy::render::render_resource::TextureView; 4],
    constants: Buffer,
    binding: bevy::render::render_resource::BindGroup,
    source_extent: [u32; 2],
}

impl NativeDepthPyramid {
    pub fn new(
        device: &RenderDevice,
        pipelines: &crate::tiled_lighting_gpu::NativeTlfPipelines,
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
                "native depth reduction requires the actual single-sample 2D depth input".into(),
            );
        }
        let source_extent = [source.width(), source.height()];
        let [width, height] = depth_pyramid_extent(source_extent)?;
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("native four-mip DownsampledLinearDepth"),
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
            label: Some("native depth reduction owner constants"),
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
                label: Some("native four-mip depth bindings"),
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
        pipelines: &crate::tiled_lighting_gpu::NativeTlfPipelines,
        active_source_extent: [u32; 2],
        inverse_projection: crate::render_shadow::ShadowMatrix,
    ) -> Result<NativeDepthReductionCoverage, String> {
        use bevy::render::render_resource::ComputePassDescriptor;
        if (0..2).any(|axis| {
            active_source_extent[axis] == 0 || active_source_extent[axis] > self.source_extent[axis]
        }) {
            return Err("native depth reduction extent exceeds its source owner".into());
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
            label: Some("native simultaneous four-level depth reduction"),
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
        Ok(NativeDepthReductionCoverage {
            allocated_mips,
            written_mips,
        })
    }
}

/// FUN_1402fc6a0's coarseDepthGen constants (average/max channels X/Z).
/// Dimensions belong to the last
/// downsampled-depth mip, not to the full camera target. No capture constants
/// or assumed full-size viewport replace the owner's scale/inverse projection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeCoarseDepthParameters {
    pub source_max: [u32; 2],
    pub source_per_destination: [f32; 2],
    pub inverse_projection: [f32; 4],
}

impl NativeCoarseDepthParameters {
    /// Retain FUN_1402fc6a0's actual viewport scale, and verify that it cannot
    /// address an unwritten part of the persistent reduction image. No derived
    /// crop scale, clearing, or substitute depth fills the missing region.
    pub fn from_reduction(
        coverage: NativeDepthReductionCoverage,
        viewport_scale: [f32; 2],
        inverse_projection: crate::render_shadow::ShadowMatrix,
    ) -> Result<Self, String> {
        let parameters = Self::from_owner(
            coverage.allocated_mips[3],
            viewport_scale,
            inverse_projection,
        )?;
        if (0..2).any(|axis| parameters.source_max[axis] >= coverage.written_mips[3][axis]) {
            return Err("native coarseDepth viewport addresses unwritten reduction pixels".into());
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
            return Err("native coarseDepth owner has an invalid scaled source extent".into());
        }
        Ok(Self {
            source_max: scaled.map(|v| (v as i32 - 1) as u32),
            source_per_destination: scaled.map(|v| v / COARSE_DEPTH_SIZE as f32),
            inverse_projection: [
                inverse_projection[3][2],
                inverse_projection[3][3],
                inverse_projection[2][3],
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
    recorded: bool,
}

pub struct NativeCoarseDepthReadback {
    current: u32,
    slots: [StagingSlot; 4],
}

/// Native read order, including wraparound. No oldest-completed or fixed-lag
/// shortcut replaces the actual current, current-1, current-2, current-3 order.
pub fn coarse_depth_read_order(current: u32) -> [usize; 4] {
    std::array::from_fn(|age| (current.wrapping_sub(age as u32) & 3) as usize)
}

impl NativeCoarseDepthReadback {
    pub fn new(device: &RenderDevice, initial_owner_index: u32) -> Result<Self, String> {
        if initial_owner_index > 3 {
            return Err("native coarse-depth staging owner index is outside four-slot ring".into());
        }
        Ok(Self {
            current: initial_owner_index,
            slots: std::array::from_fn(|_| StagingSlot {
                buffer: device.create_buffer(&BufferDescriptor {
                    label: Some("native coarseDepth CPU staging"),
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

    /// Record the original task's single staging copy in the caller's frame
    /// command encoder. No additional queue submit is introduced. The native
    /// offscreen branch records no copy and does not advance the owner index.
    pub fn record_copy(
        &mut self,
        encoder: &mut CommandEncoder,
        source: &Texture,
        offscreen: bool,
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
            return Err("native coarseDepth copy needs the owner's 64x64 single-sample R32G32 COPY_SRC texture".into());
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
                            Some(format!("native coarse-depth staging map failed: {error}"));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coarse_constants_use_scaled_last_mip_and_original_matrix_slots() {
        let rows = std::array::from_fn(|r| std::array::from_fn(|c| (r * 4 + c) as f32));
        let p = NativeCoarseDepthParameters::from_owner([117, 69], [0.85, 0.93], rows).unwrap();
        assert_eq!(p.source_max, [98, 63]);
        assert_eq!(
            p.source_per_destination.map(f32::to_bits),
            [(117.0f32 * 0.85) / 64.0, (69.0f32 * 0.93) / 64.0].map(f32::to_bits)
        );
        assert_eq!(p.inverse_projection, [14.0, 15.0, 11.0, 0.0]);
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
                14.0f32.to_bits(),
                15.0f32.to_bits(),
                11.0f32.to_bits(),
                0
            ]
        );
    }

    #[test]
    fn cropped_reduction_requires_the_consumers_real_viewport_scale() {
        let coverage = NativeDepthReductionCoverage {
            allocated_mips: [[32, 32], [16, 16], [8, 8], [4, 4]],
            written_mips: [[16, 8], [8, 4], [4, 2], [2, 1]],
        };
        let inverse = [[0.0; 4]; 4];
        assert!(
            NativeCoarseDepthParameters::from_reduction(coverage, [1.0, 1.0], inverse).is_err()
        );
        let p =
            NativeCoarseDepthParameters::from_reduction(coverage, [0.5, 0.25], inverse).unwrap();
        assert_eq!(p.source_max, [1, 0]);
        assert!(
            NativeCoarseDepthParameters::from_reduction(coverage, [0.75, 0.25], inverse).is_err()
        );
    }

    #[test]
    fn malformed_coarse_extent_does_not_invent_a_viewport() {
        let rows = [[0.0; 4]; 4];
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 33554432.0] {
            assert!(NativeCoarseDepthParameters::from_owner([64, 64], [scale, 1.0], rows).is_err());
        }
        assert!(NativeCoarseDepthParameters::from_owner([0, 64], [1.0, 1.0], rows).is_err());
    }

    #[test]
    fn pyramid_allocation_matches_native_minimum_and_half_alignment() {
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
    fn staging_read_order_matches_native_four_attempt_loop() {
        assert_eq!(coarse_depth_read_order(0), [0, 3, 2, 1]);
        assert_eq!(coarse_depth_read_order(1), [1, 0, 3, 2]);
        assert_eq!(coarse_depth_read_order(2), [2, 1, 0, 3]);
        assert_eq!(coarse_depth_read_order(3), [3, 2, 1, 0]);
    }
}
