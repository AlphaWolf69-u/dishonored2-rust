//! Numerical buffer readback only: no window, screenshots or game process.
use super::*;
use bevy::{prelude::*, render::RenderApp};

#[test]
#[ignore = "requires graphics adapter; compares installed wave shaders"]
fn ocean_wave_compute_matches_installed_shaders() {
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
    let errors = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = errors.clone();
    device
        .wgpu_device()
        .on_uncaptured_error(std::sync::Arc::new(move |error| {
            captured.lock().unwrap().push(format!("{error}"));
        }));
    let mut waves = OceanWaves::new(
        device,
        queue,
        WaveSpectrumParameters {
            wind: [1., 0.],
            amplitude: 1.,
            wind_speed: 10.,
            opposite_wind_attenuation: 0.1,
            domain_length: 256.,
            short_wave_divisor: 100.,
        },
    )
    .unwrap();
    let ids = [
        &waves.initial,
        &waves.frequency,
        &waves.spectrum,
        &waves.scratch,
        &waves.transformed,
        &waves.clock,
    ]
    .map(Buffer::id);
    let payload = waves.buffer_bytes();
    let size = waves.spectrum.size();
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("ocean numeric fixture readback"),
        size: size * 2,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut input = include_str!("../../../../tests/fixtures/ocean_wave_compute.txt")
        .lines()
        .peekable();
    let mut frames = 0;
    while let Some(header) = input.next() {
        let words = header.split_whitespace().collect::<Vec<_>>();
        assert_eq!(words[0], "frame");
        let time = f32::from_bits(u32::from_str_radix(words[1], 16).unwrap());
        let scale = f32::from_bits(u32::from_str_radix(words[2], 16).unwrap());
        waves.prepare_time(queue, time, scale);
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("ocean fixture frame"),
        });
        waves.record_spectrum(&mut encoder);
        encoder.copy_buffer_to_buffer(&waves.spectrum, 0, &staging, 0, size);
        waves.record_fft(&mut encoder);
        encoder.copy_buffer_to_buffer(waves.transformed_spectrum(), 0, &staging, size, size);
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = staging.slice(..).get_mapped_range();
        let mut samples = 0;
        let mut maximum_error = [0.0_f32; 2];
        while input.peek().is_some_and(|line| !line.starts_with("frame ")) {
            let row = input
                .next()
                .unwrap()
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            let index = row[0] as usize;
            for component in 0..4 {
                let channel = component / 2;
                let offset = channel * size as usize + index * 8 + (component % 2) * 4;
                let actual = f32::from_le_bytes(mapped[offset..offset + 4].try_into().unwrap());
                let expected = f32::from_bits(row[component + 1]);
                let tolerance = if channel == 0 { 0.00002 } else { 0.0002 };
                let error = (actual - expected).abs();
                maximum_error[channel] = maximum_error[channel].max(error);
                assert!(actual.is_finite() && error <= tolerance * (1.0 + expected.abs()),
                    "frame{frames} index{index} component{component}: {actual} != {expected}, error{error}");
            }
            samples += 1;
        }
        assert_eq!(samples, 300);
        eprintln!("ocean frame{frames} numeric max errors spectrum/FFT={maximum_error:?}, payload={payload}");
        drop(mapped);
        staging.unmap();
        assert_eq!(
            [
                &waves.initial,
                &waves.frequency,
                &waves.spectrum,
                &waves.scratch,
                &waves.transformed,
                &waves.clock
            ]
            .map(Buffer::id),
            ids
        );
        assert_eq!(waves.buffer_bytes(), payload);
        frames += 1;
    }
    assert_eq!(frames, 3);
    check_surfaces(device, queue, &mut waves);
    assert!(
        errors.lock().unwrap().is_empty(),
        "GPU errors: {:?}",
        errors.lock().unwrap()
    );
}

fn check_surfaces(device: &RenderDevice, queue: &RenderQueue, waves: &mut OceanWaves) {
    let image_bytes = u64::from(WAVE_RESOLUTION) * u64::from(WAVE_RESOLUTION) * 8;
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("ocean surface numeric readback"),
        size: image_bytes * 3 + image_bytes / 2 + image_bytes / 4,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let gradient_id = waves.gradients().id();
    let mut last_texture = waves.displacement().id();
    let mut previous_pixels: Vec<(usize, [u16; 4])> = Vec::new();
    let mut lines = include_str!("../../../../tests/fixtures/ocean_wave_surfaces.txt")
        .lines()
        .peekable();
    let mut frames = 0;
    let mut foam_lines = include_str!("../../../../tests/fixtures/ocean_wave_foam.txt")
        .lines()
        .peekable();
    let foam_id = waves.foam_mask().id();
    let collision_id = waves.collision_height().id();
    let mut collision_lines = include_str!("../../../../tests/fixtures/ocean_wave_collision.txt")
        .lines()
        .peekable();
    while let Some(header) = lines.next() {
        let row = header
            .strip_prefix("frame ")
            .unwrap()
            .split_whitespace()
            .map(|v| f32::from_bits(u32::from_str_radix(v, 16).unwrap()))
            .collect::<Vec<_>>();
        waves.prepare_time(queue, row[0], row[1]);
        waves.prepare_choppiness(queue, row[2]);
        let foam_header = foam_lines
            .next()
            .unwrap()
            .strip_prefix("frame ")
            .unwrap()
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        let mode = FoamMode::from_type(foam_header[0] as i32);
        waves.prepare_foam(
            queue,
            FoamSettings::from_parameters(
                crate::rendering::ocean::WaveFoamParameters {
                    threshold: f32::from_bits(foam_header[1]),
                    spawn_rate: f32::from_bits(foam_header[2]),
                    decay_rate: f32::from_bits(foam_header[3]),
                },
                f32::from_bits(foam_header[4]),
            ),
        );
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("ocean surface fixture"),
        });
        waves.record_spectrum(&mut encoder);
        waves.record_fft(&mut encoder);
        waves.record_surfaces(&mut encoder);
        waves.record_collision_height(&mut encoder);
        waves.record_foam(&mut encoder, mode);
        assert_eq!(waves.collision_height().id(), collision_id);
        assert_eq!(waves.foam_mask().id(), foam_id);
        assert_eq!(waves.previous_displacement().id(), last_texture);
        assert_ne!(waves.displacement().id(), last_texture);
        assert_eq!(waves.gradients().id(), gradient_id);
        last_texture = waves.displacement().id();
        for (index, texture) in [
            waves.displacement(),
            waves.gradients(),
            waves.previous_displacement(),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(texture.format(), TextureFormat::Rgba16Float);
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset: index as u64 * image_bytes,
                        bytes_per_row: Some(WAVE_RESOLUTION * 8),
                        rows_per_image: Some(WAVE_RESOLUTION),
                    },
                },
                texture.size(),
            );
        }
        encoder.copy_texture_to_buffer(
            waves.foam_mask().as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: image_bytes * 3,
                    bytes_per_row: Some(WAVE_RESOLUTION * 4),
                    rows_per_image: Some(WAVE_RESOLUTION),
                },
            },
            waves.foam_mask().size(),
        );
        encoder.copy_texture_to_buffer(
            waves.collision_height().as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: image_bytes * 3 + image_bytes / 2,
                    bytes_per_row: Some(WAVE_RESOLUTION * 2),
                    rows_per_image: Some(WAVE_RESOLUTION),
                },
            },
            waves.collision_height().size(),
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = staging.slice(..).get_mapped_range();
        let read_half = |offset| u16::from_le_bytes(mapped[offset..offset + 2].try_into().unwrap());
        let mut current_pixels = Vec::new();
        let mut samples = 0;
        let mut maximum_error = [0.0_f32; 2];
        while lines.peek().is_some_and(|line| !line.starts_with("frame ")) {
            let row = lines
                .next()
                .unwrap()
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(row.len(), 9);
            let pixel = row[0] as usize;
            let current = std::array::from_fn(|channel| read_half(pixel * 8 + channel * 2));
            for channel in 0..8 {
                let image = channel / 4;
                let bits = read_half(image * image_bytes as usize + pixel * 8 + (channel % 4) * 2);
                let actual = half::f16::from_bits(bits).to_f32();
                let expected = half::f16::from_bits(row[channel + 1] as u16).to_f32();
                let error = (actual - expected).abs();
                maximum_error[image] = maximum_error[image].max(error);
                assert!(
                    actual.is_finite() && error <= 0.0015 * (1.0 + expected.abs()),
                    "surface frame{frames} pixel{pixel} channel{channel}: {actual} != {expected}"
                );
            }
            if frames > 0 {
                assert_eq!(previous_pixels[samples].0, pixel);
                let previous: [u16; 4] = std::array::from_fn(|channel| {
                    read_half(image_bytes as usize * 2 + pixel * 8 + channel * 2)
                });
                assert_eq!(
                    previous, previous_pixels[samples].1,
                    "previous wave overwritten at pixel{pixel}"
                );
            }
            current_pixels.push((pixel, current));
            samples += 1;
        }
        assert_eq!(samples, 100);
        let mut foam_samples = 0;
        let mut foam_error = 0.0_f32;
        while foam_lines
            .peek()
            .is_some_and(|line| !line.starts_with("frame "))
        {
            let row = foam_lines
                .next()
                .unwrap()
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            let offset = image_bytes as usize * 3 + row[0] as usize * 4;
            let actual = f32::from_le_bytes(mapped[offset..offset + 4].try_into().unwrap());
            let expected = f32::from_bits(row[1]);
            foam_error = foam_error.max((actual - expected).abs());
            assert!(
                actual.is_finite() && (actual - expected).abs() < 0.00002,
                "foam frame{frames} pixel{}: {actual} != {expected}",
                row[0]
            );
            foam_samples += 1;
        }
        assert_eq!(foam_samples, 100);
        assert_eq!(collision_lines.next().unwrap(), format!("frame {frames}"));
        let mut collision_samples = 0;
        let mut collision_error = 0.0_f32;
        while collision_lines
            .peek()
            .is_some_and(|line| !line.starts_with("frame "))
        {
            let row = collision_lines
                .next()
                .unwrap()
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            let offset = (image_bytes * 3 + image_bytes / 2) as usize + row[0] as usize * 2;
            let actual = half::f16::from_bits(read_half(offset)).to_f32();
            let expected = half::f16::from_bits(row[1] as u16).to_f32();
            let error = (actual - expected).abs();
            collision_error = collision_error.max(error);
            assert!(
                actual.is_finite() && error <= 0.0015 * (1.0 + expected.abs()),
                "collision frame{frames} pixel{}: {actual} != {expected}",
                row[0]
            );
            collision_samples += 1;
        }
        assert_eq!(collision_samples, 100);
        eprintln!("ocean collision frame{frames} max error={collision_error}");
        eprintln!("ocean foam frame{frames} {mode:?} max error={foam_error}");
        eprintln!("ocean surface frame{frames} displacement/gradient max errors={maximum_error:?}");
        previous_pixels = current_pixels;
        drop(mapped);
        staging.unmap();
        frames += 1;
    }
    assert_eq!(frames, 3);
}
