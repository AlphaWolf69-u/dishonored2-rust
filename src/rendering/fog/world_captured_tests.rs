//! Unchanged captured PS75677; synthetic inputs only. See analysis/probe_captured_world_fog.py.
use super as world;
use super::*;
#[test]
fn compiled_world_fog_shadow_modulation_matches() {
    let text = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/fog_compiled_modulation.txt"
    ));
    let mut maximum = 0.0_f32;
    for (case, line) in text.lines().enumerate() {
        let v: Vec<f32> = line
            .split_whitespace()
            .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
            .collect();
        assert_eq!(v.len(), 54);
        let p = world::WorldFog {
            flags: 120,
            distance: Vec4::from_slice(&v[16..20]),
            distance_start: Vec4::from_slice(&v[20..24]),
            distance_end: Vec4::from_slice(&v[0..4]),
            vertical: Vec4::from_slice(&v[4..8]),
            vertical_start: Vec4::from_slice(&v[8..12]),
            vertical_end: Vec4::from_slice(&v[12..16]),
            height_start: v[31],
            height_end: v[32],
            density_falloff: v[33],
            scattering_intensity: v[35],
            scattering_phase: v[34],
            scattering_direction: Vec3::from_slice(&v[28..31]),
            scattering_color: Vec3::from_slice(&v[24..27]),
        };
        let s = world::WorldFogSample {
            view_ray: Vec3::from_slice(&v[39..42]),
            camera_height: v[37],
            scene_depth: v[42],
            interior_depth: v[43],
            distance_curve: v[44],
            shadow: v[45],
            sky_cloud_density: Vec4::from_slice(&v[46..50]),
            turbulence: 0.,
        };
        let e = world::WorldFogEffects {
            far_clip: v[38],
            scattering_depth_start: v[36],
            god_ray_shadow: v[27],
            ..Default::default()
        };
        for (channel, (actual, expected)) in p
            .shade(s, e)
            .to_array()
            .into_iter()
            .zip(&v[50..54])
            .enumerate()
        {
            let error = (actual - expected).abs();
            maximum = maximum.max(error);
            assert!(
                actual.is_finite()
                    && expected.is_finite()
                    && error <= 0.00002 * expected.abs().max(1.),
                "case{case} channel{channel}: {actual} vs {expected}"
            );
        }
    }
    println!("PASS 36 compiled original fog-shadow modulation cases / 144 channels; max_abs_error={maximum}");
}
