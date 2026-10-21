//! Area-volume distance fog: 30b2f0's parameters and renderMinMax/renderVolume.
use super::{alpha::VolumeFog, world::WorldFog};
use crate::assets::environment::ParameterSource;
use bevy_math::{Vec3, Vec4};

fn unit(value: f32) -> f32 {
    if value > 0. {
        value.min(1.)
    } else {
        0.
    }
}

#[derive(Clone, Debug)]
pub struct VolumeSettings {
    pub flags: u32,
    pub distance: Vec4,
    pub color_start: Vec4,
    pub color_end: Vec4,
    pub density_factor: f32,
    pub falloff: f32,
    pub scattering_phase: f32,
    pub scattering_intensity: f32,
    pub artist_direction: Vec3,
    pub artist_color: Vec3,
    pub use_sun_direction: bool,
    pub use_sun_color: bool,
    pub fade_start: f32,
    pub fade_end: f32,
}
impl VolumeSettings {
    pub fn resolve(read: &impl ParameterSource) -> Result<Option<Self>, String> {
        let scalar = |name| read.scalar(&format!("env/atm/distancefog/{name}"));
        let extinction = scalar("extinctioncoefficient")?;
        // 307530 excludes these volumes before fetching their other inputs.
        if !(extinction > 0.) {
            return Ok(None);
        }
        let intensity = scalar("scatteringintensity")?;
        let varying = read.boolean("atm/distancefog/enablevaryingdensity")?;
        let start = scalar("startdistance")?;
        let end = scalar("enddistance")?;
        let direction = crate::render_shadow::environment_sun_direction(
            scalar("scatteringangletheta")?,
            scalar("scatteringanglephi")?,
        );
        Ok(Some(Self {
            flags: 1 | if intensity > 0. { 2 } else { 0 } | if varying { 4 } else { 0 },
            distance: Vec4::new(start, 1. / (end - start), extinction, 0.),
            color_start: Vec4::from_array(read.vector("env/atm/distancefog/colorstart")?),
            color_end: Vec4::from_array(read.vector("env/atm/distancefog/colorend")?),
            density_factor: scalar("densityfactor")?,
            falloff: scalar("densityfalloffspeed")?,
            scattering_phase: scalar("scatteringphase")?,
            scattering_intensity: intensity,
            artist_direction: Vec3::from_array(direction),
            artist_color: Vec4::from_array(
                read.vector("env/atm/distancefog/artistscatteringcolor")?,
            )
            .truncate(),
            use_sun_direction: read.boolean("env/atm/distancefog/useactualsundirection")?,
            use_sun_color: read.boolean("env/atm/distancefog/useactualsuncolor")?,
            fade_start: scalar("fadeinstartdistance")?,
            fade_end: scalar("fadeinenddistance")?,
        }))
    }

    pub fn prepare(
        &self,
        eye: Vec3,
        origin: Vec3,
        target: Option<Vec3>,
        sun: Option<(Vec3, Vec3)>,
    ) -> Result<VolumeFog, String> {
        let direction = if self.use_sun_direction {
            sun.ok_or("area fog requests unresolved sun direction")?.0
        } else if let Some(target) = target {
            let delta = target - eye;
            delta
                / delta
                    .length_squared()
                    .max(f32::from_bits(0x00800000))
                    .sqrt()
        } else {
            self.artist_direction
        };
        let color = if self.use_sun_color {
            sun.ok_or("area fog requests unresolved sun colour")?.1
        } else {
            self.artist_color
        };
        let fade = unit(
            (origin.distance_squared(eye) - self.fade_start)
                / (self.fade_end - self.fade_start).max(f32::from_bits(0x3a83126f)),
        );
        Ok(VolumeFog {
            volume_type: WorldFog::alpha_volume_type(self.flags),
            distance: self.distance.with_w(fade),
            color_start: self.color_start.truncate(),
            color_end: self.color_end.truncate(),
            density_factor: self.density_factor,
            falloff: self.falloff,
            scattering_direction: direction,
            scattering_phase: self.scattering_phase,
            scattering_color: color,
            scattering_intensity: self.scattering_intensity,
        })
    }
}

/// RGB MIN / alpha ADD contribution; the clear is (1e6,0,0,0).
pub fn interval_contribution(z: f32, scene_depth: f32, front: bool) -> Vec4 {
    let end = z.min(scene_depth);
    Vec4::new(z, 1. - end, end, if front { -end } else { end })
}

/// Opaque renderVolume differs from ComputeVolumeFog: scattering uses a
/// colour-derived weight, and the varying-density integral has no epsilon.
pub fn shade(fog: &VolumeFog, ray: Vec3, camera_height: f32, interval: Vec4) -> Option<Vec4> {
    if interval.w < 0.001 {
        return None;
    }
    let mut distance = unit((interval.w - fog.distance.x) * fog.distance.y);
    let mut color = fog.color_start.lerp(fog.color_end, distance);
    let extinction = if fog.volume_type.z > 0. {
        let start = if interval.x < 1. - interval.y {
            interval.x
        } else {
            0.
        };
        let altitude = camera_height + ray.length() * start * (ray.z / ray.length());
        let dy = ray.z / ray.length();
        distance *= fog.density_factor;
        let density = fog.distance.z
            * ((-fog.falloff * altitude).exp() - (-fog.falloff * (altitude + dy * distance)).exp())
            / (fog.falloff * dy);
        (-density).exp()
    } else {
        (-fog.distance.z * distance).exp()
    };
    if fog.volume_type.y > 0. {
        let phase = super::world::mie_phase(
            fog.scattering_direction.dot(ray.normalize()),
            fog.scattering_phase,
        );
        let scattered = fog.scattering_color * (fog.scattering_intensity * phase);
        color += scattered * unit(scattered.max_element());
    }
    Some(color.extend(1. - extinction))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn area_volume_shader_bodies_match_installed_outputs() {
        let reference = include_str!("../../../tests/fixtures/fog_volume.txt")
            .split_whitespace()
            .map(|v| f32::from_bits(u32::from_str_radix(v, 16).unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(reference.len(), 128 * 4);
        for id in 0..64 {
            let p = VolumeFog {
                volume_type: WorldFog::alpha_volume_type(1 | ((id % 4) << 1)),
                distance: Vec4::new(3., 1. / 40., 2.5, 0.),
                color_start: Vec3::new(0.4, 0.2, 0.6),
                color_end: Vec3::new(0.8, 0.9, 0.3),
                density_factor: 2.,
                falloff: 0.03,
                scattering_direction: Vec3::new(0.6, 0., 0.8),
                scattering_phase: 0.75,
                scattering_color: Vec3::new(10., 9.8, 7.2),
                scattering_intensity: 0.3,
            };
            let interval = Vec4::new(
                (id % 3) as f32 * 5.,
                1. - (id % 4) as f32 * 12.,
                0.,
                (id % 13) as f32 * 7.,
            );
            let ray = Vec3::new(
                ((id % 5) as f32 - 2.) * 0.3,
                ((id % 7) as f32 - 3.) * 0.2,
                1.,
            );
            let radiance =
                shade(&p, ray, (id % 9) as f32 - 4., interval).unwrap_or(Vec4::splat(-99.));
            let contribution =
                interval_contribution((id % 11) as f32 * 3., (id % 7) as f32 * 5., id & 1 != 0);
            for (channel, (actual, expected)) in radiance
                .to_array()
                .into_iter()
                .chain(contribution.to_array())
                .zip(
                    reference[id as usize * 8..id as usize * 8 + 8]
                        .iter()
                        .copied(),
                )
                .enumerate()
            {
                assert!(
                    (actual - expected).abs() < 0.0003,
                    "area volume case{id} channel{channel}: {actual} != {expected}"
                );
            }
        }
    }
}
