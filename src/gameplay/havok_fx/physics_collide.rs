//! hkfxEffects::PhysicsCollide (constructor 140fbea70, base WorldCollide):
//! fills WorldCollide's static and dynamic contacts from the physics world.
//! Two stages: the CD pass (stage 0, 140fc1370 with param 0) and the CCD
//! pass (stage 1).
//!
//! bevy_backend: the original queries the hknp world through a private copy
//! of its query dispatcher whose compressed-mesh closest-point function is
//! hkfxCompressedMeshClosest (140fe3360, "unsignedDistanceWithNmp" for a
//! point query). The rewrite's level collision is the decoded triangle map
//! (`navigation::CollisionMap`); sphere bodies take the closest point of its
//! triangles inside the query box, as that function does over the mesh tree.
use super::world_collide::{contact_material, Contact, SurfaceMaterial, WorldCollide, NO_MATERIAL};
use crate::float::{add, multiply, subtract};
use crate::gameplay::{collision_filter::QueryFilter, navigation::CollisionMap};

/// 140fbea10's tuning block at +0x7c.
#[derive(Clone, Copy, Debug)]
pub struct CollideTuning {
    /// +0x7c / +0x80: the query distance's clamp.
    pub minimum_distance: f32,
    pub maximum_distance: f32,
    /// +0x98 radius and +0x9c displacement factors of the query distance.
    pub radius_factor: f32,
    pub displacement_factor: f32,
}

impl Default for CollideTuning {
    fn default() -> Self {
        Self {
            minimum_distance: 0.,
            maximum_distance: f32::from_bits(0x7f7f_ffee),
            radius_factor: 10.,
            displacement_factor: 10.,
        }
    }
}

/// The level the bodies collide with: the map, its active owners and the
/// body filter word (14081a5d0 of the descriptor layer, masked 0xf7dfffff).
pub struct CollideWorld<'a> {
    pub map: &'a CollisionMap,
    pub owners: &'a [bool],
    pub filter: u32,
}

/// Scratch buffers kept between steps.
#[derive(Default)]
pub struct CollideScratch {
    found: Vec<usize>,
}

/// 140fc1370's CD pass, static part, collide accuracy GROUND_TRUTH (mode 4,
/// 3383-3680) for sphere bodies. `radius` is the pipeline radius (+0x34),
/// `own` the solver's (friction, restitution), `resting` whether each body's
/// solver state is at least 3, `plane` the pipeline's plane lock (+0x90).
#[allow(clippy::too_many_arguments)]
pub fn static_ground_truth(
    dt: f32,
    tuning: CollideTuning,
    radius: f32,
    own: [f32; 2],
    plane: [f32; 4],
    world: &CollideWorld<'_>,
    bodies: &[i32],
    linear: &[[[f32; 4]; 2]],
    resting: &dyn Fn(i32) -> bool,
    collide: &mut WorldCollide,
    scratch: &mut CollideScratch,
) -> Result<(), String> {
    let filter = QueryFilter::World(world.filter);
    let mut triangles = Vec::new();
    for &body in bodies {
        let index = body as usize;
        let [p, v] = linear[index];
        let body_radius = multiply(v[3], radius);
        // |dt v|, through RSQRTPS like the original.
        let step: [f32; 3] = std::array::from_fn(|i| subtract(add(multiply(dt, v[i]), p[i]), p[i]));
        let squared = add(add(multiply(step[1], step[1]), multiply(step[0], step[0])), multiply(step[2], step[2]));
        let displacement = multiply(crate::gameplay::constraint_solver::inverse_length(squared), squared);
        let distance = add(
            add(multiply(displacement, tuning.displacement_factor), multiply(body_radius, tuning.radius_factor)),
            body_radius,
        );
        let distance = if distance < tuning.maximum_distance { distance } else { tuning.maximum_distance };
        let distance = if tuning.minimum_distance > distance { tuning.minimum_distance } else { distance };
        let early_out = add(distance, body_radius);
        let centre = bevy::math::Vec3::new(p[0], p[1], p[2]);
        let reach = bevy::math::Vec3::splat(distance);
        world.map.triangles_in(centre - reach, centre + reach, filter, world.owners, &mut scratch.found, &mut triangles);
        // hkfxClosestHitCollector: the nearest hit under the early-out.
        let mut best: Option<(f32, bevy::math::Vec3, Option<&crate::assets::entities::physics_material::PhysicsMaterial>)> = None;
        for (_, vertices, material) in &triangles {
            let (point, _) = crate::gameplay::shape_cast::closest_on_triangle(*vertices, centre);
            let gap = (centre - point).length();
            let limit = best.map_or(early_out, |(gap, ..)| gap);
            if gap < limit {
                best = Some((gap, point, *material));
            }
        }
        collide.static_contacts[index] = Contact::default();
        collide.static_materials[index] = NO_MATERIAL;
        let Some((gap, point, material)) = best else { continue };
        if !(gap > 0.) {
            continue;
        }
        let normal = (centre - point) / gap;
        let mut contact_plane = [normal.x, normal.y, normal.z, 0.];
        contact_plane[3] = subtract(
            0.,
            add(add(multiply(normal.x, point.x), multiply(normal.y, point.y)), multiply(normal.z, point.z)),
        );
        // Project the plane off the pipeline's plane lock.
        let along = add(add(multiply(contact_plane[1], plane[1]), multiply(contact_plane[0], plane[0])), multiply(contact_plane[2], plane[2]));
        let contact_plane: [f32; 4] = std::array::from_fn(|i| subtract(contact_plane[i], multiply(along, plane[i])));
        collide.static_contacts[index].plane = contact_plane;
        collide.static_materials[index] =
            contact_material(material.map(SurfaceMaterial::from_physics), own, resting(body))?;
    }
    Ok(())
}
