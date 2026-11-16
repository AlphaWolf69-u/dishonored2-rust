//! b88330/baba50 lower-dimensional admission and ce1e30/b8c950 geometry.
use super::*;
use crate::gameplay::constraint_solver::inverse_length;
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normal(points: &[[f32; 4]]) -> [f32; 4] {
    let mut best = [0.; 3];
    let mut squared = 0.;
    'search: for i in 0..points.len() {
        for j in i + 1..points.len() {
            for k in j + 1..points.len() {
                let a = std::array::from_fn(|axis| points[j][axis] - points[i][axis]);
                let b = std::array::from_fn(|axis| points[k][axis] - points[i][axis]);
                let n = cross(a, b);
                let size = dot(n, n);
                if squared < size {
                    best = n;
                    squared = size;
                    if f32::from_bits(0x3727c5ac) < size {
                        break 'search;
                    }
                }
            }
        }
    }
    if squared <= 0. {
        best = [1., 0., 0.]
    } else {
        let inverse = inverse_length(squared);
        best = best.map(|value| inverse * value)
    }
    [
        best[0],
        best[1],
        best[2],
        -dot(best, [points[0][0], points[0][1], points[0][2]]),
    ]
}
impl Hull {
    /// The full dimension fallback of b88330/baba50, using the same hull owner.
    fn geometry_dimensions(points: &[[f32; 4]]) -> Result<(Self, [f32; 4]), String> {
        if points.is_empty() {
            return Err("hkgpConvexHull geometry has no source points".into());
        }
        let domain = Domain::new(points)?;
        if points.len() >= 4 {
            // Domain was validated above; the only remaining failure is the
            // source bad720 coplanar result, which admits the lower branch.
            if let Ok(hull) = Self::build(points) {
                return Ok((hull, [0.; 4]));
            }
        }
        let plane = normal(points);
        if points.len() >= 3 {
            if let Some(hull) = Self::build_planar(points, plane) {
                return Ok((hull, plane));
            }
        }
        let mut records: Vec<_> = points
            .iter()
            .map(|&position| Point {
                position,
                grid: domain.quantize(position),
                extra: 0,
                index: -1,
            })
            .collect();
        if records.len() > 1 {
            let last = records.len() as i32 - 1;
            sort_points(&mut records, 0, last)
        }
        let direction = if points.len() > 1 {
            std::array::from_fn(|i| points[1][i] - points[0][i])
        } else {
            [0.; 3]
        };
        let projection = |record: &Point| {
            dot(
                [record.position[0], record.position[1], record.position[2]],
                direction,
            )
        };
        let (mut low, mut high) = (0, 0);
        for i in 1..records.len() {
            let value = projection(&records[i]);
            if value < projection(&records[low]) {
                low = i
            }
            if projection(&records[high]) < value {
                high = i
            }
        }
        // baba50's source dimension-one branch compares projected XY integer
        // coordinates after undoing its planar axis swap, then falls to dim0.
        let line = records[low].grid[..2] != records[high].grid[..2];
        let mut hull = Self {
            domain,
            vertices: Vec::new(),
            vertex_head: NONE,
            triangles: Vec::new(),
            triangle_head: NONE,
            dimensions: i32::from(line),
            faces: Vec::new(),
            mark: 0,
        };
        if line {
            hull.new_vertex(records[low]);
            hull.new_vertex(records[high]);
        } else {
            hull.new_vertex(records[0]);
        }
        Ok((hull, plane))
    }
    /// ce1e30 build mode1, including native b8c950 radius expansion for all
    /// dimensions. Construction samples retire with this call; only Hull owns
    /// the resulting vertices/faces. The existing 3D-only stage API is intact.
    pub fn build_geometry(points: &[[f32; 4]], radius: f32) -> Result<Self, String> {
        let (source, plane) = Self::geometry_dimensions(points)?;
        if radius <= 0. {
            return Ok(source);
        }
        let order = source.vertex_order();
        let original: Vec<_> = order
            .iter()
            .map(|&index| source.vertices[index].point.position)
            .collect();
        let mut samples = Vec::new();
        match source.dimensions {
            0 => {
                for axis in 0..3 {
                    for sign in [1., -1.] {
                        let mut point = original[0];
                        point[axis] += sign * (radius * 0.5);
                        samples.push(point)
                    }
                }
            }
            1 => {
                samples.extend_from_slice(&original);
                let delta = std::array::from_fn(|i| original[1][i] - original[0][i]);
                // aeb260's stable perpendicular selection, exactly as b8c950
                // inlines it; the second transverse axis is cross(first,delta).
                let use_z = delta[1] * delta[1] + delta[0] * delta[0]
                    < delta[0] * delta[0] + delta[2] * delta[2];
                let first = if use_z {
                    [-delta[2], 0., delta[0]]
                } else {
                    [-delta[1], delta[0], 0.]
                };
                let second = cross(first, delta);
                let first =
                    first.map(|value| inverse_length(dot(first, first)) * value * (radius * 0.5));
                let second = second
                    .map(|value| inverse_length(dot(second, second)) * value * (radius * 0.5));
                for point in &original {
                    for offset in [first, second] {
                        for sign in [1., -1.] {
                            samples.push([
                                point[0] + sign * offset[0],
                                point[1] + sign * offset[1],
                                point[2] + sign * offset[2],
                                point[3],
                            ])
                        }
                    }
                }
            }
            2 => {
                samples.extend_from_slice(&original);
                for point in &original {
                    for sign in [1., -1.] {
                        samples.push([
                            point[0] + sign * radius * plane[0],
                            point[1] + sign * radius * plane[1],
                            point[2] + sign * radius * plane[2],
                            point[3] + sign * radius * plane[3],
                        ])
                    }
                }
            }
            3 => {
                samples.extend_from_slice(&original);
                for triangle in source.triangle_order() {
                    let points = source.triangles[triangle]
                        .vertices
                        .map(|vertex| source.vertices[vertex].point.position);
                    let a = std::array::from_fn(|i| points[1][i] - points[0][i]);
                    let b = std::array::from_fn(|i| points[2][i] - points[0][i]);
                    let normal = cross(a, b);
                    let inverse = inverse_length(dot(normal, normal));
                    let normal = normal.map(|value| inverse * value);
                    if normal.iter().all(|value| !value.is_nan())
                        && (dot(normal, normal) - 1.).abs() < f32::from_bits(0x38d1b717)
                    {
                        let plane = -dot([points[0][0], points[0][1], points[0][2]], normal);
                        let offset = [
                            normal[0] * radius,
                            normal[1] * radius,
                            normal[2] * radius,
                            plane * radius,
                        ];
                        for point in points {
                            samples.push(std::array::from_fn(|i| offset[i] + point[i]))
                        }
                    }
                }
            }
            _ => return Err("hkgpConvexHull geometry dimension invalid".into()),
        }
        let (result, _) = Self::geometry_dimensions(&samples)?;
        if source.dimensions == 2 && result.dimensions != 3 {
            // b8c950 preserves the original centroid before extrusion. If
            // quantization still collapses its result, rebuild that point and
            // invoke the same dimension-specific expansion once more.
            let mut center = [0.; 4];
            for point in &original {
                for i in 0..4 {
                    center[i] = point[i] + center[i];
                }
            }
            let inverse = 1. / original.len() as f32;
            center = center.map(|value| value * inverse);
            return Self::build_geometry(&[center], radius);
        }
        Ok(result)
    }
    /// b928a0(param2=0,param5=1): original coordinates, native linked order,
    /// with the source's reversed duplicate fan for a planar hull.
    pub fn geometry_triangles(&self) -> Vec<[[f32; 3]; 3]> {
        let point = |index: usize| {
            let p = self.vertices[index].point.position;
            [p[0], p[1], p[2]]
        };
        if self.dimensions == 2 {
            let order = self.vertex_order();
            let mut triangles = Vec::new();
            for i in 1..order.len() - 1 {
                triangles.push([point(order[0]), point(order[i]), point(order[i + 1])]);
                triangles.push([point(order[0]), point(order[i + 1]), point(order[i])]);
            }
            triangles
        } else {
            self.triangle_order()
                .into_iter()
                .map(|index| self.triangles[index].vertices.map(point))
                .collect()
        }
    }
}
