//! cb0fd0's convex plane cast. World/compound traversal owns filtering and
//! transforms; the returned fraction is ready for the closest-hit collector.
use crate::{
    assets::physworld::polytope::ConvexPolytope,
    float::{add, multiply, subtract},
    gameplay::constraint_solver::reciprocal,
};

#[derive(Clone, Copy, Debug)]
pub struct PolytopeRayHit {
    pub fraction: f32,
    pub normal: [f32; 3],
    pub face: usize,
    /// cb0fd0 publishes bit1 for a hit and bit2 for an admitted inside exit.
    pub flags: u32,
}

fn min(first: f32, second: f32) -> f32 {
    if first < second {
        first
    } else {
        second
    }
}
fn max(first: f32, second: f32) -> f32 {
    if first > second {
        first
    } else {
        second
    }
}

/// Query flag4 admits a ray starting inside and reports its exit. Radius is
/// subtracted from each authored plane as in the shape's castRay virtual.
/// `maximum_fraction` is the collector's current distance, not ray length.
pub fn cast(
    shape: &ConvexPolytope,
    origin: [f32; 3],
    direction: [f32; 3],
    maximum_fraction: f32,
    query_flags: u32,
) -> Result<Option<PolytopeRayHit>, String> {
    if shape.planes.len() < 4 {
        return Err("polytope ray requires at least four authored planes".into());
    }
    let end: [f32; 3] =
        std::array::from_fn(|i| add(multiply(direction[i], maximum_fraction), origin[i]));
    // These are the installed constants1423ccc10 and1423ccc40. They are
    // dimensionless segment fractions, not +/-FLT_MAX distance sentinels.
    let mut entering = [-1.; 4];
    let mut leaving = [2.; 4];
    let mut enter_faces = [0; 4];
    let mut leave_faces = [0; 4];
    let mut first = shape.planes.len() - 4;
    let mut processed = 0;
    loop {
        for lane in 0..4 {
            let face = first + lane;
            let plane = shape.planes[face];
            let start_distance = subtract(
                add(
                    add(
                        add(plane[3], multiply(plane[2], origin[2])),
                        multiply(plane[1], origin[1]),
                    ),
                    multiply(plane[0], origin[0]),
                ),
                shape.radius,
            );
            let end_distance = subtract(
                add(
                    add(plane[3], multiply(plane[2], end[2])),
                    add(multiply(plane[1], end[1]), multiply(plane[0], end[0])),
                ),
                shape.radius,
            );
            let start_outside = 0. <= start_distance;
            let end_outside = 0. <= end_distance;
            if start_outside && end_outside {
                return Ok(None);
            }
            let fraction = multiply(
                reciprocal(subtract(start_distance, end_distance)),
                start_distance,
            );
            if start_outside && !end_outside && entering[lane] < fraction {
                entering[lane] = fraction;
                enter_faces[lane] = face;
            }
            if !start_outside && end_outside && fraction < leaving[lane] {
                leaving[lane] = fraction;
                leave_faces[lane] = face;
            }
        }
        // The native SIMD loop starts with the last four planes, then walks
        // four-plane groups from zero. Preserve overlapping tail groups/ties.
        if shape.planes.len() - 4 <= processed {
            break;
        }
        first = processed;
        processed += 4;
    }
    let entry_pairs: [f32; 4] = std::array::from_fn(|i| max(entering[(i + 2) & 3], entering[i]));
    let exit_pairs: [f32; 4] = std::array::from_fn(|i| min(leaving[(i + 2) & 3], leaving[i]));
    let entry = max(entry_pairs[0], entry_pairs[1]);
    let exit = min(exit_pairs[0], exit_pairs[1]);
    let inside = entry < 0. && query_flags & 4 != 0;
    if !(if inside {
        exit != 2.
    } else {
        0. <= entry && entry <= exit
    }) {
        return Ok(None);
    }
    let values = if inside {
        leaving.map(|v| f32::from_bits(v.to_bits() ^ 0x8000_0000))
    } else {
        entering
    };
    let faces = if inside { leave_faces } else { enter_faces };
    let pair_faces: [usize; 4] = std::array::from_fn(|i| {
        if values[i] <= values[(i + 1) & 3] {
            faces[(i + 1) & 3]
        } else {
            faces[i]
        }
    });
    let pair_values: [f32; 4] = std::array::from_fn(|i| max(values[(i + 1) & 3], values[i]));
    let face = if pair_values[0] <= pair_values[2] {
        pair_faces[2]
    } else {
        pair_faces[0]
    };
    let plane = shape.planes[face];
    Ok(Some(PolytopeRayHit {
        fraction: multiply(maximum_fraction, if inside { exit } else { entry }),
        normal: [plane[0], plane[1], plane[2]],
        face,
        flags: if inside { 6 } else { 2 },
    }))
}
