//! Construct the one-instance physhape compound from 14008c9a0.
//! The caller supplies each actual resource identity, outermost first, and
//! keeps its resource lease alongside the resulting geometry.
use super::{QueryGeometry, QueryShape, QueryShapes};
use crate::assets::physworld::{
    compound::{CompoundInstance, CompoundShape, types::{CompoundChildren, CompoundNode}},
    shape::ShapeHeader,
};
use crate::gameplay::shape_transform::ShapeMatrix;

pub(crate) struct AliasInstance {
    pub scale: f32,
    pub resource_identity: u64,
}

impl QueryShapes {
    /// `140808ea0` creates one compound with a primary and an extra-HD
    /// child. `140c98040` gives two instances two key bits. `141232e80`
    /// inserts primary first and extra second; `14123f5c0` does not rebuild
    /// a tree with fewer than three leaves. Internal node indices may differ,
    /// but the child order, bounds and shape keys are retained.
    pub(crate) fn compound_pair(mut self, mut extra: QueryShapes, tags: [u16; 2]) -> Result<Self, String> {
        let identity: ShapeMatrix = [[1., 0., 0., 0.], [0., 1., 0., 0.],
            [0., 0., 1., 0.], [0., 0., 0., 1.]];
        let primary_root = self.root;
        let primary_bounds = self.bounds(primary_root, identity)?;
        let extra_bounds = extra.bounds(extra.root, identity)?;
        let offset = self.shapes.keys().next_back().copied().unwrap_or(0)
            .checked_add(1).ok_or("compound shape index overflow")?;
        let extra_root = extra.root.checked_add(offset).ok_or("compound child index overflow")?;
        for (item, mut shape) in std::mem::take(&mut extra.shapes) {
            if let QueryGeometry::Compound(compound) = &mut shape.geometry {
                for instance in &mut compound.instances {
                    instance.shape = instance.shape.checked_add(offset).ok_or("compound child index overflow")?;
                }
            }
            let item = item.checked_add(offset).ok_or("compound shape index overflow")?;
            if self.shapes.insert(item, shape).is_some() { return Err("compound shape index collision".into()); }
        }
        let root = self.shapes.keys().next_back().copied().unwrap_or(0)
            .checked_add(1).ok_or("compound root index overflow")?;
        let boxes: [[[f32; 3]; 2]; 2] = [primary_bounds, extra_bounds].map(|bounds|
            [bounds[0][..3].try_into().unwrap(), bounds[1][..3].try_into().unwrap()]);
        let combined: [[f32; 3]; 2] = [std::array::from_fn(|axis| boxes[0][0][axis].min(boxes[1][0][axis])),
            std::array::from_fn(|axis| boxes[0][1][axis].max(boxes[1][1][axis]))];
        let flags = 0x3f00_0040;
        let instance = |shape, shape_tag| {
            let mut transform = identity;
            transform[0][3] = f32::from_bits(flags);
            CompoundInstance { transform, flags, scale: [1.; 4], shape, shape_tag }
        };
        let nodes = vec![
            CompoundNode { bounds: [[0.; 3]; 2], children: CompoundChildren::Instance(0) },
            CompoundNode { bounds: boxes[0], children: CompoundChildren::Instance(0) },
            CompoundNode { bounds: boxes[1], children: CompoundChildren::Instance(1) },
            CompoundNode { bounds: combined, children: CompoundChildren::Branch { left: 1, right: 2 } },
        ];
        let bounds = [[combined[0][0], combined[0][1], combined[0][2], 0.5],
            [combined[1][0], combined[1][1], combined[1][2], 0.]];
        self.shapes.insert(root, QueryShape {
            header: ShapeHeader { flags: 4, shape_type: 12, key_bits: 2,
                dispatch_type: 3, radius: 0., user_data: 0 },
            geometry: QueryGeometry::Compound(CompoundShape { bounds, key_bits: 2, is_mutable: false,
                instances: vec![instance(primary_root, tags[0]), instance(extra_root, tags[1])], nodes, root: 3 }),
        });
        self.root = root;
        self.absorb_resources(&mut extra);
        Ok(self)
    }

    /// Wrap the retained tree without copying its existing shape payloads.
    /// Each wrapper consumes one shape-key bit, including nested aliases. If
    /// construction fails, the caller must discard the partially wrapped tree.
    pub(crate) fn wrap_aliases(&mut self, wrappers: &[AliasInstance]) -> Result<(), String> {
        let identity: ShapeMatrix = [[1., 0., 0., 0.], [0., 1., 0., 0.],
            [0., 0., 1., 0.], [0., 0., 0., 1.]];
        for wrapper in wrappers.iter().rev() {
            let child = self.root;
            let shape = self.get(child)?;
            let scaled = wrapper.scale != 1.;
            // 08cb5e passes literal 1 in R8D to c96c20, so bit 0x20 is clear.
            let keep_radius = true;
            let bounds = if !scaled {
                self.bounds(child, identity)?
            } else if shape.header.flags & 1 != 0 {
                let vertices = match &shape.geometry {
                    QueryGeometry::Polytope(polytope) => &polytope.vertices[..],
                    _ => return Err("convex alias child has unsupported source geometry".into()),
                };
                crate::gameplay::convex_support::scaled_bounds::bounds(shape.header.shape_type,
                    vertices, shape.header.radius, identity,
                    [wrapper.scale, wrapper.scale, wrapper.scale, 1.], keep_radius)?
            } else {
                let mut frame = identity;
                for column in 0..3 {
                    for lane in 0..4 { frame[column][lane] *= wrapper.scale; }
                }
                self.bounds(child, frame)?
            };
            let next = self.shapes.keys().next_back().copied().unwrap_or(0)
                .checked_add(1).ok_or("physics alias shape index overflow")?;
            let flags = 0x3f00_0040 | if scaled { 8 } else { 0 }
                | if keep_radius { 0 } else { 32 };
            let mut transform = identity;
            transform[0][3] = f32::from_bits(flags);
            let instance = CompoundInstance { transform, flags,
                scale: [wrapper.scale, wrapper.scale, wrapper.scale, 1.],
                shape: child, shape_tag: u16::MAX };
            let xyz = [bounds[0][..3].try_into().unwrap(), bounds[1][..3].try_into().unwrap()];
            // The free-list's zero slot is not a live node. The single leaf
            // occupies slot 1 and refers to instance slot 0.
            let nodes = vec![
                CompoundNode { bounds: [[0.; 3]; 2], children: CompoundChildren::Instance(0) },
                CompoundNode { bounds: xyz, children: CompoundChildren::Instance(0) },
            ];
            // 141231530 unions the BVH node's XYZ and packed W lanes.
            let compound_bounds = [
                [bounds[0][0], bounds[0][1], bounds[0][2], 0.5],
                [bounds[1][0], bounds[1][1], bounds[1][2], 0.],
            ];
            self.shapes.insert(next, QueryShape {
                header: ShapeHeader { flags: 4, shape_type: 12, key_bits: 1,
                    dispatch_type: 3, radius: 0., user_data: wrapper.resource_identity },
                geometry: QueryGeometry::Compound(CompoundShape { bounds: compound_bounds,
                    key_bits: 1, is_mutable: false, instances: vec![instance], nodes, root: 1 }),
            });
            self.root = next;
        }
        Ok(())
    }
}
