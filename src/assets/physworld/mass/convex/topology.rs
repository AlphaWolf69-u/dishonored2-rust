//! Three-dimensional incremental hull topology (140bad720 / 140bad050).
//! Host arenas replace SDK chunk pointers; linked traversal and edge ordering
//! remain explicit. All topology storage is construction-owned and transient.

#[derive(Clone, Copy, Debug)]
pub(super) struct Point {
    pub source: [f32; 4],
    pub integer: [i32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Edge {
    pub face: usize,
    pub edge: usize,
}
impl Edge {
    fn next(self) -> Self {
        Self {
            edge: (self.edge + 1) % 3,
            ..self
        }
    }
    fn previous(self) -> Self {
        Self {
            edge: (self.edge + 2) % 3,
            ..self
        }
    }
}

pub(super) struct Vertex {
    pub point: Point,
    pub index: usize,
    references: u32,
    previous: Option<usize>,
    pub next: Option<usize>,
}

pub(super) struct Face {
    pub vertices: [usize; 3],
    pub adjacent: [Option<Edge>; 3],
    pub normal: [i32; 3],
    pub distance: i64,
    pub index: usize,
    pub group: Option<usize>,
    marker: u32,
    previous: Option<usize>,
    pub next: Option<usize>,
}
impl Face {
    fn side(&self, point: [i32; 3]) -> i64 {
        i64::from(self.normal[1]) * i64::from(point[1])
            + i64::from(self.normal[2]) * i64::from(point[2])
            + i64::from(self.normal[0]) * i64::from(point[0])
            + self.distance
    }
}

fn plane(points: [[i32; 3]; 3]) -> ([i32; 3], i64) {
    let a: [i32; 3] = std::array::from_fn(|i| points[1][i] - points[0][i]);
    let b: [i32; 3] = std::array::from_fn(|i| points[2][i] - points[0][i]);
    let normal = [
        a[1] * b[2] - b[1] * a[2],
        a[2] * b[0] - b[2] * a[0],
        a[0] * b[1] - b[0] * a[1],
    ];
    let distance = -(i64::from(normal[2]) * i64::from(points[0][2])
        + i64::from(normal[0]) * i64::from(points[0][0])
        + i64::from(normal[1]) * i64::from(points[0][1]));
    (normal, distance)
}

pub(super) struct Topology {
    pub vertices: Vec<Option<Vertex>>,
    pub faces: Vec<Option<Face>>,
    pub first_vertex: Option<usize>,
    pub first_face: Option<usize>,
    free_vertices: Vec<usize>,
    free_faces: Vec<usize>,
    marker: u32,
}

impl Topology {
    /// Returns None for a domain with no non-coplanar tetrahedron. The outer
    /// builder, rather than this 3D routine, owns the original planar fallback.
    pub(super) fn spatial(points: &mut Vec<Point>) -> Result<Option<Self>, String> {
        let count = points.len();
        if count < 4 {
            return Err("spatial convex hull requires four input points".into());
        }
        // 140baba50's default build configuration sorts integer coordinates
        // with 140bace30 before selecting its first non-coplanar seed.
        crate::sorting::midpoint_by_key(points, |point| point.integer);
        let mut seed = None;
        'seed: for i in 0..count {
            for j in i + 1..count {
                for k in j + 1..count {
                    let (normal, distance) =
                        plane([points[i].integer, points[j].integer, points[k].integer]);
                    if normal == [0; 3] {
                        continue;
                    }
                    for l in k + 1..count {
                        let p = points[l].integer;
                        let side = (i64::from(normal[2]) * i64::from(p[2])
                            + i64::from(normal[1]) * i64::from(p[1])
                            + distance)
                            + i64::from(normal[0]) * i64::from(p[0]);
                        if side != 0 {
                            seed = Some(([i, j, k, l], side > 0));
                            break 'seed;
                        }
                    }
                }
            }
        }
        let Some((seed, reverse)) = seed else {
            return Ok(None);
        };
        let mut hull = Self {
            vertices: Vec::new(),
            faces: Vec::new(),
            first_vertex: None,
            first_face: None,
            free_vertices: Vec::new(),
            free_faces: Vec::new(),
            marker: 0,
        };
        // A triangulated convex hull has at most 2V-4 faces. Slots are reused
        // during construction, so retired faces do not grow a history array.
        let face_capacity = count
            .checked_mul(2)
            .ok_or("convex hull face capacity overflow")?;
        hull.vertices
            .try_reserve_exact(count)
            .map_err(|e| format!("allocating hull vertices: {e}"))?;
        hull.free_vertices
            .try_reserve_exact(count)
            .map_err(|e| format!("allocating hull vertex slots: {e}"))?;
        hull.faces
            .try_reserve_exact(face_capacity)
            .map_err(|e| format!("allocating hull faces: {e}"))?;
        hull.free_faces
            .try_reserve_exact(face_capacity)
            .map_err(|e| format!("allocating hull face slots: {e}"))?;
        let mut vertices = [0; 4];
        for (slot, index) in seed.into_iter().enumerate() {
            vertices[slot] = hull.add_vertex(points[index])?;
        }
        // The SDK removes the chosen candidates back to front by replacing
        // each with the last remaining point; retain that subsequent order.
        for index in seed.into_iter().rev() {
            points.swap_remove(index);
        }
        if reverse {
            vertices.swap(0, 1);
        }
        let [a, b, c, d] = vertices;
        let faces = [
            hull.add_face([a, b, c])?,
            hull.add_face([a, d, b])?,
            hull.add_face([a, c, d])?,
            hull.add_face([b, d, c])?,
        ];
        for (face, edges) in [
            [(1, 2), (3, 2), (2, 0)],
            [(2, 2), (3, 0), (0, 0)],
            [(0, 2), (3, 1), (1, 0)],
            [(1, 1), (2, 1), (0, 1)],
        ]
        .into_iter()
        .enumerate()
        {
            hull.faces[faces[face]].as_mut().unwrap().adjacent = edges.map(|(face, edge)| {
                Some(Edge {
                    face: faces[face],
                    edge,
                })
            });
        }
        while let Some(point) = points.pop() {
            let mut face = hull.first_face;
            while let Some(index) = face {
                let value = hull.faces[index]
                    .as_ref()
                    .ok_or("convex hull face link is retired")?;
                if value.side(point.integer) > 0 {
                    break;
                }
                face = value.next;
            }
            let Some(face) = face else { continue };
            let vertex = hull.add_vertex(point)?;
            hull.marker = hull.marker.wrapping_add(1);
            let mut boundary = None;
            hull.remove_visible(point.integer, Edge { face, edge: 0 }, &mut boundary)?;
            let mut start = boundary.ok_or("convex hull visible region has no boundary")?;
            for _ in 0..3 {
                if hull.adjacent(start)?.is_none() {
                    break;
                }
                start = start.next();
            }
            if hull.adjacent(start)?.is_some() {
                return Err("convex hull boundary is closed".into());
            }
            let mut boundary = start;
            let mut first: Option<usize> = None;
            let mut previous: Option<usize> = None;
            loop {
                boundary = boundary.next();
                while let Some(opposite) = hull.adjacent(boundary)? {
                    boundary = opposite.next();
                }
                let old = hull.faces[boundary.face]
                    .as_ref()
                    .ok_or("convex hull boundary face is retired")?;
                let face = hull.add_face([
                    vertex,
                    old.vertices[boundary.next().edge],
                    old.vertices[boundary.edge],
                ])?;
                hull.faces[boundary.face].as_mut().unwrap().adjacent[boundary.edge] =
                    Some(Edge { face, edge: 1 });
                hull.faces[face].as_mut().unwrap().adjacent[1] = Some(boundary);
                if let Some(previous) = previous {
                    hull.faces[previous].as_mut().unwrap().adjacent[0] =
                        Some(Edge { face, edge: 2 });
                    hull.faces[face].as_mut().unwrap().adjacent[2] = Some(Edge {
                        face: previous,
                        edge: 0,
                    });
                } else {
                    first = Some(face);
                }
                previous = Some(face);
                if boundary == start {
                    break;
                }
            }
            let first = first.ok_or("convex hull boundary produced no faces")?;
            let last = previous.ok_or("convex hull boundary produced no last face")?;
            hull.faces[last].as_mut().unwrap().adjacent[0] = Some(Edge {
                face: first,
                edge: 2,
            });
            hull.faces[first].as_mut().unwrap().adjacent[2] = Some(Edge {
                face: last,
                edge: 0,
            });
        }
        Ok(Some(hull))
    }

    fn adjacent(&self, edge: Edge) -> Result<Option<Edge>, String> {
        Ok(self
            .faces
            .get(edge.face)
            .and_then(Option::as_ref)
            .ok_or("convex hull edge refers to a retired face")?
            .adjacent[edge.edge])
    }

    fn add_vertex(&mut self, point: Point) -> Result<usize, String> {
        let vertex = Vertex {
            point,
            index: usize::MAX,
            references: 0,
            previous: None,
            next: self.first_vertex,
        };
        let index = if let Some(index) = self.free_vertices.pop() {
            self.vertices[index] = Some(vertex);
            index
        } else {
            self.vertices
                .try_reserve(1)
                .map_err(|e| format!("growing hull vertices: {e}"))?;
            self.vertices.push(Some(vertex));
            self.vertices.len() - 1
        };
        if let Some(first) = self.first_vertex {
            self.vertices[first].as_mut().unwrap().previous = Some(index);
        }
        self.first_vertex = Some(index);
        Ok(index)
    }

    fn add_face(&mut self, vertices: [usize; 3]) -> Result<usize, String> {
        let mut points = [[0; 3]; 3];
        for (corner, &index) in vertices.iter().enumerate() {
            points[corner] = self.vertices[index]
                .as_ref()
                .ok_or("convex hull face vertex is retired")?
                .point
                .integer;
        }
        let (normal, distance) = plane(points);
        let face = Face {
            vertices,
            adjacent: [None; 3],
            normal,
            distance,
            index: usize::MAX,
            group: None,
            marker: 0,
            previous: None,
            next: self.first_face,
        };
        let index = if let Some(index) = self.free_faces.pop() {
            self.faces[index] = Some(face);
            index
        } else {
            self.faces
                .try_reserve(1)
                .map_err(|e| format!("growing hull faces: {e}"))?;
            self.faces.push(Some(face));
            self.faces.len() - 1
        };
        for vertex in vertices {
            self.vertices[vertex].as_mut().unwrap().references += 1;
        }
        if let Some(first) = self.first_face {
            self.faces[first].as_mut().unwrap().previous = Some(index);
        }
        self.first_face = Some(index);
        Ok(index)
    }

    /// 140bad050 visits entry, next, previous edges, then removes the visible
    /// face. A zero plane distance is included in the removed region.
    fn remove_visible(
        &mut self,
        point: [i32; 3],
        entry: Edge,
        boundary: &mut Option<Edge>,
    ) -> Result<(), String> {
        let Some(face) = self.faces[entry.face].as_mut() else {
            return Ok(());
        };
        if face.marker == self.marker {
            return Ok(());
        }
        face.marker = self.marker;
        if face.side(point) < 0 {
            *boundary = Some(entry);
            return Ok(());
        }
        for edge in [entry, entry.next(), entry.previous()] {
            if let Some(adjacent) = self.adjacent(edge)? {
                self.remove_visible(point, adjacent, boundary)?;
            }
        }
        for edge in [entry, entry.next(), entry.previous()] {
            if let Some(adjacent) = self.adjacent(edge)? {
                self.faces[adjacent.face]
                    .as_mut()
                    .ok_or("convex hull adjacent face retired before unlink")?
                    .adjacent[adjacent.edge] = None;
            }
            self.faces[edge.face].as_mut().unwrap().adjacent[edge.edge] = None;
        }
        let face = self.faces[entry.face]
            .take()
            .ok_or("convex hull face retired twice")?;
        for index in face.vertices {
            let vertex = self.vertices[index]
                .as_mut()
                .ok_or("convex hull face references a retired vertex")?;
            vertex.references -= 1;
            if vertex.references == 0 {
                let vertex = self.vertices[index].take().unwrap();
                if let Some(next) = vertex.next {
                    self.vertices[next].as_mut().unwrap().previous = vertex.previous;
                }
                if let Some(previous) = vertex.previous {
                    self.vertices[previous].as_mut().unwrap().next = vertex.next;
                } else {
                    self.first_vertex = vertex.next;
                }
                self.free_vertices.push(index);
            }
        }
        if let Some(next) = face.next {
            self.faces[next].as_mut().unwrap().previous = face.previous;
        }
        if let Some(previous) = face.previous {
            self.faces[previous].as_mut().unwrap().next = face.next;
        } else {
            self.first_face = face.next;
        }
        self.free_faces.push(entry.face);
        Ok(())
    }
}
