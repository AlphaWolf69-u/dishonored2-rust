//! Polytope closest-point helpers. Broadphase candidates are not query hits.
use crate::assets::physworld::polytope::{ConvexPolytope, PolytopeEdge};
pub mod ray;

/// The finite, unscaled, zero-radius/zero-distance point case of 140d11cb0 ->
/// 1412b7aa0, used by render-view environment queries. With no radius, the
/// outside-plane rejection and inside-face return cover the entire predicate;
/// the wrapper's strict distance test excludes the boundary itself.
///
/// `None` requires the general distance kernel. In particular, missing source
/// connectivity must not be treated as admission to the specialized kernel.
/// The point is already in the source shape's local coordinates.
pub fn zero_distance_point_query(
    shape: &ConvexPolytope,
    point: [f32; 3],
) -> Result<Option<bool>, String> {
    Ok(zero_distance_point_depth(shape, point)?.map(|distance| distance < 0.))
}

/// 12b7aa0's finite, unscaled, zero-radius plane reduction. A negative result
/// is the signed inside-face distance published to a closest-hit collector.
/// Nonnegative results suffice to reject a zero-distance query; they are not
/// the general Euclidean distance to an outside edge or vertex.
pub fn zero_distance_point_depth(
    shape: &ConvexPolytope,
    point: [f32; 3],
) -> Result<Option<f32>, String> {
    if shape.radius != 0. || shape.connectivity.is_none() || !point.iter().all(|v| v.is_finite()) {
        return Ok(None);
    }
    if shape.planes.len() < 4 {
        return Err("polytope point query requires at least four source planes".into());
    }
    let mut maximum = -f32::MAX;
    for plane in &shape.planes {
        if !plane.iter().all(|v| v.is_finite()) {
            return Ok(None);
        }
        // 1412b7e39..43 and 1412b7ebe..c8 retain the two SIMD pairs.
        let distance =
            (plane[3] + plane[2] * point[2]) + (plane[1] * point[1] + plane[0] * point[0]);
        if !distance.is_finite() {
            return Ok(None);
        }
        if maximum < distance {
            maximum = distance;
        }
    }
    Ok(Some(maximum))
}

fn min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}
fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

fn reciprocal_sqrt_estimate(value: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        // This helper uses the estimate itself, unlike the refined distance
        // and normal calculations elsewhere in the closest-point routine.
        unsafe { _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(value))) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        1. / value.sqrt()
    }
}

/// 1412b8e90: walk the incident edge ring and return its nonnegative distance
/// hint. This is an auxiliary bound, not a substitute for a closest-point hit.
pub fn vertex_edge_hint(
    shape: &ConvexPolytope,
    vertex: usize,
    point: [f32; 3],
) -> Result<f32, String> {
    let connectivity = shape
        .connectivity
        .as_ref()
        .ok_or("polytope connectivity missing")?;
    let mut edge = *connectivity
        .vertex_edges
        .get(vertex)
        .ok_or("polytope vertex edge missing")?;
    let origin = *shape
        .vertices
        .get(vertex)
        .ok_or("polytope vertex missing")?;
    let neighbour = |edge: PolytopeEdge, previous: bool| -> Result<(usize, usize), String> {
        let face = shape
            .faces
            .get(usize::from(edge.face))
            .ok_or("polytope edge face missing")?;
        let indices = shape.face_vertices(usize::from(edge.face))?;
        let at = usize::from(edge.edge);
        if indices.is_empty() || at >= indices.len() || usize::from(indices[at]) != vertex {
            return Err("polytope vertex edge does not start at the queried vertex".into());
        }
        let adjacent = if previous {
            (at + indices.len() - 1) % indices.len()
        } else {
            (at + 1) % indices.len()
        };
        Ok((
            usize::from(indices[adjacent]),
            usize::from(face.first_index) + adjacent,
        ))
    };
    let distance = |index: usize| -> Result<f32, String> {
        let other = shape
            .vertices
            .get(index)
            .ok_or("polytope neighbouring vertex missing")?;
        let delta: [f32; 3] = std::array::from_fn(|i| origin[i] - other[i]);
        let relative: [f32; 3] = std::array::from_fn(|i| point[i] - origin[i]);
        let squared = (delta[1] * delta[1] + delta[0] * delta[0]) + delta[2] * delta[2];
        let dot = (delta[1] * relative[1] + delta[0] * relative[0]) + delta[2] * relative[2];
        Ok(dot * reciprocal_sqrt_estimate(squared))
    };
    let first = neighbour(edge, false)?.0;
    let mut hint = distance(first)?;
    for _ in 0..=connectivity.face_links.len() {
        let (next, link) = neighbour(edge, true)?;
        edge = *connectivity
            .face_links
            .get(link)
            .ok_or("polytope face link missing")?;
        hint = min(hint, distance(next)?);
        if next == first {
            return Ok(max(0., hint));
        }
    }
    Err("polytope incident edge ring does not close".into())
}

#[cfg(test)]
#[path = "polytope_query/tests.rs"]
mod tests;
