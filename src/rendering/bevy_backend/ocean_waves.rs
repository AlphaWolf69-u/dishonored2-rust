//! Persistent UpdateSpectrum and six-stage FFT resources. The render graph must
//! supply actual ocean controls/time and schedule displacement/gradient draws.
use crate::rendering::ocean::{
    initialize_wave_spectrum, ocean_fft_passes, FftBuffer, WaveSpectrumParameters,
};
use bevy::{
    prelude::default,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

/// 1402fa110 initializes this simulation resolution; the two FFT programs and
/// six stages are the source's 512x512, three-channel implementation.
pub const WAVE_RESOLUTION: u32 = 512;

mod collision;
mod foam;
mod surfaces;
pub use foam::{FoamMode, FoamSettings};

pub struct OceanWaves {
    initial: Buffer,
    frequency: Buffer,
    spectrum: Buffer,
    scratch: Buffer,
    transformed: Buffer,
    clock: Buffer,
    spectrum_pipeline: ComputePipeline,
    spectrum_binding: BindGroup,
    fft_pipelines: [ComputePipeline; 2],
    fft_bindings: [BindGroup; 6],
    surfaces: surfaces::WaveSurfaces,
    foam: foam::WaveFoam,
    collision: collision::WaveCollision,
    current_index: usize,
}

fn words(values: impl IntoIterator<Item = u32>) -> Vec<u8> {
    values.into_iter().flat_map(u32::to_le_bytes).collect()
}

impl OceanWaves {
    pub fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        parameters: WaveSpectrumParameters,
    ) -> Result<Self, String> {
        let buffer = |label, size, usage| {
            device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let n = u64::from(WAVE_RESOLUTION);
        let padded = (n + 4) * (n + 1);
        let initial = buffer(
            "ocean H0",
            padded * 8,
            BufferUsages::STORAGE | BufferUsages::COPY_DST,
        );
        let frequency = buffer(
            "ocean Omega",
            padded * 4,
            BufferUsages::STORAGE | BufferUsages::COPY_DST,
        );
        let work_usage = BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST;
        let spectrum = buffer("ocean Ht/Dx/Dy", n * n * 3 * 8, work_usage);
        let scratch = buffer("ocean FFT scratch", n * n * 3 * 8, work_usage);
        let transformed = buffer("ocean transformed Ht/Dx/Dy", n * n * 3 * 8, work_usage);
        let clock = buffer(
            "ocean spectrum parameters",
            48,
            BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        );
        let spectrum_shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("ocean spectrum"),
            source: ShaderSource::Wgsl(
                include_str!("../../../assets/shaders/dh2_ocean_spectrum.wgsl").into(),
            ),
        });
        let spectrum_pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: Some("ocean UpdateSpectrum"),
            layout: None,
            module: &spectrum_shader,
            entry_point: Some("update_spectrum"),
            compilation_options: default(),
            cache: None,
        });
        let spectrum_layout = spectrum_pipeline.get_bind_group_layout(0).into();
        let spectrum_binding = device.create_bind_group(
            "ocean spectrum inputs",
            &spectrum_layout,
            &[&clock, &initial, &frequency, &spectrum]
                .into_iter()
                .enumerate()
                .map(|(i, buffer)| BindGroupEntry {
                    binding: i as u32,
                    resource: buffer.as_entire_binding(),
                })
                .collect::<Vec<_>>(),
        );
        let fft_shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("ocean FFT"),
            source: ShaderSource::Wgsl(
                include_str!("../../../assets/shaders/dh2_ocean_fft.wgsl").into(),
            ),
        });
        let fft_pipelines = ["radix_eight", "radix_eight_final"].map(|entry| {
            device.create_compute_pipeline(&RawComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &fft_shader,
                entry_point: Some(entry),
                compilation_options: default(),
                cache: None,
            })
        });
        let fft_bindings = ocean_fft_passes(WAVE_RESOLUTION, 3).map(|pass| {
            let uniform = device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("ocean FFT constants"),
                usage: BufferUsages::UNIFORM,
                contents: &words([
                    pass.thread_count,
                    pass.output_stride,
                    pass.input_stride,
                    pass.phase_stride,
                    pass.phase_base.to_bits(),
                    0,
                    0,
                    0,
                ]),
            });
            let select = |which| match which {
                FftBuffer::Input => &spectrum,
                FftBuffer::Scratch => &scratch,
                FftBuffer::Output => &transformed,
            };
            let layout = fft_pipelines[usize::from(pass.final_radix)]
                .get_bind_group_layout(0)
                .into();
            device.create_bind_group(
                "ocean FFT pass",
                &layout,
                &[&uniform, select(pass.source), select(pass.destination)]
                    .into_iter()
                    .enumerate()
                    .map(|(i, buffer)| BindGroupEntry {
                        binding: i as u32,
                        resource: buffer.as_entire_binding(),
                    })
                    .collect::<Vec<_>>(),
            )
        });
        let surfaces = surfaces::WaveSurfaces::new(device, &transformed);
        let foam = foam::WaveFoam::new(
            device,
            queue,
            [surfaces.displacement_view(0), surfaces.displacement_view(1)],
            surfaces.gradient_view(),
        );
        let collision = collision::WaveCollision::new(
            device,
            [surfaces.displacement_view(0), surfaces.displacement_view(1)],
        )?;
        let result = Self {
            initial,
            frequency,
            spectrum,
            scratch,
            transformed,
            clock,
            spectrum_pipeline,
            spectrum_binding,
            fft_pipelines,
            fft_bindings,
            surfaces,
            foam,
            collision,
            // 1402f9a80 initializes simulation owner+108 to zero.
            current_index: 0,
        };
        result.upload_initial_spectrum(queue, parameters)?;
        Ok(result)
    }

    /// Invoke on the source's spectrum-invalidated branch, not every frame.
    pub fn upload_initial_spectrum(
        &self,
        queue: &RenderQueue,
        parameters: WaveSpectrumParameters,
    ) -> Result<(), String> {
        let count = ((WAVE_RESOLUTION + 4) * (WAVE_RESOLUTION + 1)) as usize;
        let mut h0 = vec![[0u16; 2]; count];
        let mut omega = vec![0u16; count];
        initialize_wave_spectrum(WAVE_RESOLUTION, parameters, &mut h0, &mut omega)?;
        let decode = |value| half::f16::from_bits(value).to_f32().to_bits();
        queue.write_buffer(
            &self.initial,
            0,
            &words(h0.into_iter().flatten().map(decode)),
        );
        queue.write_buffer(&self.frequency, 0, &words(omega.into_iter().map(decode)));
        Ok(())
    }

    /// Prepare once for this frame's wave submission before recording commands.
    pub fn prepare_time(&self, queue: &RenderQueue, local_time: f32, time_scale: f32) {
        let n = WAVE_RESOLUTION;
        queue.write_buffer(
            &self.clock,
            0,
            &words([
                n,
                n + 4,
                n,
                n,
                n * n,
                n * n * 2,
                0,
                0,
                local_time.to_bits(),
                time_scale.to_bits(),
                0,
                0,
            ]),
        );
    }

    pub fn record_spectrum(&mut self, encoder: &mut CommandEncoder) {
        // 1402fb380 advances before spectrum initialization/dispatch.
        self.current_index = 1 - self.current_index;
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("ocean spectrum"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.spectrum_pipeline);
        pass.set_bind_group(0, &self.spectrum_binding, &[]);
        pass.dispatch_workgroups(
            WAVE_RESOLUTION.div_ceil(16),
            WAVE_RESOLUTION.div_ceil(16),
            1,
        );
    }

    pub fn record_fft(&self, encoder: &mut CommandEncoder) {
        // Distinct passes establish the storage-write -> storage-read dependency
        // between each ping-pong stage; bindings/constants persist across frames.
        for (index, step) in ocean_fft_passes(WAVE_RESOLUTION, 3).into_iter().enumerate() {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("ocean FFT"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.fft_pipelines[usize::from(step.final_radix)]);
            pass.set_bind_group(0, &self.fft_bindings[index], &[]);
            pass.dispatch_workgroups(step.workgroups[0], step.workgroups[1], step.workgroups[2]);
        }
    }

    /// Data is produced only after record_spectrum/record_fft have executed in
    /// the caller's command sequence. This accessor does not imply readiness.
    pub fn transformed_spectrum(&self) -> &Buffer {
        &self.transformed
    }

    pub fn prepare_choppiness(&self, queue: &RenderQueue, choppiness: f32) {
        self.surfaces.prepare(queue, choppiness);
    }

    /// Schedule after this frame's FFT, preserving the other displacement slot.
    pub fn record_surfaces(&self, encoder: &mut CommandEncoder) {
        self.surfaces.record(encoder, self.current_index);
    }

    pub fn displacement(&self) -> &Texture {
        self.surfaces.displacement(self.current_index)
    }
    pub fn displacement_view(&self) -> &TextureView {
        self.surfaces.displacement_view(self.current_index)
    }
    /// The first previous slot has not been rendered. Allocation alone does
    /// not establish that this texture contains a completed previous frame.
    pub fn previous_displacement(&self) -> &Texture {
        self.surfaces.displacement(1 - self.current_index)
    }
    pub fn previous_displacement_view(&self) -> &TextureView {
        self.surfaces.displacement_view(1 - self.current_index)
    }
    pub fn gradients(&self) -> &Texture {
        self.surfaces.gradients()
    }
    pub fn gradient_view(&self) -> &TextureView {
        self.surfaces.gradient_view()
    }
    pub fn prepare_foam(&self, queue: &RenderQueue, settings: FoamSettings) {
        self.foam.prepare(queue, settings);
    }
    pub fn record_foam(&self, encoder: &mut CommandEncoder, mode: FoamMode) {
        self.foam.record(encoder, self.current_index, mode);
    }
    pub fn foam_mask(&self) -> &Texture {
        self.foam.texture()
    }
    pub fn foam_view(&self) -> &TextureView {
        self.foam.view()
    }
    pub fn record_collision_height(&self, encoder: &mut CommandEncoder) {
        self.collision.record(encoder, self.current_index);
    }
    pub fn collision_height(&self) -> &Texture {
        self.collision.texture()
    }
    pub fn collision_height_view(&self) -> &TextureView {
        self.collision.view()
    }

    /// Owned buffer payload bytes, excluding driver/pipeline allocation overhead.
    pub fn buffer_bytes(&self) -> u64 {
        [
            &self.initial,
            &self.frequency,
            &self.spectrum,
            &self.scratch,
            &self.transformed,
            &self.clock,
        ]
        .into_iter()
        .map(|buffer| buffer.size())
        .sum::<u64>()
            + 6 * 32
            + self.surfaces.uniform_bytes()
            + self.foam.uniform_bytes()
            + self.collision.buffer_bytes()
    }
}

#[cfg(test)]
mod tests;
