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

#[cfg(test)]
#[path = "world_tests.rs"]
mod tests;
