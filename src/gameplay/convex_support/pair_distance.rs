//! Feature-pair GSK distance (141212780), with d10960's cold settings.
//! Vertices remain separate query/target features; penetration uses c0ed40.
use super::{
    distance::{ConvexClosestPoint, ConvexDistanceSolution, finish_distance},
    feature_query::{ConvexFeatures, query_penetration},
    penetration::PenetrationSupport,
    seed::{cross, dot, inverse_length, subtract},
    vertices::support_pair,
};
use crate::gameplay::{
    constraint_solver::reciprocal,
    shape_transform::{ShapeMatrix, inverse_rotate_vector},
};
#[path = "point_distance/simplex.rs"]
mod point_simplex;

/// Four vertex IDs followed by query-count/status/target-count, as in hkGskCache.
#[derive(Clone, Copy, Debug)]
pub struct PairCache(pub [u8; 5]);
impl Default for PairCache {
    fn default() -> Self {
        Self([0, 0, 0, 0, 0x41])
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PairSettings {
    pub distance_limit: f32,
    pub epsilon_factor: f32,
    pub penetration: bool,
}
impl Default for PairSettings {
    fn default() -> Self {
        Self {
            distance_limit: f32::from_bits(0x7f7f_ffee),
            epsilon_factor: f32::from_bits(0x3fb3_3333),
            penetration: true,
        }
    }
}

pub fn query_pair(
    query: &[[f32; 4]],
    target: &[[f32; 4]],
    relative: ShapeMatrix,
    query_radius: f32,
    target_radius: f32,
    threshold: f32,
) -> Result<Option<ConvexClosestPoint>, String> {
    let mut output = ConvexDistanceSolution {
        status: 0,
        distance: [0.; 4],
        normal: [0.; 4],
        witness: [0.; 4],
    };
    solve_pair_distance(
        query,
        target,
        relative,
        &mut PairCache::default(),
        &mut output,
    )?;
    Ok(finish_distance(
        output,
        query_radius,
        target_radius,
        relative,
        threshold,
    ))
}

pub fn solve_pair_distance(
    query: &[[f32; 4]],
    target: &[[f32; 4]],
    relative: ShapeMatrix,
    cache: &mut PairCache,
    output: &mut ConvexDistanceSolution,
) -> Result<(), String> {
    solve_pair_distance_with_settings(
        query,
        target,
        relative,
        cache,
        PairSettings::default(),
        output,
    )
}

/// The extended transform's +40/+50/+60 settings in 141212780.
/// Status 5/6 preserve the source's partially written output packet.
pub fn solve_pair_distance_with_settings(
    query: &[[f32; 4]],
    target: &[[f32; 4]],
    relative: ShapeMatrix,
    cache: &mut PairCache,
    settings: PairSettings,
    output: &mut ConvexDistanceSolution,
) -> Result<(), String> {
    let qc = (cache.0[4] >> 6) as usize;
    let tc = (cache.0[4] & 3) as usize;
    if qc == 0 || tc == 0 || qc + tc > 4 {
        return Err("invalid GSK feature cache counts".into());
    }
    let mut features = ConvexFeatures {
        query: [[0.; 4]; 4],
        target: [[0.; 4]; 4],
        query_count: qc,
        target_count: tc,
    };
    for i in 0..qc {
        features.query[i] = *query
            .get(cache.0[i] as usize)
            .ok_or("GSK query cache vertex out of range")?;
    }
    for i in 0..tc {
        let vertex = *target
            .get(cache.0[qc + i] as usize)
            .ok_or("GSK target cache vertex out of range")?;
        let mut mapped = inverse_rotate_vector(relative, subtract(vertex, relative[3]));
        mapped[3] = vertex[3];
        features.target[i] = mapped;
    }
    let support = PenetrationSupport {
        query,
        target,
        relative,
    };
    let mut state = FeatureState {
        previous_target_count: tc,
        normal: [0.; 4],
        barycentric: [0.; 3],
    };
    let mut epsilon = f32::from_bits(0x3727_c5ac);
    loop {
        epsilon *= settings.epsilon_factor;
        let reduced = reduce(&mut features, &mut state)?;
        let squared = dot(reduced.direction, reduced.direction);
        let factor = inverse_length(squared);
        let normal = reduced.direction.map(|v| v * factor);
        let separation = dot(
            subtract(features.query[0], features.target[0]),
            reduced.direction,
        ) * factor;
        if reduced.force
            || separation < epsilon
            || squared < ((epsilon * epsilon) * epsilon) * epsilon
        {
            if settings.penetration {
                *output = query_penetration(&mut features, &support)?;
            } else {
                if features.query_count + features.target_count > 4 {
                    if features.target_count < features.query_count {
                        features.query_count = 3;
                        features.target_count = 1;
                    } else {
                        features.query_count = 1;
                        features.target_count = 3;
                    }
                }
                output.status = 6;
            }
            save_cache(cache, &features, output.status)?;
            return Ok(());
        }
        state.normal = normal;
        state.previous_target_count = features.target_count;
        let pair = support_pair(query, target, normal.map(|v| -v), relative)?;
        let next_query = pair.query;
        let mut next_target = pair.target_in_query;
        next_target[3] = pair.target[3];
        let distance = dot(subtract(next_query, next_target), normal);
        if settings.distance_limit < distance {
            output.status = 5;
            output.distance = [distance; 4];
            output.normal = normal;
            save_cache(cache, &features, 5)?;
            return Ok(());
        }
        let query_improvement = dot(subtract(features.query[0], next_query), normal);
        let target_improvement = dot(subtract(next_target, features.target[0]), normal);
        let improve_query = epsilon < query_improvement;
        let improve_target = epsilon < target_improvement;
        // Source picks one side before duplicate checking; ties select query.
        if improve_query && (!improve_target || !(query_improvement < target_improvement)) {
            if !(1..features.query_count)
                .any(|i| dot(subtract(features.query[i], next_query), normal) < epsilon)
            {
                features.query[features.query_count] = next_query;
                features.query_count += 1;
                continue;
            }
        } else if improve_target {
            if !(1..features.target_count)
                .any(|i| dot(subtract(next_target, features.target[i]), normal) < epsilon)
            {
                features.target[features.target_count] = next_target;
                features.target_count += 1;
                continue;
            }
        }
        let distance = dot(subtract(features.query[0], features.target[0]), normal);
        output.status = 0;
        output.distance = [distance; 4];
        output.normal = normal;
        output.witness = if features.query_count == 1 {
            features.query[0]
        } else if features.target_count == 1 {
            std::array::from_fn(|i| features.target[0][i] + distance * normal[i])
        } else {
            reduced.witness
        };
        save_cache(cache, &features, 0)?;
        return Ok(());
    }
}

fn save_cache(cache: &mut PairCache, f: &ConvexFeatures, status: i32) -> Result<(), String> {
    if f.query_count + f.target_count > 4 || f.query_count > 3 || f.target_count > 3 {
        return Err("GSK result did not compact its features".into());
    }
    for i in 0..f.query_count {
        cache.0[i] = f.query[i][3].to_bits() as u8;
    }
    for i in 0..f.target_count {
        cache.0[f.query_count + i] = f.target[i][3].to_bits() as u8;
    }
    cache.0[4] = ((f.query_count as u8) << 6) | (((status as u8) & 15) << 2) | f.target_count as u8;
    Ok(())
}

struct FeatureState {
    previous_target_count: usize,
    normal: [f32; 4],
    barycentric: [f32; 3],
}
struct Reduction {
    direction: [f32; 4],
    witness: [f32; 4],
    force: bool,
}
impl Reduction {
    fn direction(direction: [f32; 4]) -> Self {
        Self {
            direction,
            witness: [0.; 4],
            force: false,
        }
    }
    fn penetration() -> Self {
        Self {
            direction: [0.; 4],
            witness: [0.; 4],
            force: true,
        }
    }
}

fn reduce(f: &mut ConvexFeatures, state: &mut FeatureState) -> Result<Reduction, String> {
    loop {
        if f.query_count == 1 {
            let (direction, force) =
                point_simplex::reduce(f.query[0], &mut f.target, &mut f.target_count)?;
            if f.target_count == 3 {
                state.barycentric = triangle_coordinates(f.query[0], &f.target);
            }
            return Ok(Reduction {
                direction,
                force,
                witness: f.query[0],
            });
        }
        if f.target_count == 1 {
            let (direction, force) =
                point_simplex::reduce_query(f.target[0], &mut f.query, &mut f.query_count)?;
            if f.query_count == 3 {
                state.barycentric = triangle_coordinates(f.target[0], &f.query);
            }
            return Ok(Reduction {
                direction,
                force,
                witness: [0.; 4],
            });
        }
        match (f.query_count, f.target_count) {
            (2, 2) => {
                if let Some(result) = segments(f) {
                    return Ok(result);
                }
            }
            (2, 3) | (3, 2) => {
                if let Some(result) = segment_triangle(f, state) {
                    return Ok(result);
                }
            }
            _ => return Ok(Reduction::penetration()),
        }
    }
}

fn sign_direction(direction: [f32; 4], projection: f32) -> [f32; 4] {
    direction.map(|v| f32::from_bits(v.to_bits() ^ (projection.to_bits() & 0x8000_0000)))
}
fn lerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|i| (b[i] - a[i]) * t + a[i])
}
fn min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}
fn max(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}
fn clamp_unit(a: f32) -> f32 {
    max(0., min(a, 1.))
}

/// 141213505: interior tests first, then the source's clamped edge parameters.
fn segments(f: &mut ConvexFeatures) -> Option<Reduction> {
    let [q0, q1, _, _] = f.query;
    let [t0, t1, _, _] = f.target;
    let a = subtract(q1, q0);
    let b = subtract(t1, t0);
    let offset = subtract(q0, t0);
    let n = cross(a, b);
    let an = cross(n, b);
    let bn = cross(n, a);
    let h = [
        dot(offset, an),
        dot(subtract(t0, q1), an),
        dot(offset, bn),
        dot(subtract(t1, q0), bn),
    ];
    let mask = h
        .iter()
        .enumerate()
        .fold(0, |mask, (i, &v)| mask | ((0. < v) as usize) << i);
    match mask {
        15 => {
            return Some(Reduction {
                direction: sign_direction(n, dot(offset, n)),
                witness: lerp(q0, q1, reciprocal(h[0] + h[1]) * h[0]),
                force: false,
            });
        }
        7 => {
            f.target[0] = t1;
            f.target_count = 1;
            return None;
        }
        11 => {
            f.target_count = 1;
            return None;
        }
        13 => {
            f.query[0] = q1;
            f.query_count = 1;
            return None;
        }
        14 => {
            f.query_count = 1;
            return None;
        }
        _ => {}
    }
    let (s, t) = edge_parameters(q0, q1, t0, t1, true);
    if s == 0. || s == 1. || t == 0. || t == 1. {
        if s == 1. {
            f.query[0] = q1;
        }
        if t == 1. {
            f.target[0] = t1;
        }
        if s == 0. || s == 1. {
            f.query_count = 1;
        }
        if t == 0. || t == 1. {
            f.target_count = 1;
        }
        return None;
    }
    Some(Reduction {
        direction: sign_direction(n, dot(offset, n)),
        witness: lerp(q0, q1, s),
        force: false,
    })
}

/// Same arithmetic/order is shared by 141212d66's two incident-edge tests.
fn edge_parameters(
    a0: [f32; 4],
    a1: [f32; 4],
    b0: [f32; 4],
    b1: [f32; 4],
    segment_pair: bool,
) -> (f32, f32) {
    let a = subtract(a1, a0);
    let b = subtract(b1, b0);
    let offset = subtract(b0, a0);
    let aa = dot(a, a);
    let bb = dot(b, b);
    let ab = dot(a, b);
    let aq = dot(offset, a);
    let bt = dot(offset, b);
    let floor = f32::from_bits(0x3400_0000);
    let den = max(aa * bb - ab * ab, floor);
    let initial = if den <= floor {
        1.
    } else {
        max(0., min(bb * aq - bt * ab, den)) * reciprocal(den)
    };
    let t = clamp_unit(if segment_pair {
        initial * reciprocal(bb) * ab - reciprocal(bb) * bt
    } else {
        initial * ab * reciprocal(bb) - bt * reciprocal(bb)
    });
    let s = clamp_unit(ab * reciprocal(aa) * t + aq * reciprocal(aa));
    (s, t)
}

fn triangle_normal(v: &[[f32; 4]; 4]) -> [f32; 4] {
    cross(subtract(v[2], v[1]), subtract(v[0], v[2]))
}
fn triangle_coordinates(point: [f32; 4], v: &[[f32; 4]; 4]) -> [f32; 3] {
    let n = triangle_normal(v);
    [
        dot(cross(subtract(point, v[1]), subtract(v[2], v[1])), n),
        dot(cross(subtract(point, v[2]), subtract(v[0], v[2])), n),
        dot(cross(subtract(point, v[0]), subtract(v[1], v[0])), n),
    ]
}
fn negative_mask(v: [f32; 3]) -> usize {
    v.iter()
        .enumerate()
        .fold(0, |mask, (i, &v)| mask | ((v < 0.) as usize) << i)
}

/// The plane-crossing test uses offset triple products, rather than the
/// endpoint triangle-coordinate expression (141212c0d/1412135e6).
fn crosses_triangle(point: [f32; 4], triangle: &[[f32; 4]; 4]) -> bool {
    let n = cross(
        subtract(triangle[1], triangle[0]),
        subtract(triangle[2], triangle[0]),
    );
    let a = subtract(triangle[0], point);
    let b = subtract(triangle[1], point);
    let c = subtract(triangle[2], point);
    let project = |u: [f32; 4], v: [f32; 4]| {
        ((v[1] * n[2] - v[2] * n[1]) * u[0] + (v[2] * n[0] - v[0] * n[2]) * u[1])
            + (v[0] * n[1] - v[1] * n[0]) * u[2]
    };
    0. < project(b, c) && 0. < project(c, a) && 0. < project(a, b)
}

/// 141212a0f/141212a19. The last feature added determines which of the
/// two source paths applies: a new triangle vertex, or a new segment endpoint.
fn segment_triangle(f: &mut ConvexFeatures, state: &mut FeatureState) -> Option<Reduction> {
    let query_is_segment = f.query_count == 2;
    let (segment, triangle) = if query_is_segment {
        (&mut f.query, &mut f.target)
    } else {
        (&mut f.target, &mut f.query)
    };
    let mut n;
    let reduce_segment;
    if state.previous_target_count == 2 {
        n = triangle_normal(triangle);
        let plane_normal = cross(
            subtract(triangle[1], triangle[2]),
            subtract(triangle[0], triangle[2]),
        );
        let p0 = dot(subtract(segment[0], triangle[0]), plane_normal);
        let p1 = dot(subtract(segment[1], triangle[0]), plane_normal);
        if 0. <= p0 * p1 {
            let selected = if p0 * p0 < p1 * p1 {
                segment[0]
            } else {
                segment[1]
            };
            let mut coordinates = triangle_coordinates(selected, triangle);
            if negative_mask(coordinates) == 7 {
                let offset = if query_is_segment {
                    subtract(segment[0], triangle[0])
                } else {
                    subtract(triangle[0], segment[0])
                };
                if dot(offset, n) < 0. {
                    n = n.map(|v| -v);
                    triangle.swap(0, 1);
                    coordinates.swap(0, 1);
                }
                state.barycentric = coordinates;
                segment[0] = selected;
                reduce_segment = true;
            } else {
                incident_edge(segment, triangle);
                reduce_segment = false;
            }
        } else {
            let point = lerp(segment[0], segment[1], reciprocal(p0 - p1) * p0);
            if crosses_triangle(point, triangle) {
                return Some(Reduction::penetration());
            }
            incident_edge(segment, triangle);
            reduce_segment = false;
        }
    } else {
        n = state.normal;
        let p0 = dot(subtract(segment[0], triangle[0]), n);
        let p1 = dot(subtract(segment[1], triangle[0]), n);
        if p1 * p0 < 0. {
            let point = lerp(segment[0], segment[1], reciprocal(p0 - p1) * p0);
            if crosses_triangle(point, triangle) {
                return Some(Reduction::penetration());
            }
        }
        let next = triangle_coordinates(segment[1], triangle);
        let previous = state.barycentric;
        let mask = negative_mask(next);
        state.barycentric = next;
        if mask == 7 {
            segment[0] = segment[1];
            reduce_segment = true;
        } else {
            let edge = match mask {
                6 => 0,
                5 => 1,
                3 => 2,
                2 => {
                    if (next[2] - previous[2]) * previous[0]
                        <= (next[0] - previous[0]) * previous[2]
                    {
                        2
                    } else {
                        0
                    }
                }
                4 => {
                    if (next[1] - previous[1]) * previous[0] < (next[0] - previous[0]) * previous[1]
                    {
                        1
                    } else {
                        0
                    }
                }
                1 => {
                    if (next[1] - previous[1]) * previous[2]
                        <= (next[2] - previous[2]) * previous[1]
                    {
                        1
                    } else {
                        2
                    }
                }
                _ => 0,
            };
            if edge != 2 {
                triangle[edge] = triangle[2];
            }
            reduce_segment = false;
        }
    }
    if reduce_segment {
        if query_is_segment {
            f.query_count = 1;
        } else {
            f.target_count = 1;
        }
        Some(Reduction::direction(n))
    } else {
        f.query_count = 2;
        f.target_count = 2;
        None
    }
}

fn incident_edge(segment: &[[f32; 4]; 4], triangle: &mut [[f32; 4]; 4]) {
    // Only edges incident to the newly inserted vertex are candidates here.
    // Source ties select 2->1, then feeds the pair through 141213505.
    let distance = |end| {
        let (s, t) = edge_parameters(segment[0], segment[1], triangle[2], end, false);
        let delta = subtract(lerp(segment[0], segment[1], s), lerp(triangle[2], end, t));
        dot(delta, delta)
    };
    let d21 = distance(triangle[1]);
    let d20 = distance(triangle[0]);
    if !(d20 < d21) {
        triangle[0] = triangle[1];
    }
    triangle[1] = triangle[2];
}
