//! Map probe lookup backed by the ordered PNW resource list and PRB headers.

use crate::{archive, bprobe, pnetwork};
use std::collections::HashMap;

const TETRA_GRID_CELL_SIZE: f32 = 16.0;
const MAX_TETRA_GRID_CELLS: usize = 512;

#[derive(Clone, Debug)]
pub struct ProbeLighting {
    pub resource: String,
    pub position: [f32; 3],
    pub static_lights_illuminance: [f32; 3],
    pub sky_occlusion_factor: f32,
    pub sun_irradiance_factor: f32,
    pub illuminance_estimate: [f32; 3],
    pub cubemap_name: String,
    pub static_lighting_sh: [[f32; 3]; 9],
    pub sun_factor_sh: [[f32; 3]; 9],
    pub filtered_sun_factor_sh: [[f32; 3]; 9],
    pub sky_occlusion_sh: [f32; 9],
    pub filtered_sky_occlusion_sh: [f32; 9],
}

#[derive(Clone, Debug)]
pub struct ProbeLightingSet {
    pub network_resource: String,
    pub probes: Vec<ProbeLighting>,
    pub secondary_resources: Vec<String>,
    pub rejected_resources: Vec<(String, String)>,
    pub spatial: pnetwork::SpatialData,
    tetra_grid: TetraGrid,
}

#[derive(Clone, Debug, Default)]
struct TetraGrid {
    cells: HashMap<[i32; 3], Vec<usize>>,
    large_tetrahedra: Vec<usize>,
}

impl TetraGrid {
    fn build(spatial: &pnetwork::SpatialData) -> Self {
        let mut grid = Self::default();
        for (tetrahedron_index, tetrahedron) in spatial.tetrahedra.iter().enumerate() {
            let points = tetrahedron
                .vertex_indices
                .map(|index| spatial.vertices[index as usize].position);
            let mut low = [f32::INFINITY; 3];
            let mut high = [f32::NEG_INFINITY; 3];
            for point in points {
                for axis in 0..3 {
                    low[axis] = low[axis].min(point[axis]);
                    high[axis] = high[axis].max(point[axis]);
                }
            }
            let first = low.map(|value| (value / TETRA_GRID_CELL_SIZE).floor() as i32);
            let last = high.map(|value| (value / TETRA_GRID_CELL_SIZE).floor() as i32);
            let spans = std::array::from_fn::<_, 3, _>(|axis| {
                (last[axis] - first[axis] + 1).max(1) as usize
            });
            let cell_count = spans[0]
                .checked_mul(spans[1])
                .and_then(|count| count.checked_mul(spans[2]));
            if cell_count.is_none_or(|count| count > MAX_TETRA_GRID_CELLS) {
                grid.large_tetrahedra.push(tetrahedron_index);
                continue;
            }
            for x in first[0]..=last[0] {
                for y in first[1]..=last[1] {
                    for z in first[2]..=last[2] {
                        grid.cells
                            .entry([x, y, z])
                            .or_default()
                            .push(tetrahedron_index);
                    }
                }
            }
        }
        grid
    }

    fn candidates(&self, position: [f32; 3]) -> Vec<usize> {
        let cell = position.map(|value| (value / TETRA_GRID_CELL_SIZE).floor() as i32);
        let mut candidates = self.cells.get(&cell).cloned().unwrap_or_default();
        candidates.extend(self.large_tetrahedra.iter().copied());
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}

#[derive(Clone, Debug)]
pub struct InterpolatedProbeLighting {
    pub tetrahedron_index: usize,
    pub barycentrics: [f32; 4],
    pub probe_indices: [u16; 4],
    pub probe_weights: [f32; 4],
    // Keep these distinct from the SH arrays. The installed
    // generated/decls/renderprog/indirectlighting/update/accumulatesh.decl
    // uses factorStatic = globalFactor * envBounceStatic * probeBounceStatic;
    // factorSun = globalFactor * envBounceSun * probeBounceSun * sunColorAtGround;
    // factorSky = globalFactor * probeBounceSky. It computes
    // ResultSH[i] = factorStatic*staticSH[i] + factorSun*sunSH[i]
    //             + ZHSHProductSky(factorSky*skyZH, skySH)[i]
    //             + factorDynamic*dynamicSH[i],
    // then adds parmsFloat.y * 3.5449077 * probeAmbientFactor * envAmbient
    // to coefficient 0. These serialized fields are exposed for the audit;
    // their exact mapping into ProbeInfo, the runtime sky-ZH inputs, and
    // the required source factors remain open. Do not add raw sky SH to the
    // material: it is an input to the ZH/SH product, not the final result.
    pub static_lights_illuminance: [f32; 3],
    pub sky_occlusion_factor: f32,
    pub sun_irradiance_factor: f32,
    pub illuminance_estimate: [f32; 3],
    pub static_lighting_sh: [[f32; 3]; 9],
    pub sun_factor_sh: [[f32; 3]; 9],
    pub filtered_sun_factor_sh: [[f32; 3]; 9],
    pub sky_occlusion_sh: [f32; 9],
}

/// Resolve a map's PNW network by its extracted world-file stem, then follow
/// the ordered primary list into installed .bprobe resources.
pub fn load_for_world(
    catalog: &archive::Catalog,
    world_stem: &str,
) -> Result<Option<ProbeLightingSet>, String> {
    let candidates: Vec<_> = catalog
        .paths_containing(world_stem)
        .into_iter()
        .filter(|resource| resource.ends_with(".probesnetwork"))
        .collect();
    if candidates.len() != 1 {
        return Ok(None);
    }
    let network_resource = candidates[0].clone();
    // Mission PNW files can be hundreds of MiB. The ordered name tables are
    // at the front, so only read the bounded prefix needed to parse them.
    let network = catalog
        .read_prefix(&network_resource, 64 * 1024 * 1024)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("missing probe network {network_resource}"))?;
    let (_, names, secondary_resources, spatial_offset) =
        pnetwork::ordered_probe_names(&network).map_err(|error| error.to_string())?;
    let spatial =
        pnetwork::spatial_data(&network, spatial_offset).map_err(|error| error.to_string())?;
    let tetra_grid = TetraGrid::build(&spatial);
    let mut probes = Vec::with_capacity(names.len());
    let mut rejected_resources = Vec::new();
    for name in names {
        let resource = format!("{name}.bprobe");
        let parsed = catalog
            .read(&resource)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("PNW references missing probe resource {resource}"))
            .and_then(|bytes| bprobe::parse(&bytes).map_err(|error| error.to_string()));
        match parsed {
            Ok(probe) => probes.push(ProbeLighting {
                resource,
                position: probe.position,
                static_lights_illuminance: probe.static_lights_illuminance,
                sky_occlusion_factor: probe.sky_occlusion_factor,
                sun_irradiance_factor: probe.sun_irradiance_factor,
                illuminance_estimate: probe.illuminance_estimate,
                cubemap_name: probe.cubemap_name,
                static_lighting_sh: probe.static_lighting_sh,
                sun_factor_sh: probe.sun_factor_sh,
                filtered_sun_factor_sh: probe.filtered_sun_factor_sh,
                sky_occlusion_sh: probe.sky_occlusion_sh,
                filtered_sky_occlusion_sh: probe.filtered_sky_occlusion_sh,
            }),
            Err(error) => rejected_resources.push((resource, error)),
        }
    }
    Ok(Some(ProbeLightingSet {
        network_resource,
        probes,
        secondary_resources,
        rejected_resources,
        spatial,
        tetra_grid,
    }))
}

impl ProbeLightingSet {
    /// Use the game shader's tetrahedron and per-vertex packed-probe-weight
    /// interpolation, returning the authored SH coefficients for a world point.
    pub fn interpolate_static_sh(&self, position: [f32; 3]) -> Option<InterpolatedProbeLighting> {
        let barycentric = |points: [[f32; 3]; 4]| {
            let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
            let cross = |a: [f32; 3], b: [f32; 3]| {
                [
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                ]
            };
            let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
            let dot_cross = |a: [f32; 3], b: [f32; 3], c: [f32; 3]| dot(a, cross(b, c));
            let delta = sub(position, points[3]);
            let edge0 = sub(points[0], points[3]);
            let edge1 = sub(points[1], points[3]);
            let edge2 = sub(points[2], points[3]);
            let determinant = dot_cross(edge0, edge1, edge2);
            let reciprocal = if determinant.abs() > 0.0 {
                determinant.recip()
            } else {
                0.0
            };
            let first = reciprocal * dot_cross(delta, edge1, edge2);
            let second = reciprocal * dot_cross(delta, edge2, edge0);
            let third = reciprocal * dot_cross(delta, edge0, edge1);
            [first, second, third, 1.0 - first - second - third]
        };

        'tetrahedron: for tetrahedron_index in self.tetra_grid.candidates(position) {
            let tetrahedron = &self.spatial.tetrahedra[tetrahedron_index];
            let points = tetrahedron
                .vertex_indices
                .map(|index| self.spatial.vertices[index as usize].position);
            let barycentrics = barycentric(points);
            if barycentrics
                .iter()
                .any(|weight| !weight.is_finite() || *weight < -1.0e-5 || *weight > 1.00001)
            {
                continue;
            }

            let mut probe_weights = [0.0; 4];
            for (vertex, barycentric) in tetrahedron.vertex_indices.iter().zip(barycentrics) {
                let packed = self.spatial.vertices[*vertex as usize].probe_weights;
                for probe in 0..4 {
                    probe_weights[probe] += barycentric * (packed[probe] as f32 / 255.0);
                }
            }
            let mut static_lighting_sh = [[0.0; 3]; 9];
            let mut sun_factor_sh = [[0.0; 3]; 9];
            let mut filtered_sun_factor_sh = [[0.0; 3]; 9];
            let mut sky_occlusion_sh = [0.0; 9];
            let mut static_lights_illuminance = [0.0; 3];
            let mut sky_occlusion_factor = 0.0;
            let mut sun_irradiance_factor = 0.0;
            let mut illuminance_estimate = [0.0; 3];
            for (probe_slot, &probe_index) in tetrahedron.probe_indices.iter().enumerate() {
                if probe_index == u16::MAX {
                    if probe_weights[probe_slot] > 0.0 {
                        continue 'tetrahedron;
                    }
                    continue;
                }
                if probe_index as usize >= self.probes.len() {
                    if probe_weights[probe_slot] > 0.0 {
                        continue 'tetrahedron;
                    }
                    continue;
                }
                let probe_sh = &self.probes[probe_index as usize].static_lighting_sh;
                let probe = &self.probes[probe_index as usize];
                for channel in 0..3 {
                    static_lights_illuminance[channel] +=
                        probe_weights[probe_slot] * probe.static_lights_illuminance[channel];
                    illuminance_estimate[channel] +=
                        probe_weights[probe_slot] * probe.illuminance_estimate[channel];
                }
                sky_occlusion_factor += probe_weights[probe_slot] * probe.sky_occlusion_factor;
                sun_irradiance_factor += probe_weights[probe_slot] * probe.sun_irradiance_factor;
                for coefficient in 0..9 {
                    sky_occlusion_sh[coefficient] +=
                        probe_weights[probe_slot] * probe.sky_occlusion_sh[coefficient];
                    for channel in 0..3 {
                        static_lighting_sh[coefficient][channel] +=
                            probe_weights[probe_slot] * probe_sh[coefficient][channel];
                    }
                }
                let probe_sun_sh = &self.probes[probe_index as usize].sun_factor_sh;
                let probe_filtered_sun_sh =
                    &self.probes[probe_index as usize].filtered_sun_factor_sh;
                for coefficient in 0..9 {
                    for channel in 0..3 {
                        sun_factor_sh[coefficient][channel] +=
                            probe_weights[probe_slot] * probe_sun_sh[coefficient][channel];
                        filtered_sun_factor_sh[coefficient][channel] +=
                            probe_weights[probe_slot] * probe_filtered_sun_sh[coefficient][channel];
                    }
                }
            }
            return Some(InterpolatedProbeLighting {
                tetrahedron_index,
                barycentrics,
                probe_indices: tetrahedron.probe_indices,
                probe_weights,
                static_lights_illuminance,
                sky_occlusion_factor,
                sun_irradiance_factor,
                illuminance_estimate,
                static_lighting_sh,
                sun_factor_sh,
                filtered_sun_factor_sh,
                sky_occlusion_sh,
            });
        }
        None
    }
}
