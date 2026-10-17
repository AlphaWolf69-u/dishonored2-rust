use super::*;

fn auxiliary_input() -> NativeAuxiliaryReceiverInput {
    NativeAuxiliaryReceiverInput {
        camera_clip_rows: [
            [1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0],
        ],
        camera_position: [0.0; 3],
        ray_direction: [0.3, -0.7, -1.0],
        maximum_distance: 0.25,
    }
}

#[test]
fn auxiliary_and_ordinary_share_original_plane_setup() {
    let input = auxiliary_input();
    let auxiliary = initialize_auxiliary_receiver(input).unwrap().unwrap();
    let mut projection = NativeReceiverDepthProjection {
        receiver_rows: [[0.0; 4]; 4], transposed_rows: [[0.0; 4]; 4],
        mask: Box::new([0; 128 * 128]),
    };
    let depth = [1.0; 4096];
    let ordinary = initialize_native_receiver(NativeReceiverInput {
        flags: 0,
        camera_clip_rows: input.camera_clip_rows, camera_position: input.camera_position,
        ray_direction: input.ray_direction,
        maximum_depth: Some(1.0), maximum_distance: input.maximum_distance,
        world_bounds: [[-2.0, -2.0, 0.1], [2.0, 2.0, 1.0]],
        dynamic_bounds: &[], ordinary_depth: &depth,
    }, &mut projection).unwrap();
    assert_eq!(auxiliary.distance.to_bits(), ordinary.distance.to_bits());
    assert_eq!(auxiliary.finite_far, ordinary.finite_far);
    assert_eq!(auxiliary.cutoff.to_bits(), ordinary.cutoff.to_bits());
    assert_eq!(auxiliary.plane_count, ordinary.plane_count);
    for (a, b) in auxiliary.planes.iter().zip(ordinary.planes) {
        assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
    }
    assert_eq!(auxiliary.far_plane.map(f32::to_bits), ordinary.far_plane.map(f32::to_bits));
    assert_eq!(auxiliary.far_plane_signs, ordinary.far_plane_signs);
}

#[test]
fn auxiliary_infinite_far_requires_finite_distance() {
    let mut input = auxiliary_input();
    input.camera_clip_rows = [
        [1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, -1.0, -0.1], [0.0, 0.0, -1.0, 0.0],
    ];
    input.maximum_distance = f32::MAX;
    assert!(initialize_auxiliary_receiver(input).unwrap().is_none());
    input.maximum_distance = 100.0;
    let output = initialize_auxiliary_receiver(input).unwrap().unwrap();
    assert!(!output.finite_far);
    assert!(output.plane_count > 0);
}

fn word(bytes: &[u8], cursor: &mut usize) -> u32 {
    let value = u32::from_le_bytes(bytes[*cursor..*cursor + 4].try_into().unwrap());
    *cursor += 4;
    value
}

fn floats<const N: usize>(bytes: &[u8], cursor: &mut usize) -> [f32; N] {
    std::array::from_fn(|_| f32::from_bits(word(bytes, cursor)))
}

fn rows(bytes: &[u8], cursor: &mut usize) -> ShadowMatrix {
    std::array::from_fn(|_| floats(bytes, cursor))
}

fn mask(bytes: &[u8], cursor: &mut usize) -> Box<[u16; 128 * 128]> {
    let values = (0..128 * 128)
        .map(|_| {
            let value = u16::from_le_bytes(bytes[*cursor..*cursor + 2].try_into().unwrap());
            *cursor += 2;
            value
        })
        .collect::<Vec<_>>();
    values.into_boxed_slice().try_into().unwrap()
}

fn exact_stage(name: &str, actual: &[f32], expected: &[f32]) {
    let differences = actual
        .iter()
        .zip(expected)
        .enumerate()
        .filter(|(_, (a, b))| a.to_bits() != b.to_bits())
        .map(|(index, (&a, &b))| (index, a.to_bits(), b.to_bits(), a, b))
        .collect::<Vec<_>>();
    assert!(differences.is_empty(), "{name} mismatches: {differences:?}");
}

/// Isolate each leaf with native inputs so a caller error is not mistaken for
/// tolerance/renderer precision. Results are local original gameplay data.
#[test]
#[ignore = "requires local original gameplay leaf stage outputs"]
fn receiver_leaves_match_original_gameplay_stages() {
    let bytes = std::fs::read("artifacts/original-shadow-receiver-gameplay-stages.bin").unwrap();
    assert_eq!(&bytes[..4], b"DHS1");
    let mut cursor = 4;
    let camera = rows(&bytes, &mut cursor);
    let direction = floats(&bytes, &mut cursor);
    let planes = std::array::from_fn::<_, 6, _>(|_| floats::<4>(&bytes, &mut cursor));
    let world = floats(&bytes, &mut cursor);
    let expected_basis = rows(&bytes, &mut cursor);
    let expected_corners = std::array::from_fn::<_, 8, _>(|_| floats::<3>(&bytes, &mut cursor));
    let expected_bounds = floats(&bytes, &mut cursor);
    let expected_ortho = rows(&bytes, &mut cursor);
    let expected_projection = rows(&bytes, &mut cursor);
    assert_eq!(cursor, bytes.len());
    let basis = camera_aligned_receiver_rows(camera, direction);
    exact_stage(
        "aligned basis",
        basis.as_flattened(),
        expected_basis.as_flattened(),
    );
    let corners = crate::tome::shadow_caster::native_frustum_plane_corners(&planes);
    exact_stage(
        "frustum corners",
        corners.as_flattened(),
        expected_corners.as_flattened(),
    );
    let bounds = receiver_intersection_bounds(expected_basis, planes, world);
    exact_stage("intersection", &bounds, &expected_bounds);
    let ortho = receiver_orthographic_rows(expected_bounds);
    exact_stage(
        "orthographic",
        ortho.as_flattened(),
        expected_ortho.as_flattened(),
    );
    let product = multiply_rows(expected_ortho, expected_basis);
    exact_stage(
        "product",
        product.as_flattened(),
        expected_projection.as_flattened(),
    );
}

/// Actual gameplay input/return capture, not a synthetic camera/bounds case.
/// Keep copyrighted gameplay data local under artifacts/. Generate the binary
/// with analysis/convert_shadow_receiver_capture.py before explicitly running.
#[test]
#[ignore = "requires the local original gameplay receiver capture"]
fn receiver_initializer_matches_original_gameplay_projection_and_mask() {
    let bytes = std::fs::read("artifacts/original-shadow-receiver-gameplay.bin").unwrap();
    assert_eq!(&bytes[..4], b"DHR1");
    let mut cursor = 4;
    let flags = word(&bytes, &mut cursor);
    let count = word(&bytes, &mut cursor) as usize;
    let camera_clip_rows = rows(&bytes, &mut cursor);
    let camera_position = floats(&bytes, &mut cursor);
    let ray_direction = floats(&bytes, &mut cursor);
    let [maximum_distance] = floats(&bytes, &mut cursor);
    let depth_valid = word(&bytes, &mut cursor);
    let [maximum_depth] = floats(&bytes, &mut cursor);
    let world_bounds = [floats(&bytes, &mut cursor), floats(&bytes, &mut cursor)];
    let bounds = (0..count)
        .map(|_| [floats(&bytes, &mut cursor), floats(&bytes, &mut cursor)])
        .collect::<Vec<_>>();
    let depths = floats(&bytes, &mut cursor);
    let initial_mask = mask(&bytes, &mut cursor);
    let expected_rows = rows(&bytes, &mut cursor);
    let expected_transposed = rows(&bytes, &mut cursor);
    let expected_mask = mask(&bytes, &mut cursor);
    assert_eq!(cursor, bytes.len());

    let input = NativeReceiverInput {
        flags,
        camera_clip_rows,
        camera_position,
        ray_direction,
        maximum_depth: (depth_valid != 0).then_some(maximum_depth),
        maximum_distance,
        world_bounds,
        dynamic_bounds: &bounds,
        ordinary_depth: &depths,
    };
    let mut projection = NativeReceiverDepthProjection {
        receiver_rows: [[0.0; 4]; 4],
        transposed_rows: [[0.0; 4]; 4],
        mask: initial_mask,
    };
    let result = initialize_native_receiver(input, &mut projection).unwrap();
    assert_eq!(result.status, NativeReceiverStatus::Initialized);
    let mut matrices = Vec::new();
    for row in 0..4 {
        for column in 0..4 {
            for (kind, actual, expected) in [
                (
                    "receiver",
                    projection.receiver_rows[row][column],
                    expected_rows[row][column],
                ),
                (
                    "transposed",
                    projection.transposed_rows[row][column],
                    expected_transposed[row][column],
                ),
            ] {
                if actual.to_bits() != expected.to_bits() {
                    matrices.push((kind, row, column, actual.to_bits(), expected.to_bits()));
                }
            }
        }
    }
    if !matrices.is_empty() {
        eprintln!(
            "receiver rows Rust={:?} original={expected_rows:?}; cutoff={:?} bounds={:?}",
            projection.receiver_rows, result.cutoff, result.bounds
        );
    }
    assert!(
        matrices.is_empty(),
        "receiver matrix mismatches: {matrices:?}"
    );
    let mismatch = projection
        .mask
        .iter()
        .zip(expected_mask.iter())
        .enumerate()
        .filter(|(_, (actual, expected))| actual != expected)
        .take(16)
        .map(|(index, (&actual, &expected))| (index, actual, expected))
        .collect::<Vec<_>>();
    assert!(
        mismatch.is_empty(),
        "receiver depth mismatch (index, Rust, original): {mismatch:?}"
    );
}
