//! 141215710's vector-shape support pair, including four-lane tie ordering.
use crate::gameplay::shape_transform::{inverse_rigid, rotate_vector, ShapeMatrix};

fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

/// The count-three specialization selects the last tied vertex. Larger arrays
/// keep per-lane earliest maxima, then reduce lane pairs in source order.
pub fn support_vertex(vertices: &[[f32; 4]], direction: [f32; 4]) -> Result<[f32; 4], String> {
    let count = vertices.len();
    if count == 0 {
        return Err("convex support query requires vertices".into());
    }
    let dot = |i: usize| {
        let v = vertices[i];
        (v[1] * direction[1] + v[0] * direction[0]) + v[2] * direction[2]
    };
    if count == 3 {
        let a = dot(0);
        let b = dot(1);
        let c = dot(2);
        let first = if b < a { 0 } else { 1 };
        return Ok(vertices[if c < max(a, b) { first } else { 2 }]);
    }
    let (mut scores, mut indices, mut next) = match count & 3 {
        1 => ([dot(0); 4], [0; 4], 1),
        2 => (
            [dot(0), dot(1), f32::NEG_INFINITY, f32::NEG_INFINITY],
            [0, 1, 2, 3],
            2,
        ),
        _ => (
            std::array::from_fn(dot),
            [0, 1, 2, 3],
            if count & 3 == 3 { 3 } else { 4 },
        ),
    };
    while next < count {
        for lane in 0..4 {
            let score = dot(next + lane);
            if scores[lane] < score {
                indices[lane] = next + lane;
            }
            scores[lane] = max(score, scores[lane]);
        }
        next += 4;
    }
    let pair0 = if scores[0] < scores[1] {
        indices[1]
    } else {
        indices[0]
    };
    let pair2 = if scores[2] < scores[3] {
        indices[3]
    } else {
        indices[2]
    };
    let index = if max(scores[1], scores[0]) < max(scores[3], scores[2]) {
        pair2
    } else {
        pair0
    };
    Ok(vertices[index])
}

pub struct SupportPair {
    pub query: [f32; 4],
    pub target: [f32; 4],
    pub target_in_query: [f32; 4],
}
pub fn support_pair(
    query: &[[f32; 4]],
    target: &[[f32; 4]],
    direction: [f32; 4],
    relative: ShapeMatrix,
) -> Result<SupportPair, String> {
    let query = support_vertex(query, direction)?;
    let target_direction = rotate_vector(relative, direction.map(|v| -v));
    let target = support_vertex(target, target_direction)?;
    let delta = std::array::from_fn(|i| target[i] - relative[3][i]);
    let target_in_query = rotate_vector(inverse_rigid(relative), delta);
    Ok(SupportPair {
        query,
        target,
        target_in_query,
    })
}
