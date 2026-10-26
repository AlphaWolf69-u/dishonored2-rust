//! c22f10's ordered visible-face walk and boundary-edge discovery.
use super::faces::{FaceEdge, PenetrationFace};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HorizonStatus {
    Complete,
    Invalid,
}

pub struct HorizonWorkspace {
    pub lower: f32,
    pub upper: f32,
    pub retired: Vec<usize>,
    pub boundary_links: Vec<Option<FaceEdge>>,
    pub boundary_count: usize,
    pub last_edge: Option<FaceEdge>,
}
impl HorizonWorkspace {
    pub fn new(vertex_count: usize, lower: f32, upper: f32) -> Self {
        Self {
            lower,
            upper,
            retired: Vec::new(),
            boundary_links: vec![None; vertex_count],
            boundary_count: 0,
            last_edge: None,
        }
    }

    /// The caller has already marked `start` visible. Face/edge order is part
    /// of the algorithm: later hull expansion follows the resulting vertex links.
    pub fn collect(
        &mut self,
        points: &[[f32; 4]],
        faces: &mut [PenetrationFace],
        start: usize,
        point: [f32; 4],
    ) -> Result<HorizonStatus, String> {
        self.collect_with_vertices(&|index| points.get(index).copied(), faces, start, point)
    }

    pub(super) fn collect_with_vertices(
        &mut self,
        points: &impl Fn(usize) -> Option<[f32; 4]>,
        faces: &mut [PenetrationFace],
        start: usize,
        point: [f32; 4],
    ) -> Result<HorizonStatus, String> {
        let mut status = HorizonStatus::Complete;
        for edge_index in 0..3 {
            let edge = faces
                .get(start)
                .ok_or("penetration horizon face missing")?
                .opposite[edge_index];
            let face = *faces
                .get(edge.face)
                .ok_or("penetration horizon adjacent face missing")?;
            let mut mark = face.mark;
            if mark == 0 {
                let vertex =
                    points(face.vertices[0]).ok_or("penetration horizon vertex missing")?;
                let terms: [f32; 3] =
                    std::array::from_fn(|i| (point[i] - vertex[i]) * face.normal[i]);
                let projection = (terms[1] + terms[0]) + terms[2];
                mark = 1;
                if !(projection > self.lower) {
                    let lower = projection + f32::from_bits(0x33d6_bf95);
                    if lower > self.lower {
                        self.lower = lower;
                    }
                    mark = 2;
                } else if !(projection >= self.upper) {
                    self.upper = projection;
                }
                if !(self.upper > self.lower) {
                    mark = 4;
                }
                faces[edge.face].mark = mark;
                if mark == 1 {
                    status = self.collect_with_vertices(points, faces, edge.face, point)?;
                    if status == HorizonStatus::Invalid {
                        break;
                    }
                    continue;
                }
                if mark == 4 {
                    status = HorizonStatus::Invalid;
                    break;
                }
            }
            if mark == 2 {
                let vertex = *face
                    .vertices
                    .get(edge.edge)
                    .ok_or("penetration horizon edge missing")?;
                *self
                    .boundary_links
                    .get_mut(vertex)
                    .ok_or("penetration horizon vertex link missing")? = Some(edge);
                self.boundary_count += 1;
                self.last_edge = Some(edge);
            }
        }
        faces
            .get_mut(start)
            .ok_or("penetration horizon face missing")?
            .distance = f32::from_bits(0x7f7f_ffee);
        if self.retired.len() >= 90 {
            return Ok(HorizonStatus::Invalid);
        }
        self.retired.push(start);
        Ok(status)
    }
}
