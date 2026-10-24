//! Placed trigger shapes from installed CVD/Havok resources, as loaded by
//! arkCpntTrigger's init 140779530 (shape key type 4, unit scale). Camera
//! environment areas and wind-manager centers both test points against these
//! convex hulls; render-model bounds do not define containment.
use crate::{archive, physworld};

#[derive(Clone, Debug)]
pub struct TriggerVolume {
    pub bounds: [[f32; 3]; 2],
    pub hulls: Vec<Vec<[f32; 4]>>,
    /// Placed hull vertices, for body distance queries.
    pub hull_vertices: Vec<Vec<[f32; 3]>>,
}

/// Material 4 `narrow_phase_trigger` (live material library, world+0x988):
/// trigger type 2 with hkUFloat8 tolerance byte 6 = 0.05999756. Trigger
/// shapes are cooked with zero convex radius (key radius 0 in 140779530).
pub const NARROW_PHASE_TRIGGER_TOLERANCE: f32 = f32::from_bits(0x3d75c000);

type V = [f64; 3];
fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn scale(a: V, s: f64) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// Closest point to the origin on a simplex, reducing it to the supporting
/// feature (point, segment, triangle or tetrahedron).
fn closest_on_simplex(simplex: &mut Vec<V>) -> V {
    match simplex.len() {
        1 => simplex[0],
        2 => {
            let (a, b) = (simplex[0], simplex[1]);
            let ab = sub(b, a);
            let t = (-dot(a, ab) / dot(ab, ab)).clamp(0., 1.);
            if t <= 0. {
                simplex.truncate(1);
            } else if t >= 1. {
                simplex.remove(0);
            }
            add(a, scale(ab, t))
        }
        3 => closest_on_triangle(simplex),
        _ => {
            // Origin inside the tetrahedron means intersection; otherwise the
            // closest feature lies on one of its faces.
            let mut best: Option<(f64, V, Vec<V>)> = None;
            let faces = [[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]];
            let inside = faces.iter().all(|f| {
                let other = (0..4).find(|i| !f.contains(i)).unwrap();
                let n = cross(sub(simplex[f[1]], simplex[f[0]]), sub(simplex[f[2]], simplex[f[0]]));
                let side_origin = dot(n, sub([0.; 3], simplex[f[0]]));
                let side_other = dot(n, sub(simplex[other], simplex[f[0]]));
                side_origin * side_other >= 0.
            });
            if inside {
                return [0.; 3];
            }
            for f in faces {
                let mut face: Vec<V> = f.iter().map(|&i| simplex[i]).collect();
                let point = closest_on_triangle(&mut face);
                let d = dot(point, point);
                if best.as_ref().is_none_or(|(b, _, _)| d < *b) {
                    best = Some((d, point, face));
                }
            }
            let (_, point, face) = best.unwrap();
            *simplex = face;
            point
        }
    }
}

fn closest_on_triangle(simplex: &mut Vec<V>) -> V {
    let (a, b, c) = (simplex[0], simplex[1], simplex[2]);
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub([0.; 3], a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0. && d2 <= 0. {
        *simplex = vec![a];
        return a;
    }
    let bp = sub([0.; 3], b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0. && d4 <= d3 {
        *simplex = vec![b];
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0. && d1 >= 0. && d3 <= 0. {
        *simplex = vec![a, b];
        return add(a, scale(ab, d1 / (d1 - d3)));
    }
    let cp = sub([0.; 3], c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0. && d5 <= d6 {
        *simplex = vec![c];
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0. && d2 >= 0. && d6 <= 0. {
        *simplex = vec![a, c];
        return add(a, scale(ac, d2 / (d2 - d6)));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0. && (d4 - d3) >= 0. && (d5 - d6) >= 0. {
        *simplex = vec![b, c];
        return add(b, scale(sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6))));
    }
    let denominator = 1. / (va + vb + vc);
    add(a, add(scale(ab, vb * denominator), scale(ac, vc * denominator)))
}

/// Distance between two convex vertex sets (zero when they intersect).
pub fn convex_distance(a: &[[f32; 3]], b: &[[f32; 3]]) -> f32 {
    let support = |d: V| -> V {
        let pick = |points: &[[f32; 3]], d: V| {
            points
                .iter()
                .map(|p| p.map(f64::from))
                .max_by(|x, y| dot(*x, d).total_cmp(&dot(*y, d)))
                .unwrap()
        };
        sub(pick(a, d), pick(b, scale(d, -1.)))
    };
    let mut simplex = vec![support([1., 0., 0.])];
    let mut closest = simplex[0];
    for _ in 0..64 {
        let squared = dot(closest, closest);
        if squared < 1e-18 {
            return 0.;
        }
        let w = support(scale(closest, -1.));
        if squared - dot(closest, w) <= squared * 1e-10 + 1e-14 {
            break;
        }
        simplex.push(w);
        closest = closest_on_simplex(&mut simplex);
    }
    dot(closest, closest).sqrt() as f32
}

impl TriggerVolume {
    pub fn contains(&self, point: [f32; 3]) -> bool {
        (0..3)
            .all(|axis| point[axis] >= self.bounds[0][axis] && point[axis] <= self.bounds[1][axis])
            && self.hulls.iter().any(|planes| {
                planes
                    .iter()
                    .all(|p| ((p[0] * point[0] + p[1] * point[1]) + p[2] * point[2]) + p[3] <= 0.0)
            })
    }

    /// arkCpntTrigger narrow-phase membership: some body hull (with its
    /// convex radius) lies within the trigger tolerance of some trigger hull.
    pub fn touches(&self, body_hulls: &[Vec<[f32; 3]>], body_radius: f32) -> bool {
        let reach = NARROW_PHASE_TRIGGER_TOLERANCE + body_radius;
        self.hull_vertices.iter().any(|trigger| {
            body_hulls
                .iter()
                .any(|body| convex_distance(body, trigger) < reach)
        })
    }

    pub fn size_squared(&self) -> f32 {
        let d: [f32; 3] = std::array::from_fn(|i| self.bounds[1][i] - self.bounds[0][i]);
        d[2] * d[2] + d[1] * d[1] + d[0] * d[0]
    }

    /// `id` names the owning entity in errors; `model` is its trigger model.
    pub fn load(
        catalog: &archive::Catalog,
        id: &str,
        model: &str,
        position: [f32; 3],
        rotation: [[f32; 3]; 3],
    ) -> Result<Self, String> {
        let key = physworld::physics_shape_key(model, 4, [1.0; 3], 0.0, false)
            .ok_or("invalid trigger shape key")?;
        let path = physworld::physics_shape_asset_path(&key);
        let bytes = catalog
            .read(&path)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("{id}: missing trigger {path}"))?;
        let physics = physworld::parse_hkx(&bytes)?;
        let geometry = physics.standalone_shapes(&bytes)?;
        if !geometry.unknown_shapes.is_empty()
            || !geometry.compressed_decode_failures.is_empty()
            || geometry.convex.is_empty()
        {
            return Err(format!(
                "{id}: unsupported trigger geometry: {:?} {:?}",
                geometry.unknown_shapes, geometry.compressed_decode_failures
            ));
        }
        let placed = |v: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|row| {
                position[row] + (0..3).map(|axis| rotation[axis][row] * v[axis]).sum::<f32>()
            })
        };
        let mut bounds = [[f32::INFINITY; 3], [f32::NEG_INFINITY; 3]];
        let mut hulls = Vec::new();
        let mut hull_vertices = Vec::new();
        for hull in &geometry.convex {
            let vertices: Vec<_> = hull.vertices.iter().copied().map(placed).collect();
            let center: [f32; 3] = std::array::from_fn(|i| {
                vertices.iter().map(|v| v[i]).sum::<f32>() / vertices.len() as f32
            });
            for v in &vertices {
                for i in 0..3 {
                    bounds[0][i] = bounds[0][i].min(v[i]);
                    bounds[1][i] = bounds[1][i].max(v[i]);
                }
            }
            let mut planes = Vec::new();
            for triangle in geometry
                .triangles
                .iter()
                .filter(|t| t.source_shape_item == hull.source_shape_item)
            {
                let [a, b, c] = triangle.vertices.map(placed);
                let u: [f32; 3] = std::array::from_fn(|i| b[i] - a[i]);
                let v: [f32; 3] = std::array::from_fn(|i| c[i] - a[i]);
                let n = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                if n.iter().all(|v| *v == 0.0) {
                    continue;
                }
                let mut plane = [
                    n[0],
                    n[1],
                    n[2],
                    -((n[0] * a[0] + n[1] * a[1]) + n[2] * a[2]),
                ];
                if ((n[0] * center[0] + n[1] * center[1]) + n[2] * center[2]) + plane[3] > 0.0 {
                    plane = plane.map(|v| -v);
                }
                planes.push(plane);
            }
            if planes.len() < 4 {
                return Err(format!("{id}: trigger hull is not closed"));
            }
            hulls.push(planes);
            hull_vertices.push(vertices);
        }
        Ok(Self {
            bounds,
            hulls,
            hull_vertices,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cube(center: [f32; 3], half: f32) -> Vec<[f32; 3]> {
        (0..8)
            .map(|i| std::array::from_fn(|a| center[a] + if i & (1 << a) != 0 { half } else { -half }))
            .collect()
    }
    #[test]
    fn convex_distance_measures_faces_edges_and_overlap() {
        let a = cube([0.; 3], 1.);
        assert!((convex_distance(&a, &cube([3., 0., 0.], 1.)) - 1.).abs() < 1e-5);
        let diagonal = convex_distance(&a, &cube([3., 3., 0.], 1.));
        assert!((diagonal - 2f32.sqrt()).abs() < 1e-5, "{diagonal}");
        let corner = convex_distance(&a, &cube([3., 3., 3.], 1.));
        assert!((corner - 3f32.sqrt()).abs() < 1e-5, "{corner}");
        assert_eq!(convex_distance(&a, &cube([1.5, 0.2, 0.], 1.)), 0.);
    }
}
