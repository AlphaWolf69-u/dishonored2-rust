//! Default five-layer configuration (cc3b10..cc3ba0) and 1285f00 insertion.
use super::*;

#[derive(Clone, Copy, Debug)]
pub struct BroadphaseLayerSettings {
    pub collision_mask: u32,
    /// The layer descriptor's two control bytes, retained without inferring
    /// their effect on unported update and collision-pair generation paths.
    pub flags: [u8; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct BroadphaseBody {
    pub body: PointQueryBody,
    pub flags: u32,
    pub status: u16,
}

/// Runtime body +44 flags and +48 status, not body-creation declaration flags.
pub fn body_layer(flags: u32, status: u16) -> usize {
    if flags & 0x100 != 0 {
        1
    } else if flags & 8 != 0 {
        0
    } else if flags & 1 == 0 {
        if status & 0x91 != 0 {
            0
        } else {
            2
        }
    } else if status & 0x91 != 0 {
        3
    } else {
        4
    }
}

pub struct HybridBroadphase {
    trees: [HybridTree; 5],
    layers: [BroadphaseLayerSettings; 5],
    dirty: [bool; 5],
    layer_pairs: Vec<[u8; 2]>,
    pending_bodies: crate::gameplay::bit_field::BitField,
    first_pending_body: u32,
    last_pending_body: u32,
}

impl Default for HybridBroadphase {
    fn default() -> Self {
        let layers = [
            BroadphaseLayerSettings {
                collision_mask: 0x1d,
                flags: [1, 1],
            },
            BroadphaseLayerSettings {
                collision_mask: 0,
                flags: [1, 0],
            },
            BroadphaseLayerSettings {
                collision_mask: 9,
                flags: [0, 0],
            },
            BroadphaseLayerSettings {
                collision_mask: 5,
                flags: [0, 1],
            },
            BroadphaseLayerSettings {
                collision_mask: 1,
                flags: [0, 0],
            },
        ];
        let mut layer_pairs = Vec::new();
        for a in 0..layers.len() {
            for b in a..layers.len() {
                if (layers[a].collision_mask >> b) & 1 != 0
                    || (layers[b].collision_mask >> a) & 1 != 0
                {
                    layer_pairs.push([a as u8, b as u8]);
                }
            }
        }
        Self {
            trees: std::array::from_fn(|_| HybridTree::default()),
            layers,
            dirty: [false; 5],
            layer_pairs,
            pending_bodies: Default::default(),
            first_pending_body: u32::MAX,
            last_pending_body: 0,
        }
    }
}

impl HybridBroadphase {
    /// Host spatial-index maintenance for1286530's marked body slots. Refit
    /// the existing leaves/ancestor bounds without making a second collision
    /// world; changes of the source's five-layer classification relocate the
    /// actual body through the same insertion owner as initial publication.
    /// Internal tree topology is host state, not collision geometry.
    pub(crate) fn update_marked_bodies(
        &mut self,
        bodies: &mut crate::gameplay::body_storage::BodyStorage,
        packing: &PackedQuantization,
    ) -> Result<(), String> {
        let Some([first, last]) = self.pending_body_range() else {
            return Ok(());
        };
        let mut slot = first;
        while slot <= last {
            let word = self
                .pending_bodies
                .words()
                .get(slot as usize >> 5)
                .copied()
                .unwrap_or(0);
            let mask = word & (u32::MAX << (slot & 31));
            if mask == 0 {
                let Some(next) = (slot | 31).checked_add(1) else {
                    break;
                };
                slot = next;
                continue;
            }
            slot = (slot & !31) + mask.trailing_zeros();
            if slot > last {
                break;
            }
            if let Some(body) = bodies.get_by_index(slot) {
                let id = body.id;
                let location = body.broadphase_id;
                if location != u32::MAX {
                    let previous_layer = (location >> 29) as usize;
                    let leaf = (location & 0x1fff_ffff) as usize;
                    let layer = body_layer(body.flags, body.status);
                    let entry = PointQueryBody {
                        index: id,
                        bounds: packing.query_bounds(body.bounds),
                    };
                    if previous_layer == layer {
                        self.trees[layer].refit_body(id, leaf, entry.bounds)?;
                    } else {
                        let mut missing = false;
                        self.trees[layer].insert(entry, |id, leaf| {
                            if let Some(record) = bodies.get_mut(id) {
                                record.broadphase_id = ((layer as u32) << 29) | leaf as u32;
                            } else {
                                missing = true;
                            }
                        })?;
                        if missing {
                            return Err("relocated broadphase body is stale".into());
                        }
                        self.trees[previous_layer].remove_body(id, leaf)?;
                        self.trees[previous_layer].refit_ancestors(leaf)?;
                        self.dirty[previous_layer] = true;
                    }
                    self.dirty[layer] = true;
                }
            }
            self.pending_bodies.reset(slot as usize)?;
            let Some(next) = slot.checked_add(1) else {
                break;
            };
            slot = next;
        }
        self.first_pending_body = u32::MAX;
        self.last_pending_body = 0;
        Ok(())
    }

    pub(crate) fn remove_bodies(&mut self, bodies: &[(u32, u32)]) -> Result<(), String> {
        for &(id, location) in bodies {
            if location & 0x1fff_ffff == 0x1fff_ffff {
                continue;
            }
            let layer = (location >> 29) as usize;
            let leaf = (location & 0x1fff_ffff) as usize;
            let tree = self
                .trees
                .get_mut(layer)
                .ok_or("broadphase removal layer is missing")?;
            tree.remove_body(id, leaf)?;
            self.dirty[layer] = true;
            let slot = (id & 0x00ff_ffff) as usize;
            if slot < self.pending_bodies.len() {
                self.pending_bodies.reset(slot)?;
            }
        }
        Ok(())
    }

    pub fn with_body_capacity(capacity: usize) -> Self {
        Self {
            pending_bodies: crate::gameplay::bit_field::BitField::zeroed(capacity),
            ..Self::default()
        }
    }
    pub fn layer_pairs(&self) -> &[[u8; 2]] {
        &self.layer_pairs
    }
    pub fn pending_body_bits(&self) -> &crate::gameplay::bit_field::BitField {
        &self.pending_bodies
    }
    pub fn pending_body_range(&self) -> Option<[u32; 2]> {
        (self.first_pending_body <= self.last_pending_body)
            .then_some([self.first_pending_body, self.last_pending_body])
    }
    /// 1286530's virtual+20 marks body slots. Its fourth argument is the
    /// source record stride (4 for c91230), not a dirty-flag mask.
    pub fn mark_bodies(&mut self, ids: impl IntoIterator<Item = u32>) {
        for id in ids {
            let slot = id & 0xffffff;
            if self.pending_bodies.len() <= slot as usize {
                self.pending_bodies.resize(((slot + 32) & !31) as usize, 0);
            }
            self.pending_bodies.set(slot as usize).unwrap();
            self.first_pending_body = self.first_pending_body.min(slot);
            self.last_pending_body = self.last_pending_body.max(slot);
        }
    }
    pub fn layer_settings(&self) -> &[BroadphaseLayerSettings; 5] {
        &self.layers
    }
    pub fn dirty_layers(&self) -> &[bool; 5] {
        &self.dirty
    }
    pub fn query_trees(&self) -> [PointQueryTree<'_>; 5] {
        std::array::from_fn(|i| self.trees[i].query_tree())
    }

    /// Publish the complete body broadphase ID: high three layer bits and low
    /// 29 leaf bits. IDs remain owned by the world's body buffer.
    pub fn add_bodies(
        &mut self,
        bodies: &[BroadphaseBody],
        mut publish: impl FnMut(u32, u32),
    ) -> Result<(), String> {
        if let [body] = bodies {
            let layer = body_layer(body.flags, body.status);
            self.trees[layer].insert(body.body, |id, leaf| {
                publish(id, ((layer as u32) << 29) | leaf as u32)
            })?;
            self.dirty[layer] = true;
        } else {
            let classes: Vec<_> = bodies
                .iter()
                .map(|body| body_layer(body.flags, body.status))
                .collect();
            for layer in 0..self.trees.len() {
                let group: Vec<_> = bodies
                    .iter()
                    .zip(&classes)
                    .filter_map(|(body, &class)| (class == layer).then_some(body.body))
                    .collect();
                if !group.is_empty() {
                    self.trees[layer].insert_batch(&group, |id, leaf| {
                        publish(id, ((layer as u32) << 29) | leaf as u32)
                    })?;
                    self.dirty[layer] = true;
                }
            }
        }
        Ok(())
    }
}

impl HybridTree {
    fn refit_body(&mut self, id: u32, leaf: usize, bounds: QuantizedBounds) -> Result<(), String> {
        let PointNodeChildren::Leaf { block } = self
            .nodes
            .get(leaf)
            .ok_or("refit broadphase leaf is absent")?
            .children
        else {
            return Err("refit broadphase body does not reference a leaf".into());
        };
        let entries = self
            .blocks
            .get_mut(block)
            .ok_or("refit broadphase block is absent")?;
        let entry = entries
            .iter_mut()
            .find(|entry| entry.index == id)
            .ok_or("refit broadphase body is absent from its leaf")?;
        entry.bounds = bounds;
        sort_leaf_bodies(entries);
        let mut bounds = QuantizedBounds {
            min: [u16::MAX; 3],
            max: [0; 3],
        };
        for entry in entries {
            for axis in 0..3 {
                bounds.min[axis] = bounds.min[axis].min(entry.bounds.min[axis]);
                bounds.max[axis] = bounds.max[axis].max(entry.bounds.max[axis]);
            }
        }
        self.nodes[leaf].bounds = bounds;
        self.refit_ancestors(leaf)
    }

    fn refit_ancestors(&mut self, mut node: usize) -> Result<(), String> {
        for _ in 0..self.nodes.len() {
            let Some(parent) = self.parents.get(node).copied().flatten() else {
                return Ok(());
            };
            let PointNodeChildren::Branch { left, right } = self
                .nodes
                .get(parent)
                .ok_or("refit broadphase parent is absent")?
                .children
            else {
                return Err("refit broadphase parent is not a branch".into());
            };
            let a = self
                .nodes
                .get(left)
                .ok_or("refit broadphase left child is absent")?
                .bounds;
            let b = self
                .nodes
                .get(right)
                .ok_or("refit broadphase right child is absent")?
                .bounds;
            self.nodes[parent].bounds = QuantizedBounds {
                min: std::array::from_fn(|axis| a.min[axis].min(b.min[axis])),
                max: std::array::from_fn(|axis| a.max[axis].max(b.max[axis])),
            };
            node = parent;
        }
        Err("refit broadphase parent chain does not end".into())
    }
}

#[cfg(test)]
#[path = "world_tests.rs"]
mod tests;
