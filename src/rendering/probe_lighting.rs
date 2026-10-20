//! Map probe lookup backed by the ordered PNW resource list and PRB headers.

use crate::{archive, bprobe, pnetwork};

#[path = "probe_lighting/query.rs"]
mod query;
#[path = "probe_lighting/update.rs"]
mod update;
pub use query::ProbeQuery;
pub use update::{sky_zh, ProbeLightingParameters};

/// PC sphericalharmonics.decl EvaluateSH (four- and nine-coefficient overloads).
pub fn evaluate_sh<const N: usize>(direction: [f32; 3], sh: &[[f32; 3]; N]) -> [f32; 3] {
    assert!(N == 4 || N == 9);
    let [x, y, z] = direction;
    let f0 = 0.28209479177387814_f32;
    let f1 = 0.4886025119029199_f32;
    let f2 = 1.092548430592079_f32;
    let f3 = 0.31539156525252_f32;
    let weights = [
        f0,
        f1 * y,
        f1 * z,
        f1 * x,
        f2 * x * y,
        f2 * y * z,
        f3 * (3.0 * z * z - 1.0),
        f2 * x * z,
        f2 * 0.5 * (x * x - y * y),
    ];
    std::array::from_fn(|channel| {
        (1..N)
            .fold(weights[0] * sh[0][channel], |value, i| {
                value + weights[i] * sh[i][channel]
            })
            .max(0.0)
    })
}

/// PC sphericalharmonics.decl EvaluateSHIrradiance. Do not normalize directions:
/// the installed principal-direction constants deliberately use 0.7071, etc.
pub fn evaluate_sh_irradiance([x, y, z]: [f32; 3], sh: &[[f32; 3]; 9]) -> [f32; 3] {
    let c1 = 0.4290427654048917_f32;
    let c2 = 0.5116633539732442_f32;
    let c3 = 0.2477079561003757_f32;
    let c4 = 0.886226925452758_f32;
    std::array::from_fn(|c| {
        ((c1 * (x * x - y * y)) * sh[8][c]
            + (c3 * (3.0 * z * z - 1.0)) * sh[6][c]
            + c4 * sh[0][c]
            + 2.0 * c1 * (sh[4][c] * x * y + sh[7][c] * x * z + sh[5][c] * y * z)
            + 2.0 * c2 * (sh[3][c] * x + sh[1][c] * y + sh[2][c] * z))
            .max(0.0)
    })
}

const PRINCIPAL_RAYS: [[f32; 3]; 3] = [
    [0.7071, 0.0, 0.7071],
    [-0.35355, -0.61237, 0.7071],
    [-0.35355, 0.61237, 0.7071],
];

/// indirectLighting/common.decl PackSHPrincipalDirections, given the authored
/// independently transformed T/B/N and already-selected/interpolated SH.
pub fn pack_principal_irradiance(
    sh: &[[f32; 3]; 9],
    basis: [[f32; 3]; 3],
    two_sided: bool,
) -> [[f32; 3]; 4] {
    std::array::from_fn(|i| {
        let direction = if i == 3 {
            basis[2].map(|v| if two_sided { -v } else { v })
        } else {
            std::array::from_fn(|c| {
                PRINCIPAL_RAYS[i][0] * basis[0][c]
                    + PRINCIPAL_RAYS[i][1] * basis[1][c]
                    + PRINCIPAL_RAYS[i][2] * basis[2][c]
            })
        };
        evaluate_sh_irradiance(direction, sh)
    })
}

/// indirectLighting/common.decl UnPackSHPrincipalDirections, both retail paths.
/// One-sided weights are NOT saturated; the fourth 1e-6 lane preserves the ABI.
pub fn unpack_principal_irradiance(
    mut direction: [f32; 3],
    basis: [[f32; 3]; 3],
    irradiance: [[f32; 3]; 4],
    ao: f32,
    two_sided: bool,
) -> [f32; 3] {
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    if !two_sided {
        direction = std::array::from_fn(|i| basis[2][i] * (1.0 - ao) + direction[i] * ao);
        let length = dot(direction, direction).sqrt();
        let scale = if length > 1e-6 { length.recip() } else { 1.0 };
        direction = direction.map(|v| v * scale);
    }
    let local = basis.map(|axis| dot(direction, axis));
    let weights: [f32; 4] = std::array::from_fn(|i| {
        let value = if i == 3 {
            if two_sided {
                -local[2]
            } else {
                local[2]
            }
        } else {
            dot(local, PRINCIPAL_RAYS[i])
        };
        if two_sided {
            0.5 * value.clamp(0.0, 1.0)
        } else {
            0.5 * value * if i == 3 { 1e-6 } else { 1.0 }
        }
    });
    std::array::from_fn(|c| {
        ao * (weights[0] * irradiance[0][c]
            + weights[1] * irradiance[1][c]
            + weights[2] * irradiance[2][c]
            + weights[3] * irradiance[3][c])
    })
}

/// colorimetry.decl: chroma ranges are twice the common Wikipedia convention.
pub fn rgb_to_ycocg([r, g, b]: [f32; 3]) -> [f32; 3] {
    let co = r - b;
    let t = b + co * 0.5;
    let cg = g - t;
    [t + cg * 0.5, co, cg]
}

pub fn ycocg_to_rgb([y, co, cg]: [f32; 3]) -> [f32; 3] {
    let t = y - cg * 0.5;
    let g = cg + t;
    let b = t - co * 0.5;
    [co + b, g, b]
}

/// PC WRITE/READ_INDIRECT_LIGHTING interpolants: one shared luminance and
/// four Co/Cg pairs (the installed pack is intentionally lossy).
#[derive(Clone, Copy, Debug)]
pub struct PackedIrradiance {
    pub luminance: f32,
    pub chroma: [[f32; 2]; 4],
}

impl PackedIrradiance {
    pub fn pack(irradiance: [[f32; 3]; 4]) -> Self {
        let ycocg = irradiance.map(rgb_to_ycocg);
        let luminance = ycocg.iter().map(|v| v[0]).fold(f32::NEG_INFINITY, f32::max);
        let factor = (luminance * luminance).max(1e-4).recip();
        Self {
            luminance,
            chroma: ycocg.map(|v| {
                let scale = factor * v[0];
                [v[1] * scale, v[2] * scale]
            }),
        }
    }

    pub fn unpack(self) -> [[f32; 3]; 4] {
        self.chroma.map(|[co, cg]| {
            ycocg_to_rgb([self.luminance, co * self.luminance, cg * self.luminance])
        })
    }
}

#[derive(Clone, Debug)]
pub struct ProbeLighting {
    pub resource: String,
    pub position: [f32; 3],
    pub static_lights_illuminance: [f32; 3],
    pub sky_occlusion_factor: f32,
    pub sun_irradiance_factor: f32,
    pub illuminance_estimate: [f32; 3],
    pub cubemap_name: String,
    pub sources: bprobe::ProbeSources,
    pub samples: Vec<bprobe::Sample>,
    pub render_lighting_sh: [[f32; 3]; 9],
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
    pub reflection_probes: Vec<bprobe::Probe>,
    pub rejected_resources: Vec<(String, String)>,
    pub spatial: pnetwork::SpatialData,
    pub vertex_references: pnetwork::VertexReferences,
    pub cell_geometry: Vec<query::CellGeometry>,
}

#[derive(Clone, Debug)]
pub struct InterpolatedProbeLighting {
    pub tetrahedron_index: usize,
    pub barycentrics: [f32; 4],
    pub probe_indices: [u16; 4],
    pub probe_weights: [f32; 4],
    pub reflection_probe: u32,
    pub reflection_luminance: f32,
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
    // update.rs now implements the verified static/sun/sky/ambient assembly.
    // Runtime dynamic-light SH generation remains separate. Raw sky SH is
    // an input to the ZH/SH product, not material irradiance.
    pub static_lights_illuminance: [f32; 3],
    pub sky_occlusion_factor: f32,
    pub sun_irradiance_factor: f32,
    pub illuminance_estimate: [f32; 3],
    pub render_lighting_sh: [[f32; 3]; 9],
    pub static_lighting_sh: [[f32; 3]; 9],
    pub sun_factor_sh: [[f32; 3]; 9],
    pub filtered_sun_factor_sh: [[f32; 3]; 9],
    pub sky_occlusion_sh: [f32; 9],
}

impl InterpolatedProbeLighting {
    /// Backend per-object record. RGB holds nine SH coefficients; the first
    /// two padding lanes carry a numeric reflection index (-1 for fallback)
    /// and its luminance factor. Numeric indices avoid denormal float payloads.
    pub fn object_shader_record(&self) -> [[f32; 4]; 9] {
        let mut record = self.render_lighting_sh.map(|[r, g, b]| [r, g, b, 0.]);
        record[0][3] = if self.reflection_probe == u32::MAX {
            -1.
        } else {
            self.reflection_probe as f32
        };
        record[1][3] = self.reflection_luminance;
        record
    }
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
    // FUN_1402b15e0 loads both trailing world-vertex buffers, not just the
    // ordered names/Voronoi prefix. Bound decoding by the indexed asset size;
    // EOTW's PNW is 247 MiB and its reference buffer follows the first 64 MiB.
    let network_size = catalog
        .resource_size(&network_resource)
        .ok_or_else(|| format!("missing probe network {network_resource}"))?;
    let network = catalog
        .read_prefix(&network_resource, network_size)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("missing probe network {network_resource}"))?;
    if network.len() != network_size {
        return Err(format!(
            "truncated probe network {network_resource}: {} of {network_size} bytes",
            network.len()
        ));
    }
    let (_, names, secondary_resources, spatial_offset) =
        pnetwork::ordered_probe_names(&network).map_err(|error| error.to_string())?;
    let spatial =
        pnetwork::spatial_data(&network, spatial_offset).map_err(|error| error.to_string())?;
    let vertex_references =
        pnetwork::vertex_references(&network, spatial.bytes_consumed, secondary_resources.len())?;

    let mut probes = Vec::with_capacity(names.len());
    let mut reflection_probes = Vec::with_capacity(secondary_resources.len());
    for name in &secondary_resources {
        let path = format!("{name}.bprobe");
        let bytes = catalog
            .read(&path)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("missing reflection probe {path}"))?;
        reflection_probes
            .push(bprobe::parse(&bytes).map_err(|e| format!("reflection probe {path}: {e}"))?);
    }
    let rejected_resources = Vec::new();
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
                sources:probe.sources,
                samples:probe.samples,
                render_lighting_sh:probe.static_lighting_sh,
                static_lighting_sh: probe.static_lighting_sh,
                sun_factor_sh: probe.sun_factor_sh,
                filtered_sun_factor_sh: probe.filtered_sun_factor_sh,
                sky_occlusion_sh: probe.sky_occlusion_sh,
                filtered_sky_occlusion_sh: probe.filtered_sky_occlusion_sh,
            }),
            // FUN_1402b15e0:160-166 writes a slot for every primary name.
            // Compacting failed decodes shifts every later native tetra ID.
            // An unsupported required probe must fail loading, not become a
            // fabricated black probe or a misindexed, partially loaded array.
            Err(error) => return Err(format!("required primary probe {resource}: {error}; native ordered probe IDs cannot be compacted")),
        }
    }
    let mut cell_geometry = Vec::with_capacity(spatial.cells.len());
    for cell in &spatial.cells {
        let name = format!("{}.bprobe", cell.probe_name);
        let probe = probes
            .iter()
            .find(|p| p.resource == name)
            .ok_or_else(|| format!("missing CPU Voronoi probe {name}"))?;
        cell_geometry.push(query::geometry(
            probe.position,
            &probe.samples,
            &cell.occluder_samples,
        )?);
    }
    Ok(Some(ProbeLightingSet {
        network_resource,
        probes,
        secondary_resources,
        reflection_probes,
        rejected_resources,
        spatial,
        vertex_references,
        cell_geometry,
    }))
}

impl ProbeLightingSet {
    pub fn reflection_info(&self, probe: usize) -> (u32, f32) {
        let index = self
            .spatial
            .cells
            .get(probe)
            .and_then(|cell| cell.reflection_cell)
            .and_then(|i| self.spatial.cells.get(i))
            .map_or(0, |cell| cell.reflection_probe);
        let source = &self.probes[probe].sources;
        (index, source.lighting_scale * source.reflection_luminance)
    }
    pub fn initialize_indirect_lighting(&mut self, parameters: &ProbeLightingParameters) {
        for probe in &mut self.probes {
            probe.render_lighting_sh = update::accumulate(probe, parameters);
        }
    }

    /// Repack the native typed SRVs into one read-only WGSL word buffer.
    /// The layout changes; the indices and 24/16/4-byte native records do not.
    pub fn vertex_shader_words(&self) -> Vec<u32> {
        let refs = &self.vertex_references;
        let mut words = vec![0u32; 8];
        words[0] = words.len().try_into().expect("PNW GPU word count");
        words.extend_from_slice(&refs.instance_offsets);
        words[1] = words.len().try_into().expect("PNW GPU word count");
        words.extend_from_slice(&refs.tetrahedron_indices);
        words[2] = words.len().try_into().expect("PNW GPU word count");
        for tetra in &self.spatial.tetrahedra {
            words.extend_from_slice(&tetra.vertex_indices);
            words.push(u32::from(tetra.probe_indices[0]) | u32::from(tetra.probe_indices[1]) << 16);
            words.push(u32::from(tetra.probe_indices[2]) | u32::from(tetra.probe_indices[3]) << 16);
        }
        words[3] = words.len().try_into().expect("PNW GPU word count");
        for vertex in &self.spatial.vertices {
            words.extend(vertex.position.map(f32::to_bits));
            words.push(u32::from_le_bytes(vertex.probe_weights));
        }
        words[4] = refs
            .instance_offsets
            .len()
            .try_into()
            .expect("PNW instance count");
        words[5] = refs
            .tetrahedron_indices
            .len()
            .try_into()
            .expect("PNW reference count");
        words[6] = self
            .spatial
            .tetrahedra
            .len()
            .try_into()
            .expect("PNW tetrahedron count");
        words[7] = self
            .spatial
            .vertices
            .len()
            .try_into()
            .expect("PNW vertex count");
        words
    }

    /// Use the game shader's tetrahedron and per-vertex packed-probe-weight
    /// interpolation, returning the authored SH coefficients for a world point.
    pub fn interpolate_static_sh(&self, position: [f32; 3]) -> Option<InterpolatedProbeLighting> {
        self.interpolate_query_sh(position, &mut ProbeQuery::default())
    }
    pub fn interpolate_query_sh(
        &self,
        position: [f32; 3],
        query: &mut ProbeQuery,
    ) -> Option<InterpolatedProbeLighting> {
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
            let reciprocal = if determinant.abs() > 1e-6 {
                determinant.recip()
            } else {
                0.0
            };
            let first = reciprocal * dot_cross(delta, edge1, edge2);
            let second = reciprocal * dot_cross(delta, edge2, edge0);
            let third = reciprocal * dot_cross(delta, edge0, edge1);
            [first, second, third, 1.0 - first - second - third]
        };

        let selected = query.select(self, position)?;
        let runtime = &self.spatial.runtime_tetrahedra[selected];
        'tetrahedron: for tetrahedron_index in [runtime.shader_index] {
            let tetrahedron = &self.spatial.tetrahedra[tetrahedron_index];
            let barycentrics = barycentric(runtime.positions);

            let mut probe_weights = [0.0; 4];
            for (vertex, barycentric) in tetrahedron.vertex_indices.iter().zip(barycentrics) {
                let packed = self.spatial.vertices[*vertex as usize].probe_weights;
                for probe in 0..4 {
                    probe_weights[probe] += barycentric * (packed[probe] as f32 / 255.0);
                }
            }
            let mut render_lighting_sh = [[0.0; 3]; 9];
            let mut reflection_probe = 0;
            let mut strongest = -f32::MAX;
            let mut reflection_luminance = 0.;
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
                let (reflection, luminance) = self.reflection_info(probe_index as usize);
                let weight = probe_weights[probe_slot];
                if probe_slot == 0 || weight > strongest {
                    strongest = weight;
                    reflection_probe = reflection;
                }
                reflection_luminance += weight * luminance;
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
                        render_lighting_sh[coefficient][channel] += probe_weights[probe_slot]
                            * probe.render_lighting_sh[coefficient][channel];
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
                reflection_probe,
                reflection_luminance: reflection_luminance.max(0.0001),
                static_lights_illuminance,
                sky_occlusion_factor,
                sun_irradiance_factor,
                illuminance_estimate,
                render_lighting_sh,
                static_lighting_sh,
                sun_factor_sh,
                filtered_sun_factor_sh,
                sky_occlusion_sh,
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires installed game archives"]
    fn installed_vista_motion_probe_queries() {
        let catalog =
            archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
        let set = load_for_world(&catalog, "dust_p_lowchaos")
            .unwrap()
            .unwrap();
        let bytes = catalog
            .read("maps/campaign/dust/dust_p_lowchaos.entities")
            .unwrap()
            .unwrap();
        let doc = crate::entities::parse(&crate::entities::declaration_text(&bytes)).unwrap();
        for placement in crate::entities::render_placements(&doc)
            .into_iter()
            .filter(|p| p.resource_path.contains("giant_wind_turbine_propeller_01"))
        {
            let model =
                crate::bmodel::parse(&catalog.read(&placement.resource_path).unwrap().unwrap())
                    .unwrap();
            let bounds = crate::render_model::transformed_bounds(
                model.authored_bounds,
                placement.position,
                placement.rotation,
                placement.scale,
            );
            let p = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5);
            let mut query = ProbeQuery::default();
            let light = set.interpolate_query_sh(p, &mut query).unwrap();
            println!(
                "{} center{p:?} query{query:?} L0{:?}",
                placement.id, light.render_lighting_sh[0]
            );
        }
    }

    #[test]
    fn indirect_lighting_matches_installed_hlsl() {
        let fixture = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/indirect_lighting_cases.txt"
        ));
        assert_eq!(fixture.lines().count(), 64);
        for (id, line) in fixture.lines().enumerate() {
            let expected = line
                .split_whitespace()
                .map(|word| f32::from_bits(u32::from_str_radix(word, 16).unwrap()))
                .collect::<Vec<_>>();
            assert_eq!(expected.len(), 84);
            let sh: [[f32; 3]; 9] = std::array::from_fn(|i| {
                std::array::from_fn(|c| {
                    if id == 0 {
                        0.0
                    } else {
                        (((id * 17 + i * 5 + c * 3) % 19) as i32 - 9) as f32 / 16.0
                    }
                })
            });
            let direction = [
                ((id % 5) as i32 - 2) as f32 * 0.25,
                (((id / 5) % 5) as i32 - 2) as f32 * 0.25,
                (((id / 25) % 5) as i32 - 2) as f32 * 0.25,
            ]
            .map(|v| v * if id >= 32 { 3.0 } else { 1.0 });
            let mut basis = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            if id & 1 != 0 {
                basis[0] = [0.0, 1.0, 0.0];
                basis[1] = [-1.0, 0.0, 0.0];
            }
            if id & 2 != 0 {
                basis[1] = basis[1].map(|v| -v);
                basis[2] = basis[2].map(|v| -v);
            }
            let ao = (id % 5) as f32 * 0.25;
            let rays0 = pack_principal_irradiance(&sh, basis, false);
            let rays1 = pack_principal_irradiance(&sh, basis, true);
            let packed = PackedIrradiance::pack(rays0);
            let mut actual = [[0.0; 4]; 21];
            let sh4: [[f32; 3]; 4] = sh[..4].try_into().unwrap();
            let values = [
                evaluate_sh(direction, &sh4),
                evaluate_sh(direction, &sh),
                evaluate_sh_irradiance(direction, &sh),
                unpack_principal_irradiance(direction, basis, rays0, ao, false),
                unpack_principal_irradiance(direction, basis, rays1, ao, true),
            ];
            for (i, value) in values.into_iter().enumerate() {
                actual[i][..3].copy_from_slice(&value);
            }
            for (i, value) in rays0.into_iter().enumerate() {
                actual[5 + i][..3].copy_from_slice(&value);
            }
            actual[9][0] = packed.luminance;
            actual[10] = [
                packed.chroma[0][0],
                packed.chroma[0][1],
                packed.chroma[1][0],
                packed.chroma[1][1],
            ];
            actual[11] = [
                packed.chroma[2][0],
                packed.chroma[2][1],
                packed.chroma[3][0],
                packed.chroma[3][1],
            ];
            for (i, value) in packed.unpack().into_iter().enumerate() {
                actual[12 + i][..3].copy_from_slice(&value);
            }
            actual[16][..3].copy_from_slice(&unpack_principal_irradiance(
                direction,
                basis,
                packed.unpack(),
                ao,
                false,
            ));
            let packed_two = PackedIrradiance::pack(rays1);
            actual[17][0] = packed_two.luminance;
            actual[18] = [
                packed_two.chroma[0][0],
                packed_two.chroma[0][1],
                packed_two.chroma[1][0],
                packed_two.chroma[1][1],
            ];
            actual[19] = [
                packed_two.chroma[2][0],
                packed_two.chroma[2][1],
                packed_two.chroma[3][0],
                packed_two.chroma[3][1],
            ];
            actual[20][..3].copy_from_slice(&unpack_principal_irradiance(
                direction,
                basis,
                packed_two.unpack(),
                ao,
                true,
            ));
            for (index, (actual, expected)) in
                actual.into_iter().flatten().zip(expected).enumerate()
            {
                assert!(
                    (actual - expected).abs() <= 3e-5 * expected.abs().max(1.0),
                    "HLSL case{id} field{index}: {actual} != {expected}"
                );
            }
        }
    }

    #[test]
    #[ignore = "requires installed EOTW assets and the extracted BWM"]
    fn installed_vertex_references_match_world_instances() {
        let catalog =
            archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
        let set = load_for_world(&catalog, "eotw_p_lowchaos")
            .unwrap()
            .unwrap();
        let world =
            crate::world_mesh::parse(&std::fs::read("artifacts/eotw_p_lowchaos.bwm").unwrap())
                .unwrap();
        assert!(
            set.rejected_resources.is_empty(),
            "{:?}",
            set.rejected_resources
        );
        eprintln!("PNW vertex lighting: probes={} cells={} tetrahedra={} offsets={} BWM_instances={} references={} bytes={}",
            set.probes.len(), set.spatial.cell_count, set.spatial.tetrahedra.len(),
            set.vertex_references.instance_offsets.len(), world.matrices.len(),
            set.vertex_references.tetrahedron_indices.len(), set.vertex_references.bytes_consumed);
        assert_eq!(
            set.vertex_references.instance_offsets.len(),
            world.matrices.len()
        );
        let mut vertices = 0;
        for object in &world.objects {
            let mesh = &world.meshes[object.mesh_index];
            for instance in object.matrix_start..object.matrix_end {
                let offset = set.vertex_references.instance_offsets[instance] as usize;
                let end = offset.checked_add(mesh.positions.len()).unwrap();
                let references = set
                    .vertex_references
                    .tetrahedron_indices
                    .get(offset..end)
                    .unwrap_or_else(|| panic!("instance{instance} reference range{offset}..{end}"));
                for &tetrahedron in references {
                    assert!(
                        (tetrahedron as usize) < set.spatial.tetrahedra.len(),
                        "instance{instance} invalid tetrahedron{tetrahedron}"
                    );
                }
                vertices += references.len();
            }
        }
        eprintln!("Validated {vertices} BWM instance-vertex references against PNW tetrahedra");
    }

    #[test]
    fn color_conversion_uses_chroma_range() {
        assert_eq!(rgb_to_ycocg([1.0, 0.0, 0.0]), [0.25, 1.0, -0.5]);
        assert_eq!(ycocg_to_rgb([0.25, 1.0, -0.5]), [1.0, 0.0, 0.0]);
        let packed = PackedIrradiance::pack([[0.0; 3]; 4]);
        assert_eq!(packed.unpack(), [[0.0; 3]; 4]);
    }
}
