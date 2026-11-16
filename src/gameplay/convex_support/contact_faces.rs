//! Contact face virtuals +50/+58/+60 (c82520/cb0640/cb07d0/cc77d0).
//! Inputs borrow the retained Havok topology and the installed half-angle table.
use super::{faces::reciprocal_sqrt_estimate, pair_distance::PairCache};
use crate::assets::physworld::polytope::{ConvexPolytope, PolytopeEdge, PolytopeFace};

#[derive(Clone, Copy)]
pub(crate) struct SelectedFace {
    pub index: u32,
    pub plane: [f32; 4],
    pub minimum_half_angle: u16,
}

fn dot(a: [f32; 4], b: [f32; 3]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}

/// c82520: plane and the face's encoded minimum half angle.
pub(crate) fn face(polytope: &ConvexPolytope, index: u32) -> Result<SelectedFace, String> {
    let descriptor = polytope
        .faces
        .get(index as usize)
        .ok_or("contact face missing")?;
    Ok(SelectedFace {
        index,
        plane: *polytope
            .planes
            .get(index as usize)
            .ok_or("contact face plane missing")?,
        minimum_half_angle: u16::from(descriptor.min_half_angle) << 8,
    })
}

/// cb0640: retain authored cyclic order; the clipping workspace handles padding.
pub(crate) fn vertices(
    polytope: &ConvexPolytope,
    index: u32,
    output: &mut [[f32; 4]; 16],
) -> Result<usize, String> {
    let indices = polytope.face_vertices(index as usize)?;
    if indices.len() > output.len() {
        return Err("contact face exceeds source 16-vertex workspace".into());
    }
    for (out, &index) in output.iter_mut().zip(indices) {
        *out = *polytope
            .vertices
            .get(index as usize)
            .ok_or("contact face vertex missing")?;
    }
    Ok(indices.len())
}

fn feature(cache: PairCache, target: bool) -> Result<([u8; 4], usize), String> {
    let query_count = usize::from(cache.0[4] >> 6);
    let target_count = usize::from(cache.0[4] & 3);
    if query_count == 0 || target_count == 0 || query_count + target_count > 4 {
        return Err("contact face received invalid GSK cache".into());
    }
    let count = if target { target_count } else { query_count };
    let offset = if target { query_count } else { 0 };
    let mut ids = [0; 4];
    ids[..count].copy_from_slice(&cache.0[offset..offset + count]);
    Ok((ids, count))
}

fn edge_indices(p: &ConvexPolytope, edge: PolytopeEdge) -> Result<(&[u8], usize), String> {
    let indices = p.face_vertices(usize::from(edge.face))?;
    let local = usize::from(edge.edge);
    if local >= indices.len() {
        return Err("contact connectivity edge out of bounds".into());
    }
    Ok((indices, local))
}

fn previous_link(p: &ConvexPolytope, edge: PolytopeEdge) -> Result<(u8, PolytopeEdge), String> {
    let (indices, local) = edge_indices(p, edge)?;
    let previous = (local + indices.len() - 1) % indices.len();
    let first = usize::from(p.faces[usize::from(edge.face)].first_index);
    let link = *p
        .connectivity
        .as_ref()
        .ok_or("contact connectivity missing")?
        .face_links
        .get(first + previous)
        .ok_or("contact face link missing")?;
    Ok((indices[previous], link))
}

fn edge_score(
    p: &ConvexPolytope,
    vertex: u8,
    other: u8,
    toward: [f32; 3],
    refined: bool,
) -> Result<f32, String> {
    let a = *p
        .vertices
        .get(usize::from(vertex))
        .ok_or("contact support vertex missing")?;
    let b = *p
        .vertices
        .get(usize::from(other))
        .ok_or("contact adjacent vertex missing")?;
    let v: [f32; 4] = std::array::from_fn(|i| a[i] - b[i]);
    let length = (v[1] * v[1] + v[0] * v[0]) + v[2] * v[2];
    let inverse = if refined {
        let r = reciprocal_sqrt_estimate(length);
        ((3. - r * length * r) * r) * 0.5
    } else if length <= 0. {
        0.
    } else {
        1. / length.sqrt()
    };
    Ok((inverse * v[1] * toward[1] + inverse * v[0] * toward[0]) + inverse * v[2] * toward[2])
}

/// cb2520's topology walk. None is the original ffffffff result, distinct from
/// malformed retained connectivity (an error, never an empty manifold).
fn connected_face(
    p: &ConvexPolytope,
    ids: [u8; 4],
    count: usize,
    toward: [f32; 3],
) -> Result<Option<u32>, String> {
    if count != 1 && count != 2 {
        return Ok(None);
    }
    let links = p
        .connectivity
        .as_ref()
        .ok_or("contact connectivity missing")?;
    let mut edge = *links
        .vertex_edges
        .get(usize::from(ids[0]))
        .ok_or("contact vertex edge missing")?;
    let (indices, local) = edge_indices(p, edge)?;
    let initial_successor = indices[(local + 1) % indices.len()];
    let mut selected = edge;
    if count == 1 {
        let mut best = edge_score(p, ids[0], initial_successor, toward, true)?;
        // The source circles this vertex. Bound malformed topology by the real
        // number of links rather than allocating a per-query visited set.
        let mut closed = false;
        for _ in 0..links.face_links.len() {
            let (other, next) = previous_link(p, edge)?;
            edge = next;
            let score = edge_score(p, ids[0], other, toward, true)?;
            if score < best {
                best = score;
                selected = edge;
            }
            if other == initial_successor {
                closed = true;
                break;
            }
        }
        if !closed {
            return Err("contact vertex connectivity does not close".into());
        }
    } else if initial_successor != ids[1] {
        let mut found = false;
        for _ in 0..links.face_links.len() {
            let (other, next) = previous_link(p, edge)?;
            edge = next;
            if other == initial_successor {
                return Ok(None);
            }
            if other == ids[1] {
                selected = edge;
                found = true;
                break;
            }
        }
        if !found {
            return Err("contact edge connectivity does not close".into());
        }
    }
    let descriptor = p
        .faces
        .get(usize::from(selected.face))
        .ok_or("contact selected face missing")?;
    let other = links
        .face_links
        .get(usize::from(descriptor.first_index) + usize::from(selected.edge))
        .ok_or("contact selected face link missing")?
        .face;
    let a = face(p, u32::from(selected.face))?;
    let b = face(p, u32::from(other))?;
    // cb2520 selects the linked face on a tie.
    Ok(Some(if dot(b.plane, toward) < dot(a.plane, toward) {
        a.index
    } else {
        b.index
    }))
}

/// cb07d0's cached face, connected support-feature path and four-lane scan.
pub(crate) fn select(
    p: &ConvexPolytope,
    shape_flags: u16,
    direction: [f32; 3],
    cache: PairCache,
    target: bool,
    previous: Option<u32>,
    half_angle_cosines: &[f32; 256],
) -> Result<SelectedFace, String> {
    let toward = direction.map(|v| -v);
    let aligned = |index| -> Result<bool, String> {
        let f = face(p, index)?;
        Ok(!(dot(f.plane, toward) < half_angle_cosines[usize::from(f.minimum_half_angle >> 8)]))
    };
    if let Some(previous) = previous {
        if aligned(previous)? {
            return face(p, previous);
        }
    }
    let (ids, count) = feature(cache, target)?;
    let connectivity_failed = if p.connectivity.is_some() {
        if let Some(index) = connected_face(p, ids, count, toward)? {
            return face(p, index);
        }
        true
    } else {
        false
    };
    if p.planes.len() < 4 {
        return Err("contact plane array lacks source four-plane scan window".into());
    }
    let tail = p.planes.len() - 4;
    let mut indices: [usize; 4] = std::array::from_fn(|i| tail + i);
    let mut scores: [f32; 4] = std::array::from_fn(|i| dot(p.planes[tail + i], toward));
    for first in (0..tail).step_by(4) {
        for lane in 0..4 {
            let score = dot(p.planes[first + lane], toward);
            if scores[lane] < score {
                scores[lane] = score;
                indices[lane] = first + lane;
            }
        }
    }
    let lane0 = if scores[0] < scores[1] { 1 } else { 0 };
    let lane2 = if scores[2] < scores[3] { 3 } else { 2 };
    let lane = if scores[lane0] < scores[lane2] {
        lane2
    } else {
        lane0
    };
    let mut index = indices[lane] as u32;
    if shape_flags & 0x80 == 0
        && !connectivity_failed
        && (previous == Some(index) || !aligned(index)?)
    {
        if count == 1 {
            let mut best_score = 1.;
            let mut best_alignment = 0.;
            for candidate in 0..p.faces.len() {
                let polygon = p.face_vertices(candidate)?;
                for edge in 0..polygon.len() {
                    let a = polygon[(edge + polygon.len() - 1) % polygon.len()];
                    let b = polygon[edge];
                    let other = if a == ids[0] {
                        b
                    } else if b == ids[0] {
                        a
                    } else {
                        continue;
                    };
                    let score = edge_score(p, ids[0], other, toward, false)?;
                    let alignment = dot(p.planes[candidate], toward);
                    if score < best_score || (score == best_score && best_alignment < alignment) {
                        best_score = score;
                        best_alignment = alignment;
                        index = candidate as u32;
                    }
                }
            }
        } else if count == 2 {
            let mut first = None;
            for candidate in 0..p.faces.len() {
                let polygon = p.face_vertices(candidate)?;
                if polygon.contains(&ids[0]) && polygon.contains(&ids[1]) {
                    if let Some(a) = first {
                        index = if dot(p.planes[a], toward) < dot(p.planes[candidate], toward) {
                            candidate
                        } else {
                            a
                        } as u32;
                        break;
                    }
                    first = Some(candidate);
                    index = candidate as u32;
                }
            }
        }
    }
    face(p, index)
}

/// cc77d0: the box's retained oriented-transform basis and exact axis tie table
/// (141e89850). This is a distinct virtual, not polytope's generic plane scan.
pub(crate) fn select_box(
    p: &ConvexPolytope,
    basis: [[f32; 4]; 3],
    direction: [f32; 3],
) -> Result<SelectedFace, String> {
    let projected = basis
        .map(|axis| (direction[0] * axis[0] + direction[1] * axis[1]) + direction[2] * axis[2]);
    let magnitude = projected.map(f32::abs);
    let max = magnitude[0].max(magnitude[1]).max(magnitude[2]);
    let mask = (0..3).fold(0, |mask, i| mask | (usize::from(max <= magnitude[i]) << i));
    let axis = [0, 0, 1, 1, 2, 2, 2, 2][mask];
    let index = axis * 2 | usize::from(projected[axis] >= 0.);
    let mut selected = face(p, index as u32)?;
    selected.minimum_half_angle = 0x8000;
    Ok(selected)
}

/// c8bed0/c8c070: native triangle's four-plane polytope header and its two
/// authored sides. Scratch holds only one decoded packed primitive, never an
/// independently retained collision mesh. Triangles repeat vertex0 in slot3.
pub(crate) struct TriangleFaces {
    shape: ConvexPolytope,
}
impl Default for TriangleFaces {
    fn default() -> Self {
        Self {
            shape: ConvexPolytope {
                vertices: vec![[0.; 4]; 4],
                planes: vec![[0.; 4]; 4],
                faces: vec![
                    PolytopeFace {
                        first_index: 0,
                        num_indices: 4,
                        min_half_angle: 0x7f,
                    },
                    PolytopeFace {
                        first_index: 4,
                        num_indices: 4,
                        min_half_angle: 0x7f,
                    },
                    PolytopeFace {
                        first_index: 0,
                        num_indices: 4,
                        min_half_angle: 0x7f,
                    },
                    PolytopeFace {
                        first_index: 0,
                        num_indices: 4,
                        min_half_angle: 0x7f,
                    },
                ],
                indices: vec![0, 1, 2, 3, 2, 1, 0, 3],
                radius: 0.,
                connectivity: None,
            },
        }
    }
}
impl TriangleFaces {
    pub(crate) fn prepare(
        &mut self,
        vertices: &[[f32; 4]],
        radius: f32,
    ) -> Result<&ConvexPolytope, String> {
        if vertices.len() != 3 && vertices.len() != 4 {
            return Err("triangle contact virtual requires three or four vertices".into());
        }
        self.shape.vertices[..vertices.len()].copy_from_slice(vertices);
        if vertices.len() == 3 {
            self.shape.vertices[3] = vertices[0];
        }
        self.shape.radius = radius;
        let a: [f32; 3] = std::array::from_fn(|i| vertices[1][i] - vertices[0][i]);
        let b: [f32; 3] = std::array::from_fn(|i| vertices[2][i] - vertices[0][i]);
        let cross = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let length = (cross[1] * cross[1] + cross[0] * cross[0]) + cross[2] * cross[2];
        let estimate = reciprocal_sqrt_estimate(length);
        let inverse = if length <= 0. {
            0.
        } else {
            (3. - estimate * length * estimate) * (estimate * 0.5)
        };
        let plane = [
            cross[0] * inverse,
            cross[1] * inverse,
            cross[2] * inverse,
            (0. - ((vertices[0][1] * cross[1] + vertices[0][0] * cross[0])
                + vertices[0][2] * cross[2]))
                * inverse,
        ];
        self.shape.planes[0] = plane;
        self.shape.planes[1] = plane.map(|v| -v);
        self.shape.planes[2] = plane;
        self.shape.planes[3] = plane;
        Ok(&self.shape)
    }
}
