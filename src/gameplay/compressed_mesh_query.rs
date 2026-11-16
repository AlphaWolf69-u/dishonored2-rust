//! 1276490/126c750 packed mesh traversal. Bounds admit actual retained leaves;
//! no mesh primitive becomes a body or a second collision world.
use super::{constraint_solver::reciprocal, ray_query::RayQueryControl};
use crate::assets::physworld::compressed_mesh::{CompressedMesh, PrimitiveKind};
pub mod triangle;

pub struct MeshLeaf<'a> {
    pub section: u32,
    pub primitive: u32,
    /// Section/primitive key before the triangle bit and parent prefix.
    pub key: u32,
    pub tag: u16,
    pub kind: PrimitiveKind,
    pub convex_kind: Option<u8>,
    pub radius: f32,
    pub vertices: &'a [[f32; 4]],
}
#[derive(Clone, Copy)]
pub struct MeshSweep {
    pub center: [f32; 3],
    pub half_extent: [f32; 3],
    pub direction: [f32; 3],
}
#[derive(Clone, Copy)]
struct Node {
    index: usize,
    bounds: [[f32; 4]; 2],
}
#[derive(Default)]
pub struct CompressedMeshQuery {
    sections: Vec<Node>,
    primitives: Vec<Node>,
    vertices: Vec<[f32; 4]>,
}
fn min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}
fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}
fn child_bounds(parent: [[f32; 4]; 2], bytes: [u8; 3]) -> [[f32; 4]; 2] {
    let scale: [f32; 3] =
        std::array::from_fn(|i| (parent[1][i] - parent[0][i]) * f32::from_bits(0x3b90_fdbc));
    [
        std::array::from_fn(|i| {
            if i < 3 {
                let q = (bytes[i] >> 4) as f32;
                q * q * scale[i] + parent[0][i]
            } else {
                0.
            }
        }),
        std::array::from_fn(|i| {
            if i < 3 {
                let q = (bytes[i] & 15) as f32;
                parent[1][i] - q * q * scale[i]
            } else {
                0.
            }
        }),
    ]
}
impl MeshSweep {
    fn interval(self, bounds: [[f32; 4]; 2], inverse: [f32; 3], limit: f32) -> [f32; 2] {
        let lower: [f32; 3] = std::array::from_fn(|i| {
            ((bounds[0][i] - self.half_extent[i]) - self.center[i]) * inverse[i]
        });
        let upper: [f32; 3] = std::array::from_fn(|i| {
            ((self.half_extent[i] + bounds[1][i]) - self.center[i]) * inverse[i]
        });
        let near: [f32; 3] = std::array::from_fn(|i| min(lower[i], upper[i]));
        let far: [f32; 3] = std::array::from_fn(|i| max(lower[i], upper[i]));
        [
            max(max(near[2], max(near[1], near[0])), 0.),
            min(min(far[2], min(far[1], far[0])), limit),
        ]
    }
}
impl CompressedMeshQuery {
    /// Both node queues and the decoded leaf buffer remain with the executor.
    /// The callback's leaf borrow ends before another primitive is decoded.
    pub fn visit_swept(
        &mut self,
        mesh: &CompressedMesh,
        sweep: MeshSweep,
        parent_radius: f32,
        control: &mut RayQueryControl,
        submit: &mut dyn FnMut(MeshLeaf<'_>, &mut RayQueryControl) -> Result<(), String>,
    ) -> Result<(), String> {
        self.sections.clear();
        self.primitives.clear();
        if mesh.top_node_count() == 0 {
            return Ok(());
        }
        let inverse_scale = reciprocal(mesh.scale());
        let sweep = MeshSweep {
            center: sweep.center.map(|v| v * inverse_scale),
            half_extent: sweep.half_extent.map(|v| v * inverse_scale),
            direction: sweep.direction.map(|v| v * inverse_scale),
        };
        let inverse = sweep.direction.map(|v| {
            if v == 0. {
                f32::from_bits(0x7f7f_ffee)
            } else {
                reciprocal(v)
            }
        });
        let root = Node {
            index: 0,
            bounds: mesh.domain(),
        };
        let interval = sweep.interval(root.bounds, inverse, control.fraction);
        if !(interval[0] <= interval[1]) {
            return Ok(());
        }
        self.sections.push(root);
        while let Some(mut current) = self.sections.pop() {
            loop {
                let node = mesh.top_node(current.index)?;
                if node[3] & 0x80 == 0 {
                    let section = ((u32::from(node[3]) << 8) | u32::from(node[4])) as usize;
                    self.visit_section(
                        mesh,
                        section,
                        sweep,
                        inverse,
                        parent_radius,
                        control,
                        submit,
                    )?;
                    break;
                }
                let offset =
                    (((usize::from(node[3] & 0x7f) << 8) | usize::from(node[4])) * 2) as usize;
                if offset == 0 {
                    return Err("compressed section tree has zero child offset".into());
                }
                let left = current.index + 1;
                let right = current.index + offset;
                let l = mesh.top_node(left)?;
                let r = mesh.top_node(right)?;
                let left = Node {
                    index: left,
                    bounds: child_bounds(current.bounds, l[..3].try_into().unwrap()),
                };
                let right = Node {
                    index: right,
                    bounds: child_bounds(current.bounds, r[..3].try_into().unwrap()),
                };
                let li = sweep.interval(left.bounds, inverse, control.fraction);
                let ri = sweep.interval(right.bounds, inverse, control.fraction);
                match (!(li[1] < li[0]), !(ri[1] < ri[0])) {
                    (true, true) => {
                        if ri[0] < li[0] {
                            self.sections.push(left);
                            current = right;
                        } else {
                            self.sections.push(right);
                            current = left;
                        }
                    }
                    (true, false) => current = left,
                    (false, true) => current = right,
                    _ => break,
                }
            }
            if control.early_out {
                break;
            }
        }
        Ok(())
    }
    fn visit_section(
        &mut self,
        mesh: &CompressedMesh,
        section: usize,
        sweep: MeshSweep,
        inverse: [f32; 3],
        parent_radius: f32,
        control: &mut RayQueryControl,
        submit: &mut dyn FnMut(MeshLeaf<'_>, &mut RayQueryControl) -> Result<(), String>,
    ) -> Result<(), String> {
        if mesh.node_count(section)? == 0 {
            return Ok(());
        }
        let root = Node {
            index: 0,
            bounds: mesh.section_bounds(section)?,
        };
        let interval = sweep.interval(root.bounds, inverse, control.fraction);
        if !(interval[0] <= interval[1]) {
            return Ok(());
        }
        self.primitives.clear();
        self.primitives.push(root);
        while let Some(mut current) = self.primitives.pop() {
            loop {
                let node = mesh.node(section, current.index)?;
                if node[3] & 1 == 0 {
                    let primitive = usize::from(node[3] >> 1);
                    let kind = mesh.vertices(section, primitive, &mut self.vertices)?;
                    if kind != PrimitiveKind::Empty {
                        // The temporary native convex interface writes support IDs
                        // into W after decoding, before any GSK query.
                        for (index, vertex) in self.vertices.iter_mut().enumerate() {
                            vertex[3] = f32::from_bits(0x3f00_0000 | index as u32);
                        }
                        submit(
                            MeshLeaf {
                                section: section as u32,
                                primitive: primitive as u32,
                                key: ((section as u32) << 8) | ((primitive as u32) << 1),
                                tag: mesh.primitive_tag(section, primitive)?,
                                kind,
                                convex_kind: mesh.convex_kind(section, primitive)?,
                                radius: mesh.primitive_radius(section, primitive, parent_radius)?,
                                vertices: &self.vertices,
                            },
                            control,
                        )?;
                    }
                    break;
                }
                let offset = usize::from(node[3] & 0xfe);
                if offset == 0 {
                    return Err("compressed primitive tree has zero child offset".into());
                }
                let left = current.index + 1;
                let right = current.index + offset;
                let l = mesh.node(section, left)?;
                let r = mesh.node(section, right)?;
                let left = Node {
                    index: left,
                    bounds: child_bounds(current.bounds, l[..3].try_into().unwrap()),
                };
                let right = Node {
                    index: right,
                    bounds: child_bounds(current.bounds, r[..3].try_into().unwrap()),
                };
                let li = sweep.interval(left.bounds, inverse, control.fraction);
                let ri = sweep.interval(right.bounds, inverse, control.fraction);
                match (!(li[1] < li[0]), !(ri[1] < ri[0])) {
                    (true, true) => {
                        if ri[0] < li[0] {
                            self.primitives.push(left);
                            current = right;
                        } else {
                            self.primitives.push(right);
                            current = left;
                        }
                    }
                    (true, false) => current = left,
                    (false, true) => current = right,
                    _ => break,
                }
            }
            if control.early_out {
                break;
            }
        }
        Ok(())
    }
}
