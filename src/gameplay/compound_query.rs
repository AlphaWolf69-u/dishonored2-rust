//! Float-BVH traversal in 14123b5e0/14123acd0 for compound closest-point queries.
//! Instance flags, tags, filters, transforms and exact geometry remain with the
//! visitor. Traversal order differs from the world's quantized hybrid tree.

pub use crate::assets::physworld::compound::types::{CompoundChildren, CompoundNode};
#[path = "compound_query/instance.rs"]
pub mod instance;
#[path = "compound_query/prepare.rs"]
pub mod prepare;
pub struct CompoundTree<'a> {
    pub nodes: &'a [CompoundNode],
    /// The source tree reserves zero for an empty root.
    pub root: u16,
}
#[derive(Clone, Copy, Debug)]
pub struct CompoundQueryBounds {
    pub center: [f32; 3],
    pub half_extent: [f32; 3],
}
pub trait CompoundQueryVisitor {
    /// Apply actual instance admission and geometry, then publish the squared
    /// collector distance in the compound's query space. Rejection preserves it.
    fn query_instance(&mut self, instance: u16, squared_distance: &mut f32);
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
impl CompoundQueryBounds {
    fn distance_squared(self, bounds: [[f32; 3]; 2]) -> f32 {
        let delta: [f32; 3] = std::array::from_fn(|i| {
            let lower = bounds[0][i] - self.half_extent[i];
            let upper = self.half_extent[i] + bounds[1][i];
            self.center[i] - min(upper, max(lower, self.center[i]))
        });
        (delta[1] * delta[1] + delta[0] * delta[0]) + delta[2] * delta[2]
    }
}

#[derive(Default)]
pub struct CompoundQuery {
    pending: Vec<u16>,
}
impl CompoundQuery {
    pub fn visit(
        &mut self,
        tree: CompoundTree<'_>,
        query: CompoundQueryBounds,
        squared_distance: &mut f32,
        visitor: &mut impl CompoundQueryVisitor,
    ) -> Result<(), String> {
        self.visit_masked(tree, query, squared_distance, None, visitor)
    }

    /// The compound's optional node bitset, indexed by BVH node (not instance).
    /// Nested child-shape masks remain the instance visitor's responsibility.
    pub fn visit_masked(
        &mut self,
        tree: CompoundTree<'_>,
        query: CompoundQueryBounds,
        squared_distance: &mut f32,
        node_mask: Option<&[u32]>,
        visitor: &mut impl CompoundQueryVisitor,
    ) -> Result<(), String> {
        self.pending.clear();
        if tree.root == 0 {
            return Ok(());
        }
        let node = |index: u16| {
            tree.nodes
                .get(usize::from(index))
                .ok_or_else(|| "compound query node index out of bounds".to_string())
        };
        let enabled = |index: u16| -> Result<bool, String> {
            match node_mask {
                None => Ok(true),
                Some(words) => words
                    .get(usize::from(index) / 32)
                    .map(|word| word >> (index & 31) & 1 != 0)
                    .ok_or_else(|| "compound query node mask index out of bounds".to_string()),
            }
        };
        let mut index = tree.root;
        if !enabled(index)? {
            return Ok(());
        }
        // UCOMISS/CMOVBE accepts unordered values too.
        if query.distance_squared(node(index)?.bounds) > *squared_distance {
            return Ok(());
        }
        loop {
            match node(index)?.children {
                CompoundChildren::Instance(instance) => {
                    if enabled(index)? {
                        visitor.query_instance(instance, squared_distance)
                    }
                }
                CompoundChildren::Branch { left, right } => {
                    let dl = query.distance_squared(node(left)?.bounds);
                    let dr = query.distance_squared(node(right)?.bounds);
                    match (
                        !(dl > *squared_distance) && enabled(left)?,
                        !(dr > *squared_distance) && enabled(right)?,
                    ) {
                        (true, true) => {
                            // Source ties choose LEFT. The world's hybrid
                            // traversal chooses RIGHT and cannot be reused here.
                            // SETC also chooses right for an unordered comparison.
                            if !(dr >= dl) {
                                self.pending.push(left);
                                index = right;
                            } else {
                                self.pending.push(right);
                                index = left;
                            }
                            continue;
                        }
                        (true, false) => {
                            index = left;
                            continue;
                        }
                        (false, true) => {
                            index = right;
                            continue;
                        }
                        (false, false) => {}
                    }
                }
            }
            let Some(next) = self.pending.pop() else {
                break;
            };
            // As in the source, a deferred node is not retested against its own
            // AABB after a nearer instance changed the collector's distance.
            index = next;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "compound_query/instance_tests.rs"]
mod instance_tests;
#[cfg(test)]
#[path = "compound_query/prepare_tests.rs"]
mod prepare_tests;
#[cfg(test)]
#[path = "compound_query/tests.rs"]
mod tests;
