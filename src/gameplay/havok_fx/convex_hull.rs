//! hkgpConvexHull as arkHavokParticleStageDesc::UpdateCollisionShape
//! (1401df6a0) uses it: the 3D build (140baba50 -> 140bad720) on a 14-bit
//! integer grid with the configuration of 140b88010.
//!
//! The pools and intrusive lists of the original are index vectors here;
//! list order (new records at the head) is kept because later stages
//! enumerate vertices and triangles in that order.
use crate::float::{add, multiply, subtract};

const FAR: f32 = f32::from_bits(0x7f7f_ffee);

/// MINPS / MAXPS lane semantics (the first operand when it compares).
fn minps(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

fn maxps(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}
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

/// 140b97e70: quicksort of (triangle, area) by decreasing area.
fn sort_by_area(items: &mut [(usize, f32)], low: i32, high: i32) {
    let mut low = low;
    loop {
        let pivot = items[((high + low) >> 1) as usize].1;
        let (mut i, mut j) = (low, high);
        loop {
            while 0. < subtract(items[i as usize].1, pivot) {
                i += 1;
            }
            while 0. < subtract(pivot, items[j as usize].1) {
                j -= 1;
            }
            if j < i {
                break;
            }
            if j != i {
                items.swap(i as usize, j as usize);
            }
            i += 1;
            j -= 1;
            if j < i {
                break;
            }
        }
        if low < j {
            sort_by_area(items, low, j);
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
    /// +0x70: face planes (unit normal, offset) after `build_indices`.
    pub faces: Vec<[f32; 4]>,
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
            faces: Vec::new(),
            mark: 0,
        };
        if !hull.build_3d(&mut points) {
            return Err("hkgpConvexHull input is not three-dimensional".into());
        }
        // 140bacdf3: the configuration builds indices (140b94be0).
        hull.build_indices();
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

    /// buildIndices (140b94be0, 3D part): vertex indices in list order, and
    /// faces: triangles by decreasing area (140b97e70) seed a face with
    /// their unit plane and flood across edges into triangles whose plane
    /// is degenerate or within the configuration's 0.99999 of it.
    pub fn build_indices(&mut self) {
        let order = self.triangle_order();
        let position = |hull: &Self, v: usize| hull.grid_position(v);
        // Areas, then the quicksort by decreasing area.
        let mut by_area: Vec<(usize, f32)> = order
            .iter()
            .map(|&t| {
                let [v0, v1, v2] = self.triangles[t].vertices;
                let (p0, p1, p2) = (position(self, v0), position(self, v1), position(self, v2));
                let a: [f32; 3] = std::array::from_fn(|i| subtract(p2[i], p0[i]));
                let b: [f32; 3] = std::array::from_fn(|i| subtract(p1[i], p0[i]));
                let z = subtract(multiply(a[1], b[0]), multiply(b[1], a[0]));
                let x = subtract(multiply(a[2], b[1]), multiply(b[2], a[1]));
                let y = subtract(multiply(a[0], b[2]), multiply(b[0], a[2]));
                let squared = add(add(multiply(y, y), multiply(x, x)), multiply(z, z));
                (t, multiply(crate::gameplay::constraint_solver::inverse_length(squared), squared))
            })
            .collect();
        if by_area.len() > 1 {
            let last = by_area.len() as i32 - 1;
            sort_by_area(&mut by_area, 0, last);
        }
        // Unit planes in list order.
        let mut planes = vec![[0f32; 4]; self.triangles.len()];
        for &t in &order {
            let [v0, v1, v2] = self.triangles[t].vertices;
            let (p0, p1, p2) = (position(self, v0), position(self, v1), position(self, v2));
            let a: [f32; 3] = std::array::from_fn(|i| subtract(p2[i], p0[i]));
            let b: [f32; 3] = std::array::from_fn(|i| subtract(p1[i], p0[i]));
            let z = subtract(multiply(a[1], b[0]), multiply(b[1], a[0]));
            let x = subtract(multiply(a[2], b[1]), multiply(b[2], a[1]));
            let y = subtract(multiply(a[0], b[2]), multiply(b[0], a[2]));
            let squared = add(add(multiply(y, y), multiply(x, x)), multiply(z, z));
            let n = if 0. < squared {
                let inverse = crate::gameplay::constraint_solver::inverse_length(squared);
                [multiply(inverse, x), multiply(inverse, y), multiply(inverse, z)]
            } else {
                [x, y, z]
            };
            let w = subtract(0., add(add(multiply(n[1], p0[1]), multiply(n[0], p0[0])), multiply(n[2], p0[2])));
            planes[t] = [n[0], n[1], n[2], w];
            self.triangles[t].face = -1;
        }
        for (index, v) in self.vertex_order().into_iter().enumerate() {
            self.vertices[v].point.index = index as i32;
        }
        self.faces.clear();
        let tolerance = f32::from_bits(0x3f7f_ff58);
        for &(seed, _) in &by_area {
            if self.triangles[seed].face != -1 {
                continue;
            }
            let face = self.faces.len() as i32;
            let p = planes[seed];
            self.triangles[seed].face = face;
            self.faces.push(p);
            let mut stack: Vec<Link> = self.triangles[seed].links.iter().filter_map(|l| *l).collect();
            while let Some(at) = stack.pop() {
                if self.triangles[at.triangle].face != -1 {
                    continue;
                }
                let q = planes[at.triangle];
                let valid = q[..3].iter().all(|v| !v.is_nan());
                let length = subtract(add(add(multiply(q[1], q[1]), multiply(q[0], q[0])), multiply(q[2], q[2])), 1.).abs();
                let dot = add(add(multiply(p[1], q[1]), multiply(p[0], q[0])), multiply(p[2], q[2]));
                if !valid || f32::from_bits(0x38d1_b717) <= length || tolerance <= dot {
                    self.triangles[at.triangle].face = face;
                    for edge in [next_edge(at.edge), previous_edge(at.edge)] {
                        if let Some(link) = self.triangles[at.triangle].links[edge] {
                            stack.push(link);
                        }
                    }
                }
            }
        }
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
    // 140b896a0: half the smallest half extent of the hull's box.
    collision_shape_with(points, |hull| Ok(multiply(0.5, hull.smallest_box_half_extent())))
}

/// `collision_shape` with the decimation tolerance from `tolerance`.
pub fn collision_shape_with(
    points: &[[f32; 4]],
    tolerance: impl Fn(&Hull) -> Result<f32, String>,
) -> Result<(Vec<[f32; 4]>, [f32; 3]), String> {
    let mut hull = Hull::build(points)?;
    if hull.vertex_order().len() > 16 {
        let limit = f32::from_bits(0x5f7f_fff0);
        let t = tolerance(&hull)?;
        hull.decimate(16, if limit < t { limit } else { t })?;
        hull.ensure_containment(points)?;
        let t = tolerance(&hull)?;
        hull.decimate(16, if limit < t { limit } else { t })?;
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

/// hkMatrix4 inverse with a determinant tolerance (140adf2d0 via
/// 140b5ce50), generated from its instructions: `m` is four columns, None
/// when |det| < `tolerance`.
pub fn invert(m: &[f32; 16], tolerance: f32) -> Option<[f32; 16]> {
    use crate::float::divide;
    let t0 = subtract(multiply(m[15], m[10]), multiply(m[14], m[11]));
    let t1 = multiply(m[13], m[11]);
    let t2 = multiply(m[15], m[9]);
    let t3 = subtract(t1, t2);
    let t4 = subtract(multiply(m[14], m[9]), multiply(m[13], m[10]));
    let t5 = add(add(multiply(m[5], t0), multiply(t3, m[6])), multiply(t4, m[7]));
    let t6 = multiply(m[12], m[10]);
    let t7 = multiply(m[14], m[8]);
    let t8 = subtract(t6, t7);
    let t9 = subtract(multiply(m[13], m[8]), multiply(m[12], m[9]));
    let t10 = add(add(multiply(m[4], t4), multiply(t8, m[5])), multiply(t9, m[6]));
    let t11 = multiply(t10, m[3]);
    let t12 = subtract(multiply(m[12], m[11]), multiply(m[15], m[8]));
    let t13 = subtract(t7, t6);
    let t14 = add(add(multiply(m[6], t12), multiply(t13, m[7])), multiply(t0, m[4]));
    let t15 = multiply(t14, m[1]);
    let t16 = add(t11, t15);
    let t17 = subtract(t2, t1);
    let t18 = add(add(multiply(m[7], t9), multiply(t17, m[4])), multiply(t12, m[5]));
    let t19 = -multiply(t18, m[2]);
    let t20 = -multiply(t5, m[0]);
    let t21 = add(t19, t20);
    let det = add(t16, t21);
    if !(tolerance <= det.abs()) {
        return None;
    }
    let t22 = divide(f32::from_bits(0x3f800000), add(t16, t21));
    let t23 = subtract(multiply(m[6], m[1]), multiply(m[2], m[5]));
    let t24 = multiply(m[3], m[5]);
    let t25 = multiply(m[1], m[7]);
    let t26 = subtract(t24, t25);
    let t27 = subtract(multiply(m[7], m[2]), multiply(m[3], m[6]));
    let t28 = divide(f32::from_bits(0x3f800000), add(t21, t16));
    let t29 = multiply(m[0], m[6]);
    let t30 = multiply(m[2], m[4]);
    let t31 = subtract(t29, t30);
    let t32 = subtract(multiply(m[4], m[3]), multiply(m[0], m[7]));
    let t33 = add(t15, t11);
    let t34 = add(t20, t19);
    let t35 = divide(f32::from_bits(0x3f800000), add(t33, t34));
    let t36 = subtract(t25, t24);
    let t37 = subtract(multiply(m[5], m[0]), multiply(m[1], m[4]));
    let t38 = divide(f32::from_bits(0x3f800000), add(t34, t33));
    let t39 = subtract(t30, t29);
    Some([
        multiply(-t5, t22),
        multiply(add(multiply(m[3], t4), add(multiply(t3, m[2]), multiply(t0, m[1]))), t22),
        multiply(-add(multiply(m[15], t23), add(multiply(t26, m[14]), multiply(t27, m[13]))), t22),
        multiply(add(multiply(m[11], t23), add(multiply(t26, m[10]), multiply(t27, m[9]))), t22),
        multiply(t14, t28),
        multiply(-add(multiply(m[0], t0), add(multiply(t13, m[3]), multiply(t12, m[2]))), t28),
        multiply(add(multiply(m[12], t27), add(multiply(t31, m[15]), multiply(t32, m[14]))), t28),
        multiply(-add(multiply(m[8], t27), add(multiply(t31, m[11]), multiply(t32, m[10]))), t28),
        multiply(-t18, t35),
        multiply(add(multiply(m[1], t12), add(multiply(t17, m[0]), multiply(t9, m[3]))), t35),
        multiply(-add(multiply(m[13], t32), add(multiply(t36, m[12]), multiply(t37, m[15]))), t35),
        multiply(add(multiply(m[9], t32), add(multiply(t36, m[8]), multiply(t37, m[11]))), t35),
        multiply(t10, t38),
        multiply(-add(multiply(m[2], t9), add(multiply(t8, m[1]), multiply(t4, m[0]))), t38),
        multiply(add(multiply(m[14], t37), add(multiply(t39, m[13]), multiply(t23, m[12]))), t38),
        multiply(-add(multiply(m[10], t37), add(multiply(t39, m[9]), multiply(t23, m[8]))), t38),
    ])
}

/// sqrt(1.19e-7) through RSQRTPS + Newton (140b896a0's local_258).
fn epsilon_root() -> f32 {
    let e = f32::from_bits(0x3400_0000);
    multiply(crate::gameplay::constraint_solver::inverse_length(e), e)
}

/// One candidate collapse of 140b896a0: position (w = 1) and cost.
#[derive(Clone, Copy, Debug)]
struct Collapse {
    position: [f32; 4],
    cost: f32,
}

type Quadric = [f32; 16];

/// v^T Q v with 140b8a355's transpose order: row i = (q[4i+3] v3 +
/// q[4i+2] v2) + (q[4i+1] v1 + q[4i] v0), then (t3 + t1) + (t2 + t0).
fn quadric_cost(q: &Quadric, v: [f32; 4]) -> f32 {
    let t: [f32; 4] = std::array::from_fn(|i| {
        let row = &q[4 * i..4 * i + 4];
        let dot = add(
            add(multiply(row[3], v[3]), multiply(row[2], v[2])),
            add(multiply(row[1], v[1]), multiply(row[0], v[0])),
        );
        multiply(dot, v[i])
    });
    let sum = add(add(t[3], t[1]), add(t[2], t[0]));
    if 0. > sum { 0. } else { sum }
}

impl Hull {
    /// 140b9bc60 with planes: inside every face plane within `tolerance`.
    fn accepts(&self, p: [f32; 4], tolerance: f32) -> bool {
        self.faces.iter().all(|f| {
            !(tolerance < add(add(add(multiply(f[1], p[1]), multiply(f[0], p[0])), multiply(f[2], p[2])), f[3]))
        })
    }

    /// The optimal collapse of an edge (140b5ce50 inverse, checked both
    /// ways against the identity, then the plane test), or the midpoint.
    fn collapse(&self, sum: &Quadric, small: [f32; 4], large: [f32; 4], root: f32, tolerance: f32) -> Collapse {
        let m: [f32; 16] = [
            sum[0], sum[4], sum[8], 0., sum[1], sum[5], sum[9], 0., sum[2], sum[6], sum[10], 0., sum[3], sum[7], sum[11], 1.,
        ];
        if let Some(inv) = invert(&m, root) {
            let col = |x: &[f32; 16], k: usize| [x[4 * k], x[4 * k + 1], x[4 * k + 2], x[4 * k + 3]];
            let mut worst = 0f32;
            for c in 0..4 {
                let (ic, mcol) = (col(&inv, c), col(&m, c));
                for r in 0..4 {
                    let mi = add(
                        add(add(multiply(ic[1], m[4 + r]), multiply(ic[0], m[r])), multiply(ic[2], m[8 + r])),
                        multiply(ic[3], m[12 + r]),
                    );
                    let im = add(
                        add(add(multiply(mcol[1], inv[4 + r]), multiply(mcol[0], inv[r])), multiply(mcol[2], inv[8 + r])),
                        multiply(mcol[3], inv[12 + r]),
                    );
                    let identity = if c == r { 1. } else { 0. };
                    for value in [subtract(mi, identity), subtract(im, identity)] {
                        let value = value.abs();
                        worst = if worst > value { worst } else { value };
                    }
                }
            }
            if worst < root {
                let candidate = [inv[12], inv[13], inv[14], 1.];
                if self.accepts(candidate, tolerance) {
                    return Collapse { position: candidate, cost: quadric_cost(sum, candidate) };
                }
            }
        }
        let mid = [
            multiply(add(large[0], small[0]), 0.5),
            multiply(add(large[1], small[1]), 0.5),
            multiply(add(large[2], small[2]), 0.5),
            1.,
        ];
        Collapse { position: mid, cost: quadric_cost(sum, mid) }
    }

    /// 140b896a0 (decimateVertices): quadric edge collapses over the edges
    /// between different faces until at most `target` such vertices remain
    /// and no collapse costs less than tolerance^2; then the hull of the
    /// surviving positions. `tolerance` is min(limit, half the smallest
    /// OBB extent).
    pub fn decimate(&mut self, target: usize, tolerance: f32) -> Result<(), String> {
        self.build_indices();
        let root = epsilon_root();
        let order = self.vertex_order();
        let n = order.len();
        let mut positions: Vec<[f32; 4]> = order.iter().map(|&v| self.vertices[v].point.position).collect();
        let original = positions.clone();
        let mut quadrics = vec![[0f32; 16]; n];
        let mut planes: Vec<Vec<[f32; 4]>> = vec![Vec::new(); n];
        let mut keys: Vec<u32> = Vec::new();
        for t in self.triangle_order() {
            let tri = self.triangles[t];
            let index = |v: usize| self.vertices[v].point.index as u32;
            for edge in 0..3 {
                let neighbour = tri.links[edge].expect("closed hull").triangle;
                if self.triangles[neighbour].face != tri.face {
                    let a = index(tri.vertices[edge]);
                    let b = index(tri.vertices[next_edge(edge)]);
                    let key = if a < b { a << 16 | (b & 0xffff) } else { b << 16 | (a & 0xffff) };
                    if !keys.contains(&key) {
                        keys.push(key);
                    }
                }
            }
            let p = self.faces[tri.face as usize];
            for k in 0..3 {
                let v = index(tri.vertices[k]) as usize;
                let known = planes[v].iter().any(|q| (0..3).all(|i| subtract(q[i], p[i]).abs() <= root));
                planes[v].push(p);
                if !known {
                    let q = &mut quadrics[v];
                    for i in 0..4 {
                        for j in 0..4 {
                            q[4 * i + j] = add(multiply(p[i], p[j]), q[4 * i + j]);
                        }
                    }
                }
            }
        }
        let words = (n + 31) >> 5;
        let mut removed = vec![u32::MAX; words];
        let mut collapses = Vec::with_capacity(keys.len());
        let far = f32::from_bits(0x7f7f_ffee);
        let (mut best, mut best_cost) = (usize::MAX, far);
        for (i, &key) in keys.iter().enumerate() {
            let (small, large) = ((key >> 16) as usize, (key & 0xffff) as usize);
            let sum: Quadric = std::array::from_fn(|k| add(quadrics[small][k], quadrics[large][k]));
            let c = self.collapse(&sum, positions[small], positions[large], root, tolerance);
            if c.cost < best_cost {
                best_cost = c.cost;
                best = i;
            }
            collapses.push(c);
            for v in [large, small] {
                removed[v >> 5] &= !(1 << (v & 31));
            }
        }
        let mut set = 0u32;
        let mut mask = (!(u32::MAX << ((n as u32).wrapping_sub(1) & 31))) << 1 | 1;
        for w in (0..words).rev() {
            set += (removed[w] & mask).count_ones();
            mask = u32::MAX;
        }
        let mut alive = n as i32 - set as i32;
        let limit = multiply(tolerance, tolerance);
        while (target as i32) < alive || (best_cost < limit && 6 < keys.len()) {
            let key = keys[best];
            let (keep, gone) = ((key >> 16) as usize, (key & 0xffff) as usize);
            positions[keep] = collapses[best].position;
            removed[gone >> 5] |= 1 << (gone & 31);
            let moved = quadrics[gone];
            for k in 0..16 {
                quadrics[keep][k] = add(moved[k], quadrics[keep][k]);
            }
            keys.swap_remove(best);
            collapses.swap_remove(best);
            let mut i = keys.len() as i32 - 1;
            while 0 <= i {
                let at = i as usize;
                let (a, b) = (keys[at] & 0xffff, keys[at] >> 16);
                if [a, b].iter().any(|&v| v as usize == keep || v as usize == gone) {
                    let a = if a as usize == gone { keep as u32 } else { a };
                    let b = if b as usize == gone { keep as u32 } else { b };
                    let (small, large) = if a < b { (a, b) } else { (b, a) };
                    let key = small << 16 | large;
                    keys[at] = key;
                    if keys.iter().rposition(|&k| k == key) == Some(at) {
                        let (small, large) = (small as usize, large as usize);
                        let sum: Quadric = std::array::from_fn(|k| add(quadrics[small][k], quadrics[large][k]));
                        collapses[at] = self.collapse(&sum, positions[small], positions[large], root, tolerance);
                    } else {
                        keys.swap_remove(at);
                        collapses.swap_remove(at);
                    }
                }
                i -= 1;
            }
            best = 0;
            best_cost = collapses.first().map_or(far, |c| c.cost);
            for (j, c) in collapses.iter().enumerate().skip(1) {
                if c.cost < best_cost {
                    best_cost = c.cost;
                    best = j;
                }
            }
            alive -= 1;
        }
        // Compact the surviving positions (swap from the end).
        let mut count = n;
        for v in (0..n).rev() {
            if removed[v >> 5] >> (v & 31) & 1 != 0 {
                count -= 1;
                if count != v {
                    positions[v] = positions[count];
                }
            }
        }
        if count < n {
            let rebuilt = if 2 < alive { Hull::build(&positions[..count]).ok() } else { None };
            *self = match rebuilt {
                Some(hull) => hull,
                None => Hull::build(&original)?,
            };
        }
        Ok(())
    }
}

/// 140ae12e0 on the 140ae1cb0 transpose of three planes' rows: the inverse
/// rows (out[0..12]) or None when |det| is below 1.19e-7 of the product of
/// the rows' absolute sums.
pub fn invert_transposed(i: [f32; 4], j: [f32; 4], k: [f32; 4]) -> Option<[f32; 12]> {
    let (a, s, m) = (add, subtract, multiply);
    // 140ae1cb0: rows (i0, j0, k0, k1), (i1, j1, k1, k3), (i2, j2, k2, k3).
    let r0 = [i[0], j[0], k[0], k[1]];
    let r1 = [i[1], j[1], k[1], k[3]];
    let r2 = [i[2], j[2], k[2], k[3]];
    let (f2, f3, f4) = (r0[0], r0[1], r0[2]);
    let (f5, f6, f11) = (r1[0], r1[1], r1[2]);
    let (f7, f8, f9) = (r2[0], r2[1], r2[2]);
    let c22 = s(m(f8, f5), m(f6, f7));
    let c23 = s(m(f9, f6), m(f11, f8));
    let c24 = s(m(f7, f11), m(f5, f9));
    let c25 = s(m(f5, f4), m(f2, f11));
    let c26 = s(m(r1[3], r0[3]), m(r0[3], r1[3]));
    let det = a(a(m(f3, c24), m(f2, c23)), m(f4, c22));
    let sum1 = a(a(f6.abs(), f5.abs()), f11.abs());
    let sum2 = a(a(f8.abs(), f7.abs()), f9.abs());
    let sum0 = a(a(f3.abs(), f2.abs()), f4.abs());
    if !(m(m(m(sum2, sum1), sum0), f32::from_bits(0x3400_0000)) < det.abs()) {
        return None;
    }
    let r = crate::gameplay::constraint_solver::reciprocal(det);
    Some([
        m(r, c23),
        m(r, s(m(f4, f8), m(f9, f3))),
        m(r, s(m(f11, f3), m(f4, f6))),
        m(r, c25),
        m(c24, r),
        m(s(m(f2, f9), m(f7, f4)), r),
        m(c25, r),
        m(c26, r),
        m(r, c22),
        m(r, s(m(f3, f7), m(f8, f2))),
        m(r, s(m(f6, f2), m(f3, f5))),
        m(r, c26),
    ])
}

impl Hull {
    /// 140b886c0: the hull of every plane triple's intersection point that
    /// lies within 1e-5 inside all other planes.
    pub fn from_planes(planes: &[[f32; 4]]) -> Result<Self, String> {
        let tolerance = f32::from_bits(0x3727_c5ac);
        let mut points = Vec::new();
        let n = planes.len();
        for i in 0..n {
            for j in i + 1..n {
                for k in j + 1..n {
                    let Some(out) = invert_transposed(planes[i], planes[j], planes[k]) else { continue };
                    let (wi, wj, wk) = (subtract(0., planes[i][3]), subtract(0., planes[j][3]), subtract(0., planes[k][3]));
                    let p = [
                        add(add(multiply(wj, out[4]), multiply(wi, out[0])), multiply(wk, out[8])),
                        add(add(multiply(wj, out[5]), multiply(wi, out[1])), multiply(wk, out[9])),
                        add(add(multiply(wj, out[6]), multiply(wi, out[2])), multiply(wk, out[10])),
                        add(add(multiply(wj, out[7]), multiply(wi, out[3])), multiply(wk, out[11])),
                    ];
                    let inside = (0..n).filter(|&q| q != i && q != j && q != k).all(|q| {
                        let f = planes[q];
                        !(tolerance < add(add(add(f[3], multiply(p[1], f[1])), multiply(p[2], f[2])), multiply(p[0], f[0])))
                    });
                    if inside {
                        points.push(p);
                    }
                }
            }
        }
        if points.len() < 4 {
            return Err("hkgpConvexHull from planes has fewer than four vertices".into());
        }
        Hull::build(&points)
    }

    /// ensureContainment (140b9ab30, 3D part): every face plane moved out
    /// to contain `points`, then the hull of the moved planes.
    pub fn ensure_containment(&mut self, points: &[[f32; 4]]) -> Result<(), String> {
        if self.faces.len() < 4 {
            return Err("hkgpConvexHull::ensureContainment requires at least 4 planes".into());
        }
        let planes: Vec<[f32; 4]> = self
            .faces
            .iter()
            .map(|f| {
                let mut reach = 0f32;
                for p in points {
                    let d = add(add(f[3], multiply(f[1], p[1])), add(multiply(f[2], p[2]), multiply(f[0], p[0])));
                    reach = if reach > d { reach } else { d };
                }
                [f[0], f[1], f[2], subtract(f[3], reach)]
            })
            .collect();
        *self = Hull::from_planes(&planes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(fields: &[&str]) -> Vec<u32> {
        fields.iter().map(|h| u32::from_str_radix(h, 16).unwrap()).collect()
    }

    /// Outputs of the installed 140adf2d0 and 140ae1cb0 + 140ae12e0
    /// (analysis/probe_hull_matrix_inverses.py).
    #[test]
    fn matrix_inverses_match_the_original() {
        let text = include_str!("../../../tests/fixtures/hull_matrix_inverse_cases.txt");
        let (mut four, mut three) = (0, 0);
        for line in text.lines().filter(|line| !line.starts_with('#')) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let values = words(&fields[1..]);
            let f = |w: u32| f32::from_bits(w);
            if fields[0] == "M4" {
                let m: [f32; 16] = std::array::from_fn(|i| f(values[i]));
                let ok = values[16] == 0;
                let result = invert(&m, 0.00034527_f32);
                assert_eq!(result.is_some(), ok, "status of {line}");
                if let Some(r) = result {
                    let ours: Vec<u32> = r.iter().map(|v| v.to_bits()).collect();
                    assert_eq!(ours, values[17..33].to_vec(), "{line}");
                }
                four += 1;
            } else {
                let p = |k: usize| [f(values[4 * k]), f(values[4 * k + 1]), f(values[4 * k + 2]), f(values[4 * k + 3])];
                let ok = values[12] == 0;
                let result = invert_transposed(p(0), p(1), p(2));
                assert_eq!(result.is_some(), ok, "status of {line}");
                if let Some(r) = result {
                    let ours: Vec<u32> = r.iter().map(|v| v.to_bits()).collect();
                    assert_eq!(ours, values[13..25].to_vec(), "{line}");
                }
                three += 1;
            }
        }
        assert_eq!((four, three), (400, 400));
    }
}

/// The axis swap of 140baba50's planar build (the dominant normal axis
/// becomes z), undone after the chain.
fn swap_axes(records: &mut [Point], axis: usize) {
    for r in records.iter_mut() {
        if axis == 0 {
            r.grid.swap(0, 2);
        } else if axis == 1 {
            r.grid.swap(1, 2);
        }
    }
}

/// The monotone chain's turn: (a - o) x (p - o) in the grid's x, y.
fn cross(o: &Point, a: &Point, p: &Point) -> i64 {
    (a.grid[0] - o.grid[0]) as i64 * (p.grid[1] - o.grid[1]) as i64
        - (p.grid[0] - o.grid[0]) as i64 * (a.grid[1] - o.grid[1]) as i64
}

/// 140c0d200: the minimum and maximum of `points` projected on `d`, four
/// points at a time ((z + y) + x per lane) and the rest one at a time
/// ((y + x) + z).
fn project(d: [f32; 4], points: &[[f32; 4]]) -> (f32, f32) {
    let far = f32::from_bits(0x7f7f_ffee);
    let n = points.len();
    let mut low = [far; 4];
    let mut high = [subtract(0., far); 4];
    let mut done = 0;
    if n as i64 - 3 > 0 {
        let blocks = ((n - 3 - 1) >> 2) + 1;
        done = blocks * 4;
        for b in 0..blocks {
            for l in 0..4 {
                let p = points[4 * b + l];
                let v = add(add(multiply(p[2], d[2]), multiply(p[1], d[1])), multiply(p[0], d[0]));
                low[l] = minps(low[l], v);
                high[l] = maxps(high[l], v);
            }
        }
    }
    // Horizontal reduction: (l2, l3, l0, l1) then (l1, l0, l3, l2).
    let low = [minps(low[2], low[0]), minps(low[3], low[1])];
    let mut low = minps(low[0], low[1]);
    let high = [maxps(high[2], high[0]), maxps(high[3], high[1])];
    let mut high = maxps(high[0], high[1]);
    for p in &points[done.min(n)..] {
        let v = add(add(multiply(d[1], p[1]), multiply(d[0], p[0])), multiply(d[2], p[2]));
        low = minps(low, v);
        high = maxps(high, v);
    }
    (low, high)
}

/// 1/sqrt through SQRTPS and DIVPS, masked to 0 (140c0d380's normalize).
fn normalized(v: [f32; 4]) -> [f32; 4] {
    let s = add(add(multiply(v[1], v[1]), multiply(v[0], v[0])), multiply(v[2], v[2]));
    let inverse = if s <= 0. { 0. } else { crate::float::divide(1., s.sqrt()) };
    [multiply(v[0], inverse), multiply(v[1], inverse), multiply(v[2], inverse), 0.]
}

impl Hull {
    /// 140b884e0 -> 140baba50 forced to two dimensions: the 2D hull of
    /// `points` in the plane with normal `normal` (monotone chain on the
    /// quantized coordinates with the dominant normal axis dropped), with
    /// its edge planes (140b94be0, dims 2). None when degenerate.
    fn build_planar(points: &[[f32; 4]], normal: [f32; 4]) -> Option<Self> {
        let domain = Domain::new(points).ok()?;
        let mut records: Vec<Point> = points
            .iter()
            .map(|&position| Point { position, grid: domain.quantize(position), extra: 0, index: -1 })
            .collect();
        let a = [normal[0].abs(), normal[1].abs(), normal[2].abs()];
        let xy = if a[1] > a[0] { a[1] } else { a[0] };
        let largest = if a[2] > xy { a[2] } else { xy };
        let mask = (0..3).fold(0, |m, i| m | (usize::from(largest <= a[i]) << i));
        let axis = [0, 0, 1, 1, 2, 2, 2, 2][mask];
        swap_axes(&mut records, axis);
        if records.len() > 1 {
            let last = records.len() as i32 - 1;
            sort_points(&mut records, 0, last);
        }
        let mut chain: Vec<Point> = Vec::new();
        for p in &records {
            while chain.len() >= 2 && !(0 < cross(&chain[chain.len() - 2], &chain[chain.len() - 1], p)) {
                chain.pop();
            }
            chain.push(*p);
        }
        let lower = chain.len() + 1;
        for p in records.iter().rev().skip(1) {
            while chain.len() >= lower && !(0 < cross(&chain[chain.len() - 2], &chain[chain.len() - 1], p)) {
                chain.pop();
            }
            chain.push(*p);
        }
        if chain.len() < 4 {
            return None;
        }
        chain.pop();
        swap_axes(&mut chain, axis);
        // Orientation: the largest edge cross product about the centroid
        // against the plane normal reverses the order when opposite.
        let count = chain.len() as f32;
        let mut sum = [0f32; 3];
        for p in &chain {
            sum = std::array::from_fn(|i| add(sum[i], p.position[i]));
        }
        let inverse = crate::gameplay::constraint_solver::reciprocal(count);
        let c: [f32; 3] = sum.map(|s| multiply(s, inverse));
        let mut best = [0f32; 3];
        let mut previous = chain.len() - 1;
        for current in 0..chain.len() {
            let q: [f32; 3] = std::array::from_fn(|i| subtract(chain[previous].position[i], c[i]));
            let p: [f32; 3] = std::array::from_fn(|i| subtract(chain[current].position[i], c[i]));
            let z = subtract(multiply(p[1], q[0]), multiply(q[1], p[0]));
            let x = subtract(multiply(p[2], q[1]), multiply(q[2], p[1]));
            let y = subtract(multiply(p[0], q[2]), multiply(q[0], p[2]));
            let size = |v: [f32; 3]| add(add(multiply(v[1], v[1]), multiply(v[0], v[0])), multiply(v[2], v[2]));
            if size(best) < size([x, y, z]) {
                best = [x, y, z];
            }
            previous = current;
        }
        if add(add(multiply(best[1], normal[1]), multiply(best[0], normal[0])), multiply(best[2], normal[2])) < 0. {
            chain.reverse();
        }
        let mut hull = Self {
            domain,
            vertices: Vec::new(),
            vertex_head: NONE,
            triangles: Vec::new(),
            triangle_head: NONE,
            dimensions: 2,
            faces: Vec::new(),
            mark: 0,
        };
        for p in chain {
            hull.new_vertex(p);
        }
        // 140b94be0, dims 2: an outward plane per edge, from the tail.
        let order = hull.vertex_order();
        let mut mean = [0f32; 3];
        for &v in &order {
            let p = hull.grid_position(v);
            mean = std::array::from_fn(|i| add(p[i], mean[i]));
        }
        let inverse = crate::float::divide(1., order.len() as f32);
        let mean: [f32; 3] = mean.map(|s| multiply(s, inverse));
        let mut previous = *order.last()?;
        for &v in &order {
            let p = hull.grid_position(previous);
            let q = hull.grid_position(v);
            let e: [f32; 3] = std::array::from_fn(|i| subtract(q[i], p[i]));
            let nz = subtract(multiply(e[1], normal[0]), multiply(normal[1], e[0]));
            let nx = subtract(multiply(e[2], normal[1]), multiply(normal[2], e[1]));
            let ny = subtract(multiply(e[0], normal[2]), multiply(normal[0], e[2]));
            let s = add(add(multiply(ny, ny), multiply(nx, nx)), multiply(nz, nz));
            let n = if 0. < s {
                let r = crate::gameplay::constraint_solver::inverse_length(s);
                [multiply(r, nx), multiply(r, ny), multiply(r, nz)]
            } else {
                [nx, ny, nz]
            };
            let w = subtract(0., add(add(multiply(p[1], n[1]), multiply(p[0], n[0])), multiply(p[2], n[2])));
            let side = add(add(w, multiply(mean[1], n[1])), add(multiply(mean[2], n[2]), multiply(mean[0], n[0])));
            let plane = if 0. <= side { [-n[0], -n[1], -n[2], -w] } else { [n[0], n[1], n[2], w] };
            hull.faces.push(plane);
            previous = v;
        }
        Some(hull)
    }

    /// 140c0d380 for a 3D hull (via 140b90690 / 140b90eb0): the box with
    /// the smallest half surface area among the face-plane frames, whose
    /// smallest half extent is returned.
    pub fn smallest_box_half_extent(&self) -> f32 {
        let points: Vec<[f32; 4]> = self.vertex_order().iter().map(|&v| self.vertices[v].point.position).collect();
        let mut best = f32::from_bits(0x7f7f_ffee);
        let mut halves = [0f32; 3];
        for face in &self.faces {
            let n1 = normalized(*face);
            let (low1, high1) = project(n1, &points);
            let e1 = subtract(high1, low1);
            let Some(planar) = Hull::build_planar(&points, n1) else { continue };
            for edge in &planar.faces {
                let n2 = normalized(*edge);
                let (low2, high2) = project(n2, &points);
                let e2 = subtract(high2, low2);
                let n3 = [
                    subtract(multiply(n1[2], n2[1]), multiply(n2[2], n1[1])),
                    subtract(multiply(n1[0], n2[2]), multiply(n2[0], n1[2])),
                    subtract(multiply(n1[1], n2[0]), multiply(n2[1], n1[0])),
                    0.,
                ];
                let (low3, high3) = project(n3, &points);
                let e3 = subtract(high3, low3);
                let area = add(add(multiply(e3, e2), multiply(e2, e1)), multiply(e3, e1));
                if area < best {
                    best = area;
                    halves = [multiply(e3, 0.5), multiply(e2, 0.5), multiply(e1, 0.5)];
                }
            }
        }
        // 140b90eb0: the minimum over the three dimensions.
        minps(minps(halves[0], halves[1]), halves[2])
    }
}
