//! 12871a0 unfiltered packed-AABB traversal used by c91680.
use super::*;

fn intersects(a: QuantizedBounds, b: QuantizedBounds) -> bool {
    let low_a = u32::from(a.min[0]) | u32::from(a.min[1]) << 16;
    let low_b = u32::from(b.min[0]) | u32::from(b.min[1]) << 16;
    let high_a = u32::from(a.max[0]) | u32::from(a.max[1]) << 16;
    let high_b = u32::from(b.max[0]) | u32::from(b.max[1]) << 16;
    (u32::from(a.max[2]).wrapping_sub(u32::from(b.min[2]))
        | u32::from(b.max[2]).wrapping_sub(u32::from(a.min[2]))
        | high_b.wrapping_sub(low_a)
        | high_a.wrapping_sub(low_b))
        & 0x80008000
        == 0
}

impl HybridBroadphase {
    pub fn overlap_bodies(
        &self,
        bounds: PackedBodyBounds,
        packing: &PackedQuantization,
        output: &mut Vec<u32>,
    ) -> Result<(), String> {
        self.filtered_overlaps(bounds, packing, |_| true, |_, _| true, output)
    }

    /// Filters correspond to virtual38(kind5, tree) and virtual30(kind5, body).
    /// The physics owner supplies body metadata to its filter through the ID.
    pub fn filtered_overlaps(
        &self,
        bounds: PackedBodyBounds,
        packing: &PackedQuantization,
        mut accept_tree: impl FnMut(usize) -> bool,
        mut accept_body: impl FnMut(usize, u32) -> bool,
        output: &mut Vec<u32>,
    ) -> Result<(), String> {
        let query = packing.query_bounds(bounds);
        let start = output.len();
        let result = (|| {
            let mut pending = Vec::new();
            let mut leaves = Vec::new();
            for (layer, tree) in self.query_trees().iter().enumerate() {
                if !accept_tree(layer) {
                    continue;
                }
                pending.clear();
                leaves.clear();
                pending.extend(tree.root);
                let mut visited = 0;
                while let Some(index) = pending.pop() {
                    if visited >= tree.nodes.len() {
                        return Err("cyclic broadphase overlap tree".into());
                    }
                    visited += 1;
                    let node = tree
                        .nodes
                        .get(index)
                        .ok_or("overlap tree node is missing")?;
                    if !intersects(query, node.bounds) {
                        continue;
                    }
                    match node.children {
                        PointNodeChildren::Branch { left, right } => {
                            pending.push(left);
                            pending.push(right);
                        }
                        PointNodeChildren::Leaf { block } => leaves.push(block),
                    }
                }
                for &block in &leaves {
                    for body in tree
                        .blocks
                        .get(block)
                        .ok_or("overlap leaf block is missing")?
                    {
                        if intersects(query, body.bounds) && accept_body(layer, body.index) {
                            output.push(body.index);
                        }
                    }
                }
            }
            Ok(())
        })();
        if result.is_err() {
            output.truncate(start);
        }
        result
    }
}

#[cfg(test)]
#[path = "overlap_tests.rs"]
mod tests;
