//! Owned hybrid trees, body classification, insertion and world quantization.
//! Shape bounds, runtime body flags and activation come from the physics world.
use super::point_query::{
    insertion::{refresh_branch_bounds, select_leaf, sort_leaf_bodies},
    PointNodeChildren, PointQueryBody, PointQueryNode, PointQueryTree, QuantizedBounds,
};
#[path = "broadphase/batch.rs"]
mod batch;
#[path = "broadphase/rebuild.rs"]
mod rebuild;
#[path = "broadphase/sort.rs"]
mod sort;
#[path = "broadphase/world.rs"]
mod world;
#[path = "broadphase/overlap.rs"]
mod overlap;
pub use world::{body_layer, BroadphaseBody, BroadphaseLayerSettings, HybridBroadphase};
#[path = "broadphase/quantization.rs"]
mod quantization;
pub use quantization::{
    BoundsPackingLayout, BroadphaseQuantization, PackedBodyBounds, PackedQuantization,
};
#[path = "broadphase/storage.rs"]
mod storage;

// 1285610 constructs every hybrid subtree through 12b9040 with this limit.
const LEAF_LIMIT: usize = 29;

pub struct HybridTree {
    nodes: Vec<PointQueryNode>,
    parents: Vec<Option<usize>>,
    branch_leaf_counts: Vec<u32>,
    free_nodes: Vec<usize>,
    blocks: Vec<Vec<PointQueryBody>>,
    free_blocks: Vec<usize>,
    root: Option<usize>,
    leaf_count: usize,
    body_count: u32,
}

impl Default for HybridTree {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            parents: Vec::new(),
            branch_leaf_counts: Vec::new(),
            free_nodes: Vec::new(),
            blocks: (0..128).map(|_| Vec::new()).collect(),
            free_blocks: (0..128).rev().collect(),
            root: None,
            leaf_count: 0,
            body_count: 0,
        }
    }
}

impl HybridTree {
    pub fn query_tree(&self) -> PointQueryTree<'_> {
        PointQueryTree {
            root: self.root,
            nodes: &self.nodes,
            blocks: &self.blocks,
        }
    }

    pub fn body_count(&self) -> u32 {
        self.body_count
    }

    /// Updates the world's body-to-leaf mapping whenever insertion or splitting
    /// places a body. The world retains its high three tree-index bits.
    pub fn insert(
        &mut self,
        body: PointQueryBody,
        mut set_body_leaf: impl FnMut(u32, usize),
    ) -> Result<(), String> {
        self.body_count = self.body_count.wrapping_add(1);
        let leaf = match self.root {
            None => {
                let root = self.allocate_node();
                self.root = Some(root);
                let block = self.allocate_block();
                self.nodes[root] = PointQueryNode {
                    bounds: QuantizedBounds {
                        min: [u16::MAX; 3],
                        max: [0; 3],
                    },
                    children: PointNodeChildren::Leaf { block },
                };
                root
            }
            Some(root) => {
                let center = std::array::from_fn(|i| {
                    i32::from(body.bounds.min[i] >> 1) + i32::from(body.bounds.max[i] >> 1)
                });
                select_leaf(&self.nodes, Some(root), center, u32::MAX)?
                    .ok_or("hybrid insertion found no leaf")?
            }
        };
        self.insert_leaf(leaf, std::slice::from_ref(&body), &mut set_body_leaf)?;
        refresh_branch_bounds(
            &mut self.nodes,
            &mut self.branch_leaf_counts,
            self.root.unwrap(),
        )?;
        Ok(())
    }
    /// 128f600 appends a group or replaces its target leaf by a subtree.
    fn insert_leaf(
        &mut self,
        leaf: usize,
        incoming: &[PointQueryBody],
        set_body_leaf: &mut impl FnMut(u32, usize),
    ) -> Result<(), String> {
        let PointNodeChildren::Leaf { block } = self.nodes[leaf].children else {
            return Err("hybrid insertion selected a branch".into());
        };
        if incoming.len() < LEAF_LIMIT && self.blocks[block].len() < LEAF_LIMIT - incoming.len() {
            for &body in incoming {
                set_body_leaf(body.index, leaf);
                self.blocks[block].push(body);
                sort_leaf_bodies(&mut self.blocks[block]);
                let bounds = &mut self.nodes[leaf].bounds;
                for i in 0..3 {
                    bounds.min[i] = bounds.min[i].min(body.bounds.min[i]);
                    bounds.max[i] = bounds.max[i].max(body.bounds.max[i]);
                }
            }
        } else {
            let mut bodies = self.blocks[block].clone();
            bodies.extend_from_slice(incoming);
            let centers = center_bounds(&bodies);
            self.blocks[block].clear();
            self.free_blocks.push(block);
            self.leaf_count -= 1;
            self.ensure_free_nodes((bodies.len() / (LEAF_LIMIT >> 2)) * 2 + 1);
            self.rebuild(leaf, &mut bodies, centers, set_body_leaf)?;
        }
        Ok(())
    }
}

fn center_bounds(bodies: &[PointQueryBody]) -> QuantizedBounds {
    let mut result = QuantizedBounds {
        min: [u16::MAX; 3],
        max: [0; 3],
    };
    for body in bodies {
        for i in 0..3 {
            let center = (body.bounds.min[i] >> 1) + (body.bounds.max[i] >> 1);
            result.min[i] = result.min[i].min(center);
            result.max[i] = result.max[i].max(center);
        }
    }
    result
}

#[cfg(test)]
#[path = "broadphase/tests.rs"]
mod tests;
