//! Opt-in compound query geometry from the file's Havok type schema.
use super::{float, le32, PhysWorld};
pub mod types;
pub use types::CompoundInstance;
use types::{CompoundChildren, CompoundNode};

#[derive(Debug)]
pub struct CompoundShape {
    pub bounds: [[f32; 4]; 2],
    pub key_bits: u8,
    /// c971f0 exposes the shape's mutation signals only when this is nonzero.
    pub is_mutable: bool,
    pub instances: Vec<CompoundInstance>,
    pub nodes: Vec<CompoundNode>,
    pub root: u16,
}

impl PhysWorld {
    /// c971f0 uses the authored compound flag to expose mutation signals.
    pub fn compound_is_mutable(&self, bytes: &[u8], index: usize) -> Result<bool, String> {
        let item = self.items.get(index).ok_or("compound shape item missing")?;
        if self.types[item.type_index].name != "hknpCompoundShape" {
            return Err("query shape is not a compound".into());
        }
        let shape = self.item_at(bytes, index, 0)?;
        Ok(*shape
            .get(self.field_offset(item.type_index, "isMutable")?)
            .ok_or("compound mutability outside shape")?
            != 0)
    }
    pub fn compound_bounds(&self, bytes: &[u8], index: usize) -> Result<[[f32; 4]; 2], String> {
        let item = self.items.get(index).ok_or("compound shape item missing")?;
        if self.types[item.type_index].name != "hknpCompoundShape" {
            return Err("query shape is not a compound".into());
        }
        let shape = self.item_at(bytes, index, 0)?;
        let bounds_field = self.field(item.type_index, "aabb")?;
        let bounds_data = shape
            .get(bounds_field.offset..)
            .ok_or("compound AABB outside shape")?;
        let mut bounds = [[0.; 4]; 2];
        for (row, name) in bounds.iter_mut().zip(["min", "max"]) {
            let offset = self.field_offset(bounds_field.type_index, name)?;
            for (i, value) in row.iter_mut().enumerate() {
                *value = float(bounds_data, offset + i * 4)?;
            }
        }
        Ok(bounds)
    }
    /// Does not duplicate geometry during ordinary movement extraction.
    pub fn compound_shape(&self, bytes: &[u8], index: usize) -> Result<CompoundShape, String> {
        let item = self.items.get(index).ok_or("compound shape item missing")?;
        if self.types[item.type_index].name != "hknpCompoundShape" {
            return Err("query shape is not a compound".into());
        }
        let shape = self.item_at(bytes, index, 0)?;
        let bounds = self.compound_bounds(bytes, index)?;
        let is_mutable = self.compound_is_mutable(bytes, index)?;
        let list = le32(shape, self.field_offset(item.type_index, "instances")?)? as usize;
        let mut instances = Vec::new();
        if list != 0 {
            let item = self.items.get(list).ok_or("compound instances missing")?;
            if self.types[item.type_index].name != "hkFreeListArrayElement" {
                return Err("unexpected compound instance array type".into());
            }
            let transform_at = self.field_offset(item.type_index, "transform")?;
            let scale_at = self.field_offset(item.type_index, "scale")?;
            let shape_at = self.field_offset(item.type_index, "shape")?;
            let tag_at = self.field_offset(item.type_index, "shapeTag")?;
            for element in 0..item.count {
                let data = self.item_at(bytes, list, element)?;
                let mut transform = [[0.; 4]; 4];
                for (column, values) in transform.iter_mut().enumerate() {
                    for (lane, value) in values.iter_mut().enumerate() {
                        *value = float(data, transform_at + column * 16 + lane * 4)?;
                    }
                }
                instances.push(CompoundInstance {
                    transform,
                    flags: le32(data, transform_at + 0xc)?,
                    scale: [
                        float(data, scale_at)?,
                        float(data, scale_at + 4)?,
                        float(data, scale_at + 8)?,
                        float(data, scale_at + 12)?,
                    ],
                    shape: le32(data, shape_at)? as usize,
                    shape_tag: u16::from_le_bytes(
                        data.get(tag_at..tag_at + 2)
                            .ok_or("instance tag outside item")?
                            .try_into()
                            .unwrap(),
                    ),
                });
            }
        }
        let reference = le32(
            shape,
            self.field_offset(item.type_index, "boundingVolumeData")?,
        )? as usize;
        let item = self
            .items
            .get(reference)
            .ok_or("compound BVH data missing")?;
        if self.types[item.type_index].name != "hknpDynamicCompoundShapeData" {
            return Err("unexpected compound BVH data type".into());
        }
        let data = self.item_at(bytes, reference, 0)?;
        let field = self.field(item.type_index, "aabbTree")?;
        let tree = data
            .get(field.offset..)
            .ok_or("compound tree outside data")?;
        let root_offset = self.field_offset(field.type_index, "root")?;
        let root_data = tree
            .get(root_offset..root_offset + 2)
            .ok_or("compound root missing")?;
        let root = u16::from_le_bytes(root_data.try_into().unwrap());
        let reference = le32(tree, self.field_offset(field.type_index, "nodes")?)? as usize;
        let mut nodes = Vec::new();
        if reference != 0 {
            let item = self
                .items
                .get(reference)
                .ok_or("compound node array missing")?;
            if self.types[item.type_index].name != "hkcdDynamicTree::Codec32" {
                return Err("unexpected compound node codec".into());
            }
            for element in 0..item.count {
                let data = self.item_at(bytes, reference, element)?;
                let left = u16::from_le_bytes(data[0x1c..0x1e].try_into().unwrap());
                let right = u16::from_le_bytes(data[0x1e..0x20].try_into().unwrap());
                nodes.push(CompoundNode {
                    bounds: [
                        [float(data, 0)?, float(data, 4)?, float(data, 8)?],
                        [float(data, 16)?, float(data, 20)?, float(data, 24)?],
                    ],
                    children: if left == 0 {
                        CompoundChildren::Instance(right)
                    } else {
                        CompoundChildren::Branch { left, right }
                    },
                });
            }
        }
        let result = CompoundShape {
            bounds,
            is_mutable,
            key_bits: self.shape_header(bytes, index)?.key_bits,
            instances,
            nodes,
            root,
        };
        // Free-list nodes are intentionally retained but never interpreted as
        // live leaves. Validate only the actual root's reachable topology.
        let mut pending = if root == 0 { Vec::new() } else { vec![root] };
        let mut visited = vec![false; result.nodes.len()];
        while let Some(index) = pending.pop() {
            let index = usize::from(index);
            let node = result
                .nodes
                .get(index)
                .ok_or("compound node outside array")?;
            if std::mem::replace(&mut visited[index], true) {
                return Err("compound tree repeats a node".into());
            }
            match node.children {
                CompoundChildren::Branch { left, right } => pending.extend([left, right]),
                CompoundChildren::Instance(index) => {
                    if index > i16::MAX as u16 || usize::from(index) >= result.instances.len() {
                        return Err("compound leaf instance outside array".into());
                    }
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "compound/tests.rs"]
mod tests;
