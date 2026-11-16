//! Projectile components and retained collision scratch over the canonical world.
//! Admission/removal are delegated to runtime_entities, never a private factory.
use crate::{
    assets::projectiles::Prepared,
    gameplay::{
        collision_filter::CollisionQueryFilter,
        collision_hit::ShapeHitInfo,
        component_state::{ComponentInstance, ComponentOwner},
        entity_lifetime::EntityId,
        projectiles::{Projectile, SpawnRequest},
        shape_ray_query::ShapeRayContact,
        world_point_query::level::LevelPointQueries,
        world_ray_query::{WorldRayQuery, WorldRayRequest},
    },
};
use bevy::prelude::World;
use std::sync::Arc;

pub(super) struct Component {
    pub owner: ComponentOwner,
    pub prepared: Arc<Prepared>,
    pub runtime: Option<Projectile>,
    pub request: Option<SpawnRequest>,
    pub throwable: Option<crate::gameplay::pickable_component::CheckedComponent>,
    pub reference_frame_registered: bool,
    pub hidden: bool,
    ray: WorldRayQuery,
    pub(super) hits: Vec<ShapeRayContact>,
    initial_hits: Vec<crate::gameplay::shape_convex_query::ConvexContact>,
}
impl Component {
    pub(super) fn new(prepared: Arc<Prepared>) -> Self {
        Self {
            owner: Default::default(),
            prepared,
            runtime: None,
            request: None,
            throwable: None,
            reference_frame_registered: false,
            hidden: false,
            ray: Default::default(),
            hits: Vec::new(),
            initial_hits: Vec::new(),
        }
    }
    ///626eb0 runs only after this actor's real identity and required Throwable
    ///component have been admitted by the canonical component factory.
    pub(super) fn initialize(
        &mut self,
        id: EntityId,
        request: SpawnRequest,
        throwable: crate::gameplay::pickable_component::CheckedComponent,
    ) -> Result<(), String> {
        if self.runtime.is_some() {
            return Err("projectile initialized twice".into());
        }
        self.runtime = Some(Projectile::from_request(
            self.prepared.declaration.clone(),
            id,
            request,
            self.prepared.bounds_max.x,
        ));
        self.throwable = Some(throwable);
        self.owner.mark_initialized();
        Ok(())
    }
    ///629590 retains the collector vector between actual physics updates;
    ///full body generation, material and shape witness survive sorting.
    pub(super) fn ray_hits<'a>(
        &'a mut self,
        world: &LevelPointQueries,
        point: &mut crate::gameplay::world_point_query::WorldPointQuery,
        start: bevy_math::Vec3,
        end: bevy_math::Vec3,
        overlapping_spawner: bool,
    ) -> Result<&'a [ShapeRayContact], String> {
        let mut layers = self.prepared.declaration.collision_layers;
        if overlapping_spawner {
            layers &= 0xfffffff8;
        }
        let request = WorldRayRequest {
            origin: start.to_array(),
            direction: (end - start).to_array(),
            maximum_fraction: 1.,
            hit: ShapeHitInfo {
                body_id: None,
                material_id: 0xffff,
                shape_key: u32::MAX,
                filter_info: 0,
                user_data: 0,
            },
            filter: Some(CollisionQueryFilter {
                layers,
                required_flags: 0x800,
            }),
        };
        world.ray_hits(&mut self.ray, &request, 2, &mut self.hits)?;
        let initial = crate::gameplay::world_point_query::WorldPointRequest {
            point: start.to_array(),
            hit: request.hit,
            filter: Some(CollisionQueryFilter {
                layers: self.prepared.declaration.collision_layers & 0xfffffff8,
                required_flags: 0x800,
            }),
        };
        world.point_hits(point, &initial, &mut self.initial_hits)?;
        self.hits
            .extend(self.initial_hits.iter().map(|hit| ShapeRayContact {
                target: hit.target,
                fraction: 0.,
                position: hit.position,
                normal: hit.normal,
                flags: hit.flags,
            }));
        self.hits.rotate_right(self.initial_hits.len());
        self.hits.sort_by(|a, b| a.fraction.total_cmp(&b.fraction));
        Ok(&self.hits)
    }
}
impl ComponentInstance for Component {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}

/// A weapon reserves this same canonical source before consuming ammunition.
pub(super) fn prepare(world: &mut World, prepared: &Prepared) -> Result<(), String> {
    super::runtime_entities::request(world, prepared.entity.clone())
}
pub(super) fn prepared_ready(world: &World, prepared: &Prepared) -> Result<bool, String> {
    super::runtime_entities::ready(world, &prepared.entity)
}
pub(super) fn spawn(
    world: &mut World,
    prepared: Arc<Prepared>,
    request: SpawnRequest,
) -> Result<EntityId, String> {
    if !prepared_ready(world, &prepared)? {
        return Err("projectile source dependencies are not ready".into());
    }
    let options = super::runtime_entities::SpawnOptions {
        frame: request.frame,
        scale: 1.,
        hidden_disabled: false,
    };
    super::runtime_entities::spawn_projectile(world, prepared.entity.clone(), options, request)
}
