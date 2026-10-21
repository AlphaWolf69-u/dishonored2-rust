//! Execute the production query pipeline, including source count guards/layout.
use super::*;
use bevy::{prelude::*, render::RenderApp};

#[test]
#[ignore = "requires graphics adapter; compares installed ocean height queries"]
fn ocean_height_queries_match_installed_shaders() {
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
    let noise = NoiseTextures::new(device, queue);
    let grids = include_bytes!("../../../../tests/fixtures/ocean_surface_inputs.bin");
    let mask = include_bytes!("../../../../tests/fixtures/ocean_height_query_mask.bin");
    assert_eq!(grids.len(), 1664);
    assert_eq!(mask.len(), 256);
    let mut offset = 0;
    let textures = [
        (TextureFormat::R32Float, 4),
        (TextureFormat::Rgba16Float, 8),
        (TextureFormat::Rgba16Float, 8),
        (TextureFormat::R16Float, 2),
        (TextureFormat::R32Float, 4),
        (TextureFormat::R32Float, 4),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (format, stride))| {
        let bytes = if index == 5 {
            mask.as_slice()
        } else {
            let end = offset + 64 * stride;
            let bytes = &grids[offset..end];
            offset = end;
            bytes
        };
        let size = Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("synthetic source height query grid"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            view_formats: &[],
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        });
        queue.write_texture(
            texture.as_image_copy(),
            bytes,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8 * stride as u32),
                rows_per_image: Some(8),
            },
            size,
        );
        texture
    })
    .collect::<Vec<_>>();
    assert_eq!(offset, grids.len());
    let views = textures
        .iter()
        .map(|texture| texture.create_view(&default()))
        .collect::<Vec<_>>();
    let mut queries = OceanHeightQueries::new(
        device,
        OceanHeightTextures {
            noise: &noise,
            displacement: [&views[1], &views[2]],
            collision_height: &views[3],
            foam: &views[4],
            shore_distance: &views[0],
            amplitude_mask: &views[5],
        },
    );
    let ids = [
        queries.positions.id(),
        queries.evaluations.id(),
        queries.surface.id(),
        queries.controls.id(),
    ];
    let upload_address = queries.position_bytes.as_ptr();
    assert_eq!(queries.buffer_bytes(), 8192 + 16384 + 112 + 16);
    let mut requests = OceanQueryQueue::new();
    let mut readbacks = OceanQueryReadbacks::new(device);
    let readback_ids = readbacks.buffer_ids();
    let readback_addresses = readbacks.completed_rings().map(|values| values.as_ptr());
    assert_eq!(readbacks.buffer_bytes(), 4 * 16384);
    assert!(readbacks.mark_submitted(PollType::Poll).is_err());
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("height query numeric readback"),
        size: 16384,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let sentinel = 0x40a00000u32.to_le_bytes().repeat(OCEAN_QUERY_CAPACITY * 4);
    let mut lines = include_str!("../../../../tests/fixtures/ocean_height_query.txt")
        .lines()
        .peekable();
    let parse = |s: &str| {
        s.split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>()
    };
    let mut cases = 0;
    let mut small_case = None;
    while let Some(header) = lines.next() {
        let words = parse(header.strip_prefix("case ").unwrap());
        assert_eq!(words.len(), 32);
        let group = |i: usize| std::array::from_fn(|j| f32::from_bits(words[i * 4 + j]));
        let parameters = OceanSurfaceParameters {
            spatial: group(0),
            clock_view: group(1),
            camera: group(2),
            perlin: group(3),
            perlin_foam: group(4),
            shore: group(5),
            foam: group(6),
        };
        let count = words[28] as usize;
        let water_level = f32::from_bits(words[29]);
        let mut rows = Vec::new();
        while lines.peek().is_some_and(|line| !line.starts_with("case ")) {
            rows.push(parse(lines.next().unwrap()));
        }
        assert_eq!(rows.len(), OCEAN_QUERY_CAPACITY);
        assert!(rows.iter().all(|row| row.len() == 6));
        let positions = rows[..count]
            .iter()
            .map(|row| [f32::from_bits(row[0]), f32::from_bits(row[1])])
            .collect::<Vec<_>>();
        if count == 1 {
            small_case = Some((
                parameters,
                water_level,
                positions[0],
                std::array::from_fn::<_, 4, _>(|channel| f32::from_bits(rows[0][channel + 2])),
            ));
        }
        queue.write_buffer(queries.evaluations(), 0, &sentinel);
        let ring = cases % 4;
        let mut handle = requests.register((ring + 3) % 4, &positions).unwrap();
        assert!(!handle.is_invalid());
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("height query fixture"),
        });
        queries
            .record_pending(
                queue,
                &mut encoder,
                &mut requests,
                &mut readbacks,
                &parameters,
                water_level,
                cases % 2,
                ring,
            )
            .unwrap();
        assert_eq!(queries.prepared_count(), count as u32);
        assert!(requests.pending_points().is_empty());
        assert_eq!(requests.submitted_count(ring), Some(count));
        encoder.copy_buffer_to_buffer(queries.evaluations(), 0, &staging, 0, 16384);
        let submission = queue.submit([encoder.finish()]);
        readbacks
            .mark_submitted(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        assert!(!readbacks
            .read_current(device, false, ring, &requests)
            .unwrap());
        assert_eq!(
            readbacks
                .read_current(device, true, ring, &requests)
                .unwrap(),
            count > 0
        );
        let mut delivered = vec![[0.0; 4]; count];
        assert_eq!(
            requests
                .take(&mut handle, readbacks.completed_rings(), &mut delivered)
                .unwrap(),
            Some(count)
        );
        assert!(handle.is_invalid());
        for (index, value) in delivered.iter().enumerate() {
            for channel in 0..4 {
                let expected = f32::from_bits(rows[index][channel + 2]);
                assert!(
                    (value[channel] - expected).abs() <= 0.0003 * (1.0 + expected.abs()),
                    "readback count{count} index{index} channel{channel}: {} != {expected}",
                    value[channel]
                );
            }
        }
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
        let mut maximum = [0.0_f32; 4];
        for (index, row) in rows.iter().enumerate() {
            for channel in 0..4 {
                let offset = index * 16 + channel * 4;
                let bits = u32::from_le_bytes(mapped[offset..offset + 4].try_into().unwrap());
                let expected_bits = row[channel + 2];
                if index >= count {
                    assert_eq!(
                        bits, expected_bits,
                        "query wrote past count{count} at{index}"
                    );
                } else {
                    let actual = f32::from_bits(bits);
                    let expected = f32::from_bits(expected_bits);
                    let error = (actual - expected).abs();
                    maximum[channel] = maximum[channel].max(error);
                    assert!(actual.is_finite() && error<=0.0003*(1.0+expected.abs()),
                        "query count{count} index{index} channel{channel}: {actual} != {expected}, error{error}");
                }
            }
        }
        eprintln!("ocean query count{count} level{water_level} maximum errors={maximum:?}");
        drop(mapped);
        staging.unmap();
        assert_eq!(
            [
                queries.positions.id(),
                queries.evaluations.id(),
                queries.surface.id(),
                queries.controls.id()
            ],
            ids
        );
        assert_eq!(queries.position_bytes.as_ptr(), upload_address);
        assert_eq!(queries.buffer_bytes(), 8192 + 16384 + 112 + 16);
        assert_eq!(readbacks.buffer_ids(), readback_ids);
        assert_eq!(
            readbacks.completed_rings().map(|values| values.as_ptr()),
            readback_addresses
        );
        assert!(queries
            .prepare(queue, &parameters, water_level, &vec![[0.0; 2]; 1025])
            .is_err());
        assert_eq!(queries.prepared_count(), count as u32);
        cases += 1;
    }
    assert_eq!(cases, 9);
    // Reuse the full ring0 for a shorter request: only its copied prefix changes.
    let old_tail = readbacks.completed_rings()[0][1..].to_vec();
    let (parameters, level, position, expected) = small_case.unwrap();
    let mut handle = requests.register(3, &[position]).unwrap();
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("shorter query after full ring"),
    });
    queries
        .record_pending(
            queue,
            &mut encoder,
            &mut requests,
            &mut readbacks,
            &parameters,
            level,
            0,
            0,
        )
        .unwrap();
    let submission = queue.submit([encoder.finish()]);
    readbacks
        .mark_submitted(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    assert!(readbacks.read_current(device, true, 0, &requests).unwrap());
    assert!(readbacks.read_current(device, true, 0, &requests).unwrap());
    assert_eq!(readbacks.completed_rings()[0][1..], old_tail);
    let mut delivered = [[0.0; 4]; 1];
    assert_eq!(
        requests
            .take(&mut handle, readbacks.completed_rings(), &mut delivered)
            .unwrap(),
        Some(1)
    );
    for channel in 0..4 {
        assert!(
            (delivered[0][channel] - expected[channel]).abs()
                <= 0.0003 * (1.0 + expected[channel].abs())
        );
    }
    assert_eq!(readbacks.buffer_ids(), readback_ids);
    assert_eq!(
        readbacks.completed_rings().map(|values| values.as_ptr()),
        readback_addresses
    );
    assert!(
        errors.lock().unwrap().is_empty(),
        "GPU errors: {:?}",
        errors.lock().unwrap()
    );
}
