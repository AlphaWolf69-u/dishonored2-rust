use super::*;

impl HybridTree {
    /// 12bbb20: append newly available node indices in descending order.
    fn grow_nodes(&mut self, required: usize) {
        let old = self.nodes.len();
        if required <= old {
            return;
        }
        // These placeholders are free storage, never reachable from the root.
        self.nodes.resize(
            required,
            PointQueryNode {
                bounds: QuantizedBounds {
                    min: [0; 3],
                    max: [0; 3],
                },
                children: PointNodeChildren::Leaf { block: 0 },
            },
        );
        self.parents.resize(required, None);
        self.branch_leaf_counts.resize(required, 0);
        self.free_nodes.extend((old..required).rev());
    }

    /// 12bbbf0 reserves free slots, independently of allocated node count.
    pub(super) fn ensure_free_nodes(&mut self, required: usize) {
        if self.free_nodes.len() < required {
            self.grow_nodes(self.nodes.len() - self.free_nodes.len() + required);
        }
    }

    /// 12bae50 grows by 128 only when the free-node stack is empty.
    pub(super) fn allocate_node(&mut self) -> usize {
        if self.free_nodes.is_empty() {
            self.grow_nodes(self.nodes.len() + 128);
        }
        self.free_nodes.pop().unwrap()
    }

    pub(super) fn allocate_block(&mut self) -> usize {
        let block = self.free_blocks.pop().unwrap_or_else(|| {
            let index = self.blocks.len();
            self.blocks.push(Vec::new());
            index
        });
        self.blocks[block].clear();
        self.leaf_count += 1;
        block
    }
}
