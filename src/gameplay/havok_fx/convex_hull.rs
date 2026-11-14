//! hkgpConvexHull as arkHavokParticleStageDesc::UpdateCollisionShape
//! (1401df6a0) uses it: the 3D build (140baba50 -> 140bad720) on a 14-bit
//! integer grid with the configuration of 140b88010.
//!
//! The pools and intrusive lists of the original are index vectors here;
//! list order (new records at the head) is kept because later stages
//! enumerate vertices and triangles in that order.
use crate::float::{add, multiply, subtract};

const FAR: f32 = f32::from_bits(0x7f7f_ffee);
const NONE: usize = usize::MAX;

/// A point (0x40-byte record of +0x60, or a hull vertex of +0x38).
#[derive(Clone, Copy, Debug)]
pub struct Point {
    /// +0x10: the source position.
    pub position: [f32; 4],
    /// +0x20: the quantized coordinates.
    pub grid: [i32; 3],
    /// +0x2c, +0x34.
    pub extra: i32,
    pub index: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub point: Point,
    /// +0x30: triangles using this vertex.
    pub references: i32,
    next: usize,
    previous: usize,
    alive: bool,
}

/// A link to edge `edge` of triangle `triangle` (pointer | edge).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub triangle: usize,
    pub edge: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Triangle {
    /// +0x10..+0x20.
    pub vertices: [usize; 3],
    /// +0x28..+0x38: the neighbour across edge i (v[i] -> v[i + 1]).
    pub links: [Option<Link>; 3],
    /// +0x40..+0x48 and +0x50: n = (v1 - v0) x (v2 - v0), w = -n . v0.
    pub normal: [i32; 3],
    pub offset: i64,
    /// +0x58: face index, +0x60 flood mark.
    pub face: i32,
    mark: i32,
    next: usize,
    previous: usize,
    alive: bool,
}

fn next_edge(edge: usize) -> usize {
    (9 >> (edge * 2)) & 3
}

fn previous_edge(edge: usize) -> usize {
    (0x12 >> (edge * 2)) & 3
}

/// The quantization domain (140b9c1e0): origin, scale and inverse scale.
#[derive(Clone, Copy, Debug)]
pub struct Domain {
    pub origin: [f32; 4],
    pub scale: [f32; 4],
    pub inverse: [f32; 4],
}

fn rcp_newton(value: f32) -> f32 {
    crate::gameplay::constraint_solver::reciprocal(value)
}

impl Domain {
    pub fn new(points: &[[f32; 4]]) -> Result<Self, String> {
        let mut low = points[0];
        let mut high = points[0];
        for p in &points[1..] {
            low = std::array::from_fn(|i| if low[i] < p[i] { low[i] } else { p[i] });
            high = std::array::from_fn(|i| if high[i] > p[i] { high[i] } else { p[i] });
        }
        if (0..3).any(|i| low[i] < -FAR || FAR < high[i]) {
            return Err("hkgpConvexHull input domain out of range".into());
        }
        let extent: [f32; 4] = std::array::from_fn(|i| {
            let e = subtract(high[i], low[i]);
            let floor = f32::from_bits(0x37a7_c5ac);
            if e > floor { e } else { floor }
        });
        let half: [f32; 4] = extent.map(|e| multiply(e, 0.5));
        let inverse_half: [f32; 4] = half.map(rcp_newton);
        let origin = [add(half[0], low[0]), add(half[1], low[1]), add(half[2], low[2]), 0.];
        let range = f32::from_bits(0x45ff_f800);
        let scale = [multiply(range, inverse_half[0]), multiply(range, inverse_half[1]), multiply(range, inverse_half[2]), 0.];
        let inverse = [rcp_newton(scale[0]), rcp_newton(scale[1]), rcp_newton(scale[2]), 0.];
        Ok(Self { origin, scale, inverse })
    }

    /// The grid coordinate of `p` (140baba50's quantization loop).
    pub fn quantize(&self, p: [f32; 4]) -> [i32; 3] {
        let high = f32::from_bits(0x45ff_f800);
        let low = f32::from_bits(0xc600_0000);
        std::array::from_fn(|i| {
            let v = multiply(subtract(p[i], self.origin[i]), self.scale[i]);
            let v = if v < high { v } else { high };
            let v = if low > v { low } else { v };
            let v = if v < 0. { subtract(v, 0.5) } else { add(v, 0.5) };
            // CVTTPS2DQ; the 2^31 fix-up never applies inside [-8192, 8191].
            v as i32
        })
    }

    /// A grid coordinate back in space: q x inverse + origin.
    pub fn position(&self, q: [i32; 3]) -> [f32; 3] {
        std::array::from_fn(|i| add(multiply(q[i] as f32, self.inverse[i]), self.origin[i]))
    }
}

/// 140bace30: the lexicographic (x, y, z) quicksort of the point records.
fn sort_points(points: &mut [Point], low: i32, high: i32) {
    let mut low = low;
    loop {
        let mid = ((high + low) >> 1) as usize;
        let pivot = points[mid].grid;
        let (mut i, mut j) = (low, high);
        let less = |a: [i32; 3], b: [i32; 3]| a < b;
        loop {
            while less(points[i as usize].grid, pivot) {
                i += 1;
            }
            while less(pivot, points[j as usize].grid) {
                j -= 1;
            }
            if j < i {
                break;
            }
            if j != i {
                points.swap(i as usize, j as usize);
            }
            i += 1;
            j -= 1;
            if j < i {
                break;
            }
        }
        if low < j {
            sort_points(points, low, j);
        }
        if high <= i {
            return;
        }
        low = i;
    }
}

pub struct Hull {
    pub domain: Domain,
    pub vertices: Vec<Vertex>,
    pub vertex_head: usize,
    pub triangles: Vec<Triangle>,
    pub triangle_head: usize,
    /// +0x1e0.
    pub dimensions: i32,
    mark: i32,
}

impl Hull {
    fn new_vertex(&mut self, point: Point) -> usize {
        let index = self.vertices.len();
        self.vertices.push(Vertex {
            point: Point { extra: point.extra, ..point },
            references: 0,
            next: self.vertex_head,
            previous: NONE,
            alive: true,
        });
        if self.vertex_head != NONE {
            self.vertices[self.vertex_head].previous = index;
        }
        self.vertex_head = index;
        index
    }

    fn release_vertex(&mut self, vertex: usize) {
        let Vertex { next, previous, .. } = self.vertices[vertex];
        if next != NONE {
            self.vertices[next].previous = previous;
        }
        if previous == NONE {
            self.vertex_head = next;
        } else {
            self.vertices[previous].next = next;
        }
        self.vertices[vertex].alive = false;
    }

    fn plane(&self, v: [usize; 3]) -> ([i32; 3], i64) {
        let [a, b, c] = v.map(|v| self.vertices[v].point.grid);
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let w = if n == [0; 3] {
            0
        } else {
            -(n[0] as i64 * a[0] as i64 + n[1] as i64 * a[1] as i64 + n[2] as i64 * a[2] as i64)
        };
        (n, w)
    }

    fn new_triangle(&mut self, v: [usize; 3]) -> usize {
        let index = self.triangles.len();
        let (normal, offset) = self.plane(v);
        for &vertex in &v {
            self.vertices[vertex].references += 1;
        }
        self.triangles.push(Triangle {
            vertices: v,
            links: [None; 3],
            normal,
            offset,
            face: -1,
            mark: 0,
            next: self.triangle_head,
            previous: NONE,
            alive: true,
        });
        if self.triangle_head != NONE {
            self.triangles[self.triangle_head].previous = index;
        }
        self.triangle_head = index;
        index
    }

    pub fn evaluate(&self, triangle: usize, q: [i32; 3]) -> i64 {
        let t = &self.triangles[triangle];
        t.normal[2] as i64 * q[2] as i64 + t.normal[1] as i64 * q[1] as i64 + q[0] as i64 * t.normal[0] as i64 + t.offset
    }

    fn connect(&mut self, a: Link, b: Link) {
        self.triangles[a.triangle].links[a.edge] = Some(b);
        self.triangles[b.triangle].links[b.edge] = Some(a);
    }

    /// 140bad050: removes the triangles `q` sees (or lies on), depth first,
    /// recording the last horizon edge reached.
    fn flood(&mut self, q: [i32; 3], at: Link, horizon: &mut Option<Link>) {
        if self.triangles[at.triangle].mark == self.mark {
            return;
        }
        self.triangles[at.triangle].mark = self.mark;
        if self.evaluate(at.triangle, q) < 0 {
            *horizon = Some(at);
            return;
        }
        for edge in [at.edge, next_edge(at.edge), previous_edge(at.edge)] {
            if let Some(link) = self.triangles[at.triangle].links[edge] {
                self.flood(q, link, horizon);
            }
        }
        for edge in [at.edge, next_edge(at.edge), previous_edge(at.edge)] {
            if let Some(link) = self.triangles[at.triangle].links[edge].take() {
                self.triangles[link.triangle].links[link.edge] = None;
            }
        }
        let t = self.triangles[at.triangle];
        for vertex in t.vertices {
            self.vertices[vertex].references -= 1;
            if self.vertices[vertex].references == 0 {
                self.release_vertex(vertex);
            }
        }
        if t.next != NONE {
            self.triangles[t.next].previous = t.previous;
        }
        if t.previous == NONE {
            self.triangle_head = t.next;
        } else {
            self.triangles[t.previous].next = t.next;
        }
        self.triangles[at.triangle].alive = false;
    }

    /// The cone of 140bad720's insertion loop from horizon edge `start`.
    fn cone(&mut self, vertex: usize, start: Link) {
        let mut current = start;
        while self.triangles[current.triangle].links[current.edge].is_some() {
            current.edge = next_edge(current.edge);
        }
        let start = current;
        let (mut first, mut previous) = (NONE, NONE);
        let attach = |hull: &mut Self, at: Link, previous: usize| -> usize {
            let t = hull.triangles[at.triangle].vertices;
            let cone = hull.new_triangle([vertex, t[next_edge(at.edge)], t[at.edge]]);
            hull.connect(at, Link { triangle: cone, edge: 1 });
            if previous != NONE {
                hull.connect(Link { triangle: previous, edge: 0 }, Link { triangle: cone, edge: 2 });
            }
            cone
        };
        loop {
            let mut edge = next_edge(current.edge);
            let mut triangle = current.triangle;
            while let Some(link) = self.triangles[triangle].links[edge] {
                triangle = link.triangle;
                edge = next_edge(link.edge);
            }
            current = Link { triangle, edge };
            if current == start {
                break;
            }
            let cone = attach(self, current, previous);
            if previous == NONE {
                first = cone;
            }
            previous = cone;
        }
        let last = attach(self, current, previous);
        self.connect(Link { triangle: last, edge: 0 }, Link { triangle: first, edge: 2 });
    }

    /// 140bad720: false when every point is coplanar.
    fn build_3d(&mut self, points: &mut Vec<Point>) -> bool {
        let n = points.len();
        let mut found = None;
        'search: for i in 0..n {
            let pi = points[i].grid;
            for j in i + 1..n {
                for k in j + 1..n {
                    let (pj, pk) = (points[j].grid, points[k].grid);
                    let a = [pj[0] - pi[0], pj[1] - pi[1], pj[2] - pi[2]];
                    let b = [pk[0] - pi[0], pk[1] - pi[1], pk[2] - pi[2]];
                    let normal = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
                    if normal == [0; 3] {
                        continue;
                    }
                    let w = -(pi[0] as i64 * normal[0] as i64 + pi[1] as i64 * normal[1] as i64 + pi[2] as i64 * normal[2] as i64);
                    for l in k + 1..n {
                        let q = points[l].grid;
                        let d = q[2] as i64 * normal[2] as i64 + q[1] as i64 * normal[1] as i64 + w + q[0] as i64 * normal[0] as i64;
                        if d != 0 {
                            found = Some((i, j, k, l, d));
                            break 'search;
                        }
                    }
                }
            }
        }
        let Some((i, j, k, l, d)) = found else { return false };
        let vi = self.new_vertex(points[i]);
        let vj = self.new_vertex(points[j]);
        let vk = self.new_vertex(points[k]);
        let vl = self.new_vertex(points[l]);
        let (a, b) = if 0 < d { (vj, vi) } else { (vi, vj) };
        let (c, d) = (vk, vl);
        let t0 = self.new_triangle([a, b, c]);
        let t1 = self.new_triangle([a, d, b]);
        let t2 = self.new_triangle([a, c, d]);
        let t3 = self.new_triangle([b, d, c]);
        let link = |triangle, edge| Link { triangle, edge };
        self.connect(link(t0, 0), link(t1, 2));
        self.connect(link(t0, 1), link(t3, 2));
        self.connect(link(t0, 2), link(t2, 0));
        self.connect(link(t1, 0), link(t2, 2));
        self.connect(link(t2, 1), link(t3, 1));
        self.connect(link(t3, 0), link(t1, 1));
        for index in [l, k, j, i] {
            points.swap_remove(index);
        }
        // One insertion per point, from the last: the first triangle (list
        // order) the point is strictly in front of starts the flood.
        while let Some(point) = points.pop() {
            let mut triangle = self.triangle_head;
            while triangle != NONE {
                if 0 < self.evaluate(triangle, point.grid) {
                    let vertex = self.new_vertex(point);
                    self.mark += 1;
                    let mut horizon = None;
                    self.flood(point.grid, Link { triangle, edge: 0 }, &mut horizon);
                    let horizon = horizon.expect("hkgpConvexHull flood without horizon");
                    self.cone(vertex, horizon);
                    break;
                }
                triangle = self.triangles[triangle].next;
            }
        }
        true
    }

    /// 140b88330 -> 140baba50 with 140b88010's configuration: sorted input,
    /// domain from the bounds. Only the 3D result is used by the particle
    /// stage (anything else fails UpdateCollisionShape).
    pub fn build(source: &[[f32; 4]]) -> Result<Self, String> {
        if source.len() < 4 {
            return Err("hkgpConvexHull needs four points for a 3D hull".into());
        }
        let domain = Domain::new(source)?;
        let mut points: Vec<Point> = source
            .iter()
            .map(|&position| Point { position, grid: domain.quantize(position), extra: 0, index: -1 })
            .collect();
        let last = points.len() as i32 - 1;
        sort_points(&mut points, 0, last);
        let mut hull = Self {
            domain,
            vertices: Vec::new(),
            vertex_head: NONE,
            triangles: Vec::new(),
            triangle_head: NONE,
            dimensions: 3,
            mark: 0,
        };
        if !hull.build_3d(&mut points) {
            return Err("hkgpConvexHull input is not three-dimensional".into());
        }
        Ok(hull)
    }

    /// A vertex's grid position back in space (q x +0x130 + +0x110).
    pub fn grid_position(&self, vertex: usize) -> [f32; 3] {
        self.domain.position(self.vertices[vertex].point.grid)
    }

    /// 140b97b50: the mean of the vertices' grid positions, in list order.
    fn vertex_mean(&self) -> [f32; 3] {
        let mut sum = [0f32; 3];
        let order = self.vertex_order();
        for &v in &order {
            let p = self.grid_position(v);
            sum = std::array::from_fn(|i| add(p[i], sum[i]));
        }
        if order.is_empty() {
            return sum;
        }
        let inverse = crate::float::divide(1., order.len() as f32);
        sum.map(|s| multiply(s, inverse))
    }

    /// 140b959f0's volume integral, centroid part: the volume-weighted
    /// centroid of the triangle tetrahedra about the vertex mean, or an
    /// error when the volume is below 1.19e-7 of the cubed largest extent
    /// (+0x1ed, which fails UpdateCollisionShape).
    pub fn centroid(&self) -> Result<[f32; 3], String> {
        let reference = self.vertex_mean();
        let mut volume = 0f32;
        let mut moment = [0f32; 3];
        let mut low = [FAR; 3];
        let mut high = [-FAR; 3];
        for t in self.triangle_order() {
            let [a, b, c] = self.triangles[t].vertices.map(|v| {
                let p = self.grid_position(v);
                [subtract(p[0], reference[0]), subtract(p[1], reference[1]), subtract(p[2], reference[2])]
            });
            for p in [a, b, c] {
                low = std::array::from_fn(|i| if low[i] < p[i] { low[i] } else { p[i] });
                high = std::array::from_fn(|i| if high[i] > p[i] { high[i] } else { p[i] });
            }
            let z = subtract(multiply(c[1], b[0]), multiply(b[1], c[0]));
            let x = subtract(multiply(c[2], b[1]), multiply(b[2], c[1]));
            let y = subtract(multiply(c[0], b[2]), multiply(b[0], c[2]));
            let six = add(add(multiply(a[1], y), multiply(a[0], x)), multiply(a[2], z));
            volume = add(volume, six);
            moment = std::array::from_fn(|i| add(moment[i], multiply(add(add(b[i], a[i]), c[i]), six)));
        }
        let extent: [f32; 3] = std::array::from_fn(|i| subtract(high[i], low[i]));
        let ey = if extent[1] > extent[0] { extent[1] } else { extent[0] };
        let largest = if extent[2] > ey { extent[2] } else { ey };
        if volume <= multiply(multiply(multiply(largest, largest), largest), f32::from_bits(0x3400_0000)) {
            return Err("hkgpConvexHull mass properties failed (degenerate volume)".into());
        }
        let inverse = crate::float::divide(1., volume);
        Ok(std::array::from_fn(|i| add(multiply(multiply(moment[i], inverse), 0.25), reference[i])))
    }

    /// Vertices from the list head (140b90970 / 140b90a40).
    pub fn vertex_order(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut v = self.vertex_head;
        while v != NONE {
            out.push(v);
            v = self.vertices[v].next;
        }
        out
    }

    pub fn triangle_order(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut t = self.triangle_head;
        while t != NONE {
            out.push(t);
            t = self.triangles[t].next;
        }
        out
    }
}

/// arkHavokParticleStageDesc::UpdateCollisionShape (1401df6a0): the hull
/// of the collision model's vertices, at most 16 of them, relative to its
/// volume centroid (+0x1d00), and that centroid (+0x1d10).
pub fn collision_shape(points: &[[f32; 4]]) -> Result<(Vec<[f32; 4]>, [f32; 3]), String> {
    let hull = Hull::build(points)?;
    if hull.vertex_order().len() > 16 {
        return Err("hkgpConvexHull decimation (140b896a0) is not ported".into());
    }
    let centroid = hull.centroid()?;
    let vertices = hull
        .vertex_order()
        .into_iter()
        .map(|v| {
            let p = hull.vertices[v].point.position;
            [subtract(p[0], centroid[0]), subtract(p[1], centroid[1]), subtract(p[2], centroid[2]), p[3]]
        })
        .collect();
    Ok((vertices, centroid))
}
