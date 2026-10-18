//! Native probe preparation (291430), sky projection (2b0370) and accumulatesh.
use super::ProbeLighting;

#[derive(Clone, Debug)]
pub struct ProbeLightingParameters {
    pub static_factor: [f32; 3],
    pub sun_factor: [f32; 3],
    pub sun_ground_color: [f32; 3],
    pub sky_zenith: [f32; 3],
    pub sky_horizon: [f32; 3],
    pub ambient: [f32; 3],
    pub multi_bounce: f32,
    pub multi_bounce_flags: u32,
}

/// The original projects 40 equal-area samples in the upper hemisphere and
/// mirrors them at the equator. Preserve that quadrature, including its small
/// nonzero order-2 result for a constant sky.
pub fn sky_zh(zenith: [f32; 3], horizon: [f32; 3]) -> [[f32; 3]; 3] {
    let pi = std::f32::consts::PI;
    let weight = (2.0 * pi) * 2.0 / 40.0;
    let c0 = (1.0 / pi).sqrt() * 0.5 * weight;
    let c2 = (5.0 / pi).sqrt() * 0.25 * weight;
    let mut out = [[0.0; 3]; 3];
    for i in 0..40 {
        let angle = (((i as f32 + 0.5) * 0.5) / 40.0).sqrt().asin() * 2.0;
        let t = (angle * 2.0) / pi;
        let z = angle.cos();
        let p2 = (z * 3.0) * z - 1.0;
        for c in 0..3 {
            let a = if zenith[c].abs() <= 1.0e-18 {
                0.0
            } else {
                zenith[c]
            };
            let b = if horizon[c].abs() <= 1.0e-18 {
                0.0
            } else {
                horizon[c]
            };
            let value = (1.0 - t) * a + b * t;
            out[0][c] += value * c0;
            out[2][c] += p2 * value * c2;
        }
    }
    out
}

/// accumulatesh::ZHSHProductSky. The middle zonal coefficient is zero.
fn sky_product(a: [[f32; 3]; 3], b: [[f32; 3]; 9]) -> [[f32; 3]; 9] {
    let mut r = [[0.0; 3]; 9];
    for c in 0..3 {
        let a0 = a[0][c];
        let a2 = a[2][c];
        let c0a0 = 0.282094792936_f32 * a0;
        let c1a2 = -0.126156626101_f32 * a2;
        r[0][c] = c0a0 * b[0][c];
        r[1][c] = b[1][c] * (c0a0 + c1a2);
        r[2][c] = (c0a0 + 0.252313259987_f32 * a2) * b[2][c];
        r[3][c] = b[3][c] * (c0a0 + c1a2);
        r[4][c] = b[4][c] * (c0a0 - 0.180223751576_f32 * a2);
        r[5][c] = b[5][c] * (c0a0 + 0.0901118757865_f32 * a2);
        r[6][c] = 0.282094792936_f32 * (b[6][c] * a0 + b[0][c] * a2);
        let t = a2 * b[6][c];
        r[0][c] += 0.282094792936_f32 * t;
        r[6][c] += 0.180223751576_f32 * t;
        r[7][c] = b[7][c] * (c0a0 + 0.0901118757865_f32 * a2);
        r[8][c] = b[8][c] * (c0a0 - 0.180223751576_f32 * a2);
    }
    r
}

pub(super) fn accumulate(probe: &ProbeLighting, p: &ProbeLightingParameters) -> [[f32; 3]; 9] {
    let s = &probe.sources;
    let prepare = |initial: [[f32; 3]; 9], bounce: [[f32; 3]; 9], flag: u32| {
        let clear = p.multi_bounce.abs() > 0.001 && p.multi_bounce < 0.0;
        let gain = if p.multi_bounce.abs() > 0.001 && p.multi_bounce_flags & flag != 0 {
            p.multi_bounce.abs()
        } else {
            0.0
        };
        std::array::from_fn::<_, 9, _>(|i| {
            std::array::from_fn::<_, 3, _>(|c| {
                let base = if clear { 0.0 } else { initial[i][c] };
                base + gain * bounce[i][c]
            })
        })
    };
    let static_sh = prepare(probe.static_lighting_sh, s.multi_bounce_static_sh, 1);
    let sun_sh = prepare(probe.filtered_sun_factor_sh, s.multi_bounce_sun_sh, 2);
    let sky_sh = prepare(
        probe.filtered_sky_occlusion_sh.map(|v| [v; 3]),
        s.multi_bounce_sky_sh,
        4,
    );
    let sky_factor = s.bounce_factors[1].map(|v| v * s.sky_scale);
    let mut zh = sky_zh(p.sky_zenith, p.sky_horizon);
    for row in &mut zh {
        for c in 0..3 {
            row[c] *= sky_factor[c];
        }
    }
    let sky = sky_product(zh, sky_sh);
    let mut result = std::array::from_fn::<_, 9, _>(|i| {
        std::array::from_fn(|c| {
            let factor_static = p.static_factor[c] * (s.lighting_scale * s.bounce_factors[3][c]);
            let factor_sun = (p.sun_factor[c] * (s.lighting_scale * s.bounce_factors[0][c]))
                * p.sun_ground_color[c];
            (factor_static * static_sh[i][c] + factor_sun * sun_sh[i][c]) + sky[i][c]
        })
    });
    for c in 0..3 {
        result[0][c] += (3.544907701811032_f32 * s.ambient_factor[c]) * p.ambient[c];
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sky_projection_matches_original_executable() {
        for line in "00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
3f800000 3f800000 3f800000 3f800000 3f800000 3f800000 4062dfbc 4062dfbc 4062dfbc 00000000 00000000 00000000 ba225900 ba225900 ba225900
3f800000 00000000 00000000 00000000 3f800000 00000000 3fa4cad2 40107a59 00000000 00000000 00000000 00000000 3f56b372 bf56dc08 00000000
3e4ccccd 3ecccccd 3f19999a 3f800000 40000000 40400000 4020f50c 40a0f50c 40f16f8e 00000000 00000000 00000000 bf2beb58 bfabeb58 c000f083".lines() {
            let f=line.split_whitespace().map(|s|f32::from_bits(u32::from_str_radix(s,16).unwrap())).collect::<Vec<_>>();
            let actual=sky_zh(f[..3].try_into().unwrap(),f[3..6].try_into().unwrap());
            for(i,v) in actual.into_iter().flatten().enumerate() {
                assert!((v-f[6+i]).abs()<0.000008,"sky coefficient{i}: {v} != {}",f[6+i]);
            }
        }
    }

    #[test]
    #[ignore = "requires installed EOTW assets and numerical original GPU probe buffers"]
    fn installed_probe_sources_match_original_gpu_accumulation() {
        let catalog =
            crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
                .unwrap();
        let mut set = super::super::load_for_world(&catalog, "eotw_p_lowchaos")
            .unwrap()
            .unwrap();
        let read = |name: &str| {
            std::fs::read(format!(
                "artifacts/native-probe-input-ro_indirectlighting_update_{name}.bin"
            ))
            .unwrap()
        };
        let static_input = read("bufferprobessh_static");
        let sun_input = read("bufferprobessh_staticsun");
        let sky_input = read("bufferprobessh_skyocclusion");
        let info = read("bufferprobesinfo");
        let dynamic = read("bufferprobessh_dynamic");
        let output =
            std::fs::read("artifacts/native-probe-output-rw_indirectlighting_bufferprobessh.bin")
                .unwrap();
        let f = |bytes: &[u8], offset| {
            f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
        };
        let parameters = ProbeLightingParameters {
            static_factor: [1.; 3],
            sun_factor: [1.; 3],
            sun_ground_color: [123.9365234375, 98.43280792236328, 67.15886688232422],
            sky_zenith: [0.; 3],
            sky_horizon: [37.7451, 52.411766, 55.],
            ambient: [1.; 3],
            multi_bounce: 3.,
            multi_bounce_flags: 7,
        };
        assert_eq!(output.len(), set.probes.len() * 112);
        let mut source_max = 0.0_f32;
        let mut result_max = 0.0_f32;
        let mut worst = 0;
        for (p, probe) in set.probes.iter().enumerate() {
            let s = &probe.sources;
            for c in 0..3 {
                for (at, value) in [
                    (0, s.lighting_scale * s.bounce_factors[3][c]),
                    (3, s.bounce_factors[2][c]),
                    (6, s.sky_scale * s.bounce_factors[1][c]),
                    (9, s.lighting_scale * s.bounce_factors[0][c]),
                    (12, s.ambient_factor[c]),
                ] {
                    assert_eq!(
                        value.to_bits(),
                        f(&info, p * 60 + (at + c) * 4).to_bits(),
                        "probe{p} factor{at} channel{c}"
                    );
                }
            }
            for i in 0..9 {
                for c in 0..3 {
                    for (bytes, initial, bounce) in [
                        (
                            &static_input,
                            probe.static_lighting_sh[i][c],
                            s.multi_bounce_static_sh[i][c],
                        ),
                        (
                            &sun_input,
                            probe.filtered_sun_factor_sh[i][c],
                            s.multi_bounce_sun_sh[i][c],
                        ),
                        (
                            &sky_input,
                            probe.filtered_sky_occlusion_sh[i],
                            s.multi_bounce_sky_sh[i][c],
                        ),
                    ] {
                        let expected = initial + 3.0 * bounce;
                        let actual = f(bytes, p * 112 + (i * 3 + c) * 4);
                        source_max = source_max.max((actual - expected).abs());
                    }
                }
            }
            let result = accumulate(probe, &parameters);
            for i in 0..9 {
                for c in 0..3 {
                    let native = f(&output, p * 112 + (i * 3 + c) * 4);
                    let dyn_value = f(&dynamic, p * 112 + (i * 3 + c) * 4) * s.bounce_factors[2][c];
                    let error = (result[i][c] + dyn_value - native).abs();
                    if error > result_max {
                        result_max = error;
                        worst = p;
                    }
                }
            }
        }
        eprintln!(
            "original probe preparation max error={source_max}, accumulation max error={result_max} at probe{worst}"
        );
        assert!(
            source_max < 0.0001,
            "original source preparation differs: {source_max}"
        );
        assert!(
            result_max < 0.0002,
            "original accumulation differs: {result_max}"
        );
        set.initialize_indirect_lighting(&parameters);
        assert!(set
            .probes
            .iter()
            .any(|p| p.render_lighting_sh[0] != p.static_lighting_sh[0]));
    }
}
