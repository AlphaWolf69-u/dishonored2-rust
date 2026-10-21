//! Sun mask inputs and 309da0's projected mask quad. Sunlight inside the fog
//! uses a separate shadow march; this projection is the radial light-ray path.
use crate::assets::environment::ParameterSource;
use bevy_math::{Mat4, UVec2, Vec3, Vec4};
#[derive(Clone, Copy, Debug)]
pub struct RaySettings {
    pub mask_size: f32,
    pub direction: Option<Vec3>,
    pub add_scattering: bool,
    pub depth_start: f32,
    pub threshold_bias: Vec3,
    pub threshold_scale: Vec3,
    pub color: Vec4,
    pub density: f32,
    pub decay: f32,
    pub weight: f32,
}
impl RaySettings {
    pub fn resolve(read: &impl ParameterSource, sun: Option<Vec3>) -> Result<Self, String> {
        // Retain all authored branches in prepared area tracks, even when the
        // current world disables its mask or selects the actual sun direction.
        let theta = read.scalar("godraysmask/angletheta")?;
        let phi = read.scalar("godraysmask/anglephi")?;
        let direction = if read.boolean("godraysmask/usesundirection")? {
            sun
        } else {
            Some(Vec3::from_array(
                crate::render_shadow::environment_sun_direction(theta, phi),
            ))
        };
        Ok(Self {
            mask_size: read.scalar("atm/worldfog/godraymasksize")?,
            direction,
            add_scattering: read.boolean("godraysmask/addsunscattering")?,
            depth_start: read.scalar("godraysmask/depthdist")?,
            threshold_bias: Vec4::from_array(read.vector("godraysmask/thresholdbias")?).truncate(),
            threshold_scale: Vec4::from_array(read.vector("godraysmask/thresholdscale")?)
                .truncate(),
            color: Vec4::from_array(read.vector("atm/worldfog/godraycolor")?),
            density: read.scalar("godraysmask/density")?,
            decay: read.scalar("godraysmask/decay")?,
            weight: read.scalar("godraysmask/weight")?,
        })
    }
    /// Input matrix takes game Z-up positions. The original projects an
    /// absolute point at 90% of r_zfar, rather than adding the camera position.
    pub fn project(
        &self,
        clip_from_game_world: Mat4,
        far_clip: f32,
        extent: UVec2,
    ) -> Result<Option<RayProjection>, String> {
        RayProjection::project(
            self.direction,
            self.mask_size,
            clip_from_game_world,
            far_clip,
            extent,
        )
    }
}
#[derive(Clone, Copy, Debug)]
pub struct RayProjection {
    pub transform1: Vec4,
    pub transform2: Vec4,
}
impl RayProjection {
    /// 309da0 and 30a800 use the same absolute point and percent-size quad.
    pub fn project(
        direction: Option<Vec3>,
        size: f32,
        clip_from_game_world: Mat4,
        far_clip: f32,
        extent: UVec2,
    ) -> Result<Option<Self>, String> {
        if !(size > 0.) {
            return Ok(None);
        }
        let direction =
            direction.ok_or("atmosphere projection requests unresolved world sun direction")?;
        let position = direction * far_clip * f32::from_bits(0x3f666666);
        let clip = clip_from_game_world * position.extend(1.);
        let rcp = 1. / clip.w;
        if !(clip.z * rcp > 0.) {
            return Ok(None);
        }
        let radius = size * f32::from_bits(0x3c23d70a);
        let aspect = extent.x as f32 / extent.y as f32;
        Ok(Some(Self {
            transform1: Vec4::new(radius, 0., 0., radius * aspect),
            transform2: Vec4::new(clip.x * rcp, clip.y * rcp, 0., aspect),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projected_sun_masks_match_installed_function() {
        // Separate executable functions independently captured the same rule.
        assert_eq!(
            include_str!("../../../tests/fixtures/fog_ray_projection.txt"),
            include_str!("../../../tests/fixtures/fog_sun_sprite_projection.txt")
        );
        for (case, line) in include_str!("../../../tests/fixtures/fog_ray_projection.txt")
            .lines()
            .enumerate()
        {
            let v = line
                .split_whitespace()
                .map(|value| f32::from_bits(u32::from_str_radix(value, 16).unwrap()))
                .collect::<Vec<_>>();
            assert_eq!(v.len(), 35);
            let direction = if v[0] != 0. {
                Vec3::from_slice(&v[3..6])
            } else {
                Vec3::from_array(crate::render_shadow::environment_sun_direction(v[1], v[2]))
            };
            let settings = RaySettings {
                mask_size: v[7],
                direction: Some(direction),
                add_scattering: false,
                depth_start: 0.,
                threshold_bias: Vec3::ZERO,
                threshold_scale: Vec3::ONE,
                color: Vec4::ONE,
                density: 0.,
                decay: 0.,
                weight: 0.,
            };
            let rows: [f32; 16] = v[10..26].try_into().unwrap();
            let projection = settings
                .project(
                    Mat4::from_cols_array(&rows).transpose(),
                    v[6],
                    UVec2::new(v[8] as u32, v[9] as u32),
                )
                .unwrap();
            assert_eq!(projection.is_some(), v[26] != 0., "sun mask case{case}");
            if let Some(projection) = projection {
                for (field, (actual, expected)) in projection
                    .transform1
                    .to_array()
                    .into_iter()
                    .chain(projection.transform2.to_array())
                    .zip(v[27..].iter().copied())
                    .enumerate()
                {
                    assert!(
                        actual == expected
                            || (actual.is_nan() && expected.is_nan())
                            || (actual - expected).abs() < 0.00003 * expected.abs().max(1.),
                        "sun mask case{case} field{field}: {actual} != {expected}"
                    );
                }
            }
        }
    }
}
