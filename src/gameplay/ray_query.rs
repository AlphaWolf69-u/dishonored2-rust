//! 1287c70/128ed70: hybrid quantized-tree ray traversal. Shape casts and
//! collision filters remain the caller's responsibility; boxes admit work only.
use super::{
    constraint_solver::reciprocal,
    point_query::{PointNodeChildren, PointQueryTree, QuantizedBounds},
};
use crate::float::{add, multiply, subtract};

#[derive(Clone, Copy)]
pub struct RayQuantization {
    pub offset: [f32; 3],
    /// The world's stored forward scale, not a re-inverted point-query scale.
    pub scale: [f32; 3],
}
pub struct RayQueryControl {
    pub fraction: f32,
    pub early_out: bool,
}
pub trait RayQueryVisitor {
    fn accepts_tree(&mut self, _tree: usize) -> bool {
        true
    }
    fn accepts_body(&mut self, body: u32) -> bool;
    fn query_shape(&mut self, body: u32, control: &mut RayQueryControl);
}
#[derive(Default)]
pub struct HybridRayQuery {
    pending: Vec<usize>,
}

fn min(first: f32, second: f32) -> f32 {
    if first < second {
        first
    } else {
        second
    }
}
fn max(first: f32, second: f32) -> f32 {
    if first > second {
        first
    } else {
        second
    }
}
struct QuantizedRay {
    origin: [f32; 3],
    inverse_direction: [f32; 3],
}
impl QuantizedRay {
    fn new(quantization: RayQuantization, origin: [f32; 3], direction: [f32; 3]) -> Self {
        Self {
            origin: std::array::from_fn(|i| {
                multiply(
                    add(quantization.offset[i], origin[i]),
                    quantization.scale[i],
                )
            }),
            inverse_direction: std::array::from_fn(|i| {
                let direction = multiply(quantization.scale[i], direction[i]);
                if direction == 0. {
                    f32::from_bits(0x7f7f_ffee)
                } else {
                    reciprocal(direction)
                }
            }),
        }
    }
    fn interval(&self, bounds: QuantizedBounds, limit: f32) -> [f32; 2] {
        let lower: [f32; 3] = std::array::from_fn(|i| {
            multiply(
                self.inverse_direction[i],
                subtract(f32::from(bounds.min[i]), self.origin[i]),
            )
        });
        let upper: [f32; 3] = std::array::from_fn(|i| {
            multiply(
                self.inverse_direction[i],
                subtract((u32::from(bounds.max[i]) + 1) as f32, self.origin[i]),
            )
        });
        let near: [f32; 3] = std::array::from_fn(|i| min(lower[i], upper[i]));
        let far: [f32; 3] = std::array::from_fn(|i| max(lower[i], upper[i]));
        [
            max(max(near[2], max(near[1], near[0])), 0.),
            min(min(far[2], min(far[1], far[0])), limit),
        ]
    }
}
impl HybridRayQuery {
    pub fn query(
        &mut self,
        trees: &[PointQueryTree<'_>],
        quantization: RayQuantization,
        origin: [f32; 3],
        direction: [f32; 3],
        maximum_fraction: f32,
        control: &mut RayQueryControl,
        visitor: &mut impl RayQueryVisitor,
    ) -> Result<(), String> {
        let ray = QuantizedRay::new(quantization, origin, direction);
        for (index, tree) in trees.iter().enumerate() {
            if control.early_out {
                break;
            }
            if !visitor.accepts_tree(index) {
                continue;
            }
            let Some(mut node_index) = tree.root else {
                continue;
            };
            self.pending.clear();
            // A tree clips the collector to the request on entry. As in the
            // source, queued nodes are not retested against their own bounds.
            control.fraction = min(maximum_fraction, control.fraction);
            loop {
                let node = tree.nodes.get(node_index).ok_or("ray-query node absent")?;
                match node.children {
                    PointNodeChildren::Branch { left, right } => {
                        let left_bounds = tree
                            .nodes
                            .get(left)
                            .ok_or("ray-query left child absent")?
                            .bounds;
                        let right_bounds = tree
                            .nodes
                            .get(right)
                            .ok_or("ray-query right child absent")?
                            .bounds;
                        let l = ray.interval(left_bounds, control.fraction);
                        let r = ray.interval(right_bounds, control.fraction);
                        // The original left rejection uses UCOMISS/JA;
                        // the right admission uses an ordered <= comparison.
                        match (!(l[1] < l[0]), r[0] <= r[1]) {
                            (true, true) => {
                                if r[0] <= l[0] {
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
                            .ok_or("ray-query body block absent")?;
                        let tail = bodies.len().saturating_sub(4);
                        // The packed source first tests its final four bodies,
                        // then the preceding groups from zero. Each group's
                        // SIMD admission mask precedes all shape callbacks.
                        for first in std::iter::once(tail).chain((0..tail).step_by(4)) {
                            let end = if first == tail {
                                bodies.len()
                            } else {
                                (first + 4).min(tail)
                            };
                            let mut admitted = [false; 4];
                            for (lane, body) in bodies[first..end].iter().enumerate() {
                                let interval = ray.interval(body.bounds, control.fraction);
                                admitted[lane] = interval[0] <= interval[1];
                            }
                            for (lane, body) in bodies[first..end].iter().enumerate() {
                                if admitted[lane] && visitor.accepts_body(body.index) {
                                    visitor.query_shape(body.index, control);
                                    if control.early_out {
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                }
                let Some(next) = self.pending.pop() else {
                    break;
                };
                node_index = next;
            }
        }
        Ok(())
    }
}
