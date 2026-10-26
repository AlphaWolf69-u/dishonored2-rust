//! Immutable inputs published by the canonical physics world for a render
//! frame. No body creation, integration, or second collision world lives here.
use super::{PointQueryBodies, WorldPointQuery, WorldPointRequest, WorldQueryShape};
use crate::{
    assets::physworld::query_geometry::QueryShapes,
    gameplay::{
        body_storage::BodyStorage,
        collision_body::CollisionBody,
        point_query::{PointQuantization, PointQueryBody, PointQueryNode, PointQueryTree},
    },
};
use std::{collections::BTreeMap, sync::Arc};

struct Tree {
    root: Option<usize>,
    nodes: Vec<PointQueryNode>,
    blocks: Vec<Vec<PointQueryBody>>,
}
pub struct PointQuerySnapshot {
    trees: Vec<Tree>,
    bodies: BTreeMap<u32, CollisionBody>,
    shapes: Vec<Arc<QueryShapes>>,
    heights: BTreeMap<u32, f32>,
    quantization: PointQuantization,
}
impl PointQueryBodies for PointQuerySnapshot {
    fn body(&self, index: u32) -> Option<&CollisionBody> {
        self.bodies.get(&index)
    }
}
impl PointQuerySnapshot {
    /// Preserve the published native tree, block and body order. Geometry is
    /// shared with the physics asset owner, never decoded/copied per frame.
    pub fn capture(
        bodies: &BodyStorage,
        trees: &[PointQueryTree<'_>],
        quantization: PointQuantization,
        shapes: Vec<Arc<QueryShapes>>,
        mut body_top: impl FnMut(&CollisionBody) -> f32,
    ) -> Result<Self, String> {
        let mut retained = BTreeMap::new();
        let mut heights = BTreeMap::new();
        let mut output = Vec::with_capacity(trees.len());
        for tree in trees {
            let mut pending = tree.root.into_iter().collect::<Vec<_>>();
            let mut visited = vec![false; tree.nodes.len()];
            while let Some(index) = pending.pop() {
                let node = tree
                    .nodes
                    .get(index)
                    .ok_or("query publication node absent")?;
                if std::mem::replace(&mut visited[index], true) {
                    return Err("query publication tree has repeated nodes".into());
                }
                let block = match node.children {
                    crate::gameplay::point_query::PointNodeChildren::Branch { left, right } => {
                        pending.extend([left, right]);
                        continue;
                    }
                    crate::gameplay::point_query::PointNodeChildren::Leaf { block } => tree
                        .blocks
                        .get(block)
                        .ok_or("query publication block absent")?,
                };
                for reference in block {
                    let index = reference.index & 0x00ff_ffff;
                    if let std::collections::btree_map::Entry::Vacant(entry) = retained.entry(index)
                    {
                        let body = bodies
                            .get_by_index(index)
                            .ok_or("published query tree references an absent body")?;
                        if body.shape >= shapes.len() {
                            return Err("published query body has no retained geometry".into());
                        }
                        heights.insert(body.id, body_top(body));
                        entry.insert(body.clone());
                    }
                }
            }
            output.push(Tree {
                root: tree.root,
                nodes: tree.nodes.to_vec(),
                blocks: tree.blocks.to_vec(),
            });
        }
        Ok(Self {
            trees: output,
            bodies: retained,
            shapes,
            heights,
            quantization,
        })
    }

    /// 1fc710's point-query half. A miss still requires the caller's downward
    /// ray before falling back to the active ocean height.
    pub fn particle_water_body_height(
        &self,
        executor: &mut WorldPointQuery,
        point: [f32; 3],
    ) -> Result<Option<f32>, String> {
        let trees = self
            .trees
            .iter()
            .map(|tree| PointQueryTree {
                root: tree.root,
                nodes: &tree.nodes,
                blocks: &tree.blocks,
            })
            .collect::<Vec<_>>();
        let hit = executor.closest_hit(
            self,
            &trees,
            self.quantization,
            &WorldPointRequest::particle_water(point),
            |index| {
                Ok(WorldQueryShape {
                    shapes: self
                        .shapes
                        .get(index)
                        .ok_or("query snapshot shape absent")?,
                    materials: None,
                })
            },
        )?;
        hit.map(|(hit, _)| {
            let id = hit
                .target
                .body_id
                .ok_or("query snapshot hit has no body ID")?;
            self.heights
                .get(&id)
                .copied()
                .ok_or("query snapshot body height absent".into())
        })
        .transpose()
    }
}
