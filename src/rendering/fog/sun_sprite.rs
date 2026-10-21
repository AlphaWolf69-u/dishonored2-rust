//! 30a800's visible sun sprite, separate from the radial light-ray mask.
use super::rays::RayProjection;
use crate::assets::environment::ParameterSource;
use bevy_math::{Mat4, UVec2, Vec3, Vec4};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunSpriteParameters {
    pub enabled: bool,
    pub additive: bool,
    pub direction: Option<Vec3>,
    pub size: f32,
    pub color: Vec4,
}
pub struct SunSpriteSettings {
    pub parameters: SunSpriteParameters,
    pub image: String,
}
impl SunSpriteSettings {
    pub fn resolve(read: &impl ParameterSource, sun: Option<Vec3>) -> Result<Self, String> {
        // Capture both authored and actual-sun branches for prepared area tracks.
        let theta = read.scalar("atm/worldfog/sunspriteangletheta")?;
        let phi = read.scalar("atm/worldfog/sunspriteanglephi")?;
        let direction = if read.boolean("atm/worldfog/spriteusesundirection")? {
            sun
        } else {
            Some(Vec3::from_array(
                crate::render_shadow::environment_sun_direction(theta, phi),
            ))
        };
        Ok(Self {
            parameters: SunSpriteParameters {
                enabled: read.boolean("atm/worldfog/usesuntexture")?,
                additive: read.boolean("atm/worldfog/sunblendadditive")?,
                direction,
                size: read.scalar("atm/worldfog/sunspritesize")?,
                color: Vec4::from_array(read.vector("atm/worldfog/sunspritecolor")?),
            },
            image: read.image_source("atm/worldfog/suntexture")?,
        })
    }
}
impl SunSpriteParameters {
    pub fn project(
        &self,
        clip_from_game_world: Mat4,
        far_clip: f32,
        extent: UVec2,
    ) -> Result<Option<RayProjection>, String> {
        if !self.enabled {
            return Ok(None);
        }
        RayProjection::project(
            self.direction,
            self.size,
            clip_from_game_world,
            far_clip,
            extent,
        )
    }
}
