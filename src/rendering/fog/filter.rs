//! 307530's bilateral parameter preparation (3088xx), including native units.
use bevy_math::Vec4;
#[derive(Clone, Copy, Debug)]
pub struct BlurParameters {
    pub ratio: Vec4,
    pub depth_range: f32,
    pub ray_sharpness: f32,
}
impl BlurParameters {
    pub fn new(settings: crate::platform::game_settings::FogRenderSettings) -> Self {
        let sigma = (settings.blur_radius + 1.) * 0.5;
        let near = settings.bilateral_sharpness_near / settings.near_clip;
        let far = settings.bilateral_sharpness_far / settings.near_clip;
        let far = far * far;
        Self {
            ratio: Vec4::new(
                settings.blur_radius,
                1. / (sigma * 2. * sigma),
                near * near,
                far,
            ),
            depth_range: settings.bilateral_depth_range,
            ray_sharpness: settings.god_ray_depth_sharpness * far,
        }
    }
}
