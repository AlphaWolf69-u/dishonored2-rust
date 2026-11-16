//! Explosion request7b57d0, AABB/cone selection7b4380/6dc5d0,
//! safe radius7b34d0 and actual victim damage/impulse7b2690.
use crate::{
    assets::explosions::Declaration,
    gameplay::{damage::Parameters, entity_lifetime::EntityId},
};
use bevy_math::Vec3;
use std::sync::Arc;
#[derive(Clone, Debug)]
pub struct Request {
    pub declaration: Arc<Declaration>,
    pub inflictor: Option<EntityId>,
    pub attacker: Option<EntityId>,
    pub position: Vec3,
    pub direction: Vec3,
    pub cannot_incapacitate_player_and_allies: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct RadiusModifiers {
    pub source_radius: f32,
    pub player_owned_radius: f32,
}
impl Default for RadiusModifiers {
    fn default() -> Self {
        Self {
            source_radius: 1.,
            player_owned_radius: 1.,
        }
    }
}
impl Request {
    pub fn query_radius(&self, modifiers: RadiusModifiers) -> f32 {
        self.declaration.radius
            * self.declaration.player_radius_scale.max(1.)
            * modifiers.source_radius
            * modifiers.player_owned_radius
    }
    pub fn radii(&self, player: bool, modifiers: RadiusModifiers) -> (f32, f32) {
        let d = &self.declaration;
        let factor = modifiers.source_radius * modifiers.player_owned_radius;
        (
            d.radius * if player { d.player_radius_scale } else { 1. } * factor,
            d.full_radius
                * if player {
                    d.player_full_radius_scale
                } else {
                    1.
                }
                * factor,
        )
    }
    ///4660 queries distance to actual entity world bounds, not to the body
    ///center and not to a collision-box surrogate.
    pub fn distance(&self, bounds: [Vec3; 2]) -> f32 {
        self.position
            .distance(self.position.clamp(bounds[0], bounds[1]))
    }
    pub fn safe(&self, center: Vec3, distance: f32) -> bool {
        if self.declaration.angle < 360. {
            (center - (self.position + self.direction * self.declaration.safe_radius))
                .dot(self.direction)
                < 0.
        } else {
            distance < self.declaration.safe_radius
        }
    }
    pub fn in_cone(&self, bounds: [Vec3; 2], samples: &[Vec3]) -> bool {
        if self.declaration.angle >= 360. {
            return true;
        }
        let direction = self.direction
            / self
                .direction
                .length_squared()
                .max(f32::MIN_POSITIVE)
                .sqrt();
        let angle = self.declaration.angle * 0.5 * f32::from_bits(0x3c8efa35);
        if samples.len() < 2 {
            bounds_angle(self.position, direction, bounds) <= angle
        } else {
            samples.iter().any(|sample| {
                let v = *sample - self.position;
                let v = v / v.length_squared().max(f32::MIN_POSITIVE).sqrt();
                v.dot(direction).clamp(-1., 1.).acos() < angle
            })
        }
    }
    pub fn damage(
        &self,
        bounds: [Vec3; 2],
        center: Vec3,
        player: bool,
        modifiers: RadiusModifiers,
        force_full_player_owned: bool,
    ) -> Option<(Parameters, Vec3)> {
        let d = &self.declaration;
        let declaration = d.damage.as_ref()?;
        let distance = self.distance(bounds);
        let (radius, full) = self.radii(player, modifiers);
        if distance > radius || self.safe(center, distance) {
            return None;
        }
        let fraction = if distance <= full || full >= radius {
            1.
        } else {
            1. - (distance - full) / (radius - full)
        };
        let mut amount =
            declaration.minimum + (declaration.maximum - declaration.minimum) * fraction;
        if force_full_player_owned && amount > 0. {
            amount = declaration.maximum;
        }
        if player {
            amount *= d.player_damage_scale;
        }
        let offset = center - self.position;
        let length = offset.length_squared().max(f32::MIN_POSITIVE).sqrt();
        let direction = offset / length;
        let mut params = Parameters::from_declaration(declaration);
        params.set_amount(declaration, amount, false);
        params.hit_location = Some(center);
        params.hit_direction = direction;
        params.hit_distance = offset.length();
        params.hit_region = 1;
        params.affected_regions = 1;
        params.inflictor = self.inflictor;
        params.attacker = self.attacker;
        if self.cannot_incapacitate_player_and_allies {
            params.damage_type.0 |= 1u64 << 44;
        }
        //851940 applies this impulse to each actual target body. The Z
        //offset scales with damage fraction; the vector is not renormalized.
        let impulse = (direction + Vec3::Z * (d.impulse_z_offset * fraction)) * d.impulse;
        Some((params, impulse))
    }
}
///6dc5d0: line/AABB admission then twelve nearest edge points. This is the
///executable's angular-bound algorithm, not a center-only cone test.
fn bounds_angle(origin: Vec3, direction: Vec3, bounds: [Vec3; 2]) -> f32 {
    let mut near = f32::NEG_INFINITY;
    let mut far = f32::INFINITY;
    let mut intersects = true;
    for axis in 0..3 {
        if direction[axis] == 0. {
            if origin[axis] < bounds[0][axis] || origin[axis] > bounds[1][axis] {
                intersects = false;
                break;
            }
        } else {
            let a = (bounds[0][axis] - origin[axis]) / direction[axis];
            let b = (bounds[1][axis] - origin[axis]) / direction[axis];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                intersects = false;
                break;
            }
        }
    }
    if intersects {
        return if near < 0. && far < 0. { 360. } else { 0. };
    }
    let mut result = f32::MAX;
    for axis in 0..3 {
        let other = [(axis + 1) % 3, (axis + 2) % 3];
        for mask in 0..4 {
            let mut start = bounds[0];
            start[other[0]] = bounds[(mask & 1) as usize][other[0]];
            start[other[1]] = bounds[((mask >> 1) & 1) as usize][other[1]];
            let mut end = start;
            end[axis] = bounds[1][axis];
            let edge = end - start;
            let t = ((origin - start).dot(edge) / edge.length_squared()).clamp(0., 1.);
            let v = start + edge * t - origin;
            let v = v / v.length_squared().max(f32::MIN_POSITIVE).sqrt();
            result = result.min(v.dot(direction).clamp(-1., 1.).acos());
        }
    }
    result
}
