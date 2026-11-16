//! Retained hknpCompressedMeshShape tables. Primitive decoding is shared with
//! 127f590/127f030; no navigation triangles or render meshes become colliders.
use super::{float, le32, source::ShapeSource, CompressedPrimitiveKind};
use std::sync::Arc;

pub struct CompressedMesh {
    source: Arc<ShapeSource>,
    pub shape_item: usize,
    tables: [usize; 6],
    top_nodes: usize,
    domain: [[f32; 3]; 2],
    pub(crate) scale: f32,
}
impl std::fmt::Debug for CompressedMesh {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompressedMesh")
            .field("shape_item", &self.shape_item)
            .field("tables", &self.tables)
            .field("domain", &self.domain)
            .finish()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveKind {
    Empty,
    Triangle,
    Quad,
    Convex,
}
impl CompressedMesh {
    pub(crate) fn new(source: Arc<ShapeSource>, shape_item: usize) -> Result<Self, String> {
        let world = &source.physics;
        let (tree, ty) = world.compressed_tree(&source.bytes, shape_item)?;
        let mut tables = [0; 6];
        for (slot, name) in tables.iter_mut().zip([
            "sections",
            "primitives",
            "sharedVerticesIndex",
            "packedVertices",
            "sharedVertices",
            "primitiveDataRuns",
        ]) {
            *slot = le32(tree, world.field_offset(ty, name)?)? as usize;
        }
        let (low, high) = world.compressed_domain(tree, ty)?;
        let top_nodes = le32(tree, world.field_offset(ty, "nodes")?)? as usize;
        Ok(Self {
            source,
            shape_item,
            tables,
            top_nodes,
            domain: [low, high],
            scale: 1.,
        })
    }
    fn item(&self, table: usize, element: usize) -> Result<&[u8], String> {
        self.source
            .physics
            .item_at(&self.source.bytes, table, element)
    }
    fn count(&self, table: usize) -> usize {
        self.source
            .physics
            .items
            .get(table)
            .map_or(0, |item| item.count)
    }
    pub fn bounds(&self) -> [[f32; 4]; 2] {
        self.domain
            .map(|v| [v[0] * self.scale, v[1] * self.scale, v[2] * self.scale, 0.])
    }
    pub fn scale(&self) -> f32 {
        self.scale
    }
    /// The meshTree's Codec3Axis5 section tree, retained in its source asset.
    pub fn top_node_count(&self) -> usize {
        self.count(self.top_nodes)
    }
    pub fn top_node(&self, index: usize) -> Result<[u8; 5], String> {
        Ok(self.item(self.top_nodes, index)?[..5].try_into().unwrap())
    }
    pub fn domain(&self) -> [[f32; 4]; 2] {
        self.domain.map(|v| [v[0], v[1], v[2], 0.])
    }

    /// 127f590: compressed convex headers can override the parent's radius
    /// with an upper-half float. Triangle/quad leaves retain the mesh radius.
    pub fn primitive_radius(
        &self,
        section: usize,
        index: usize,
        parent_radius: f32,
    ) -> Result<f32, String> {
        if self.primitive_kind(section, index)? != PrimitiveKind::Convex {
            return Ok(parent_radius * self.scale);
        }
        let data = self.section(section)?;
        let primitive = self.primitive(section, index)?;
        let header = le32(data, 0x4c)? as usize;
        let packed = data[0x58] as usize;
        let local = primitive[0] as usize;
        if local < packed {
            return Err("compressed convex header references packed vertex".into());
        }
        let at = (header >> 8) + local - packed;
        let flags = u16::from_le_bytes(self.item(self.tables[2], at)?[..2].try_into().unwrap());
        if flags & 0xc0 == 0 {
            return Ok(parent_radius * self.scale);
        }
        let bits =
            u16::from_le_bytes(self.item(self.tables[2], at + 2)?[..2].try_into().unwrap()) as u32;
        Ok(f32::from_bits(bits << 16) * self.scale)
    }
    pub fn section_count(&self) -> usize {
        self.count(self.tables[0])
    }
    pub fn section(&self, index: usize) -> Result<&[u8], String> {
        self.item(self.tables[0], index)
    }
    pub fn primitive_count(&self, section: usize) -> Result<usize, String> {
        Ok((le32(self.section(section)?, 0x50)? & 255) as usize)
    }
    pub fn node_count(&self, section: usize) -> Result<usize, String> {
        Ok(self.count(le32(self.section(section)?, 0)? as usize))
    }
    pub fn node(&self, section: usize, index: usize) -> Result<[u8; 4], String> {
        Ok(
            self.item(le32(self.section(section)?, 0)? as usize, index)?[..4]
                .try_into()
                .unwrap(),
        )
    }
    pub fn section_bounds(&self, section: usize) -> Result<[[f32; 4]; 2], String> {
        let data = self.section(section)?;
        let mut result = [[0.; 4]; 2];
        for side in 0..2 {
            for axis in 0..4 {
                result[side][axis] = float(data, 0x10 + side * 16 + axis * 4)?;
            }
        }
        Ok(result)
    }
    pub fn node_bounds(&self, section: usize, target: usize) -> Result<[[f32; 4]; 2], String> {
        let mut bounds = self.section_bounds(section)?;
        let mut current = 0;
        while current != target {
            let node = self.node(section, current)?;
            let right = current + (node[3] & 0xfe) as usize;
            let next = if right <= target { right } else { current + 1 };
            if next <= current || next > target {
                return Err("compressed tree has invalid child offset".into());
            }
            bounds = super::compressed_tree_child_bounds(bounds, self.node(section, next)?);
            current = next;
        }
        Ok(bounds)
    }
    pub fn primitive(&self, section: usize, index: usize) -> Result<[u8; 4], String> {
        let header = le32(self.section(section)?, 0x50)? as usize;
        if index >= (header & 255) {
            return Err("compressed primitive outside section".into());
        }
        Ok(self.item(self.tables[1], (header >> 8) + index)?[..4]
            .try_into()
            .unwrap())
    }
    pub fn primitive_kind(&self, section: usize, index: usize) -> Result<PrimitiveKind, String> {
        Ok(
            match super::compressed_primitive_kind(&self.primitive(section, index)?) {
                CompressedPrimitiveKind::Empty => PrimitiveKind::Empty,
                CompressedPrimitiveKind::Triangle => PrimitiveKind::Triangle,
                CompressedPrimitiveKind::Quad => PrimitiveKind::Quad,
                CompressedPrimitiveKind::Convex => PrimitiveKind::Convex,
            },
        )
    }
    /// The low header nibble selects sphere (0), capsule (1), or convex (2).
    pub fn convex_kind(&self, section: usize, index: usize) -> Result<Option<u8>, String> {
        if self.primitive_kind(section, index)? != PrimitiveKind::Convex {
            return Ok(None);
        }
        let data = self.section(section)?;
        let primitive = self.primitive(section, index)?;
        let packed = data[0x58] as usize;
        let local = primitive[0] as usize;
        if local < packed {
            return Err("compressed convex header references packed vertex".into());
        }
        let at = (le32(data, 0x4c)? as usize >> 8) + local - packed;
        let header = u16::from_le_bytes(self.item(self.tables[2], at)?[..2].try_into().unwrap());
        Ok(Some((header & 15) as u8))
    }
    /// 124e030: retain section-local material/tag runs, including first-match
    /// semantics, rather than assigning a triangle material during extraction.
    pub fn primitive_tag(&self, section: usize, index: usize) -> Result<u16, String> {
        if index >= self.primitive_count(section)? {
            return Err("compressed primitive outside section".into());
        }
        let header = le32(self.section(section)?, 0x54)? as usize;
        for run in header >> 8..(header >> 8) + (header & 255) {
            let data = self.item(self.tables[5], run)?;
            if (data[2] as usize..data[2] as usize + data[3] as usize).contains(&index) {
                return Ok(u16::from_le_bytes(data[..2].try_into().unwrap()));
            }
        }
        Err("compressed primitive has no data run".into())
    }
    /// Decode just one actual leaf into the caller's reusable scratch. Quads
    /// remain four vertices; convex headers retain their source codec and
    /// support vertices, without triangulating or constructing a fake hull.
    pub fn vertices(
        &self,
        section_index: usize,
        primitive_index: usize,
        out: &mut Vec<[f32; 4]>,
    ) -> Result<PrimitiveKind, String> {
        out.clear();
        let section = self.section(section_index)?;
        let primitive = self.primitive(section_index, primitive_index)?;
        let kind = self.primitive_kind(section_index, primitive_index)?;
        let packed_count = section[0x58] as usize;
        let shared_count = section[0x59] as usize;
        let first_shared = (le32(section, 0x4c)? >> 8) as usize;
        let push = |out: &mut Vec<[f32; 4]>, v: [f32; 3]| {
            out.push([v[0] * self.scale, v[1] * self.scale, v[2] * self.scale, 0.])
        };
        if kind == PrimitiveKind::Empty {
            return Ok(kind);
        }
        if kind == PrimitiveKind::Convex {
            let local = primitive[0] as usize;
            if local < packed_count || local + 1 >= packed_count + shared_count {
                return Err("compressed convex header outside shared table".into());
            }
            let at = first_shared + local - packed_count;
            let header =
                u16::from_le_bytes(self.item(self.tables[2], at)?[..2].try_into().unwrap());
            let start =
                u16::from_le_bytes(self.item(self.tables[2], at + 1)?[..2].try_into().unwrap())
                    as usize;
            let count = ((header >> 8) as usize).min(0x70);
            let codec = (header >> 4) & 3;
            if header & 15 > 2 || codec > 2 || count < usize::from((header & 15) + 1) {
                return Err("unsupported compressed convex codec".into());
            }
            let domain = self.node_bounds(section_index, primitive[1] as usize)?;
            let [low, high] = if codec == 0 {
                self.domain
            } else {
                domain.map(|v| [v[0], v[1], v[2]])
            };
            let divisor = match codec {
                0 => [1. / 2097151., 1. / 2097151., 1. / 4194303.],
                1 => [
                    f32::from_bits(0x3a001002),
                    f32::from_bits(0x3a001002),
                    f32::from_bits(0x3a802008),
                ],
                2 => [
                    f32::from_bits(0x3d042108),
                    f32::from_bits(0x3d042108),
                    f32::from_bits(0x3c820821),
                ],
                _ => unreachable!(),
            };
            for vertex in 0..count {
                let q = match codec {
                    0 => {
                        let v = u64::from_le_bytes(
                            self.item(self.tables[4], start + vertex)?[..8]
                                .try_into()
                                .unwrap(),
                        );
                        [
                            (v & 0x1fffff) as f32,
                            ((v >> 21) & 0x1fffff) as f32,
                            (v >> 42) as f32,
                        ]
                    }
                    1 => {
                        let word = start * 2 + (vertex ^ 1);
                        let v = le32(self.item(self.tables[4], word / 2)?, (word & 1) * 4)?;
                        [
                            (v & 0x7ff) as f32,
                            ((v >> 11) & 0x7ff) as f32,
                            (v >> 22) as f32,
                        ]
                    }
                    2 => {
                        let word = start * 4 + (vertex ^ 3);
                        let data = self.item(self.tables[4], word / 4)?;
                        let v = u16::from_le_bytes(
                            data[(word & 3) * 2..(word & 3) * 2 + 2].try_into().unwrap(),
                        );
                        [(v & 31) as f32, ((v >> 5) & 31) as f32, (v >> 10) as f32]
                    }
                    _ => unreachable!(),
                };
                push(
                    out,
                    std::array::from_fn(|axis| {
                        q[axis] * (high[axis] - low[axis]) * divisor[axis] + low[axis]
                    }),
                );
            }
        } else {
            let first_packed = le32(section, 0x48)? as usize;
            let base = [
                float(section, 0x30)?,
                float(section, 0x34)?,
                float(section, 0x38)?,
            ];
            let scale = [
                float(section, 0x3c)?,
                float(section, 0x40)?,
                float(section, 0x44)?,
            ];
            for &corner in &primitive[..if kind == PrimitiveKind::Quad { 4 } else { 3 }] {
                let index = corner as usize;
                let v = if index < packed_count {
                    let v = le32(self.item(self.tables[3], first_packed + index)?, 0)?;
                    [
                        base[0] + (v & 0x7ff) as f32 * scale[0],
                        base[1] + ((v >> 11) & 0x7ff) as f32 * scale[1],
                        base[2] + (v >> 22) as f32 * scale[2],
                    ]
                } else {
                    if index >= packed_count + shared_count {
                        return Err("compressed corner outside shared table".into());
                    }
                    let at = u16::from_le_bytes(
                        self.item(self.tables[2], first_shared + index - packed_count)?[..2]
                            .try_into()
                            .unwrap(),
                    ) as usize;
                    let v =
                        u64::from_le_bytes(self.item(self.tables[4], at)?[..8].try_into().unwrap());
                    let q = [
                        (v & 0x1fffff) as f32 / 2097151.,
                        ((v >> 21) & 0x1fffff) as f32 / 2097151.,
                        (v >> 42) as f32 / 4194303.,
                    ];
                    std::array::from_fn(|axis| {
                        self.domain[0][axis]
                            + q[axis] * (self.domain[1][axis] - self.domain[0][axis])
                    })
                };
                push(out, v);
            }
        }
        Ok(kind)
    }
}
