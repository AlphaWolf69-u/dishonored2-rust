use super::OceanPatch;

/// Inputs to 140301ca0. Positions are in game coordinates; projection scales
/// are view+7390/+73a4 and pixel area is the fourth (XMM3) argument.
#[derive(Clone, Copy, Debug)]
pub struct OceanPatchView {
    pub eye: [f32; 3],
    pub surface_height: f32,
    pub grid_size: i32,
    pub projection_scale: [f32; 2],
    pub pixel_area: f32,
}

/// Shared bounds in 140301320/1403013e0. The horizontal displacement extent
/// is global 1432ef64c; both visibility paths use a quarter of that value.
pub fn ocean_patch_bounds(
    patch: OceanPatch,
    surface_height: f32,
    height_extent: f32,
    displacement_extent: f32,
) -> [[f32; 3]; 2] {
    let padding = displacement_extent * 0.25;
    [
        [
            patch.origin[0] - padding,
            patch.origin[1] - padding,
            (surface_height - height_extent) - 0.0,
        ],
        [
            (patch.size + patch.origin[0]) + padding,
            (patch.size + patch.origin[1]) + padding,
            (height_extent + surface_height) + 0.0,
        ],
    ]
}

/// Frustum branch of 140301320 through the existing 109df0/10a0d0 clip test.
/// Supply the game's clip matrix for game-space bounds, without axis conversion.
pub fn ocean_patch_visible(
    bounds: [[f32; 3]; 2],
    camera_clip: &crate::render_shadow::ShadowMatrix,
) -> bool {
    !crate::render_model::transformed_aabb_clip_cull(
        camera_clip.as_flattened().try_into().unwrap(), bounds[0], bounds[1],
    ).culled
}

pub fn ocean_projected_cell_area(patch: OceanPatch, view: OceanPatchView) -> f32 {
    let samples = [
        [0, 0],
        [0, 0x3f800000],
        [0x3f800000, 0],
        [0x3f800000, 0x3f800000],
        [0x3f000000, 0x3eaa7efa],
        [0x3e800000, 0x3f2ac083],
        [0x3f400000, 0x3de353f8],
        [0x3e000000, 0x3ee353f8],
        [0x3f200000, 0x3f472b02],
        [0x3ec00000, 0x3e6353f8],
        [0x3f600000, 0x3f0e5604],
        [0x3d800000, 0x3f639581],
        [0x3f100000, 0x3d178d50],
        [0x3ea00000, 0x3ebd70a4],
        [0x3f500000, 0x3f343958],
        [0x3e400000, 0x3e178d50],
    ];
    let height = view.surface_height - view.eye[2];
    let height_squared = height * height;
    let cell = patch.size / view.grid_size as f32;
    let numerator = cell * cell * view.projection_scale[0] * view.projection_scale[1];
    let mut maximum = 0.0_f32;
    for [x, y] in samples {
        let dy = (patch.size * f32::from_bits(y) + patch.origin[1]) - view.eye[1];
        let dx = (patch.size * f32::from_bits(x) + patch.origin[0]) - view.eye[0];
        // Preserve SQRTSS followed by squaring: cancelling them changes rounding.
        let distance = (dy * dy + dx * dx + height_squared).sqrt();
        let area = numerator / (distance * distance);
        if area > maximum {
            maximum = area;
        }
    }
    maximum * view.pixel_area * 0.25
}

/// Shared recursion in 140300af0/1403007e0. The caller supplies the actual
/// frustum/visibility-query predicate, including its environment disable flag.
/// Nodes are appended after their children; an entirely culled branch emits none.
pub fn build_ocean_patches(
    output: &mut Vec<OceanPatch>,
    root: OceanPatch,
    minimum_size: f32,
    split_area: f32,
    mut visible: impl FnMut(OceanPatch) -> bool,
    mut projected_area: impl FnMut(OceanPatch) -> f32,
) -> Result<Option<usize>, String> {
    if !root.size.is_finite()
        || root.size <= 0.0
        || !minimum_size.is_finite()
        || minimum_size <= 0.0
    {
        return Err("ocean quadtree requires finite positive patch sizes".into());
    }
    fn append(
        output: &mut Vec<OceanPatch>,
        mut patch: OceanPatch,
        minimum: f32,
        threshold: f32,
        visible: &mut impl FnMut(OceanPatch) -> bool,
        area: &mut impl FnMut(OceanPatch) -> f32,
    ) -> Result<Option<usize>, String> {
        if !visible(patch) {
            return Ok(None);
        }
        if area(patch) > threshold && patch.size > minimum {
            let half = patch.size * 0.5;
            let [x, y] = patch.origin;
            for (index, origin) in [
                [x, y],
                [half + x, y + 0.0],
                [half + x, half + y],
                [x + 0.0, half + y],
            ]
            .into_iter()
            .enumerate()
            {
                let child = OceanPatch {
                    origin,
                    size: half,
                    lod: 0,
                    children: [-1; 4],
                };
                patch.children[index] =
                    match append(output, child, minimum, threshold, visible, area)? {
                        Some(index) => {
                            i32::try_from(index).map_err(|_| "ocean patch index overflow")?
                        }
                        None => -1,
                    };
            }
            if patch.children == [-1; 4] {
                return Ok(None);
            }
        }
        // The source computes a candidate LOD in a dead loop, but never stores
        // it into the emitted record. Preserve the input LOD; children start at 0.
        let index = output.len();
        output.push(patch);
        Ok(Some(index))
    }
    append(
        output,
        root,
        minimum_size,
        split_area,
        &mut visible,
        &mut projected_area,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(source: &str) -> Vec<u32> {
        source
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect()
    }

    #[test]
    fn captured_nearby_ocean_view_keeps_surface_coverage() {
        use bevy_math::{Mat4, Vec3, Vec4};
        // Camera from collision-1792916072933. The flat surface/height extent
        // are explicit synthetic inputs; no screenshot pixels are inspected.
        let eye = Vec3::new(-31.709913, 188.40019, 2.8805392);
        let forward = Vec3::new(0.95979583, -0.07928357, -0.26926923);
        let up = Vec3::new(0.2684062, -0.02154933, 0.9630647);
        let left = up.cross(forward);
        let (horizontal, vertical) = crate::tome::render_fov_radians(80., 1366, 768, 1.).unwrap();
        let right = 0.01 * (horizontal * 0.5).tan();
        let top = 0.01 * (vertical * 0.5).tan();
        let projection = crate::tome::render_perspective_rows(
            [-right, right], [-top, top], 0.01, 7000.,
        ).unwrap();
        let view = crate::rendering::view_history::ViewMatrices::from_camera(
            eye.to_array(), [forward.to_array(), left.to_array(), up.to_array()], projection,
        );
        let mut patches = Vec::new();
        let metric = OceanPatchView {
            eye: eye.to_array(), surface_height: 0., grid_size: 16,
            projection_scale: [projection[0][0], projection[1][1]], pixel_area: (1366 * 768) as f32,
        };
        build_ocean_patches(
            &mut patches,
            OceanPatch { origin: [-7000.; 2], size: 14000., lod: 0, children: [-1; 4] },
            10., 4096.,
            |p| ocean_patch_visible(ocean_patch_bounds(p, 0., 1., 10.), &view.view_projection),
            |p| ocean_projected_cell_area(p, metric),
        ).unwrap();
        let inverse = Mat4::from_cols_array_2d(&view.view_projection).transpose().inverse();
        let mut samples = 0;
        for y in 0..16 {
            for x in 0..32 {
                let ndc = Vec4::new((x as f32 + 0.5) / 16. - 1.,
                    1. - (y as f32 + 0.5) / 8., 0.5, 1.);
                let point = inverse * ndc;
                let direction = point.truncate() / point.w - eye;
                let t = -eye.z / direction.z;
                if t <= 0. { continue; }
                let surface = eye + direction * t;
                if surface.distance(eye) > 500. { continue; }
                samples += 1;
                assert!(patches.iter().filter(|p| p.children == [-1; 4]).any(|p| {
                    surface.x >= p.origin[0] && surface.x <= p.origin[0] + p.size
                        && surface.y >= p.origin[1] && surface.y <= p.origin[1] + p.size
                }), "missing nearby ocean at {surface:?}, screen sample {x},{y}");
            }
        }
        assert!(samples > 100, "the captured view must exercise near surface coverage");
    }

    #[test]
    fn ocean_patch_bounds_match_executed_frustum_wrapper() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/ocean_patch_bounds.txt").lines() {
            let row = words(line);
            let values = row.iter().copied().map(f32::from_bits).collect::<Vec<_>>();
            let patch = OceanPatch {
                origin: [values[0], values[1]],
                size: values[2],
                lod: 0,
                children: [-1; 4],
            };
            let result = ocean_patch_bounds(patch, values[3], values[4], values[5]);
            let actual = result
                .into_iter()
                .flatten()
                .map(f32::to_bits)
                .collect::<Vec<_>>();
            assert_eq!(actual, &row[6..], "bounds case {count}");
            count += 1;
        }
        assert_eq!(count, 32);
    }

    #[test]
    fn projected_cell_area_matches_executed_metric() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/ocean_projected_cell_area.txt").lines() {
            let row = words(line);
            let values = row.iter().copied().map(f32::from_bits).collect::<Vec<_>>();
            let patch = OceanPatch {
                origin: [values[0], values[1]],
                size: values[2],
                lod: 0,
                children: [-1; 4],
            };
            let result = ocean_projected_cell_area(
                patch,
                OceanPatchView {
                    surface_height: values[3],
                    eye: [values[4], values[5], values[6]],
                    grid_size: values[7] as i32,
                    projection_scale: [values[8], values[9]],
                    pixel_area: values[10],
                },
            );
            assert_eq!(result.to_bits(), row[11], "metric case {count}");
            count += 1;
        }
        assert_eq!(count, 96);
    }

    #[test]
    fn ocean_patch_trees_match_executed_recursion() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/ocean_patch_trees.txt").lines() {
            let (input, expected) = line.split_once(" : ").unwrap();
            let row = words(input);
            let view = OceanPatchView {
                eye: [
                    f32::from_bits(row[0]),
                    f32::from_bits(row[1]),
                    f32::from_bits(row[2]),
                ],
                surface_height: 0.,
                grid_size: 16,
                projection_scale: [2., 1.5],
                pixel_area: (1920 * 1080) as f32,
            };
            let mut patches = Vec::new();
            let root = OceanPatch {
                origin: [-32., -32.],
                size: 64.,
                lod: 7,
                children: [-1; 4],
            };
            let result = build_ocean_patches(
                &mut patches,
                root,
                8.,
                f32::from_bits(row[3]),
                |p| match row[4] {
                    0 => true,
                    1 => p.origin[0] < 0.,
                    2 => p.size > 8.,
                    3 => p.origin[0] + p.size > 12. && p.origin[1] + p.size > 12.,
                    _ => unreachable!(),
                },
                |p| ocean_projected_cell_area(p, view),
            )
            .unwrap();
            assert_eq!(
                result.map_or(u32::MAX, |i| i as u32),
                row[5],
                "tree case {count}"
            );
            let actual = patches
                .into_iter()
                .flat_map(|p| {
                    [
                        p.origin[0].to_bits(),
                        p.origin[1].to_bits(),
                        p.size.to_bits(),
                        p.lod,
                        p.children[0] as u32,
                        p.children[1] as u32,
                        p.children[2] as u32,
                        p.children[3] as u32,
                    ]
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, words(expected), "tree case {count}");
            count += 1;
        }
        assert_eq!(count, 48);
    }
}
