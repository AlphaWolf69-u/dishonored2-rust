//! Arithmetic from the installed particles render program. Scene depth and
//! particle-instance producers remain separate from these shader helpers.
use bevy_math::{Vec2, Vec4};

fn saturate(value: f32) -> f32 {
    if value > 0.0 {
        value.min(1.0)
    } else {
        0.0
    }
}

/// ViewDependentFade consumes DirectX pixel SV_Position.w (clip W).
/// Installed pixel stage 177 uses this input directly.
pub fn view_dependent_fade(clip_w: f32, start: f32, end: f32) -> f32 {
    let t = saturate((clip_w - start) * (1.0 / (end - start)));
    t * t * (3.0 - 2.0 * t)
}

/// ComputeSoftParticleScale after the caller has sampled its depth texture.
pub fn soft_particle_scale(scene_depth: f32, fragment_depth: f32, softness: f32) -> f32 {
    let scene = 1.0 / (1.0 - scene_depth);
    let fragment = 1.0 / (1.0 - fragment_depth);
    saturate((scene - fragment) * softness)
}

/// Opacity_Cube's alpha path, given the original pixel context's depth values.
pub fn cubic_opacity(scene_depth: f32, fragment_depth: f32, radius: f32, threshold: f32) -> f32 {
    saturate(((scene_depth - fragment_depth) / radius) / threshold)
}

/// InvertSoftContact applies the complement of the selected contact mode.
pub fn invert_soft_contact(opacity: f32) -> f32 {
    1.0 - opacity
}

/// ComputeDistortionMV retains both blurred and raw distortion coordinates.
pub fn distortion_motion_vector(distortion: Vec2, blur: f32) -> Vec4 {
    (distortion * blur)
        .extend(distortion.x)
        .extend(distortion.y)
}

/// ComputeAnimCrossFadeColor after sampling both animation frames. The frame
/// fraction is not clamped; keep the shader's multiply/add order.
pub fn animation_crossfade(first: Vec4, second: Vec4, fraction: f32) -> Vec4 {
    first * (1.0 - fraction) + second * fraction
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_helpers_match_installed_hlsl() {
        let expected = include_str!("../../tests/fixtures/particle_math_cases.txt")
            .split_whitespace()
            .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(expected.len(), 64 * 16);
        for id in 0..64 {
            let d = (id % 17) as f32 / 8.0 - 0.25;
            let a = (id % 5) as f32 / 8.0;
            let b = (id % 7) as f32 / 8.0;
            let cubic = cubic_opacity(
                1.0 + (id % 13) as f32,
                (id % 7) as f32 * 0.5,
                0.25 * (1.0 + (id % 3) as f32),
                (id % 5) as f32 / 4.0,
            );
            let values = [
                Vec4::new(
                    view_dependent_fade(d, a, b),
                    view_dependent_fade(d, b, a),
                    view_dependent_fade(a, a, a),
                    view_dependent_fade(a + 0.25, a, a),
                ),
                Vec4::new(
                    soft_particle_scale(
                        (id % 9) as f32 / 8.0,
                        (id % 11) as f32 / 8.0,
                        (id % 5) as f32 * 0.2,
                    ),
                    cubic,
                    invert_soft_contact(cubic),
                    0.0,
                ),
                distortion_motion_vector(
                    Vec2::new((id % 7) as f32 - 3.0, (id % 11) as f32 - 5.0) * 0.25,
                    (id % 5) as f32 * 0.3,
                ),
                animation_crossfade(
                    Vec4::new(0.2, 0.4, 0.8, 0.6),
                    Vec4::new(0.7, 0.1, 0.9, 0.3),
                    (id % 9) as f32 / 4.0 - 0.5,
                ),
            ];
            for (field, actual) in values.into_iter().flat_map(|v| v.to_array()).enumerate() {
                let reference = expected[id * 16 + field];
                assert!(
                    (actual - reference).abs() <= 0.000002 * reference.abs().max(1.0),
                    "particle case {id} field {field}: {actual} != {reference}"
                );
            }
        }
    }
}
