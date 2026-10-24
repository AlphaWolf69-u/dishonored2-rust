//! Player posture shapes (arkPhysicsPlayerShape, FUN_1401b3d00).
//!
//! Hulls are built with Havok's default shrink-by-convex-radius
//! (build config +4 = 1, FUN_140ca0d50; the player clears only +0x11, the
//! mass-properties flag), so the shape keeps the authored vertex extent.
use bevy_math::Vec3;

/// arkPhysicsPlayerShapeSettings (0x18), ctor FUN_1401b2c40 defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeSettings {
    pub radius: f32,
    pub height: f32,
    pub step_height: f32,
    pub z_offset: f32,
    pub extrude_length: f32,
    pub gravity_strength: f32,
}

impl Default for ShapeSettings {
    fn default() -> Self {
        Self {
            radius: f32::from_bits(0x3f0c_cccd),
            height: f32::from_bits(0x3fd9_999a),
            step_height: f32::from_bits(0x3e99_999a),
            z_offset: 0.0,
            extrude_length: 0.0,
            gravity_strength: 2.0,
        }
    }
}

/// arkPlayerPosture_t (enum table 0x142393428).
pub const POSTURE_COUNT: usize = 20;
pub const POSTURE_STANDING: usize = 0;
pub const POSTURE_SNEAKING: usize = 1;
pub const POSTURE_AUTOCROUCHING: usize = 2;
pub const POSTURE_SWIMMING: usize = 7;
pub const POSTURE_KEEP_PREVIOUS: usize = 21;

pub const POSTURE_NAMES: [&str; 23] = [
    "ARK_PLAYER_POSTURE_STANDING",
    "ARK_PLAYER_POSTURE_SNEAKING",
    "ARK_PLAYER_POSTURE_AUTOCROUCHING",
    "ARK_PLAYER_POSTURE_WALLCROUCHING",
    "ARK_PLAYER_POSTURE_BLINKING",
    "ARK_PLAYER_POSTURE_SLIDING",
    "ARK_PLAYER_POSTURE_VAULTING",
    "ARK_PLAYER_POSTURE_SWIMMING",
    "ARK_PLAYER_POSTURE_GRAPPLE",
    "ARK_PLAYER_POSTURE_GRAPPLE_SNEAK",
    "ARK_PLAYER_POSTURE_GRAPPLE_CROUCH",
    "ARK_PLAYER_POSTURE_LEAN_FORWARD",
    "ARK_PLAYER_POSTURE_CHOKE",
    "ARK_PLAYER_POSTURE_VERSUS_GRAB",
    "ARK_PLAYER_POSTURE_POSSESS_RAT",
    "ARK_PLAYER_POSTURE_POSSESS_BLOODFLY",
    "ARK_PLAYER_POSTURE_POSSESS_FISH",
    "ARK_PLAYER_POSTURE_POSSESS_CORPSE",
    "ARK_PLAYER_POSTURE_POSSESS_HUMAN",
    "ARK_PLAYER_POSTURE_POSSESS_HOUND",
    "ARK_PLAYER_POSTURE_MAX",
    "ARK_PLAYER_POSTURE_KEEP_PREVIOUS",
    "ARK_PLAYER_POSTURE_KEEP_NEXT",
];

pub fn posture(name: &str) -> Option<usize> {
    POSTURE_NAMES.iter().position(|n| *n == name)
}

/// One posture record: settings plus its two hulls in player space
/// (origin at the feet, +X forward, +Z up).
#[derive(Clone, Debug, PartialEq)]
pub struct PostureShape {
    pub settings: ShapeSettings,
    /// FUN_1401b40d0: 12-gon at `height`, 12-gon at `stepHeight + zOffset`,
    /// apex at `zOffset`.
    pub body: Vec<Vec3>,
    /// FUN_1401b3d80: 12-gon at `stepHeight` and the apex at the origin.
    pub ceil_floor: Vec<Vec3>,
}

/// FUN_1401b2b80: (1,0,0) rotated about +Z by -π + i·2π/12.
fn ring() -> [Vec3; 12] {
    std::array::from_fn(|i| {
        let angle = -std::f32::consts::PI + i as f32 * (std::f32::consts::TAU / 12.0);
        Vec3::new(angle.cos(), angle.sin(), 0.0)
    })
}

fn extrude(points: &mut [Vec3], length: f32) {
    if length > 0.0 {
        for point in points.iter_mut().filter(|p| p.x > f32::EPSILON) {
            point.x += length;
        }
    }
}

impl PostureShape {
    pub fn new(settings: ShapeSettings) -> Self {
        let ring = ring();
        let r = settings.radius;
        let mut body: Vec<Vec3> = ring
            .iter()
            .map(|d| Vec3::new(r * d.x, r * d.y, settings.height - settings.z_offset))
            .chain(ring.iter().map(|d| Vec3::new(r * d.x, r * d.y, settings.step_height)))
            .chain(std::iter::once(Vec3::ZERO))
            .collect();
        if settings.z_offset != 0.0 {
            for point in &mut body {
                point.z += settings.z_offset;
            }
        }
        extrude(&mut body, settings.extrude_length);
        let mut ceil_floor: Vec<Vec3> = ring
            .iter()
            .map(|d| Vec3::new(r * d.x, r * d.y, settings.step_height))
            .chain(std::iter::once(Vec3::ZERO))
            .collect();
        extrude(&mut ceil_floor, settings.extrude_length);
        Self { settings, body, ceil_floor }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standing_hull_is_a_prism_on_a_cone_to_the_feet() {
        let shape = PostureShape::new(ShapeSettings {
            radius: 0.4,
            height: 1.8,
            step_height: 0.5,
            gravity_strength: 3.0,
            ..Default::default()
        });
        assert_eq!(shape.body.len(), 25);
        assert_eq!(shape.body[24], Vec3::ZERO);
        assert!(shape.body[..12].iter().all(|p| p.z == 1.8 && (p.truncate().length() - 0.4).abs() < 1e-6));
        assert!(shape.body[12..24].iter().all(|p| p.z == 0.5));
        assert_eq!(shape.ceil_floor.len(), 13);
        assert!(shape.body[0].distance(Vec3::new(-0.4, 0.0, 1.8)) < 1e-6);
    }

    #[test]
    fn z_offset_and_extrusion_follow_the_hull_builder() {
        let shape = PostureShape::new(ShapeSettings {
            radius: 0.4,
            height: 1.8,
            step_height: 0.5,
            z_offset: 1.0,
            extrude_length: 0.25,
            ..Default::default()
        });
        assert_eq!(shape.body[24], Vec3::new(0.0, 0.0, 1.0));
        assert!(shape.body[..12].iter().all(|p| p.z == 1.8));
        assert!(shape.body[12..24].iter().all(|p| p.z == 1.5));
        let front = shape.body[6];
        assert!((front.x - 0.65).abs() < 1e-6, "{front:?}");
        assert!(shape.ceil_floor[..12].iter().all(|p| p.z == 0.5));
    }
}
