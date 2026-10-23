//! Authored polytope query data, separate from movement's triangulated geometry.
//! Relative-array and connectivity fields use the file's inherited type schema.
use super::{float, le32, PhysWorld};

#[derive(Clone, Copy, Debug)]
pub struct PolytopeFace {
    pub first_index: u16,
    pub num_indices: u8,
    pub min_half_angle: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct PolytopeEdge {
    pub face: u16,
    pub edge: u8,
}

#[derive(Debug)]
pub struct PolytopeConnectivity {
    pub vertex_edges: Vec<PolytopeEdge>,
    pub face_links: Vec<PolytopeEdge>,
}

#[derive(Debug)]
pub struct ConvexPolytope {
    pub vertices: Vec<[f32; 4]>,
    pub planes: Vec<[f32; 4]>,
    pub faces: Vec<PolytopeFace>,
    pub indices: Vec<u8>,
    pub radius: f32,
    pub connectivity: Option<PolytopeConnectivity>,
}

impl ConvexPolytope {
    pub fn face_vertices(&self, face: usize) -> Result<&[u8], String> {
        let face = self
            .faces
            .get(face)
            .ok_or("polytope face index out of range")?;
        let first = usize::from(face.first_index);
        self.indices
            .get(first..first + usize::from(face.num_indices))
            .ok_or_else(|| "polytope face index span out of range".into())
    }
}

impl PhysWorld {
    /// Read only when a query owner needs it. Ordinary collision extraction
    /// retains its existing allocations and never duplicates this data.
    pub fn convex_polytope(&self, bytes: &[u8], index: usize) -> Result<ConvexPolytope, String> {
        let item = self.items.get(index).ok_or("polytope shape item missing")?;
        let kind = &self.types[item.type_index].name;
        if kind != "hknpConvexPolytopeShape" && kind != "hknpBoxShape" {
            return Err(format!("shape {index} is {kind}, not a convex polytope"));
        }
        let shape = self.item_at(bytes, index, 0)?;
        let reference = |name| -> Result<usize, String> {
            Ok(le32(shape, self.field_offset(item.type_index, name)?)? as usize)
        };
        let vectors = |reference: usize| -> Result<Vec<[f32; 4]>, String> {
            if reference == 0 {
                return Ok(Vec::new());
            }
            let item = self
                .items
                .get(reference)
                .ok_or("polytope vector item missing")?;
            let mut kind = item.type_index;
            let mut vector = false;
            for _ in 0..self.types.len() {
                let typ = self.types.get(kind).ok_or("polytope vector type missing")?;
                if typ.name == "hkVector4f" {
                    vector = true;
                    break;
                }
                if typ.parent == 0 {
                    break;
                }
                kind = typ.parent;
            }
            if !vector {
                return Err(format!(
                    "polytope vector item {reference} has type {}",
                    self.types[item.type_index].name
                ));
            }
            (0..item.count)
                .map(|element| {
                    let source = self.item_at(bytes, reference, element)?;
                    Ok([
                        float(source, 0)?,
                        float(source, 4)?,
                        float(source, 8)?,
                        float(source, 12)?,
                    ])
                })
                .collect()
        };
        let vertices = vectors(reference("vertices")?)?;
        let planes = vectors(reference("planes")?)?;
        let faces_ref = reference("faces")?;
        let face_item = self
            .items
            .get(faces_ref)
            .ok_or("polytope face item missing")?;
        let mut faces = Vec::with_capacity(face_item.count);
        for element in 0..face_item.count {
            let face = self.item_at(bytes, faces_ref, element)?;
            faces.push(PolytopeFace {
                first_index: u16::from_le_bytes(face[..2].try_into().unwrap()),
                num_indices: face[2],
                min_half_angle: face[3],
            });
        }
        let indices_ref = reference("indices")?;
        let index_item = self
            .items
            .get(indices_ref)
            .ok_or("polytope index item missing")?;
        let indices = (0..index_item.count)
            .map(|element| Ok(self.item_at(bytes, indices_ref, element)?[0]))
            .collect::<Result<Vec<_>, String>>()?;
        let connectivity_ref = reference("connectivity")?;
        let connectivity = if connectivity_ref == 0 {
            None
        } else {
            let item = self
                .items
                .get(connectivity_ref)
                .ok_or("polytope connectivity item missing")?;
            let data = self.item_at(bytes, connectivity_ref, 0)?;
            let edges = |name| -> Result<Vec<PolytopeEdge>, String> {
                let reference = le32(data, self.field_offset(item.type_index, name)?)? as usize;
                let edge_item = self
                    .items
                    .get(reference)
                    .ok_or("polytope edge item missing")?;
                (0..edge_item.count)
                    .map(|element| {
                        let edge = self.item_at(bytes, reference, element)?;
                        Ok(PolytopeEdge {
                            face: u16::from_le_bytes(edge[..2].try_into().unwrap()),
                            edge: edge[2],
                        })
                    })
                    .collect()
            };
            Some(PolytopeConnectivity {
                vertex_edges: edges("vertexEdges")?,
                face_links: edges("faceLinks")?,
            })
        };
        let result = ConvexPolytope {
            vertices,
            planes,
            faces,
            indices,
            connectivity,
            radius: float(shape, self.field_offset(item.type_index, "convexRadius")?)?,
        };
        for face in 0..result.faces.len() {
            for &vertex in result.face_vertices(face)? {
                if usize::from(vertex) >= result.vertices.len() {
                    return Err("polytope face references a missing vertex".into());
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "polytope/tests.rs"]
mod tests;
