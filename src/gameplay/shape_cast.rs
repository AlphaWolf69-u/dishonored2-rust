//! Convex point-hull queries against triangles: the contract of the hknp
//! world queries the character proxy uses (`castShape` world+0x478 vtable
//! +0x98, closest points +0xa8; see `player::character_proxy`).
//!
//! The answers are geometric: closest distance between the hull and a
//! triangle (GJK), and the first contact fraction of a translated hull
//! (conservative advancement, exact for convex sets because the distance is a
//! convex function of the cast fraction); an overlap is measured by EPA on
//! the hull-triangle Minkowski difference. Havok's own kernels and mesh
//! normal welding are not ported.
use bevy_math::Vec3;

/// Closest points between a hull and a triangle. `normal` points from the
/// triangle toward the hull; `distance` is negative when they overlap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub distance: f32,
    pub normal: Vec3,
    /// Closest point on the triangle.
    pub position: Vec3,
}

#[derive(Clone, Copy)]
struct Vertex {
    w: Vec3,
    b: Vec3,
}

fn support(points: &[Vec3], direction: Vec3) -> Vec3 {
    let mut best = points[0];
    let mut value = best.dot(direction);
    for &point in &points[1..] {
        let candidate = point.dot(direction);
        if candidate > value {
            value = candidate;
            best = point;
        }
    }
    best
}

/// Closest point to the origin on the simplex; keeps only contributing
/// vertices and returns their barycentric weights.
fn reduce(simplex: &mut Vec<Vertex>) -> (Vec3, Vec<f32>) {
    match simplex.len() {
        1 => (simplex[0].w, vec![1.0]),
        2 => {
            let (a, b) = (simplex[0].w, simplex[1].w);
            let ab = b - a;
            let t = (-a.dot(ab) / ab.length_squared().max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
            if t <= 0.0 {
                simplex.truncate(1);
                return (a, vec![1.0]);
            }
            if t >= 1.0 {
                simplex.remove(0);
                return (b, vec![1.0]);
            }
            (a + ab * t, vec![1.0 - t, t])
        }
        3 => {
            let (point, weights) = closest_on_triangle([simplex[0].w, simplex[1].w, simplex[2].w], Vec3::ZERO);
            retain(simplex, weights, point)
        }
        _ => {
            let w = [simplex[0].w, simplex[1].w, simplex[2].w, simplex[3].w];
            if inside_tetrahedron(w) {
                return (Vec3::ZERO, vec![0.25; 4]);
            }
            let mut best: Option<(f32, Vec3, [f32; 4])> = None;
            for (i, j, k) in [(0, 1, 2), (0, 1, 3), (0, 2, 3), (1, 2, 3)] {
                let (point, bary) = closest_on_triangle([w[i], w[j], w[k]], Vec3::ZERO);
                let distance = point.length_squared();
                if best.is_none_or(|(d, _, _)| distance < d) {
                    let mut weights = [0.0; 4];
                    weights[i] = bary[0];
                    weights[j] = bary[1];
                    weights[k] = bary[2];
                    best = Some((distance, point, weights));
                }
            }
            let (_, point, weights) = best.unwrap();
            retain(simplex, weights.to_vec(), point)
        }
    }
}

fn retain(simplex: &mut Vec<Vertex>, weights: impl IntoIterator<Item = f32>, point: Vec3) -> (Vec3, Vec<f32>) {
    let mut kept = Vec::new();
    let mut kept_weights = Vec::new();
    for (vertex, weight) in simplex.iter().zip(weights) {
        if weight > 0.0 {
            kept.push(*vertex);
            kept_weights.push(weight);
        }
    }
    if kept.is_empty() {
        kept.push(simplex[0]);
        kept_weights.push(1.0);
    }
    *simplex = kept;
    (point, kept_weights)
}

fn inside_tetrahedron([a, b, c, d]: [Vec3; 4]) -> bool {
    let same_side = |p: Vec3, q: Vec3, r: Vec3, s: Vec3| {
        let normal = (q - p).cross(r - p);
        let side_s = normal.dot(s - p);
        let side_origin = normal.dot(-p);
        side_s * side_origin >= 0.0 && side_s != 0.0
    };
    same_side(a, b, c, d) && same_side(b, c, d, a) && same_side(c, d, a, b) && same_side(d, a, b, c)
}

/// Closest point of a triangle to `p` with its barycentric weights
/// (Ericson, Real-Time Collision Detection 5.1.5).
pub fn closest_on_triangle([a, b, c]: [Vec3; 3], p: Vec3) -> (Vec3, [f32; 3]) {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, [1.0, 0.0, 0.0]);
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (b, [0.0, 1.0, 0.0]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v, [1.0 - v, v, 0.0]);
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (c, [0.0, 0.0, 1.0]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w, [1.0 - w, 0.0, w]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (b + (c - b) * w, [0.0, 1.0 - w, w]);
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    (a + ab * v + ac * w, [1.0 - v - w, v, w])
}

fn support_vertex(points: &[Vec3], triangle: &[Vec3; 3], direction: Vec3) -> Vertex {
    let a = support(points, direction);
    let b = support(triangle, -direction);
    Vertex { w: a - b, b }
}

/// GJK distance between the hull `points` and a triangle. Returns the
/// separation, the unit direction from the triangle toward the hull and the
/// closest point on the triangle, or the final simplex when they overlap.
fn separation(points: &[Vec3], triangle: &[Vec3; 3]) -> Result<(f32, Vec3, Vec3), Vec<Vertex>> {
    let mut simplex = vec![support_vertex(points, triangle, points[0] - triangle[0])];
    let mut weights = vec![1.0];
    let mut v = simplex[0].w;
    for _ in 0..64 {
        let length2 = v.length_squared();
        if length2 <= 1e-12 {
            return Err(simplex);
        }
        let w = support_vertex(points, triangle, -v);
        if length2 - v.dot(w.w) <= 1e-6 * length2.max(1e-6) {
            break;
        }
        if simplex.iter().any(|s| s.w.distance_squared(w.w) <= 1e-14) {
            break;
        }
        simplex.push(w);
        let (point, kept) = reduce(&mut simplex);
        v = point;
        weights = kept;
        if simplex.len() == 4 {
            return Err(simplex);
        }
    }
    let distance = v.length();
    if distance <= 1e-6 {
        return Err(simplex);
    }
    let on_triangle: Vec3 = simplex.iter().zip(&weights).map(|(s, &k)| s.b * k).sum();
    Ok((distance, v / distance, on_triangle))
}

/// A touching contact (zero depth) along the triangle normal facing the hull.
fn touching(points: &[Vec3], [a, b, c]: &[Vec3; 3]) -> Contact {
    let mut normal = (*b - *a).cross(*c - *a).normalize_or_zero();
    let centroid = points.iter().copied().sum::<Vec3>() / points.len() as f32;
    if normal.dot(centroid - *a) < 0.0 {
        normal = -normal;
    }
    let (position, _) = closest_on_triangle([*a, *b, *c], centroid);
    Contact { distance: 0.0, normal, position }
}

/// Grow GJK's final simplex into a tetrahedron around the origin.
fn tetrahedron(points: &[Vec3], triangle: &[Vec3; 3], mut simplex: Vec<Vertex>) -> Option<Vec<Vertex>> {
    if simplex.len() == 2 {
        let axis = (simplex[1].w - simplex[0].w).normalize_or_zero();
        let side = axis.any_orthonormal_vector();
        let w = support_vertex(points, triangle, side);
        let w = if (w.w - simplex[0].w).cross(axis).length_squared() > 1e-12 { w } else { support_vertex(points, triangle, -side) };
        simplex.push(w);
    }
    if simplex.len() == 3 {
        let n = (simplex[1].w - simplex[0].w).cross(simplex[2].w - simplex[0].w);
        if n.length_squared() <= 1e-14 {
            return None;
        }
        for direction in [n, -n] {
            let w = support_vertex(points, triangle, direction);
            if inside_tetrahedron([simplex[0].w, simplex[1].w, simplex[2].w, w.w]) {
                simplex.push(w);
                break;
            }
        }
    }
    (simplex.len() == 4 && inside_tetrahedron([simplex[0].w, simplex[1].w, simplex[2].w, simplex[3].w])).then_some(simplex)
}

fn face_plane(vertices: &[Vertex], f: [usize; 3]) -> Option<(Vec3, f32)> {
    let (a, b, c) = (vertices[f[0]].w, vertices[f[1]].w, vertices[f[2]].w);
    let n = (b - a).cross(c - a);
    let length = n.length();
    (length > 1e-12).then(|| (n / length, (n / length).dot(a)))
}

/// EPA: the penetration depth and direction of an overlapping hull.
fn penetration(points: &[Vec3], triangle: &[Vec3; 3], simplex: Vec<Vertex>) -> Contact {
    let Some(mut vertices) = tetrahedron(points, triangle, simplex) else { return touching(points, triangle) };
    let mut faces: Vec<[usize; 3]> = Vec::new();
    for f in [[0, 1, 2], [0, 3, 1], [0, 2, 3], [1, 3, 2]] {
        let Some((n, _)) = face_plane(&vertices, f) else { return touching(points, triangle) };
        let other = (0..4).find(|i| !f.contains(i)).unwrap();
        faces.push(if n.dot(vertices[other].w - vertices[f[0]].w) > 0.0 { [f[0], f[2], f[1]] } else { f });
    }
    for _ in 0..64 {
        let mut best: Option<(usize, Vec3, f32)> = None;
        for (i, &f) in faces.iter().enumerate() {
            if let Some((n, d)) = face_plane(&vertices, f) {
                if best.is_none_or(|(_, _, bd)| d < bd) {
                    best = Some((i, n, d));
                }
            }
        }
        let Some((index, normal, depth)) = best else { break };
        let w = support_vertex(points, triangle, normal);
        if normal.dot(w.w) - depth < 1e-5 || vertices.len() >= 128 {
            let f = faces[index];
            let (a, b, c) = (vertices[f[0]], vertices[f[1]], vertices[f[2]]);
            let (_, bary) = closest_on_triangle([a.w, b.w, c.w], Vec3::ZERO);
            let position = a.b * bary[0] + b.b * bary[1] + c.b * bary[2];
            return Contact { distance: -depth, normal: -normal, position };
        }
        let new = vertices.len();
        vertices.push(w);
        let mut horizon: Vec<[usize; 2]> = Vec::new();
        let mut kept = Vec::with_capacity(faces.len());
        for f in faces {
            let visible = face_plane(&vertices, f).is_some_and(|(n, _)| n.dot(w.w - vertices[f[0]].w) > 0.0);
            if visible {
                for edge in [[f[0], f[1]], [f[1], f[2]], [f[2], f[0]]] {
                    if let Some(k) = horizon.iter().position(|e| *e == [edge[1], edge[0]]) {
                        horizon.swap_remove(k);
                    } else {
                        horizon.push(edge);
                    }
                }
            } else {
                kept.push(f);
            }
        }
        faces = kept;
        for [a, b] in horizon {
            faces.push([a, b, new]);
        }
    }
    touching(points, triangle)
}

/// Closest points between the hull and a triangle.
pub fn closest(points: &[Vec3], triangle: &[Vec3; 3]) -> Contact {
    match separation(points, triangle) {
        Ok((distance, normal, position)) => Contact { distance, normal, position },
        Err(simplex) => penetration(points, triangle, simplex),
    }
}

/// First contact of `points` translated by `fraction * displacement`
/// (fraction in [0, 1]) with the triangle, to within `accuracy`.
pub fn cast(points: &[Vec3], displacement: Vec3, triangle: &[Vec3; 3], accuracy: f32) -> Option<(f32, Contact)> {
    let mut moved = points.to_vec();
    let mut fraction = 0.0f32;
    for _ in 0..64 {
        for (target, &point) in moved.iter_mut().zip(points) {
            *target = point + displacement * fraction;
        }
        let contact = closest(&moved, triangle);
        if contact.distance <= accuracy {
            return Some((fraction, contact));
        }
        let approach = -displacement.dot(contact.normal);
        if approach <= 0.0 {
            return None;
        }
        fraction += contact.distance / approach;
        if fraction > 1.0 {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(center: Vec3, half: f32) -> Vec<Vec3> {
        let mut points = Vec::new();
        for x in [-half, half] {
            for y in [-half, half] {
                for z in [-half, half] {
                    points.push(center + Vec3::new(x, y, z));
                }
            }
        }
        points
    }

    const FLOOR: [Vec3; 3] = [Vec3::new(-10.0, -10.0, 0.0), Vec3::new(10.0, -10.0, 0.0), Vec3::new(0.0, 10.0, 0.0)];

    #[test]
    fn distance_and_normal_to_a_floor() {
        let contact = closest(&cube(Vec3::new(0.0, 0.0, 1.5), 0.5), &FLOOR);
        assert!((contact.distance - 1.0).abs() < 1e-5, "{contact:?}");
        assert!(contact.normal.distance(Vec3::Z) < 1e-5);
        assert!(contact.position.z.abs() < 1e-5);
    }

    #[test]
    fn edge_contact_normal_points_from_edge_to_hull() {
        let triangle = [Vec3::new(0.0, -1.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(-2.0, 0.0, 0.0)];
        let contact = closest(&[Vec3::new(1.0, 0.0, 1.0)], &triangle);
        assert!((contact.distance - 2f32.sqrt()).abs() < 1e-5);
        assert!(contact.normal.distance(Vec3::new(1.0, 0.0, 1.0).normalize()) < 1e-5);
    }

    #[test]
    fn overlap_reports_negative_distance_along_the_face() {
        let contact = closest(&cube(Vec3::new(0.0, 0.0, 0.3), 0.5), &FLOOR);
        assert!((contact.distance + 0.2).abs() < 1e-5, "{contact:?}");
        assert!(contact.normal.distance(Vec3::Z) < 1e-5);
    }

    #[test]
    fn overlap_depth_respects_the_triangle_extent() {
        // The thin tip of a hull barely crosses a face's top edge: the depth is
        // millimetres, not the distance to the face's plane.
        let face = [Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, -1.0), Vec3::new(0.0, 0.0, 0.0)];
        let hull = [Vec3::new(0.0, 0.004, -0.01), Vec3::new(0.0, -0.4, 0.5), Vec3::new(0.0, 0.4, 0.5), Vec3::new(0.3, 0.0, 0.5)];
        let contact = closest(&hull, &face);
        assert!(contact.distance < 0.0 && contact.distance > -0.02, "{contact:?}");
    }

    #[test]
    fn cast_finds_first_contact_and_ignores_receding_motion() {
        let hull = cube(Vec3::new(0.0, 0.0, 1.5), 0.5);
        let (fraction, contact) = cast(&hull, Vec3::new(0.0, 0.0, -2.0), &FLOOR, 0.001).unwrap();
        assert!((fraction - 0.5).abs() < 0.001, "{fraction}");
        assert!(contact.normal.distance(Vec3::Z) < 1e-4);
        assert!(cast(&hull, Vec3::new(0.0, 0.0, 2.0), &FLOOR, 0.001).is_none());
        assert!(cast(&hull, Vec3::new(0.0, 0.0, -0.5), &FLOOR, 0.001).is_none());
    }
}
