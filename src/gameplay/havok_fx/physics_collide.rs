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

/// The level the bodies collide with (game axes in, the map's view axes
/// inside): the map, its active owners and the
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

/// The per-body static cache (+0x128: entries +0x00, spheres +0x10, planes
/// +0x20): the query centre and distance of the last CD pass, the plane of
/// its hit, and whether it hit.
#[derive(Clone, Copy, Default, Debug)]
pub struct StaticCache {
    pub centre: [f32; 3],
    pub radius: f32,
    pub plane: [f32; 4],
    pub hit: bool,
    pub flags: u32,
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
    cache: &mut [StaticCache],
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
        // The map is in view axes (x, z, -y); distances are unchanged.
        let centre = bevy::math::Vec3::new(p[0], p[1], p[2]);
        let view_centre = bevy::math::Vec3::new(p[0], p[2], -p[1]);
        let reach = bevy::math::Vec3::splat(distance);
        world.map.triangles_in(view_centre - reach, view_centre + reach, filter, world.owners, &mut scratch.found, &mut triangles);
        // hkfxClosestHitCollector: the nearest hit under the early-out.
        let mut best: Option<(f32, bevy::math::Vec3, Option<&crate::assets::entities::physics_material::PhysicsMaterial>)> = None;
        for (_, vertices, material) in &triangles {
            let (point, _) = crate::gameplay::shape_cast::closest_on_triangle(*vertices, view_centre);
            let point = bevy::math::Vec3::new(point.x, -point.z, point.y);
            let gap = (centre - point).length();
            let limit = best.map_or(early_out, |(gap, ..)| gap);
            if gap < limit {
                best = Some((gap, point, *material));
            }
        }
        // 3530: the entry is reset to the query sphere before the query.
        cache[index] = StaticCache { centre: [p[0], p[1], p[2]], radius: distance, plane: [0.; 4], hit: false, flags: 0 };
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
        cache[index].plane = [normal.x, normal.y, normal.z, subtract(
            0.,
            add(add(multiply(normal.x, point.x), multiply(normal.y, point.y)), multiply(normal.z, point.z)),
        )];
        cache[index].hit = true;
        collide.static_contacts[index].plane = contact_plane;
        collide.static_materials[index] =
            contact_material(material.map(SurfaceMaterial::from_physics), own, resting(body))?;
    }
    Ok(())
}

/// 140fc1370's CCD pass (stage 1, +0x72 mode 1, 9537-9778), static part:
/// a body that integration moved beyond the sphere its last query covered
/// is put back on that sphere. Bodies asleep on a still surface (solver
/// state 0x80) are left alone. `radius` is the pipeline radius (+0x30).
pub fn static_ccd(
    radius: f32,
    bodies: &[i32],
    solver_state: &dyn Fn(i32) -> i32,
    cache: &mut [StaticCache],
    linear: &mut [[[f32; 4]; 2]],
) {
    use crate::gameplay::constraint_solver::inverse_length;
    for &body in bodies {
        let index = body as usize;
        if solver_state(body) == 0x80 {
            continue;
        }
        let entry = &mut cache[index];
        let r = entry.radius;
        if !(0. <= r) {
            continue;
        }
        let body_radius = multiply(radius, linear[index][1][3]);
        let [n0, n1, n2, w] = entry.plane;
        let [c0, c1, c2] = entry.centre;
        let distance = add(add(w, multiply(n1, c1)), add(multiply(n2, c2), multiply(n0, c0)));
        let mut allowed = r;
        if 0. < r {
            allowed = if !entry.hit || r < distance {
                let shrunk = subtract(r, multiply(body_radius, 0.5));
                if shrunk > 0. { shrunk } else { 0. }
            } else {
                let floor = if f32::from_bits(0x3400_0000) > r { f32::from_bits(0x3400_0000) } else { r };
                let scaled = multiply(distance, f32::from_bits(0x3f33_3333));
                if scaled > floor { scaled } else { floor }
            };
        }
        let p = linear[index][0];
        let offset = [subtract(p[0], c0), subtract(p[1], c1), subtract(p[2], c2)];
        let squared = add(add(multiply(offset[1], offset[1]), multiply(offset[0], offset[0])), multiply(offset[2], offset[2]));
        if multiply(allowed, allowed) < squared {
            let inverse = inverse_length(squared);
            let offset = if allowed < multiply(inverse, squared) {
                offset.map(|lane| multiply(multiply(inverse, allowed), lane))
            } else {
                offset
            };
            linear[index][0] = [add(offset[0], c0), add(offset[1], c1), add(offset[2], c2), p[3]];
            entry.flags |= 1;
        }
    }
}
