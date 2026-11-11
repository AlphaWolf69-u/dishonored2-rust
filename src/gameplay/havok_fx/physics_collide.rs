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
    /// +0x8c / +0x84: bodies younger than this use the smaller maximum.
    pub young_age: f32,
    pub young_maximum: f32,
}

impl Default for CollideTuning {
    fn default() -> Self {
        Self {
            minimum_distance: 0.,
            maximum_distance: f32::from_bits(0x7f7f_ffee),
            radius_factor: 10.,
            displacement_factor: 10.,
            young_age: f32::from_bits(0x3c23_d70a),
            young_maximum: f32::from_bits(0x7f7f_ffee),
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
    /// The hit's material key (u32::MAX without a hit or material).
    pub key: u32,
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
        cache[index] = StaticCache {
            centre: [p[0], p[1], p[2]],
            radius: distance,
            plane: [0.; 4],
            hit: false,
            key: u32::MAX,
            flags: 0,
        };
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

/// RSQRTPS without refinement, as 140fc1370 uses it for speeds.
fn rsqrt_estimate(value: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        unsafe { _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(value))) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        1. / value.sqrt()
    }
}

/// RCPPS without refinement (the edge-line distances).
fn rcp_estimate(value: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rcp_ss, _mm_set_ss};
        unsafe { _mm_cvtss_f32(_mm_rcp_ss(_mm_set_ss(value))) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        1. / value
    }
}

/// (y + x) + z of a squared difference, the shuffle sums of 140fc1370.
fn squared_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d: [f32; 3] = std::array::from_fn(|i| subtract(a[i], b[i]));
    add(add(multiply(d[1], d[1]), multiply(d[0], d[0])), multiply(d[2], d[2]))
}

/// sqrt through RSQRTPS + Newton, masked to 0 at or below zero (14007ef60).
fn length(squared: f32) -> f32 {
    multiply(crate::gameplay::constraint_solver::inverse_length(squared), squared)
}

fn maxps(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

fn minps(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// The cross terms (z, x, y up to sign) of (v - p) x e in 140fc1370's order.
fn edge_terms(v: [f32; 3], e: [f32; 3], p: [f32; 3]) -> [f32; 3] {
    let d: [f32; 3] = std::array::from_fn(|i| subtract(v[i], p[i]));
    [
        subtract(multiply(d[1], e[0]), multiply(e[1], d[0])),
        subtract(multiply(d[2], e[1]), multiply(e[2], d[1])),
        subtract(multiply(d[0], e[2]), multiply(e[0], d[2])),
    ]
}

/// The squared distance from `p` to the closest edge line of a face
/// (4446-4640: three lanes for a triangle, four for a quad), with RCPPS.
fn closest_edge_line(v: &[[f32; 3]], p: [f32; 3]) -> f32 {
    let n = v.len();
    let lanes: Vec<f32> = (0..n)
        .map(|k| {
            let e: [f32; 3] = std::array::from_fn(|i| subtract(v[(k + 1) % n][i], v[k][i]));
            let [z, x, y] = edge_terms(v[k], e, p);
            let square = |a: f32| multiply(a, a);
            let numerator = add(add(square(y), square(x)), square(z));
            // A triangle sums |e|^2 as y, x, z; a quad as z, y, x.
            let denominator = if n == 3 {
                add(add(square(e[1]), square(e[0])), square(e[2]))
            } else {
                add(add(square(e[2]), square(e[1])), square(e[0]))
            };
            if denominator == 0. { 0. } else { multiply(rcp_estimate(denominator), numerator) }
        })
        .collect();
    if n == 3 {
        minps(lanes[2], minps(lanes[1], lanes[0]))
    } else {
        // Lane 0 of minps(m, (m1, m0, m3, m2)), m = minps((l2, l3, l0, l1), l).
        minps(minps(lanes[2], lanes[0]), minps(lanes[3], lanes[1]))
    }
}

/// The step inputs of collide mode 1.
pub struct CachedStep<'a> {
    pub dt: f32,
    pub tuning: CollideTuning,
    /// Pipeline +0x30 and +0x34 radii.
    pub radii: [f32; 2],
    pub own: [f32; 2],
    pub plane: [f32; 4],
    /// The collide mode (+0x28): 1 or 2.
    pub mode: i32,
    pub world: &'a CollideWorld<'a>,
}

/// 140fc1370's CD pass, static part, collide modes 1 and 2 (accuracy
/// LOW/MEDIUM/HIGH, 3681-5224) for sphere bodies on a static-only world:
/// cache reuse, the query distance from the cached sphere, the collector's
/// second-best hit as the safe radius limited by the face's edge lines,
/// and the contact from the cache. Cache sharing, the moved-body set and
/// static cache limiting only act through moving world bodies, which the
/// rewrite's collision map does not have (see rendering-gaps.md).
pub fn static_cached(
    step: &CachedStep<'_>,
    bodies: &[i32],
    linear: &[[[f32; 4]; 2]],
    states: &[super::solve_contacts::BodyState],
    collide: &mut WorldCollide,
    cache: &mut [StaticCache],
    scratch: &mut CollideScratch,
) -> Result<(), String> {
    let CachedStep { dt, tuning, radii, own, plane, mode, world } = *step;
    let filter = QueryFilter::World(world.filter);
    let epsilon = f32::from_bits(0x3400_0000);
    let far = f32::from_bits(0x7f7f_ffee);
    let mut triangles = Vec::new();
    for &body in bodies {
        let index = body as usize;
        let state = states[index].state;
        let entry = cache[index];
        let shift = |cache: &mut [StaticCache]| {
            let flags = cache[index].flags;
            cache[index].flags = (flags & !3) | ((flags >> 1) & 1);
        };
        // "Early out": asleep on a still surface with a valid cache.
        if state == 0x80 && entry.flags & 1 == 0 {
            shift(cache);
            continue;
        }
        let [p, v] = linear[index];
        let scale = v[3];
        let r30 = multiply(scale, radii[0]);
        let rq = multiply(scale, radii[1]);
        let position = [p[0], p[1], p[2]];
        let ahead: [f32; 3] = std::array::from_fn(|i| add(multiply(v[i], dt), p[i]));
        // Whether the predicted position must stay inside the sphere too.
        let speed_squared = add(add(multiply(v[1], v[1]), multiply(v[0], v[0])), multiply(v[2], v[2]));
        let speed = if speed_squared <= 0. { 0. } else { multiply(rsqrt_estimate(speed_squared), speed_squared) };
        let travel = add(multiply(multiply(speed, dt), 0.5), add(rq, r30));
        let roomy = entry.radius > travel;
        let fresh = !entry.hit && state == 0;
        let need = mode != 2 && (roomy || fresh);
        // Cache reuse (140fcd1a6..140fcd22f): the contacts stay as written.
        if entry.flags & 1 == 0 && entry.radius > 0. {
            let m = maxps(epsilon, subtract(entry.radius, r30));
            let m = multiply(m, m);
            if squared_distance(entry.centre, position) < m && (!need || squared_distance(entry.centre, ahead) < m) {
                shift(cache);
                continue;
            }
        }
        // The query distance (140fcd235..140fcd3bd).
        let displacement = length(squared_distance(ahead, position));
        let mut maximum = tuning.maximum_distance;
        if p[3] < tuning.young_age {
            maximum = minps(tuning.young_maximum, maximum);
        }
        let mut base = add(multiply(rq, tuning.radius_factor), multiply(displacement, tuning.displacement_factor));
        if entry.hit {
            let reach = add(length(squared_distance(position, entry.centre)), entry.radius.abs());
            base = minps(base, maxps(multiply(rq, 0.125), reach));
        }
        let distance = maxps(tuning.minimum_distance, minps(add(base, rq), maximum));
        let reach = subtract(distance, subtract(rq, r30));
        let (was_hit, was_key) = (entry.hit, entry.key);
        cache[index] = StaticCache {
            centre: position,
            radius: reach,
            plane: [0.; 4],
            hit: false,
            key: u32::MAX,
            flags: entry.flags & 3,
        };
        // hkfxClosestHitCollector (140fe31e0) over the leaf primitives: the
        // nearest centre distance and the second nearest, under the query
        // distance; a quad's two map triangles are one leaf.
        let centre = bevy::math::Vec3::new(p[0], p[1], p[2]);
        let view_centre = bevy::math::Vec3::new(p[0], p[2], -p[1]);
        let span = bevy::math::Vec3::splat(distance);
        world.map.triangles_in(view_centre - span, view_centre + span, filter, world.owners, &mut scratch.found, &mut triangles);
        let mut primitives: Vec<(u32, f32, bevy::math::Vec3, u32)> = Vec::new();
        for (key, vertices, _) in &triangles {
            let (point, _) = crate::gameplay::shape_cast::closest_on_triangle(*vertices, view_centre);
            let point = bevy::math::Vec3::new(point.x, -point.z, point.y);
            let gap = (centre - point).length();
            let (group, ..) = world.map.primitive(*key);
            match primitives.iter_mut().find(|found| found.0 == group) {
                Some(found) => {
                    if gap < found.1 {
                        *found = (group, gap, point, *key);
                    }
                }
                None => primitives.push((group, gap, point, *key)),
            }
        }
        let mut best = None::<(f32, bevy::math::Vec3, u32)>;
        let mut second = far;
        for &(_, gap, point, key) in &primitives {
            if !(gap < minps(distance, second)) {
                continue;
            }
            match best {
                Some((nearest, ..)) if nearest <= gap => second = gap,
                Some((nearest, ..)) => {
                    second = nearest;
                    best = Some((gap, point, key));
                }
                None => best = Some((gap, point, key)),
            }
        }
        if let Some((gap_centre, point, key)) = best.filter(|(gap, ..)| *gap > 0.) {
            let normal = (centre - point) / gap_centre;
            let w = subtract(
                0.,
                add(add(multiply(normal.y, point.y), multiply(normal.x, point.x)), multiply(normal.z, point.z)),
            );
            let gap = add(add(add(w, multiply(normal.y, p[1])), multiply(normal.z, p[2])), multiply(normal.x, p[0]));
            // The leaf's corners in game axes; a face contact limits the
            // radius to the edge lines (4446-4640).
            let (_, corners, quad) = world.map.primitive(key);
            let corners: Vec<[f32; 3]> =
                corners[..if quad { 4 } else { 3 }].iter().map(|c| [c.x, -c.z, c.y]).collect();
            let e0: [f32; 3] = std::array::from_fn(|i| subtract(corners[1][i], corners[0][i]));
            let e1: [f32; 3] = std::array::from_fn(|i| subtract(corners[2][i], corners[1][i]));
            let fz = subtract(multiply(e1[1], e0[0]), multiply(e0[1], e1[0]));
            let fx = subtract(multiply(e1[2], e0[1]), multiply(e0[2], e1[1]));
            let fy = subtract(multiply(e1[0], e0[2]), multiply(e0[0], e1[2]));
            let face = add(add(multiply(fy, fy), multiply(fx, fx)), multiply(fz, fz));
            let along = add(add(multiply(normal.y, fy), multiply(normal.x, fx)), multiply(normal.z, fz));
            let mut radius = gap;
            if subtract(face, multiply(along, along)) <= multiply(face, f32::from_bits(0x38d1_b717)) {
                radius = second;
                let edge = closest_edge_line(&corners, position);
                // ucomiss / jae: an unordered comparison takes the limit.
                if !(edge >= multiply(second, second)) {
                    radius = maxps(gap, length(edge));
                }
            }
            radius = if 0. <= gap && epsilon <= radius && gap <= reach { minps(radius, reach) } else { gap };
            cache[index].plane = [normal.x, normal.y, normal.z, w];
            cache[index].hit = true;
            cache[index].key = world.map.material_index(key);
            cache[index].radius = radius;
            // A static hit body is never shared (140fce44b).
            cache[index].flags |= 0x20;
        }
        // The contact from the cache (140fce77a..140fcea5c).
        let changed = was_hit != cache[index].hit || was_key != cache[index].key;
        collide.static_contacts[index] = Contact::default();
        if changed {
            collide.static_materials[index] = NO_MATERIAL;
        }
        if cache[index].hit {
            if changed {
                let material = world.map.material(cache[index].key);
                collide.static_materials[index] =
                    contact_material(material.map(SurfaceMaterial::from_physics), own, 3 <= state)?;
            }
            let c = cache[index].plane;
            let dot = add(add(multiply(c[1], plane[1]), multiply(c[0], plane[0])), multiply(c[2], plane[2]));
            collide.static_contacts[index].plane = std::array::from_fn(|i| subtract(c[i], multiply(dot, plane[i])));
        }
        shift(cache);
    }
    Ok(())
}
