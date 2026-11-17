//! Vector-shape GSK ray (1412151a0 -> 141215320 -> 141215de0).
//! The selected RayCastShapeInterface callback1412156e0 returns one shape
//! support and zero secondary support/offset. Consequently its secondary
//! feature is always the current ray point; only the source Nx1 reductions
//! are reachable. This is independent of the general convex-cast interface.
use super::{
    seed::{cross, dot, inverse_length, subtract},
    vertices::support_vertex,
};
use crate::gameplay::constraint_solver::reciprocal;
#[path = "point_distance/simplex.rs"]
mod simplex;

#[derive(Clone, Copy, Debug)]
pub struct ConvexRayHit {
    pub fraction: f32,
    pub normal: [f32; 4],
    /// The enclosing convex primitive callback publishes hit-kind2.
    pub flags: u32,
}

/// Casts the supplied displacement without normalizing its parameterization.
/// Like141215320, this entry rejects a zero-fraction initial-overlap result.
/// Query flags are copied into the source ray record but are not read by this
/// generic kernel; sphere/capsule specializations own their separate policy.
pub fn cast(
    vertices: &[[f32; 4]],
    origin: [f32; 4],
    direction: [f32; 4],
    radius: f32,
    maximum_fraction: f32,
    _query_flags: u32,
) -> Result<Option<ConvexRayHit>, String> {
    let first = *vertices.first().ok_or("convex ray requires vertices")?;
    let minimum = f32::from_bits(0x2880_0000); //1423cce40
    let direction_squared = dot(direction, direction);
    if direction_squared < minimum {
        return Ok(None);
    }
    let inverse_ray_length = inverse_length(direction_squared);
    let tolerance = radius + f32::from_bits(0x3a83_126f); //141ab3b04
    let tolerance_squared = tolerance * tolerance;
    let mut minimum_plane = f32::from_bits(0x4b18_9680); //141f67628
    let mut point = origin;
    let mut feature = [first; 4];
    let mut count = 0;
    let mut raw_direction = subtract(origin, first);
    let inverse = inverse_length(dot(raw_direction, raw_direction));
    let mut normal = raw_direction.map(|v| v * inverse);
    let mut fraction = 0.;
    let mut hit_normal = [0.; 4];
    let mut epsilon = minimum;
    let mut small_changes = 0;
    if !(tolerance_squared < minimum_plane * minimum_plane) {
        return Ok(None); // kernel returns fraction0; wrapper rejects it.
    }
    loop {
        //1412156e0 calls141215390 with the retained raw direction. Scanning
        // with the normalized direction instead changes its support ties.
        let next = support_vertex(vertices, raw_direction)?;
        if count == 0 {
            feature = [next; 4];
            count = 1;
        } else {
            let projection = dot(raw_direction, next);
            // Native padded feature slots are always valid. A tied support
            // can be inserted: this is not GSK distance's duplicate policy.
            if (0..3).all(|i| dot(raw_direction, feature[i]) < projection + epsilon) {
                if count >= 4 {
                    return Err("ray feature was not reduced before support insertion".into());
                }
                feature[count] = next;
                count += 1;
            }
        }
        let gap = dot(subtract(point, next), normal) - radius;
        if 0. < gap {
            let approach = dot(normal, direction);
            if 0. <= approach {
                return Ok(None);
            }
            fraction -= reciprocal(approach) * gap;
            //141216158 rejects equality and retains the source world-length
            // epsilon at the caller's fraction limit.
            if maximum_fraction - inverse_ray_length * f32::from_bits(0x3400_0000) <= fraction {
                return Ok(None);
            }
            point = std::array::from_fn(|i| fraction * direction[i] + origin[i]);
            hit_normal = normal;
        }
        let previous = raw_direction;
        //141217035 first tests the original triangle. If its projected
        // point is outside, discard the fourth vertex without tetra scoring.
        // This ray-specific gate precedes the otherwise shared Nx1 reducer.
        if count == 4 && !projects_inside(point, &feature) {
            count = 3;
        }
        // The ray's shape features are the query side of the shared feature
        // reducer:141217880 triangle,141217cf2 segment, signed4x1 tetrahedron.
        let (reduced, enclosed) = simplex::reduce_query(point, &mut feature, &mut count)?;
        if enclosed {
            break;
        }
        //141217d34 pads with the first surviving feature before the next
        // support-improvement test, then reverses the reducer direction.
        for i in count..3 {
            feature[i] = feature[0];
        }
        raw_direction = reduced.map(|v| -v);
        epsilon += epsilon;
        let inverse = inverse_length(dot(raw_direction, raw_direction));
        normal = raw_direction.map(|v| v * inverse);
        let distances =
            std::array::from_fn::<_, 3, _>(|i| dot(subtract(point, feature[i]), normal).abs());
        // MINPS ordering, including its second-operand unordered behavior.
        let minimum_first_two = min(distances[0], distances[1]);
        minimum_plane = min(min(minimum_plane, distances[2]), minimum_first_two);
        let changed = subtract(raw_direction, previous);
        let changed_enough = epsilon <= dot(changed, changed);
        if !changed_enough {
            // This is cumulative, not a reset-on-progress iteration counter.
            small_changes += 1;
        }
        if !(changed_enough || small_changes < 3)
            || !(tolerance_squared < minimum_plane * minimum_plane)
        {
            break;
        }
    }
    if fraction == 0. {
        return Ok(None);
    }
    Ok(Some(ConvexRayHit {
        fraction,
        normal: hit_normal,
        flags: 2,
    }))
}

fn min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

fn projects_inside(point: [f32; 4], feature: &[[f32; 4]; 4]) -> bool {
    let ac = subtract(feature[0], feature[2]);
    let cb = subtract(feature[2], feature[1]);
    let n = cross(cb, ac);
    dot(cross(subtract(point, feature[1]), cb), n) < 0.
        && dot(cross(subtract(point, feature[2]), ac), n) < 0.
        && dot(
            cross(
                subtract(point, feature[0]),
                subtract(feature[1], feature[0]),
            ),
            n,
        ) < 0.
}
