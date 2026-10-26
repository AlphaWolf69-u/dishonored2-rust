use super::*;

impl HybridTree {
    /// 128b6f0's subtree insertion. The world classifies the input batch and sets
    /// each body's high tree-index bits before calling its selected subtree.
    pub fn insert_batch(
        &mut self,
        mut bodies: &[PointQueryBody],
        mut set_body_leaf: impl FnMut(u32, usize),
    ) -> Result<(), String> {
        if bodies.is_empty() {
            return Ok(());
        }
        if self.root.is_none() {
            self.insert(*bodies.last().unwrap(), &mut set_body_leaf)?;
            bodies = &bodies[..bodies.len() - 1];
            if bodies.is_empty() {
                return Ok(());
            }
        }
        self.body_count = self.body_count.wrapping_add(bodies.len() as u32);
        // Assign every body against the old topology before any group mutates it.
        let mut groups = Vec::with_capacity(bodies.len());
        for (i, body) in bodies.iter().enumerate() {
            let center = std::array::from_fn(|axis| {
                i32::from(body.bounds.min[axis] >> 1) + i32::from(body.bounds.max[axis] >> 1)
            });
            let leaf = select_leaf(&self.nodes, self.root, center, u32::MAX)?
                .ok_or("hybrid batch insertion found no leaf")?;
            groups.push((leaf, i));
        }
        super::sort::by_key(&mut groups, |&(leaf, _)| leaf as u32);
        let mut start = 0;
        while start < groups.len() {
            let leaf = groups[start].0;
            let mut end = start + 1;
            while end < groups.len() && groups[end].0 == leaf {
                end += 1;
            }
            let incoming: Vec<_> = groups[start..end].iter().map(|&(_, i)| bodies[i]).collect();
            self.insert_leaf(leaf, &incoming, &mut set_body_leaf)?;
            start = end;
        }
        refresh_branch_bounds(
            &mut self.nodes,
            &mut self.branch_leaf_counts,
            self.root.unwrap(),
        )?;
        Ok(())
    }
}
