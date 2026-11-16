//! Map-owned physics query data shared by gameplay and particle simulation.
//! The frame publication carries a lease, while the trees and body slots
//! remain in their canonical owner.
use super::{WorldPointQuery, WorldPointRequest, WorldQueryShape};
use crate::{
    assets::entities::triggers::TriggerAssets,
    gameplay::{
        collision_body::CollisionBody,
        collision_filter::CollisionQueryFilter,
        physics_world::storage::WorldStorage,
        point_query::{PointNodeChildren, PointQuantization},
        ray_query::RayQuantization,
        world_ray_query::{WorldRayQuery, WorldRayRequest},
    },
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct LevelPointQueries {
    pub storage: WorldStorage,
    pub geometry: TriggerAssets,
    wind_owners: Result<BTreeMap<u64, Option<String>>, String>,
}

impl LevelPointQueries {
    pub(crate) fn new(storage: WorldStorage, geometry: TriggerAssets) -> Self {
        Self { storage, geometry, wind_owners: Err("level physics queries are not prepared".into()) }
    }

    pub(crate) fn resolve_wind_owners(
        &self,
        mut resolve: impl FnMut(&CollisionBody) -> Result<Option<String>, String>,
    ) -> Result<BTreeMap<u64, Option<String>>, String> {
        let mut bodies = BTreeSet::new();
        let mut nodes = 0;
        let mut blocks = 0;
        for tree in self.storage.broadphase.query_trees() {
            nodes += tree.nodes.len();
            blocks += tree.blocks.len();
            let mut pending = tree.root.into_iter().collect::<Vec<_>>();
            let mut visited = vec![false; tree.nodes.len()];
            while let Some(index) = pending.pop() {
                let node = tree.nodes.get(index).ok_or("query publication node absent")?;
                if std::mem::replace(&mut visited[index], true) {
                    return Err("query publication tree has repeated nodes".into());
                }
                let block = match node.children {
                    PointNodeChildren::Branch { left, right } => {
                        pending.extend([left, right]);
                        continue;
                    }
                    PointNodeChildren::Leaf { block } => tree.blocks.get(block)
                        .ok_or("query publication block absent")?,
                };
                for reference in block {
                    let index = reference.index & 0x00ff_ffff;
                    if bodies.insert(index) {
                        let body = self.storage.bodies.get_by_index(index)
                            .ok_or("published query tree references an absent body")?;
                        self.shape(body.shape)?;
                    }
                }
            }
        }
        let mut owners = BTreeMap::new();
        let filter = CollisionQueryFilter { layers: 0x0001_0000, required_flags: 0 };
        for &index in &bodies {
            let body = self.storage.bodies.get_by_index(index)
                .ok_or("published query tree references an absent body")?;
            if filter.accepts(body.filter_info) && body.user_data != 0 {
                if let std::collections::btree_map::Entry::Vacant(entry) = owners.entry(body.user_data) {
                    entry.insert(resolve(body)?);
                }
            }
        }
        if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
            eprintln!("LEVEL_POINT_QUERIES bodies={} nodes={nodes} blocks={blocks} shapes={} wind_owners={}",
                bodies.len(), self.geometry.shapes.len(), owners.len());
        }
        Ok(owners)
    }

    pub(crate) fn publish_wind_owners(
        &mut self, owners: Result<BTreeMap<u64, Option<String>>, String>,
    ) {
        self.wind_owners = owners;
    }

    pub(crate) fn ready(&self) -> Result<(), &str> {
        self.wind_owners.as_ref().map(|_| ()).map_err(String::as_str)
    }

    fn shape(&self, index: usize) -> Result<WorldQueryShape<'_>, String> {
        let shape = self.geometry.shapes.get(index).and_then(Option::as_ref).ok_or("published query body has no retained geometry")?;
        Ok(WorldQueryShape { shapes: &shape.query_shapes, materials: Some(self) })
    }

    /// Borrow the same retained geometry used by point queries and body creation.
    pub(crate) fn ray_hits(
        &self, executor: &mut WorldRayQuery, request: &WorldRayRequest,
        query_flags: u32,
        hits: &mut Vec<crate::gameplay::shape_ray_query::ShapeRayContact>,
    ) -> Result<(), String> {
        self.storage.ray_query(executor, request, query_flags, |index| self.shape(index), hits)
    }

    pub(crate) fn ray_closest_hit(
        &self, executor: &mut WorldRayQuery, request: &WorldRayRequest,
    ) -> Result<Option<crate::gameplay::shape_ray_query::ShapeRayContact>, String> {
        self.storage.ray_closest_hit(executor, request, |index| self.shape(index))
    }

    pub(crate) fn ray_any_hit(
        &self, executor: &mut WorldRayQuery, request: &WorldRayRequest,
        query_flags: u32,
    ) -> Result<Option<crate::gameplay::shape_ray_query::ShapeRayContact>, String> {
        self.storage.ray_any_hit(executor, request, query_flags, |index| self.shape(index))
    }

    pub(crate) fn convex_hits(
        &self, executor: &mut crate::gameplay::world_convex_query::WorldConvexQuery,
        request: &crate::gameplay::world_convex_query::WorldConvexRequest<'_>,
        hits: &mut Vec<crate::gameplay::shape_convex_query::ConvexContact>,
        start_hits: &mut Vec<crate::gameplay::shape_convex_query::ConvexContact>,
    ) -> Result<(), String> {
        self.storage.convex_query(executor, request, |index| self.shape(index), hits, start_hits)
    }

    pub(crate) fn convex_hits_each(
        &self, executor: &mut crate::gameplay::world_convex_query::WorldConvexQuery,
        request: &crate::gameplay::world_convex_query::WorldConvexRequest<'_>,
        submit: &mut dyn FnMut(crate::gameplay::shape_convex_query::ConvexContact, bool),
    ) -> Result<(), String> {
        self.storage.convex_query_each(executor, request, |index| self.shape(index), submit)
    }

    /// Append the actual closest witnesses for an already admitted body pair.
    pub(crate) fn convex_body_closest(
        &self, executor: &mut crate::gameplay::shape_convex_query::ShapeConvexQuery,
        request: &crate::gameplay::shape_convex_query::ConvexClosestQuery<'_>, target_id: u32,
        contacts: &mut Vec<crate::gameplay::shape_convex_query::ConvexContact>,
    ) -> Result<(), String> {
        self.storage.convex_body_closest(executor,request,target_id,|index|self.shape(index),contacts)
    }

    pub(crate) fn point_hits(
        &self,executor:&mut WorldPointQuery,request:&WorldPointRequest,
        contacts:&mut Vec<crate::gameplay::shape_convex_query::ConvexContact>,
    )->Result<(),String>{
        self.storage.point_query_geometry(executor,request,|index|self.shape(index),contacts)
    }

    /// 677ec0/678070 keep native hit order when two wind priorities are equal.
    pub(crate) fn wind_entities(
        &self, executor: &mut WorldPointQuery, point: [f32; 3],
    ) -> Result<Vec<&str>, String> {
        let owners = self.wind_owners.as_ref().map_err(Clone::clone)?;
        let mut hits = Vec::new();
        self.storage.point_query(executor, &WorldPointRequest::wind(point), |index| self.shape(index), &mut hits)?;
        let mut names = Vec::new();
        for hit in hits {
            let owner = owners.get(&hit.target.user_data)
                .ok_or("wind query component identity was not published")?;
            if let Some(owner) = owner { names.push(owner.as_str()); }
        }
        Ok(names)
    }

    /// 1fc710 selects the closest point hit before casting the downward ray.
    pub(crate) fn particle_water_height(
        &self, point_query: &mut WorldPointQuery, ray_query: &mut WorldRayQuery,
        point: [f32; 3], distance: f32,
    ) -> Result<Option<f32>, String> {
        if let Some(height) = self.particle_water_body_height(point_query, point)? {
            return Ok(Some(height));
        }
        if !(0. < distance) { return Ok(None); }
        let quantization = &self.storage.quantization;
        let hit = ray_query.closest_hit(
            &self.storage.bodies,
            &self.storage.broadphase.query_trees(),
            RayQuantization {
                offset: quantization.offset[..3].try_into().unwrap(),
                scale: quantization.scale[..3].try_into().unwrap(),
            },
            &WorldRayRequest::particle_water(point, distance),
            |index| self.shape(index),
        )?;
        hit.map(|hit| self.body_height(hit.target.body_id)).transpose()
    }

    pub(crate) fn particle_water_body_height(
        &self, executor: &mut WorldPointQuery, point: [f32; 3],
    ) -> Result<Option<f32>, String> {
        let quantization = &self.storage.quantization;
        let hit = executor.closest_hit(
            &self.storage.bodies,
            &self.storage.broadphase.query_trees(),
            PointQuantization {
                offset: quantization.offset[..3].try_into().unwrap(),
                scale: quantization.inverse_scale[..3].try_into().unwrap(),
            },
            &WorldPointRequest::water(point),
            |index| self.shape(index),
        )?;
        hit.map(|(hit, _)| self.body_height(hit.target.body_id)).transpose()
    }

    fn body_height(&self, id: Option<u32>) -> Result<f32, String> {
        let body = self.storage.bodies.get(id.ok_or("water hit has no body ID")?)
            .ok_or("water hit body is absent")?;
        Ok(self.storage.quantization.packed.unpack_bounds(body.bounds)[1][2])
    }

    /// Observe the actual query path at published body bounds centres.
    pub(crate) fn observe_queries(&self) -> Result<(), String> {
        let mut point = WorldPointQuery::new(u32::MAX)?;
        let mut ray = WorldRayQuery::default();
        let bodies: BTreeSet<_> = self.storage.broadphase.query_trees().iter()
            .flat_map(|tree| tree.blocks.iter().flatten())
            .map(|body| body.index & 0x00ff_ffff).collect();
        let mut digest = blake3::Hasher::new();
        let mut water_hits = 0;
        let mut wind_hits = 0;
        for &index in &bodies {
            let Some(body) = self.storage.bodies.get_by_index(index) else { continue };
            let bounds = self.storage.quantization.packed.unpack_bounds(body.bounds);
            let center = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5);
            let winds = self.wind_entities(&mut point, center)?;
            let height = self.particle_water_height(&mut point, &mut ray, center, bounds[1][2] - bounds[0][2])?;
            wind_hits += winds.len();
            water_hits += usize::from(height.is_some());
            digest.update(format!("{index} {winds:?} {:?}", height.map(f32::to_bits)).as_bytes());
        }
        eprintln!("LEVEL_POINT_QUERY_OBSERVATION samples={} wind_hits={wind_hits} water_hits={water_hits} signature={}",
            bodies.len(), digest.finalize());
        Ok(())
    }
}

impl crate::gameplay::shape_tags::ShapeTagSource for LevelPointQueries {
    fn mapped_material(&self, _index: usize) -> Result<u16, String> {
        Err("level material mapping requires actual collision body context".into())
    }
    fn mapped_material_for_hit(&self, hit: &crate::gameplay::collision_hit::ShapeHitInfo, index: usize) -> Result<u16, String> {
        use crate::gameplay::physics_world::storage::material_sources::MaterialSourceView;
        use crate::gameplay::shape_tags::ShapeTagSource;
        MaterialSourceView { sources: &self.storage.material_sources, materials: &self.storage.materials }
            .mapped_material_for_hit(hit, index)
    }
    fn material_flags(&self, material: u16) -> Result<u16, String> {
        self.storage.materials.shape_filter_flags(material)
    }
}

impl Drop for LevelPointQueries {
    fn drop(&mut self) {
        match self.storage.detach_shape_observers(&mut self.geometry) {
            Ok(released) if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1") => {
                eprintln!("SHAPE_OBSERVERS_RELEASE owner=triggers registrations={released}");
            }
            Ok(_) => {}
            Err(error) => eprintln!("Trigger shape observer retirement failed: {error}"),
        }
    }
}
