use super::*;

struct Task {
    start: usize,
    count: usize,
    node: usize,
    centers: QuantizedBounds,
}

impl HybridTree {
    /// 128e210 builds branch topology first, then publishes leaves in the
    /// recorded task order. This order determines both block reuse and ties.
    pub(super) fn rebuild(
        &mut self,
        root: usize,
        bodies: &mut [PointQueryBody],
        centers: QuantizedBounds,
        set_body_leaf: &mut impl FnMut(u32, usize),
    ) -> Result<(), String> {
        let mut pending = vec![Task {
            start: 0,
            count: bodies.len(),
            node: root,
            centers,
        }];
        let mut leaves = Vec::new();
        while let Some(task) = pending.pop() {
            if task.count < LEAF_LIMIT {
                leaves.push(task);
                continue;
            }
            let (count, left_bounds, right_bounds) = partition(
                &mut bodies[task.start..task.start + task.count],
                task.centers,
            );
            let left = self.allocate_node();
            let right = self.allocate_node();
            self.nodes[task.node].children = PointNodeChildren::Branch { left, right };
            self.parents[left] = Some(task.node);
            self.parents[right] = Some(task.node);
            let left = Task {
                start: task.start,
                count,
                node: left,
                centers: left_bounds,
            };
            let right = Task {
                start: task.start + count,
                count: task.count - count,
                node: right,
                centers: right_bounds,
            };
            for child in [right, left] {
                if child.count < LEAF_LIMIT {
                    leaves.push(child);
                } else {
                    pending.push(child);
                }
            }
        }
        for task in leaves {
            self.create_leaf(
                task.node,
                &bodies[task.start..task.start + task.count],
                set_body_leaf,
            )?;
        }
        Ok(())
    }

    /// 128eb70 retains file/body order while publishing the body-to-leaf link,
    /// computes signed-short bounds, then applies the rebuilt-leaf quicksort.
    fn create_leaf(
        &mut self,
        node: usize,
        bodies: &[PointQueryBody],
        set_body_leaf: &mut impl FnMut(u32, usize),
    ) -> Result<(), String> {
        let first = bodies
            .first()
            .ok_or("hybrid rebuild produced an empty leaf")?;
        let mut bounds = first.bounds;
        for body in bodies {
            set_body_leaf(body.index, node);
            for i in 0..3 {
                if (body.bounds.min[i] as i16) < (bounds.min[i] as i16) {
                    bounds.min[i] = body.bounds.min[i];
                }
                if (body.bounds.max[i] as i16) > (bounds.max[i] as i16) {
                    bounds.max[i] = body.bounds.max[i];
                }
            }
        }
        let block = self.allocate_block();
        self.blocks[block].extend_from_slice(bodies);
        sort_rebuilt_leaf(&mut self.blocks[block]);
        self.nodes[node] = PointQueryNode {
            bounds,
            children: PointNodeChildren::Leaf { block },
        };
        Ok(())
    }
}

fn partition(
    bodies: &mut [PointQueryBody],
    bounds: QuantizedBounds,
) -> (usize, QuantizedBounds, QuantizedBounds) {
    let spans: [u32; 3] =
        std::array::from_fn(|i| u32::from(bounds.max[i]).wrapping_sub(u32::from(bounds.min[i])));
    let mut axis = usize::from(spans[0] < spans[1]);
    if spans[axis] < spans[2] {
        axis = 2;
    }
    if spans[axis] < 2 {
        return (bodies.len() >> 1, bounds, bounds);
    }
    let threshold = u32::from(bounds.min[axis]) + u32::from(bounds.max[axis]);
    let mut low = 0usize;
    let mut high = bodies.len() as isize - 1;
    let mut left = SumBounds::default();
    let mut right = SumBounds::default();
    'partition: while (low as isize) < high {
        let mut a;
        loop {
            a = body_sum(bodies[low]);
            if a[axis] > threshold {
                break;
            }
            left.include(a);
            low += 1;
            if high <= low as isize {
                break 'partition;
            }
        }
        let mut b;
        loop {
            b = body_sum(bodies[high as usize]);
            if b[axis] <= threshold {
                break;
            }
            right.include(b);
            high -= 1;
            if high <= low as isize {
                break 'partition;
            }
        }
        bodies.swap(low, high as usize);
        left.include(b);
        right.include(a);
        low += 1;
        high -= 1;
    }
    (low, left.halved(), right.halved())
}

fn body_sum(body: PointQueryBody) -> [u32; 3] {
    std::array::from_fn(|i| u32::from(body.bounds.min[i]) + u32::from(body.bounds.max[i]))
}
struct SumBounds {
    min: [u32; 3],
    max: [u32; 3],
}
impl Default for SumBounds {
    fn default() -> Self {
        Self {
            min: [u32::MAX; 3],
            max: [0; 3],
        }
    }
}
impl SumBounds {
    fn include(&mut self, sum: [u32; 3]) {
        for i in 0..3 {
            self.min[i] = self.min[i].min(sum[i]);
            self.max[i] = self.max[i].max(sum[i]);
        }
    }
    fn halved(&self) -> QuantizedBounds {
        QuantizedBounds {
            min: self.min.map(|v| (v >> 1) as u16),
            max: self.max.map(|v| (v >> 1) as u16),
        }
    }
}

/// 1290410 shares the midpoint-pivot algorithm with batch pair sorting.
fn sort_rebuilt_leaf(bodies: &mut [PointQueryBody]) {
    super::sort::by_key(bodies, |body| u32::from(body.bounds.min[0]));
}
