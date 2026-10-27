//! Immutable inputs published by the canonical physics world for a render
//! frame. No body creation, integration, or second collision world lives here.
use super::{PointQueryBodies, WorldPointQuery, WorldPointRequest, WorldQueryShape};
use crate::{
    assets::physworld::query_geometry::QueryShapes,
    gameplay::{
        body_storage::BodyStorage,
        collision_body::CollisionBody,
        point_query::{PointQuantization, PointQueryBody, PointQueryNode, PointQueryTree},
        ray_query::RayQuantization,
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
    wind_owners: BTreeMap<u64, Option<String>>,
    quantization: PointQuantization,
    ray_quantization: RayQuantization,
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
        ray_quantization: RayQuantization,
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
            wind_owners: BTreeMap::new(),
            quantization,
            ray_quantization,
        })
    }

    /// Component resolution belongs to the live owner. Retain identities for
    /// each collision-data owner without building a second component world.
    pub fn with_wind_owners(
        mut self,
        mut resolve: impl FnMut(&CollisionBody) -> Result<Option<String>, String>,
    ) -> Result<Self, String> {
        for body in self.bodies.values() {
            let filter = crate::gameplay::collision_filter::CollisionQueryFilter {
                layers: 0x0001_0000,
                required_flags: 0,
            };
            if filter.accepts(body.filter_info)
                && body.user_data != 0
                && !self.wind_owners.contains_key(&body.user_data)
            {
                self.wind_owners.insert(body.user_data, resolve(body)?);
            }
        }
        Ok(self)
    }

    /// 677ec0/678070 preserve narrowphase traversal order; equal-priority
    /// winds retain the first hit. Selection and payloads belong to WindSnapshot.
    pub fn wind_entities(
        &self,
        executor: &mut WorldPointQuery,
        point: [f32; 3],
    ) -> Result<Vec<&str>, String> {
        let trees = self
            .trees
            .iter()
            .map(|tree| PointQueryTree {
                root: tree.root,
                nodes: &tree.nodes,
                blocks: &tree.blocks,
            })
            .collect::<Vec<_>>();
        let request = WorldPointRequest::wind(point);
        let mut hits = Vec::new();
        executor.all_hits(
            self,
            &trees,
            self.quantization,
            point,
            request.hit,
            request.filter,
            |index| {
                Ok(WorldQueryShape {
                    shapes: self
                        .shapes
                        .get(index)
                        .ok_or("wind query snapshot shape absent")?,
                    materials: None,
                })
            },
            &mut hits,
        )?;
        let mut owners = Vec::new();
        for hit in hits {
            let owner = self
                .wind_owners
                .get(&hit.target.user_data)
                .ok_or("wind query component identity was not published")?;
            if let Some(owner) = owner {
                owners.push(owner.as_str());
            }
        }
        Ok(owners)
    }

    /// 1fc710's complete body selection: closest point, then the authored
    /// downward ray. A completed miss leaves ocean fallback to the FX owner.
    pub fn particle_water_height(
        &self,
        point_query: &mut WorldPointQuery,
        ray_query: &mut crate::gameplay::world_ray_query::WorldRayQuery,
        point: [f32; 3],
        distance: f32,
    ) -> Result<Option<f32>, String> {
        if let Some(height) = self.particle_water_body_height(point_query, point)? {
            return Ok(Some(height));
        }
        if !(0. < distance) {
            return Ok(None);
        }
        let trees = self
            .trees
            .iter()
            .map(|tree| PointQueryTree {
                root: tree.root,
                nodes: &tree.nodes,
                blocks: &tree.blocks,
            })
            .collect::<Vec<_>>();
        let hit = ray_query.closest_hit(
            self,
            &trees,
            self.ray_quantization,
            &crate::gameplay::world_ray_query::WorldRayRequest::particle_water(point, distance),
            |index| {
                Ok(WorldQueryShape {
                    shapes: self
                        .shapes
                        .get(index)
                        .ok_or("water ray snapshot shape absent")?,
                    materials: None,
                })
            },
        )?;
        hit.map(|hit| {
            let id = hit.target.body_id.ok_or("water ray hit has no body ID")?;
            self.heights
                .get(&id)
                .copied()
                .ok_or_else(|| "water ray body height absent".to_owned())
        })
        .transpose()
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
            &WorldPointRequest::water(point),
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
