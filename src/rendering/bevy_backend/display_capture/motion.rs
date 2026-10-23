//! Optional GPU history for intermittent bloom defects. Disabled during normal play.
use super::*;
use bevy::render::renderer::RenderQueue;
use std::time::Instant;

const WIDTH: u32 = 96;
const HEIGHT: u32 = 54;
const HEADER_WORDS: u32 = 32;
const FRAME_WORDS: u32 = HEADER_WORDS + WIDTH * HEIGHT * 6;
const SLOTS: u32 = 1024;

pub(super) struct MotionCapture {
    enabled: bool,
    history: Option<History>,
    pub(super) pending: Option<Pending>,
}
impl Default for MotionCapture {
    fn default() -> Self {
        Self {
            enabled: std::env::var("DH2_TRACE_BLOOM").as_deref() == Ok("1"),
            history: None,
            pending: None,
        }
    }
}
impl MotionCapture {
    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }
    pub(super) fn record(
        &mut self,
        request: &DisplayCaptureRequest,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        textures: [&Texture; 3],
        exposure: &Buffer,
        controls: [f32; 4],
    ) {
        if !self.enabled {
            return;
        }
        if self.history.is_none() {
            match History::new(device, SLOTS) {
                Ok(history) => self.history = Some(history),
                Err(error) => {
                    warn!("Bloom history disabled: {error}");
                    self.enabled = false;
                    return;
                }
            }
        }
        let history = self.history.as_mut().unwrap();
        history.record(device, queue, encoder, textures, exposure, controls);
        if self.pending.is_some() {
            return;
        }
        let Some(path) = request.0.lock().unwrap().clone() else {
            return;
        };
        let staging = device.create_buffer(&BufferDescriptor {
            label: Some("F8 bloom history readback"),
            size: history.buffer.size(),
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&history.buffer, 0, &staging, 0, staging.size());
        self.pending = Some(Pending {
            buffer: staging, path: path.with_file_name(format!("{}-motion", path.file_name().unwrap().to_string_lossy())),
            metadata: format!("Display motion capture v2\nwidth={WIDTH} height={HEIGHT} slots={SLOTS} header_words={HEADER_WORDS} frame_words={FRAME_WORDS} next_serial={}\nHeader: serial, slot, elapsed_seconds_bits, reserved, host_units_bits, veil_strength_bits, white_level_bits, dirt_strength_bits, eight exposure words, hdr_width, hdr_height, bloom_width, bloom_height, window_width, window_height, window_serial; zero padding to header_words.\nPixels: row-major interleaved plane0, post-lens display, bloom; each RGBA16Float packed into two little-endian u32. When window_serial equals serial, plane0 is the final window buffer after UI and scaling; otherwise it is HDR before the lens pass. F8 snapshots before the last window record, so the newest frame normally still contains HDR. Sort valid slots by serial; serial zero is empty. Nearest pixel-center samples; this is diagnostic downsampling, not a rendering pass.\n", history.serial),
            mapped: false, result: default(),
        });
    }
    pub(super) fn record_window(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        view: TextureView,
        size: UVec2,
    ) -> bool {
        let Some(history) = self
            .history
            .as_mut()
            .filter(|h| h.serial > 1 && h.window_serial != h.serial - 1)
        else {
            return false;
        };
        history.record_window(device, queue, encoder, view, size);
        true
    }
}

struct History {
    buffer: Buffer,
    parameters: Buffer,
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
    bindings: Vec<([TextureId; 3], BufferId, BindGroup)>,
    serial: u32,
    slots: u32,
    start: Instant,
    window: Option<WindowHistory>,
    window_serial: u32,
}
impl History {
    fn new(device: &RenderDevice, slots: u32) -> Result<Self, String> {
        use binding_types::*;
        let size = u64::from(FRAME_WORDS) * 4 * u64::from(slots);
        if slots == 0
            || size > u64::from(device.limits().max_storage_buffer_binding_size)
            || size > device.limits().max_buffer_size
        {
            return Err("history exceeds adapter buffer limits".into());
        }
        let layout = device.create_bind_group_layout(
            Some("bloom motion history"),
            &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    uniform_buffer::<[UVec4; 2]>(false),
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    storage_buffer_read_only_sized(false, std::num::NonZeroU64::new(32)),
                    storage_buffer_sized(false, None),
                ),
            ),
        );
        let source = SHADER
            .replace("_WIDTH_", &WIDTH.to_string())
            .replace("_HEIGHT_", &HEIGHT.to_string());
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("bloom motion history"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("bloom motion history"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: Some("bloom motion history"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("record"),
            compilation_options: default(),
            cache: None,
        });
        Ok(Self {
            buffer: device.create_buffer(&BufferDescriptor {
                label: Some("bounded bloom history"),
                size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            parameters: device.create_buffer(&BufferDescriptor {
                label: Some("bloom history parameters"),
                size: 32,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            layout,
            pipeline,
            bindings: Vec::new(),
            serial: 1,
            slots,
            start: Instant::now(),
            window: None,
            window_serial: 0,
        })
    }
    fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        textures: [&Texture; 3],
        exposure: &Buffer,
        controls: [f32; 4],
    ) {
        let words = [
            self.serial,
            (self.serial - 1) % self.slots,
            self.start.elapsed().as_secs_f32().to_bits(),
            0,
            controls[0].to_bits(),
            controls[1].to_bits(),
            controls[2].to_bits(),
            controls[3].to_bits(),
        ];
        let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
        queue.write_buffer(&self.parameters, 0, &bytes);
        let ids = textures.map(Texture::id);
        let slot = self
            .bindings
            .iter()
            .position(|v| v.0 == ids && v.1 == exposure.id())
            .unwrap_or_else(|| {
                let views = textures.map(|t| t.create_view(&TextureViewDescriptor::default()));
                let binding = device.create_bind_group(
                    Some("bloom history sources"),
                    &self.layout,
                    &BindGroupEntries::sequential((
                        self.parameters.as_entire_binding(),
                        &views[0],
                        &views[1],
                        &views[2],
                        exposure.as_entire_binding(),
                        self.buffer.as_entire_binding(),
                    )),
                );
                if self.bindings.len() == 2 {
                    self.bindings.remove(0);
                }
                self.bindings.push((ids, exposure.id(), binding));
                self.bindings.len() - 1
            });
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("bloom motion history"),
            ..default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bindings[slot].2, &[]);
        pass.dispatch_workgroups(WIDTH.div_ceil(8), HEIGHT.div_ceil(8), 1);
        self.serial = self
            .serial
            .checked_add(1)
            .expect("bloom diagnostic frame counter exhausted");
    }
    fn record_window(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        view: TextureView,
        size: UVec2,
    ) {
        if self.window_serial == self.serial - 1 {
            return;
        }
        let window = self
            .window
            .get_or_insert_with(|| WindowHistory::new(device));
        let serial = self.serial - 1;
        let words = [serial, (serial - 1) % self.slots, size.x, size.y];
        let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
        queue.write_buffer(&window.parameters, 0, &bytes);
        let binding = device.create_bind_group(
            Some("final window history source"),
            &window.layout,
            &BindGroupEntries::sequential((
                &view,
                window.parameters.as_entire_binding(),
                self.buffer.as_entire_binding(),
            )),
        );
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("final window history"),
            ..default()
        });
        pass.set_pipeline(&window.pipeline);
        pass.set_bind_group(0, &binding, &[]);
        pass.dispatch_workgroups(WIDTH.div_ceil(8), HEIGHT.div_ceil(8), 1);
        self.window_serial = serial;
    }
}

struct WindowHistory {
    parameters: Buffer,
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
}
impl WindowHistory {
    fn new(device: &RenderDevice) -> Self {
        use binding_types::*;
        let layout = device.create_bind_group_layout(
            Some("final window history"),
            &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    uniform_buffer::<UVec4>(false),
                    storage_buffer_sized(false, None),
                ),
            ),
        );
        let source = WINDOW_SHADER
            .replace("_WIDTH_", &WIDTH.to_string())
            .replace("_HEIGHT_", &HEIGHT.to_string());
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("final window history"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("final window history"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: Some("final window history"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("record"),
            compilation_options: default(),
            cache: None,
        });
        let parameters = device.create_buffer(&BufferDescriptor {
            label: Some("final window history parameters"),
            size: 16,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            parameters,
            layout,
            pipeline,
        }
    }
}

const WINDOW_SHADER: &str = r#"
const WIDTH:u32=_WIDTH_;
const HEIGHT:u32=_HEIGHT_;
const FRAME_WORDS:u32=32u+WIDTH*HEIGHT*6u;
@group(0) @binding(0) var image:texture_2d<f32>;
@group(0) @binding(1) var<uniform> frame_info:vec4<u32>;
@group(0) @binding(2) var<storage,read_write> history:array<u32>;
@compute @workgroup_size(8,8) fn record(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=WIDTH || id.y>=HEIGHT {return;}
    let base=frame_info.y*FRAME_WORDS;
    if id.x==0u && id.y==0u {
        history[base+20u]=frame_info.z;history[base+21u]=frame_info.w;history[base+22u]=frame_info.x;
    }
    let uv=(vec2<f32>(id.xy)+vec2(0.5))/vec2<f32>(f32(WIDTH),f32(HEIGHT));
    let pixel=min(vec2<u32>(uv*vec2<f32>(textureDimensions(image))),textureDimensions(image)-vec2(1u));
    let value=textureLoad(image,vec2<i32>(pixel),0);
    let at=base+32u+(id.y*WIDTH+id.x)*6u;
    history[at]=pack2x16float(value.xy);history[at+1u]=pack2x16float(value.zw);
}
"#;

const SHADER: &str = r#"
const WIDTH:u32=_WIDTH_;
const HEIGHT:u32=_HEIGHT_;
const HEADER:u32=32u;
const FRAME_WORDS:u32=HEADER+WIDTH*HEIGHT*6u;
struct Parameters {frame_info:vec4<u32>,controls:vec4<u32>}
@group(0) @binding(0) var<uniform> p:Parameters;
@group(0) @binding(1) var hdr:texture_2d<f32>;
@group(0) @binding(2) var display:texture_2d<f32>;
@group(0) @binding(3) var bloom:texture_2d<f32>;
@group(0) @binding(4) var<storage,read> exposure:array<vec4<u32>>;
@group(0) @binding(5) var<storage,read_write> history:array<u32>;
fn sample_pixel(image:texture_2d<f32>,uv:vec2<f32>)->vec4<f32> {
    return textureLoad(image,vec2<i32>(min(vec2<u32>(uv*vec2<f32>(textureDimensions(image))),textureDimensions(image)-vec2(1u))),0);
}
@compute @workgroup_size(8,8) fn record(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=WIDTH || id.y>=HEIGHT {return;}
    let base=p.frame_info.y*FRAME_WORDS;
    if id.x==0u && id.y==0u {
        for(var i=0u;i<4u;i++) {history[base+i]=p.frame_info[i];history[base+4u+i]=p.controls[i];}
        for(var i=0u;i<8u;i++) {history[base+8u+i]=exposure[i/4u][i%4u];}
        history[base+16u]=textureDimensions(hdr).x;history[base+17u]=textureDimensions(hdr).y;
        history[base+18u]=textureDimensions(bloom).x;history[base+19u]=textureDimensions(bloom).y;
        history[base+20u]=0u;history[base+21u]=0u;history[base+22u]=0u;
    }
    let uv=(vec2<f32>(id.xy)+vec2(0.5))/vec2<f32>(f32(WIDTH),f32(HEIGHT));
    let a=sample_pixel(hdr,uv);let b=sample_pixel(display,uv);let c=sample_pixel(bloom,uv);
    let at=base+HEADER+(id.y*WIDTH+id.x)*6u;
    history[at]=pack2x16float(a.xy);history[at+1u]=pack2x16float(a.zw);
    history[at+2u]=pack2x16float(b.xy);history[at+3u]=pack2x16float(b.zw);
    history[at+4u]=pack2x16float(c.xy);history[at+5u]=pack2x16float(c.zw);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires GPU; verifies synchronized history sampling and ring wrap"]
    fn bloom_motion_history_preserves_frames_and_wraps() {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
        );
        app.finish();
        app.cleanup();
        let world = app.sub_app(RenderApp).world();
        let device = world.resource::<RenderDevice>();
        let queue = world.resource::<RenderQueue>();
        let mut textures = Vec::new();
        let mut payloads = Vec::new();
        for (plane, [w, h]) in [[7, 3], [11, 5], [3, 2]].into_iter().enumerate() {
            let bytes: Vec<_> = (0..h)
                .flat_map(|y| {
                    (0..w).flat_map(move |x| {
                        [x + plane as u32 * 10, y + 100, 200 + plane as u32, 1]
                            .into_iter()
                            .flat_map(|v| half::f16::from_f32(v as f32).to_bits().to_le_bytes())
                    })
                })
                .collect();
            textures.push(device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: None,
                    size: Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Rgba16Float,
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                TextureDataOrder::MipMajor,
                &bytes,
            ));
            payloads.push(bytes);
        }
        let exposure = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 32,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut history = History::new(device, 4).unwrap();
        let window_bytes: Vec<u8> = (0..5u8)
            .flat_map(|y| (0..9u8).flat_map(move |x| [128, y * 32, x * 16 + 32, 255]))
            .collect();
        let window = device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("window history sRGB fixture"),
                size: Extent3d {
                    width: 9,
                    height: 5,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Bgra8Unorm,
                usage: TextureUsages::COPY_SRC,
                view_formats: &[],
            },
            TextureDataOrder::MipMajor,
            &window_bytes,
        );
        let window_copy = device.create_texture(&TextureDescriptor {
            label: Some("window history copy fixture"),
            size: window.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Bgra8UnormSrgb,
            usage: TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        for serial in 1..=6u32 {
            let data: Vec<_> = (0..8)
                .flat_map(|i| (serial as f32 + i as f32 * 0.25).to_le_bytes())
                .collect();
            queue.write_buffer(&exposure, 0, &data);
            let mut encoder = device.create_command_encoder(&default());
            history.record(
                device,
                queue,
                &mut encoder,
                [&textures[0], &textures[1], &textures[2]],
                &exposure,
                [serial as f32 * 0.5, 0.1, 1., 2.],
            );
            queue.submit([encoder.finish()]);
            if serial <= 3 || serial == 5 {
                // A separate submission matches the final-window graph hook.
                let mut encoder = device.create_command_encoder(&default());
                encoder.copy_texture_to_texture(
                    window.as_image_copy(),
                    window_copy.as_image_copy(),
                    window.size(),
                );
                history.record_window(
                    device,
                    queue,
                    &mut encoder,
                    window_copy.create_view(&default()),
                    UVec2::new(9, 5),
                );
                queue.submit([encoder.finish()]);
            }
        }
        let staging = device.create_buffer(&BufferDescriptor {
            label: None,
            size: history.buffer.size(),
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&default());
        encoder.copy_buffer_to_buffer(&history.buffer, 0, &staging, 0, staging.size());
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging.slice(..).map_async(MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = staging.slice(..).get_mapped_range();
        for serial in 3..=6u32 {
            let offset = (((serial - 1) % 4) * FRAME_WORDS * 4) as usize;
            let word = |i: usize| {
                u32::from_le_bytes(
                    bytes[offset + i * 4..offset + i * 4 + 4]
                        .try_into()
                        .unwrap(),
                )
            };
            assert_eq!(word(0), serial);
            assert_eq!(word(1), (serial - 1) % 4);
            assert_eq!(f32::from_bits(word(4)), serial as f32 * 0.5);
            assert_eq!(f32::from_bits(word(8)), serial as f32);
            assert_eq!([word(16), word(17), word(18), word(19)], [7, 3, 3, 2]);
            let has_window = serial == 3 || serial == 5;
            assert_eq!(
                [word(20), word(21), word(22)],
                if has_window {
                    [9, 5, serial]
                } else {
                    [0, 0, 0]
                }
            );
            for (plane, payload) in payloads.iter().enumerate() {
                for pixel in [0, WIDTH as usize * HEIGHT as usize - 1] {
                    let at = offset + HEADER_WORDS as usize * 4 + pixel * 24 + plane * 8;
                    let from = if pixel == 0 { 0 } else { payload.len() - 8 };
                    if plane == 0 && has_window {
                        let from = if pixel == 0 {
                            0
                        } else {
                            window_bytes.len() - 4
                        };
                        for (channel, source) in [2, 1, 0, 3].into_iter().enumerate() {
                            let value = f32::from(window_bytes[from + source]) / 255.;
                            let expected = if channel == 3 || value <= 0.04045 {
                                if channel == 3 {
                                    value
                                } else {
                                    value / 12.92
                                }
                            } else {
                                ((value + 0.055) / 1.055).powf(2.4)
                            };
                            let actual = half::f16::from_bits(u16::from_le_bytes(
                                bytes[at + channel * 2..at + channel * 2 + 2]
                                    .try_into()
                                    .unwrap(),
                            ))
                            .to_f32();
                            assert!((actual-expected).abs()<0.001, "sRGB window serial {serial} channel {channel}: {actual} vs {expected}");
                        }
                        continue;
                    }
                    assert_eq!(
                        &bytes[at..at + 8],
                        &payload[from..from + 8],
                        "serial {serial} plane {plane} pixel {pixel}"
                    );
                }
            }
        }
        drop(bytes);
        staging.unmap();
    }
}
