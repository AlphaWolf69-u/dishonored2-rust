//! c246a0's bounded penetration hull expansion and face reuse.
use super::{
    faces::{face_geometry, face_plane, tetrahedron_faces, FaceEdge, PenetrationFace},
    horizon::{HorizonStatus, HorizonWorkspace},
    penetration::ConvexSupportPoint,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpansionOutcome {
    Continue,
    Stopped(u32),
}

pub struct PenetrationHull {
    pub vertices: Vec<ConvexSupportPoint>,
    /// Search/reset prefix. A staged vertex may already occupy the following
    /// storage slot; callers publish the count in the same order as the solver.
    pub vertex_count: usize,
    pub faces: Vec<PenetrationFace>,
    pub horizon: HorizonWorkspace,
}
fn dot(a: [f32; 4], b: [f32; 4]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}
fn empty_face() -> PenetrationFace {
    PenetrationFace {
        vertices: [0; 3],
        opposite: [FaceEdge { face: 0, edge: 0 }; 3],
        normal: [0.; 4],
        distance: 0.,
        mark: 0,
    }
}
impl PenetrationHull {
    pub fn new(vertices: [ConvexSupportPoint; 4]) -> Self {
        Self {
            faces: tetrahedron_faces(vertices.map(|v| v.difference)).to_vec(),
            vertices: vertices.to_vec(),
            vertex_count: 4,
            horizon: HorizonWorkspace::new(4, 0., 0.),
        }
    }
    pub fn expand(
        &mut self,
        candidate: usize,
        start: Option<usize>,
        direction: [f32; 4],
        tolerance: [f32; 4],
    ) -> Result<ExpansionOutcome, String> {
        use ExpansionOutcome::*;
        if self.vertex_count > self.vertices.len() || self.faces.len() > 90 {
            return Err("invalid penetration hull counts".into());
        }
        let point = self
            .vertices
            .get(candidate)
            .ok_or("penetration candidate missing")?
            .difference;
        self.horizon
            .boundary_links
            .resize(self.vertices.len(), None);
        let start = if let Some(start) = start {
            let face = self
                .faces
                .get(start)
                .ok_or("penetration expansion face missing")?;
            for i in 0..3 {
                let vertex = self
                    .vertices
                    .get(face.vertices[i])
                    .ok_or("penetration face vertex missing")?
                    .difference;
                if dot(std::array::from_fn(|j| point[j] - vertex[j]), face.normal) < tolerance[i] {
                    return Ok(Stopped(1));
                }
            }
            start
        } else {
            let mut best = None;
            let mut maximum = -f32::from_bits(0x7f7f_ffee);
            for (index, vertex) in self.vertices[..self.vertex_count].iter().enumerate() {
                let value = dot(vertex.difference, direction);
                if maximum < value {
                    maximum = value;
                    best = Some(index);
                }
            }
            if dot(direction, point) - tolerance[0] < maximum {
                return Ok(Stopped(1));
            }
            let best = best.ok_or("penetration support search found no finite maximum")?;
            self.faces
                .iter()
                .position(|face| face.vertices.contains(&best))
                .ok_or("penetration support vertex has no face")?
        };
        let snapshot = self.faces[start];
        let previous_available = self.horizon.retired.len() + 90 - self.faces.len();
        self.horizon.lower = f32::from_bits(0x33d6_bf95);
        self.horizon.upper = f32::from_bits(0x7f7f_ffee);
        self.horizon.boundary_count = 0;
        self.horizon.last_edge = None;
        let vertex = self
            .vertices
            .get(snapshot.vertices[0])
            .ok_or("penetration face vertex missing")?
            .difference;
        let projection = dot(
            std::array::from_fn(|i| point[i] - vertex[i]),
            snapshot.normal,
        );
        let epsilon = f32::from_bits(0x33d6_bf95);
        let mut mark = 1;
        if !(projection > epsilon) {
            if projection + epsilon > epsilon {
                self.horizon.lower = projection + epsilon;
            }
            mark = 2;
        } else if !(projection >= self.horizon.upper) {
            self.horizon.upper = projection;
        }
        if !(self.horizon.upper > self.horizon.lower) {
            mark = 4;
        }
        self.faces[start].mark = mark;
        if mark != 1 {
            return Ok(Stopped(7));
        }
        let vertices = &self.vertices;
        if self.horizon.collect_with_vertices(
            &|index| vertices.get(index).map(|v| v.difference),
            &mut self.faces,
            start,
            point,
        )? != HorizonStatus::Complete
        {
            return Ok(Stopped(7));
        }
        let count = self.horizon.boundary_count;
        if self.horizon.retired.len() + 90 - self.faces.len() < count {
            while self.horizon.retired.len() > previous_available {
                let index = self.horizon.retired.pop().unwrap();
                let (normal, distance) = face_plane(
                    self.faces[index]
                        .vertices
                        .map(|i| self.vertices[i].difference),
                );
                self.faces[index].normal = normal;
                self.faces[index].distance = distance;
            }
            self.faces[start] = snapshot;
            return Ok(Stopped(3));
        }
        if count > 90 {
            return Err("penetration boundary exceeds face capacity".into());
        }
        let mut allocated = [0usize; 90];
        for slot in &mut allocated[..count] {
            if let Some(index) = self.horizon.retired.pop() {
                *slot = index;
            } else {
                *slot = self.faces.len();
                self.faces.push(empty_face());
            }
        }
        let first = self.horizon.last_edge;
        let mut edge = first;
        for i in 0..count {
            let Some(boundary) = edge else {
                return Ok(Stopped(7));
            };
            let old = *self
                .faces
                .get(boundary.face)
                .ok_or("penetration boundary face missing")?;
            if boundary.edge >= 3 {
                return Err("penetration boundary edge out of range".into());
            }
            let a = old.vertices[(boundary.edge + 1) % 3];
            let b = old.vertices[boundary.edge];
            let index = allocated[i];
            let next = allocated[(i + 1) % count];
            self.faces[index].vertices = [a, b, candidate];
            self.faces[boundary.face].opposite[boundary.edge] = FaceEdge {
                face: index,
                edge: 0,
            };
            self.faces[index].opposite[0] = boundary;
            self.faces[index].opposite[2] = FaceEdge {
                face: next,
                edge: 1,
            };
            self.faces[next].opposite[1] = FaceEdge {
                face: index,
                edge: 2,
            };
            let (normal, distance, squared) = face_geometry([
                self.vertices[a].difference,
                self.vertices[b].difference,
                point,
            ]);
            if !(squared > 0.) {
                return Ok(Stopped(7));
            }
            self.faces[index].normal = normal;
            self.faces[index].distance = distance;
            edge = self
                .horizon
                .boundary_links
                .get_mut(a)
                .ok_or("penetration boundary vertex missing")?
                .take();
        }
        for face in &mut self.faces {
            face.mark = 0;
        }
        self.horizon.boundary_links[..self.vertex_count].fill(None);
        Ok(if first == edge { Continue } else { Stopped(7) })
    }
}
