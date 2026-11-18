//! Dark Vision's sonar wave while the record's mask has bit 8:
//! rendering/darkVisionDepthWaveDisto last in the distortion vectors
//! (1403222f0) and rendering/darkVisionDepthWaveColor into the scene colour
//! before the distortion resolves (140327270).
use super::{passes::pipeline, sources::DarkVisionWaveSources};
use crate::gameplay::powers::dark_vision::DarkVisionRecord;
use bevy::{math::Mat4, render::{render_resource::*, renderer::{RenderDevice, RenderQueue}}};

pub(crate) struct DarkVisionWave {
    uniform: Buffer,
    colour: Option<(TextureFormat, RenderPipeline)>,
    distortion: RenderPipeline,
    module: ShaderModule,
    /// (depth, adaptation) and the distortion and colour bind groups, each
    /// from its own pipeline's layout.
    binding: Option<((TextureViewId, BufferId), BindGroup, Option<BindGroup>)>,
}

/// The inputs both draws share.
pub(crate) struct WaveFrame<'a> {
    pub record: DarkVisionRecord,
    pub view_projection: [[f32; 4]; 4],
    pub local_time: f32,
    pub target_extent: [u32; 2],
    pub full_height: u32,
    /// Native-to-host colour units (139.26 x camera exposure).
    pub host_units: f32,
    pub depth: &'a TextureView,
    pub sampler: &'a Sampler,
    pub adaptation: &'a Buffer,
}

impl DarkVisionWave {
    pub fn active(record: Option<DarkVisionRecord>) -> Option<DarkVisionRecord> {
        let mask = record?.mask | crate::platform::console::shared().read().cvars
            .int("r_debugDarkVisionLayers").unwrap_or(0) as u32;
        (mask & 8 != 0).then_some(record?)
    }

    pub fn new(device: &RenderDevice, sources: &DarkVisionWaveSources) -> Result<Self, String> {
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("dark vision wave"),
            source: ShaderSource::Wgsl(include_str!("../../../../assets/shaders/dh2_dark_vision_wave.wgsl").into()),
        });
        let distortion = pipeline(device, &module, None, "dark vision wave distortion",
            "dark_vision_wave_vertex", "dark_vision_wave_distortion", TextureFormat::Rgba16Float, &sources.distortion_state)?;
        Ok(Self {
            uniform: device.create_buffer(&BufferDescriptor {
                label: Some("dark vision wave parameters"),
                size: 192,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            colour: None,
            distortion,
            module,
            binding: None,
        })
    }

    fn prepare(&mut self, device: &RenderDevice, queue: &RenderQueue, sources: &DarkVisionWaveSources,
        frame: &WaveFrame<'_>) -> Result<(), String> {
        let inverse = Mat4::from_cols_array_2d(&frame.view_projection).transpose().inverse();
        if !inverse.is_finite() {
            return Err("dark vision wave requires an invertible view projection".into());
        }
        let record = frame.record;
        let [inner, outer] = DarkVisionRecord::radii(record.sonar_radius);
        let mut values = [[0.; 4]; 12];
        values[..4].copy_from_slice(&inverse.to_cols_array_2d());
        values[4] = [record.centre[0], record.centre[1], record.centre[2], frame.local_time];
        values[5] = [inner, outer, record.sonar_power, frame.host_units];
        values[6] = [frame.target_extent[0] as f32, frame.target_extent[1] as f32, 1. / frame.full_height as f32, 0.];
        values[7..11].copy_from_slice(&sources.parameters);
        let mut bytes = [0; 192];
        for (destination, value) in bytes.chunks_exact_mut(4).zip(values.into_iter().flatten()) {
            destination.copy_from_slice(&value.to_le_bytes());
        }
        queue.write_buffer(&self.uniform, 0, &bytes);
        let key = (frame.depth.id(), frame.adaptation.id());
        if self.binding.as_ref().is_none_or(|binding| binding.0 != key) {
            let distortion = device.create_bind_group(Some("dark vision wave distortion inputs"),
                &self.distortion.get_bind_group_layout(0).into(), &[
                    BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                    BindGroupEntry { binding: 1, resource: BindingResource::TextureView(frame.depth) },
                ]);
            self.binding = Some((key, distortion, None));
        }
        if let (Some((_, pipeline)), Some(binding)) = (&self.colour, self.binding.as_mut()) {
            if binding.2.is_none() {
                binding.2 = Some(device.create_bind_group(Some("dark vision wave colour inputs"),
                    &pipeline.get_bind_group_layout(0).into(), &[
                        BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                        BindGroupEntry { binding: 1, resource: BindingResource::TextureView(frame.depth) },
                            BindGroupEntry { binding: 3, resource: frame.adaptation.as_entire_binding() },
                    ]));
            }
        }
        Ok(())
    }

    /// Prepare the distortion draw; the vector pass binds and draws it after
    /// its other draws.
    pub fn distortion(&mut self, device: &RenderDevice, queue: &RenderQueue, sources: &DarkVisionWaveSources,
        frame: &WaveFrame<'_>) -> Result<(&RenderPipeline, &BindGroup), String> {
        self.prepare(device, queue, sources, frame)?;
        Ok((&self.distortion, &self.binding.as_ref().unwrap().1))
    }

    /// The colour draw into the scene target.
    pub fn record_colour(&mut self, device: &RenderDevice, queue: &RenderQueue, encoder: &mut CommandEncoder,
        sources: &DarkVisionWaveSources, frame: &WaveFrame<'_>, colour: &TextureView, format: TextureFormat) -> Result<(), String> {
        if self.colour.as_ref().is_none_or(|(current, _)| *current != format) {
            if let Some(binding) = self.binding.as_mut() { binding.2 = None; }
            self.colour = Some((format, pipeline(device, &self.module, None, "dark vision wave colour",
                "dark_vision_wave_vertex", "dark_vision_wave_colour", format, &sources.colour_state)?));
        }
        self.prepare(device, queue, sources, frame)?;
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("dark vision wave colour"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: colour,
                depth_slice: None,
                resolve_target: None,
                ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.colour.as_ref().unwrap().1);
        pass.set_bind_group(0, self.binding.as_ref().unwrap().2.as_ref().unwrap(), &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}
