//! 141288af0 hybrid broadphase point traversal. Tree construction, collision
//! filters and exact shape queries remain the physics world's responsibility.

#[derive(Clone, Copy, Debug)]
pub struct QuantizedBounds {
    pub min: [u16; 3],
    pub max: [u16; 3],
}

#[derive(Clone, Copy, Debug)]
pub enum PointNodeChildren {
    Branch { left: usize, right: usize },
    Leaf { block: usize },
}

#[derive(Clone, Copy, Debug)]
pub struct PointQueryNode {
    pub bounds: QuantizedBounds,
    pub children: PointNodeChildren,
}

#[derive(Clone, Copy, Debug)]
pub struct PointQueryBody {
    pub bounds: QuantizedBounds,
    pub index: u32,
}

pub struct PointQueryTree<'a> {
    pub root: Option<usize>,
    pub nodes: &'a [PointQueryNode],
    pub blocks: &'a [Vec<PointQueryBody>],
}

#[derive(Clone, Copy, Debug)]
pub struct PointQuantization {
    pub offset: [f32; 3],
    pub scale: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct PointQueryControl {
    pub distance: f32,
    pub early_out: bool,
}

pub trait PointQueryVisitor {
    /// The default ark query filter's tree callback11d080 always accepts.
    /// Specialized query filters may override it.
    fn accepts_tree(&mut self, _tree: usize) -> bool {
        true
    }
    fn accepts_body(&mut self, body: u32) -> bool;
    /// Run the actual shape point query and update the real collector. AABB
    /// admission alone is not a collision hit or an environment contact.
    fn query_shape(&mut self, body: u32, control: &mut PointQueryControl);
}

#[derive(Default)]
pub struct HybridPointQuery {
    pending: Vec<usize>,
}

// SSE MINPS/MAXPS select the second operand for unordered comparisons.
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

impl PointQuantization {
    fn distance_squared(self, bounds: QuantizedBounds, point: [f32; 3]) -> f32 {
        let d: [f32; 3] = std::array::from_fn(|axis| {
            let lower = f32::from(bounds.min[axis]) * self.scale[axis] - self.offset[axis];
            // Integer addition happens before float conversion;65535 expands
            // to65536, not a wrapped16-bit zero.
            let upper =
                (u32::from(bounds.max[axis]) + 1) as f32 * self.scale[axis] - self.offset[axis];
            max(lower, min(upper, point[axis])) - point[axis]
        });
        (d[1] * d[1] + d[0] * d[0]) + d[2] * d[2]
    }
}

impl HybridPointQuery {
    pub fn visit(
        &mut self,
        trees: &[PointQueryTree<'_>],
        quantization: PointQuantization,
        point: [f32; 3],
        maximum_distance: f32,
        control: &mut PointQueryControl,
        visitor: &mut impl PointQueryVisitor,
    ) -> Result<(), String> {
        let initial = min(maximum_distance, control.distance);
        let mut radius_squared = initial * initial;
        for (tree_index, tree) in trees.iter().enumerate() {
            if control.early_out {
                break;
            }
            if !visitor.accepts_tree(tree_index) {
                continue;
            }
            let Some(mut node_index) = tree.root else {
                continue;
            };
            self.pending.clear();
            loop {
                let node = tree
                    .nodes
                    .get(node_index)
                    .ok_or("point-query node index out of bounds")?;
                match node.children {
                    PointNodeChildren::Branch { left, right } => {
                        let left_node = tree
                            .nodes
                            .get(left)
                            .ok_or("point-query left child out of bounds")?;
                        let right_node = tree
                            .nodes
                            .get(right)
                            .ok_or("point-query right child out of bounds")?;
                        let dl = quantization.distance_squared(left_node.bounds, point);
                        let dr = quantization.distance_squared(right_node.bounds, point);
                        // UCOMISS/CMOVBE also admit unordered comparisons.
                        match (!(dl > radius_squared), !(dr > radius_squared)) {
                            (true, true) => {
                                if dr <= dl {
                                    self.pending.push(left);
                                    node_index = right;
                                } else {
                                    self.pending.push(right);
                                    node_index = left;
                                }
                                continue;
                            }
                            (true, false) => {
                                node_index = left;
                                continue;
                            }
                            (false, true) => {
                                node_index = right;
                                continue;
                            }
                            (false, false) => {}
                        }
                    }
                    PointNodeChildren::Leaf { block } => {
                        let bodies = tree
                            .blocks
                            .get(block)
                            .ok_or("point-query leaf block out of bounds")?;
                        for body in bodies {
                            // Native rejects with JA, so NaNs pass this gate.
                            if !(quantization.distance_squared(body.bounds, point) > radius_squared)
                                && visitor.accepts_body(body.index)
                            {
                                control.distance = min(control.distance, maximum_distance);
                                visitor.query_shape(body.index, control);
                                let distance = max(0., control.distance);
                                radius_squared = distance * distance;
                                if control.early_out {
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
                let Some(next) = self.pending.pop() else {
                    break;
                };
                // A queued node is not retested against its own bound after a
                // collector shrinks the radius. Descendant/body tests still run.
                node_index = next;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "point_query/tests.rs"]
mod tests;
