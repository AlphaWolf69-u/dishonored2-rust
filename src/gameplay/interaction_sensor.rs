//! Five camera-forward sphere casts from arkCpntPlayerSensorInteraction.
//! 1403b82c0 creates the spheres; 1403b8d60 starts each radius ahead of the
//! camera and casts to m_maxDistance. Collision resolution here uses the
//! host triangle index in place of Havok's broad phase.
use bevy_math::Vec3;
use crate::assets::entities::usable::Sensor;
use crate::gameplay::{collision_filter::QueryFilter, navigation::CollisionWorld, shape_cast};

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub fraction: f32,
    pub owner: u32,
    pub position: Vec3,
}

pub struct Scratch {
    /// 1403b8430 processes cast groups in this order, closest hits first.
    pub hits: [Vec<Hit>; 5],
    candidates: Vec<usize>,
    triangles: Vec<(u32, [Vec3; 3], u32)>,
}
impl Default for Scratch {
    fn default() -> Self { Self { hits: std::array::from_fn(|_| Vec::new()), candidates: Vec::new(), triangles: Vec::new() } }
}

#[derive(Default)]
pub struct OcclusionScratch {
    candidates: Vec<usize>,
    triangles: Vec<(u32, [Vec3; 3], u32)>,
}

/// Geometric equivalent of a sphere/triangle shape cast. The conservative
/// advancement reuses the player collision owner's point/triangle distance.
fn sphere_triangle(start: Vec3, displacement: Vec3, radius: f32, triangle: &[Vec3; 3]) -> Option<(f32, Vec3)> {
    let mut fraction = 0.0;
    for _ in 0..64 {
        let contact = shape_cast::closest(&[start + displacement * fraction], triangle);
        let separation = contact.distance - radius;
        if separation <= 0.001 { return Some((fraction, contact.position)); }
        let approach = -displacement.dot(contact.normal);
        if approach <= 0.0 { return None; }
        fraction += separation / approach;
        if fraction > 1.0 { return None; }
    }
    None
}

/// 1403b7b90 casts the chosen sensor sphere from the eye to the hit using
/// the occlusion filter. A hit on the selected owner is not an occluder.
#[derive(Clone, Copy)]
pub enum OcclusionHit { Target, Ignore, Block }

pub fn occluded(world: &CollisionWorld, eye: Vec3, hit: Vec3, radius: f32,
    classify: impl Fn(u32, u32) -> OcclusionHit, scratch: &mut OcclusionScratch) -> bool {
    let motion = hit - eye;
    let margin = Vec3::splat(radius);
    let filter = QueryFilter::layers(0xc43e, 4);
    let mut nearest: Option<(f32, OcclusionHit)> = None;
    world.triangles_in_reuse(eye.min(hit) - margin, eye.max(hit) + margin, filter,
        &mut scratch.candidates, &mut scratch.triangles);
    for &(key, triangle, owner) in &scratch.triangles {
        let kind = classify(key, owner);
        if matches!(kind, OcclusionHit::Ignore) { continue; }
        if let Some((fraction, _)) = sphere_triangle(eye, motion, radius, &triangle) {
            if nearest.is_none_or(|(best, _)| fraction < best) { nearest = Some((fraction, kind)); }
        }
    }
    nearest.is_some_and(|(_, kind)| matches!(kind, OcclusionHit::Block))
}

/// Coordinates are the collision world's Bevy-space metres. 1403b8d60 uses
/// layer masks 0x20cc38 for casts 0..2 and 0x408 for casts 3..4.
pub fn cast(world: &CollisionWorld, eye: Vec3, forward: Vec3, sensor: Sensor, scratch: &mut Scratch)
{
    let forward = forward.normalize_or_zero();
    for (i, hits) in scratch.hits.iter_mut().enumerate() {
        hits.clear();
        let radius = sensor.shape_radii[i];
        if radius <= 0.0 || radius > sensor.max_distance || forward == Vec3::ZERO { continue; }
        let start = eye + forward * radius;
        let displacement = forward * (sensor.max_distance - radius);
        let end = start + displacement;
        let margin = Vec3::splat(radius);
        let filter = QueryFilter::layers(if i < 3 { 0x20cc38 } else { 0x408 }, 0);
        world.triangles_in_reuse(start.min(end) - margin, start.max(end) + margin, filter,
            &mut scratch.candidates, &mut scratch.triangles);
        for &(_, triangle, owner) in &scratch.triangles {
            if let Some((fraction, position)) = sphere_triangle(start, displacement, radius, &triangle) {
                hits.push(Hit { fraction, owner, position });
            }
        }
        hits.sort_unstable_by(|a, b| a.fraction.total_cmp(&b.fraction));
    }
}
