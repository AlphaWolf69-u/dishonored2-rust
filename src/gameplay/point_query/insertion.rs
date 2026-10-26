//! Hybrid tree insertion helpers: 12baea0 and 12bab70.
use super::{PointNodeChildren, PointQueryBody, PointQueryNode, QuantizedBounds};

/// 12903a0's stable ascending order of the packed lower X endpoint.
pub fn sort_leaf_bodies(bodies: &mut [PointQueryBody]) {
    bodies.sort_by_key(|body| body.bounds.min[0]);
}

fn center_distance(bounds: QuantizedBounds, center: [i32; 3]) -> u32 {
    (0..3).fold(0u32, |sum, axis| {
        let midpoint = i32::from(bounds.min[axis] >> 1) + i32::from(bounds.max[axis] >> 1);
        let delta = midpoint.wrapping_sub(center[axis]);
        sum.wrapping_add(delta.wrapping_mul(delta) as u32)
    })
}

/// Select the insertion leaf using the source's center-distance pruning and
/// stack order, including its asymmetric child admission when right is nearer.
pub fn select_leaf(
    nodes: &[PointQueryNode],
    root: Option<usize>,
    center: [i32; 3],
    mut limit: u32,
) -> Result<Option<usize>, String> {
    let Some(root) = root else { return Ok(None) };
    let mut pending = vec![root];
    let mut selected = None;
    while let Some(index) = pending.pop() {
        let node = nodes.get(index).ok_or("insertion node outside tree")?;
        match node.children {
            PointNodeChildren::Leaf { .. } => {
                let distance = center_distance(node.bounds, center);
                if distance < limit {
                    selected = Some(index);
                    limit = distance;
                }
            }
            PointNodeChildren::Branch { left, right } => {
                let a = center_distance(
                    nodes.get(left).ok_or("left child outside tree")?.bounds,
                    center,
                );
                let b = center_distance(
                    nodes.get(right).ok_or("right child outside tree")?.bounds,
                    center,
                );
                if b <= a {
                    if b < limit {
                        pending.push(left);
                    }
                    if a < limit {
                        pending.push(right);
                    }
                } else {
                    if b < limit {
                        pending.push(right);
                    }
                    if a < limit {
                        pending.push(left);
                    }
                }
            }
        }
    }
    Ok(selected)
}

/// Refresh branch bounds and cached descendant-leaf counts. The parallel count
/// array holds the node's +18 field; leaf entries are left untouched, as in the
/// source. The result counts leaves, independently of bodies in their blocks.
pub fn refresh_branch_bounds(
    nodes: &mut [PointQueryNode],
    branch_leaf_counts: &mut [u32],
    root: usize,
) -> Result<u32, String> {
    let node = nodes.get(root).ok_or("branch root outside tree")?;
    let PointNodeChildren::Branch { left, right } = node.children else {
        return Ok(1);
    };
    let a = refresh_branch_bounds(nodes, branch_leaf_counts, left)?;
    let b = refresh_branch_bounds(nodes, branch_leaf_counts, right)?;
    let count = a.wrapping_add(b);
    *branch_leaf_counts
        .get_mut(root)
        .ok_or("branch count outside tree")? = count;
    let low = nodes[left].bounds;
    let high = nodes[right].bounds;
    nodes[root].bounds = QuantizedBounds {
        min: std::array::from_fn(|i| low.min[i].min(high.min[i])),
        max: std::array::from_fn(|i| low.max[i].max(high.max[i])),
    };
    Ok(count)
}

#[cfg(test)]
#[path = "insertion/tests.rs"]
mod tests;
