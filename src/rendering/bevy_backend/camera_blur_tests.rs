use super::*;
use bevy::{prelude::*, render::RenderApp};

fn input_bytes(slot: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    for y in 0..8 {
        for x in 0..8 {
            let values = if slot == 0 {
                let strength = [0u16, 1, 2, 4, 6][(x + 2 * y) % 5];
                let angle = ((x * 317 + y * 193) % 4096) as u16;
                [
                    0.,
                    half::f16::from_bits((strength << 12) | angle).to_f32(),
                    0.,
                    1.,
                ]
            } else if slot <= 2 {
                [
                    ((x + 2 * y + slot) % 5) as f32 / 4.,
                    ((3 * x + y + 2 * slot) % 5) as f32 / 4.,
                    0.,
                    1.,
                ]
            } else {
                [
                    ((x * 7 + y * 3 + slot * 5) % 23) as f32 / 4.,
                    ((x * 3 + y * 11 + slot) % 19) as f32 / 3.,
                    ((x + y * 5 + slot * 3) % 17) as f32 / 5.,
                    1.,
                ]
            };
            bytes.extend(
                values
                    .into_iter()
                    .flat_map(|v| half::f16::from_f32(v).to_bits().to_le_bytes()),
            );
        }
    }
    bytes
}

#[test]
#[ignore = "requires GPU and installed assets; native camera blur raster and stores"]
fn camera_colour_blur_matches_installed_shader() {
    let catalog =
        crate::assets::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
            .unwrap();
    let bank = crate::assets::camera_blur::KernelBank::load(&catalog).unwrap();
    let program = catalog
        .read("generated/decls/renderprog/postfx_dof.decl")
        .unwrap()
        .unwrap();
    let mut state = [0; 9];
    crate::render_state::reset_render_state(&mut state);
    let state =
        crate::render_state::material_render_state(&String::from_utf8_lossy(&program), state)
            .unwrap();
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
    let kernels: Vec<_> = (0..6)
        .map(|kind| BlurKernelBuffer::new(device, bank.get(kind).unwrap()))
        .collect();
    let inputs: Vec<_> = (0..5)
        .map(|slot| {
            device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some("camera colour blur fixture"),
                    size: Extent3d {
                        width: 8,
                        height: 8,
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
                &input_bytes(slot),
            )
        })
        .collect();
    let images: Vec<_> = inputs.iter().map(|t| t.create_view(&default())).collect();
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("camera colour blur readback"),
        size: 256 * 20,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut channels = 0;
    let mut max_relative = 0f32;
    let mut max_packed = 0;
    let mut max_alpha = 0;
    let mut float_mismatches = 0;
    for format in [TextureFormat::Rgba32Float, TextureFormat::Rg11b10Ufloat] {
        let mut passes: Vec<_> = [DofPass::Alpha, DofPass::Near, DofPass::Far]
            .into_iter()
            .map(|pass| {
                CameraBlurPass::new(
                    device,
                    CameraBlurConfig {
                        pass,
                        colour_format: format,
                        alpha_format: (pass != DofPass::Far).then_some(TextureFormat::R8Unorm),
                        depth_format: None,
                        state,
                    },
                )
                .unwrap()
            })
            .collect();
        let mut lines = include_str!("../../../tests/fixtures/camera_blur.txt").lines();
        let mut cases = 0;
        while let Some(header) = lines.next() {
            let words: Vec<_> = header.split_whitespace().collect();
            let w: u32 = words[1].parse().unwrap();
            let h: u32 = words[2].parse().unwrap();
            let hex = words[3];
            let raw: Vec<_> = (0..96)
                .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap())
                .collect();
            let f = |i: usize| f32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap());
            let u = |i: usize| u32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap());
            let mode = u(20) as usize;
            let kind = u(21) as usize;
            let flags = u(23);
            let parameters = CameraBlurParameters {
                target_size: [f(0), f(1), f(2), f(3)],
                fringing: [f(4), f(5), f(6)],
                bokeh_size: f(7),
                vignette: [f(8), f(9), f(10), f(11)],
                centre: [f(12), f(13)],
                resolution_scale: [f(14), f(15)],
                position_to_texture: [f(16), f(17)],
                use_fringing: flags & 1 != 0,
                cat_eye: flags & 2 != 0,
            };
            let pass = [DofPass::Alpha, DofPass::Near, DofPass::Far][mode];
            assert_eq!(parameters.bytes(pass, &kernels[kind]).as_slice(), raw);
            let decode = |s: &str| {
                (0..s.len() / 2)
                    .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
                    .collect::<Vec<_>>()
            };
            let expected_float = decode(lines.next().unwrap().strip_prefix("FLOAT ").unwrap());
            let expected_dynamic = decode(lines.next().unwrap().strip_prefix("DYNAMIC ").unwrap());
            let expected_packed = decode(lines.next().unwrap().strip_prefix("PACKED ").unwrap());
            let expected_alpha = decode(lines.next().unwrap().strip_prefix("ALPHA ").unwrap());
            let desc = |format| TextureDescriptor {
                label: Some("camera colour blur target"),
                size: Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                view_formats: &[],
            };
            let colour = device.create_texture(&desc(format));
            let cv = colour.create_view(&default());
            let alpha = device.create_texture(&desc(TextureFormat::R8Unorm));
            let av = alpha.create_view(&default());
            let mut encoder = device.create_command_encoder(&default());
            passes[mode]
                .record(
                    device,
                    queue,
                    &mut encoder,
                    &parameters,
                    CameraBlurInputs {
                        images: std::array::from_fn(|i| &images[i]),
                        kernel: &kernels[kind],
                    },
                    CameraBlurTargets {
                        colour: &cv,
                        alpha: (mode != 2).then_some(&av),
                        depth_stencil: None,
                        stencil_reference: 0,
                        viewport: [0, 0, w, h],
                        scissor: [0, 0, w, h],
                    },
                )
                .unwrap();
            for (plane, texture) in std::iter::once(&colour)
                .chain((mode != 2).then_some(&alpha))
                .enumerate()
            {
                encoder.copy_texture_to_buffer(
                    texture.as_image_copy(),
                    TexelCopyBufferInfo {
                        buffer: &staging,
                        layout: TexelCopyBufferLayout {
                            offset: plane as u64 * 256 * 10,
                            bytes_per_row: Some(256),
                            rows_per_image: Some(h),
                        },
                    },
                    desc(format).size,
                );
            }
            let submission = queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            staging.slice(..).map_async(MapMode::Read, move |v| {
                let _ = tx.send(v);
            });
            device
                .poll(PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(std::time::Duration::from_secs(30)),
                })
                .unwrap();
            rx.recv().unwrap().unwrap();
            let data = staging.slice(..).get_mapped_range();
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let pixel = y * w as usize + x;
                    if format == TextureFormat::Rgba32Float {
                        for c in 0..4 {
                            let at = y * 256 + x * 16 + c * 4;
                            let actual = f32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                            let at = pixel * 16 + c * 4;
                            let want =
                                f32::from_le_bytes(expected_float[at..at + 4].try_into().unwrap());
                            let error = (actual - want).abs() / want.abs().max(1.);
                            max_relative = max_relative.max(error);
                            assert!(
                                actual.is_finite() && want.is_finite(),
                                "case{cases} kind{kind} mode{mode} flags{flags} pixel{x},{y} channel{c}: {actual} != {want}, normalized error{error}"
                            );
                            // A fixed-count and an equal uniform-count D3D11 loop can
                            // compile to slightly different tap positions. Preserve the
                            // fixed-count diagnostic, but compare the raw precision of
                            // equivalent runtime loops separately. R11/R8 below always
                            // compare against the unmodified fixed-count source.
                            if error > 0.00005 {
                                float_mismatches += 1;
                            }
                            let dynamic = f32::from_le_bytes(
                                expected_dynamic[at..at + 4].try_into().unwrap(),
                            );
                            let dynamic_error = (actual - dynamic).abs() / dynamic.abs().max(1.);
                            assert!(dynamic.is_finite() && dynamic_error <= 0.00005,
                                "runtime-loop case{cases} kind{kind} mode{mode} flags{flags} pixel{x},{y} channel{c}: {actual} != {dynamic}, normalized error{dynamic_error}");
                            channels += 1;
                        }
                    } else {
                        let at = y * 256 + x * 4;
                        let actual = u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                        let at = pixel * 4;
                        let want =
                            u32::from_le_bytes(expected_packed[at..at + 4].try_into().unwrap());
                        for (shift, mask) in [(0, 2047), (11, 2047), (22, 1023)] {
                            let distance =
                                ((actual >> shift) & mask).abs_diff((want >> shift) & mask);
                            max_packed = max_packed.max(distance);
                            assert!(
                                distance <= 1,
                                "packed case{cases} kind{kind} mode{mode} flags{flags} pixel{x},{y} shift{shift}: {actual:08x} != {want:08x}, {distance} steps"
                            );
                            channels += 1;
                        }
                    }
                    if mode != 2 {
                        let actual = data[256 * 10 + y * 256 + x];
                        let want = expected_alpha[pixel];
                        let error = actual.abs_diff(want);
                        max_alpha = max_alpha.max(error);
                        assert!(
                            error <= 1,
                            "MRT alpha case{cases} pixel{x},{y}: {actual} != {want}"
                        );
                        channels += 1;
                    }
                }
            }
            drop(data);
            staging.unmap();
            cases += 1;
        }
        assert_eq!(cases, 102);
    }
    assert_eq!(channels, 54804);
    eprintln!(
        "Compared 102 camera colour blur variants, {channels} channels; maximum fixed-loop float diagnostic error {max_relative} ({float_mismatches} channels above 5e-5), R11 {max_packed} steps, MRT alpha {max_alpha} bytes"
    );
}
