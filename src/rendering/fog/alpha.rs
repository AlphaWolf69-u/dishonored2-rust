//! fogHelpers' transparent-surface fog. The distance argument is supplied
//! directly by its caller: pixel SV_POSITION.w or per-vertex clip-position.w.
//! The opaque path's ray-length scaling and sky mask do not apply.
use super::world::{mie_phase, WorldFog, WorldFogEffects};
use bevy_math::{Vec2, Vec3, Vec4};

fn saturate(value: f32) -> f32 {
    if value > 0. {
        value.min(1.)
    } else {
        0.
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WorldSample {
    pub ray: Vec3,
    pub distance: f32,
    pub camera_height: f32,
    pub curve: f32,
    pub canvas: f32,
}

/// ComputeWorldFog. The source declares a scattering flag but implements no
/// scattering calculation in this helper; keep the same distance/height mix.
pub fn world(
    fog: &WorldFog,
    world_type: Vec4,
    sample: WorldSample,
    effects: WorldFogEffects,
) -> Vec4 {
    if world_type.element_sum() <= 0. {
        return Vec4::ZERO;
    }
    let direction = sample.ray / sample.ray.length();
    let distance = if world_type.x > 0. {
        fog.distance_color(
            saturate((sample.distance - fog.distance.x) * fog.distance.y),
            sample.curve,
        )
    } else {
        Vec4::ZERO
    };
    let vertical = if world_type.z > 0. {
        fog.vertical_color(
            direction.z,
            sample.camera_height,
            sample.distance,
            saturate((sample.distance - fog.vertical.x) * fog.vertical.y),
        )
    } else {
        Vec4::ZERO
    };
    let mut color = distance.lerp(vertical, vertical.w);
    if fog.flags & 128 != 0 {
        let noisy = color
            .truncate()
            .lerp(fog.distance_end.truncate(), sample.canvas);
        let factor = saturate(
            (sample.distance - effects.turbulence_start)
                / (effects.turbulence_end - effects.turbulence_start),
        );
        color = color.truncate().lerp(noisy, factor).extend(color.w);
    }
    color
}

/// Local env/atm/distanceFog uniforms used by ComputeVolumeFog, distinct from
/// the world distance/height ramps. `interval` is alphaFog/ZMinMaxAcc.
#[derive(Clone, Copy, Debug)]
pub struct VolumeFog {
    pub volume_type: Vec3,
    pub distance: Vec4,
    pub color_start: Vec3,
    pub color_end: Vec3,
    pub density_factor: f32,
    pub falloff: f32,
    pub scattering_direction: Vec3,
    pub scattering_phase: f32,
    pub scattering_color: Vec3,
    pub scattering_intensity: f32,
}
impl VolumeFog {
    pub fn shade(&self, ray: Vec3, distance: f32, camera_height: f32, interval: Vec2) -> Vec4 {
        if self.volume_type.x <= 0. {
            return Vec4::ZERO;
        }
        let distance = distance.min(interval.y) - interval.x.max(0.);
        let mut normalized = saturate((distance - self.distance.x) * self.distance.y);
        let mut color = self.color_start.lerp(self.color_end, normalized);
        let mut ray_length = 0.;
        let mut direction = Vec3::ZERO;
        if self.volume_type.y > 0. || self.volume_type.z > 0. {
            ray_length = ray.length();
            direction = ray / ray_length;
        }
        let extinction = if self.volume_type.z > 0. {
            let start = if interval.x < interval.y {
                interval.x
            } else {
                0.
            };
            let altitude = camera_height + (ray_length * start) * direction.z;
            let dy = direction.z + if direction.z > 0. { 0.00001 } else { -0.00001 };
            normalized *= self.density_factor;
            let density = self.distance.z
                * ((-self.falloff * altitude).exp()
                    - (-self.falloff * (altitude + dy * normalized)).exp())
                / (self.falloff * dy);
            // The compiled exp/saturate path lowers to min(exp,1), including
            // the same zero-opacity result at zero falloff as vertical fog.
            1. - (-density).exp().min(1.)
        } else {
            1. - (-self.distance.z * normalized).exp().min(1.)
        };
        if self.volume_type.y > 0. {
            let phase = mie_phase(
                self.scattering_direction.dot(direction),
                self.scattering_phase,
            );
            color += self.scattering_color * (self.scattering_intensity * phase);
        }
        color.extend(extinction)
    }
}

/// The shared premultiplied mix in ComputeFogWithLightScattering and the
/// per-vertex helper. The pixel path supplies an unclamped depth ratio; the
/// per-vertex caller saturates its ratio before this operation.
pub fn mixed(
    world: Vec4,
    volume: Vec4,
    scattering: Vec3,
    precomputed: Vec4,
    depth_ratio: f32,
) -> Vec4 {
    let rgb =
        volume.truncate() * volume.w + world.truncate() * world.w * (1. - volume.w) + scattering;
    let alpha = saturate(volume.w + world.w * (1. - volume.w));
    rgb.extend(alpha).lerp(precomputed, depth_ratio)
}

pub fn composite(color: Vec3, fog: Vec4) -> Vec3 {
    fog.truncate() + color * (1. - fog.w)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pixel_composite_keeps_mesh_and_particle_ratios_unclamped() {
        let reference = include_str!("../../../tests/fixtures/fog_alpha_composite.txt")
            .split_whitespace()
            .map(|word| f32::from_bits(u32::from_str_radix(word, 16).unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(reference.len(), 128 * 4);
        for id in 0..64 {
            let world = Vec4::new(
                0.1 + (id % 3) as f32 * 0.05,
                0.3,
                0.8,
                (id % 5) as f32 * 0.4,
            );
            let volume = Vec4::new(0.4, 0.2, 0.1, (id % 7) as f32 * 0.25);
            let precomputed = Vec4::new(0.9, 0.7, 0.4, (id % 6) as f32 * 0.2);
            let ratios = [
                (1. - (id % 9) as f32 / 8.) / (1.0001 - (id % 5) as f32 * 0.25),
                (id % 11) as f32 * 3. / (0.01 + (id % 7) as f32 * 5. + 1.),
            ];
            for (kind, ratio) in ratios.into_iter().enumerate() {
                let value = composite(
                    Vec3::new(0.2, 0.5, 0.9),
                    mixed(
                        world,
                        volume,
                        Vec3::new(0.03, 0.06, 0.09),
                        precomputed,
                        ratio,
                    ),
                );
                for (channel, actual) in value.to_array().into_iter().enumerate() {
                    let expected = reference[id * 8 + kind * 4 + channel];
                    assert!((actual-expected).abs()<0.0003+expected.abs()*0.00001,
                        "pixel composite id={id} kind={kind} channel={channel}: {actual} != {expected}");
                }
            }
        }
    }
    #[test]
    fn transparent_world_and_volume_match_installed_helpers() {
        let reference = include_str!("../../../tests/fixtures/fog_alpha.txt")
            .split_whitespace()
            .map(|word| f32::from_bits(u32::from_str_radix(word, 16).unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(reference.len(), 128 * 4);
        for id in 0..64 {
            let fog = WorldFog {
                flags: if id & 8 != 0 { 128 } else { 0 },
                distance: Vec4::new(10., 1. / 90., 3.5, 0.),
                distance_start: Vec4::new(0.1, 0.3, 0.8, 0.2),
                distance_end: Vec4::new(1.1, 0.8, 0.2, 0.7),
                vertical: Vec4::new(2., 1. / 70., 0.4, 0.),
                vertical_start: Vec4::new(0.2, 0.7, 1.1, 0.3),
                vertical_end: Vec4::new(1.8, 0.4, 0.1, 0.8),
                height_start: -2.,
                height_end: 14.,
                density_falloff: 0.03,
                ..Default::default()
            };
            let flags = Vec3::new(
                (id & 1) as f32,
                ((id >> 1) & 1) as f32,
                ((id >> 2) & 1) as f32,
            );
            let ray = Vec3::new(
                ((id % 5) as f32 - 2.) * 0.3,
                ((id % 7) as f32 - 3.) * 0.2,
                1.,
            );
            let distance = (id % 13) as f32 * 17.;
            let camera = (id % 11) as f32 - 5.;
            let world = world(
                &fog,
                flags.extend(if id & 32 != 0 { 2. } else { 0. }),
                WorldSample {
                    ray,
                    distance,
                    camera_height: camera,
                    curve: (id % 9) as f32 / 8.,
                    canvas: (id % 5) as f32 / 4.,
                },
                WorldFogEffects {
                    turbulence_start: 5.,
                    turbulence_end: 100.,
                    ..Default::default()
                },
            );
            let volume = VolumeFog {
                volume_type: flags,
                distance: Vec4::new(3., 1. / 40., 2.5, 0.),
                color_start: Vec3::new(0.4, 0.2, 0.6),
                color_end: Vec3::new(0.8, 0.9, 0.3),
                density_factor: 2.,
                falloff: if id & 16 != 0 { 0. } else { 0.03 },
                scattering_direction: Vec3::new(0.6, 0., 0.8),
                scattering_phase: 0.75,
                scattering_color: Vec3::new(10., 9.8, 7.2),
                scattering_intensity: 0.3,
            }
            .shade(
                ray,
                distance,
                camera,
                Vec2::new(((id % 5) as f32 - 2.) * 3., (id % 7) as f32 * 8.),
            );
            for (part, actual) in [world, volume].into_iter().enumerate() {
                for (channel, value) in actual.to_array().into_iter().enumerate() {
                    let expected = reference[id * 8 + part * 4 + channel];
                    assert!(
                        (value - expected).abs() < 0.0003,
                        "alpha id={id} part={part} channel={channel}: {value} != {expected}"
                    );
                }
            }
        }
    }
}
