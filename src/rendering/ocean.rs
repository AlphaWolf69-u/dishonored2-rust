//! Ocean drawing rules from 140303500. Geometry, wave simulation and program
//! ownership are separate: these overrides require the selected program state.

use super::render_state::RenderState;

mod collision;
mod fft;
pub use collision::{build_wave_collision_mesh, WaveCollisionMesh};
mod geometry;
pub use fft::{ocean_fft_passes, FftBuffer, FftPass};
pub mod environment;
mod patches;
mod settings;
mod spectrum;
mod update;
mod view;
pub use settings::{
    OceanSettings, OceanSurfaceSettings, PerlinWaveSettings, ProjectedShadowSettings,
    ShoreWaveSettings, WaveFoamParameters,
};
pub use update::OceanUpdateState;
pub use view::{prepare_ocean_view, OceanViewInput, OceanViewState, OceanWaveControls};
mod queries;
pub use geometry::{
    append_ocean_edges, append_ocean_strip, build_ocean_geometry, OceanGeometry,
    OceanGeometryRecord,
};
pub use patches::{
    build_ocean_patches, ocean_patch_bounds, ocean_patch_visible, ocean_projected_cell_area,
    OceanPatchView,
};
pub use queries::{OceanQueryHandle, OceanQueryQueue, OCEAN_QUERY_CAPACITY};
pub use spectrum::{initialize_wave_spectrum, WaveSpectrumParameters};

/// 1402fb010 copies all nine wave controls and latches spectrum invalidation
/// for changes in fields 0,1,4,5,8,6,7. Fields 2 and 3 do not invalidate it.
/// This is parameter tracking, not the spectral simulation itself.
pub fn update_ocean_wave_parameters(current: &mut [f32; 9], dirty: &mut bool, next: [f32; 9]) {
    let threshold = f32::from_bits(0x3400_0000);
    if [0, 1, 4, 5, 8, 6, 7].into_iter().any(|index| {
        let difference = (next[index] - current[index]).abs();
        threshold.partial_cmp(&difference) != Some(std::cmp::Ordering::Greater)
    }) {
        *dirty = true;
    }
    *current = next;
}

/// A 32-byte source quadtree record. Child indices are integers despite the
/// decompiler displaying the shared record as eight floats. The root is last.
#[derive(Clone, Copy, Debug)]
pub struct OceanPatch {
    pub origin: [f32; 2],
    pub size: f32,
    pub lod: u32,
    pub children: [i32; 4],
}

/// 303500's per-leaf ocean/PatchTransform. The source tests LOD as signed,
/// then repeatedly shifts the signed grid; a shift count of 32 must not wrap.
pub fn ocean_patch_transform(patch: OceanPatch, surface_height: f32, grid_size: i32) -> [f32; 4] {
    let grid = if (patch.lod as i32) > 0 {
        grid_size.checked_shr(patch.lod).unwrap_or(grid_size >> 31)
    } else {
        grid_size
    };
    [
        patch.origin[0],
        patch.origin[1],
        surface_height,
        patch.size / grid as f32,
    ]
}

/// Select the prebuilt strip/triangle record from 140304cf0. Each LOD has
/// 3^4 records for its four neighbour edge densities. This selects geometry;
/// it neither generates indices nor changes the quadtree's visibility.
pub fn ocean_geometry_record(
    patch: OceanPatch,
    patches: &[OceanPatch],
    sample_spacing: f32,
    grid_size: i32,
) -> Result<usize, String> {
    fn neighbour(patches: &[OceanPatch], point: [f32; 2]) -> Result<Option<usize>, String> {
        let mut current = patches.len().checked_sub(1).ok_or("empty ocean quadtree")?;
        let mut result = None;
        for _ in 0..patches.len() {
            let node = &patches[current];
            if node.children == [-1; 4] {
                return Ok(result);
            }
            let mut selected = None;
            for child in node.children {
                if child == -1 {
                    continue;
                }
                let index = usize::try_from(child).map_err(|_| "negative ocean child index")?;
                let child = patches
                    .get(index)
                    .ok_or("ocean child index out of bounds")?;
                if child.origin[0] <= point[0]
                    && point[0] <= child.origin[0] + child.size
                    && child.origin[1] <= point[1]
                    && point[1] <= child.origin[1] + child.size
                {
                    selected = Some(index);
                    break;
                }
            }
            match selected {
                Some(index) => {
                    current = index;
                    result = Some(index);
                }
                None => return Ok(None),
            }
        }
        Err("cyclic ocean quadtree".into())
    }

    let [x, y] = patch.origin;
    let half_size = patch.size * 0.5;
    let outside = sample_spacing * 0.5 + patch.size;
    let points = [
        [sample_spacing * -0.5 + x, y + half_size],
        [outside + x, y + half_size],
        [x + half_size, sample_spacing * -0.5 + y],
        [x + half_size, outside + y],
    ];
    let mut record = patch.lod as usize;
    for point in points {
        let mut edge = 0;
        if let Some(index) = neighbour(patches, point)? {
            let adjacent = patches[index];
            if patch.size * f32::from_bits(0x3f7f_be77) < adjacent.size {
                let density = ((grid_size >> (patch.lod & 31)) as f32
                    * (adjacent.size / patch.size))
                    / (grid_size >> (adjacent.lod & 31)) as f32;
                // COMISS/JBE treats unordered as the <= branch, as in draw state.
                edge = if density.partial_cmp(&f32::from_bits(0x407f_ef9e))
                    == Some(std::cmp::Ordering::Greater)
                {
                    2
                } else if density > f32::from_bits(0x3fff_df3b) {
                    1
                } else {
                    0
                };
            }
        }
        record = record
            .checked_mul(3)
            .and_then(|v| v.checked_add(edge))
            .ok_or("ocean geometry record index overflow")?;
    }
    Ok(record)
}

#[derive(Clone, Copy, Debug)]
pub struct OceanDrawInputs {
    pub depth_pass: bool,
    /// Game-space Z, not the backend's coordinate-converted vertical axis.
    pub eye_height: f32,
    pub surface_height: f32,
    /// Ocean owner+88; the culling band includes both boundaries.
    pub height_extent: f32,
    pub wireframe: bool,
    pub mirrored: bool,
}

/// The seven owner+cd0..d00 program slots populated by302960. Depth uses the
/// default material's zPrepassProgram; other slots use each material's mainProgram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum OceanProgram {
    Depth = 0,
    Surface = 1,
    Reflection = 2,
    ReflectionRoughCull = 3,
    SkyCube = 4,
    ReflectionSkyCube = 5,
    ReflectionRoughCullSkyCube = 6,
}
impl OceanProgram {
    pub fn material(self) -> &'static str {
        match self {
            Self::Depth | Self::Surface => "ocean/default.material",
            Self::Reflection => "ocean/SSR.material",
            Self::ReflectionRoughCull => "ocean/SSR_SSRCull.material",
            Self::SkyCube => "ocean/SkyCube.material",
            Self::ReflectionSkyCube => "ocean/SSR_SkyCube.material",
            Self::ReflectionRoughCullSkyCube => "ocean/SSR_SSRCull_SkyCube.material",
        }
    }
}

/// Supply effective SSR admission after the view's disable bit (303ae0).
/// The rough-water option has no effect when SSR is disabled.
pub fn ocean_program(depth: bool, ssr: bool, cull_rough_ssr: bool, sky_cube: bool) -> OceanProgram {
    if depth {
        return OceanProgram::Depth;
    }
    match (sky_cube, ssr, cull_rough_ssr) {
        (false, false, _) => OceanProgram::Surface,
        (false, true, false) => OceanProgram::Reflection,
        (false, true, true) => OceanProgram::ReflectionRoughCull,
        (true, false, _) => OceanProgram::SkyCube,
        (true, true, false) => OceanProgram::ReflectionSkyCube,
        (true, true, true) => OceanProgram::ReflectionRoughCullSkyCube,
    }
}

/// Preserve all other program bits, including authored wireframe when the
/// view does not request it. The depth pass also writes motion-vector colour;
/// its state and stencil reference differ from ordinary model-water depth.
pub fn ocean_draw_state(mut state: RenderState, input: OceanDrawInputs) -> (RenderState, u32) {
    if input.wireframe {
        state[5] |= 0x0400_0000;
    }
    // COMISS/JBE takes each branch for unordered operands too. The decompiler's
    // <= spelling loses that case; retain the measured machine-code behavior.
    if input
        .eye_height
        .partial_cmp(&(input.surface_height + input.height_extent))
        != Some(std::cmp::Ordering::Greater)
    {
        if (input.surface_height - input.height_extent).partial_cmp(&input.eye_height)
            != Some(std::cmp::Ordering::Greater)
        {
            state[5] &= 0xfcff_ffff;
        } else {
            state[5] = (state[5] & 0xfdff_ffff) | 0x0100_0000;
        }
    } else {
        state[5] = (state[5] & 0xfeff_ffff) | 0x0200_0000;
    }
    state[5] = (state[5] & !0x0800_0000) | (u32::from(input.mirrored) << 27);
    let reference = if input.depth_pass {
        state[0] = (state[0] & 0xfeff_fffe) | 0x00e0_0000;
        state[5] = (state[5] & 0xff04_02ff) | 0x0004_0220;
        state[4] = (state[4] & 0xff) | 0xa80a_8000;
        6
    } else {
        state[5] = (state[5] & 0xffff_ffea) | 10;
        0
    };
    (state, reference)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocean_patch_transform_matches_executed_draw_upload() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/ocean_patch_transform.txt").lines() {
            let words = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 10);
            let patch = OceanPatch {
                origin: [f32::from_bits(words[2]), f32::from_bits(words[3])],
                size: f32::from_bits(words[4]),
                lod: words[1],
                children: [-1; 4],
            };
            assert_eq!(
                ocean_patch_transform(patch, f32::from_bits(words[5]), words[0] as i32)
                    .map(f32::to_bits),
                words[6..10],
                "source transform{count}"
            );
            count += 1;
        }
        assert_eq!(count, 189);
    }

    #[test]
    fn ocean_wave_parameter_updates_match_executed_invalidation() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/ocean_wave_parameters.txt").lines() {
            let row = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            let mut next = [0.; 9];
            next[row[1] as usize] = f32::from_bits(row[2]);
            let mut current = [0.; 9];
            let mut dirty = row[0] != 0;
            update_ocean_wave_parameters(&mut current, &mut dirty, next);
            assert_eq!(current.map(f32::to_bits), next.map(f32::to_bits));
            assert_eq!(u32::from(dirty), row[3], "case {count}");
            count += 1;
        }
        assert_eq!(count, 144);
    }

    #[test]
    fn ocean_geometry_selection_matches_executed_neighbour_lookup() {
        let mut count = 0;
        let mut nodes = Vec::new();
        for line in include_str!("../../tests/fixtures/ocean_geometry_records.txt").lines() {
            let (kind, values) = line.split_once(' ').unwrap();
            let row: Vec<u32> = values
                .split_whitespace()
                .map(|value| u32::from_str_radix(value, 16).unwrap())
                .collect();
            if kind == "tree" {
                assert_eq!(row.len() % 8, 0);
                nodes = row
                    .chunks_exact(8)
                    .map(|words| OceanPatch {
                        origin: [f32::from_bits(words[0]), f32::from_bits(words[1])],
                        size: f32::from_bits(words[2]),
                        lod: words[3],
                        children: words[4..8]
                            .try_into()
                            .map(|values: [u32; 4]| values.map(|v| v as i32))
                            .unwrap(),
                    })
                    .collect::<Vec<_>>();
                continue;
            }
            assert_eq!(kind, "case");
            assert_eq!(row.len(), 4);
            let result = ocean_geometry_record(
                nodes[row[2] as usize],
                &nodes,
                f32::from_bits(row[1]),
                row[0] as i32,
            )
            .unwrap();
            assert_eq!(result, *row.last().unwrap() as usize, "source case {count}");
            count += 1;
        }
        assert_eq!(count, 6426);
    }

    #[test]
    fn ocean_draw_overrides_match_executed_draw_routine() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/ocean_draw_state.txt").lines() {
            let row: Vec<u32> = line
                .split_whitespace()
                .map(|value| u32::from_str_radix(value, 16).unwrap())
                .collect();
            assert_eq!(row.len(), 24);
            let flags = row[0];
            let program = ocean_program(
                flags & 1 != 0,
                flags & 2 != 0,
                flags & 4 != 0,
                flags & 8 != 0,
            );
            assert_eq!(
                program as u32, row[13],
                "case {count} selected material program"
            );
            let result = ocean_draw_state(
                row[4..13].try_into().unwrap(),
                OceanDrawInputs {
                    depth_pass: flags & 1 != 0,
                    eye_height: f32::from_bits(row[1]),
                    surface_height: f32::from_bits(row[2]),
                    height_extent: f32::from_bits(row[3]),
                    wireframe: flags & 16 != 0,
                    mirrored: flags & 32 != 0,
                },
            );
            assert_eq!(result.0.as_slice(), &row[15..], "case {count}");
            assert_eq!(result.1, row[14], "case {count} stencil reference");
            count += 1;
        }
        assert_eq!(count, 2304);
    }
}
