//! Canonical owned buffers used by creation, publication and world queries.
//! This projects the corresponding c8c250 owners; geometry and components are
//! borrowed from the level, never duplicated in a query-only world.
use super::{
    body_creation::{BodyCreation, BodyCreationCallbacks, BodyCreationGeometry, BodyInfoModifier},
    BodyAddition, BodyPublication, PublicationContext, PublicationEvents, WorldInfo,
};
use crate::assets::physworld::body::BodyInfo;
use crate::gameplay::{
    body_properties::BodyProperties,
    body_quality::{BodyQualityConfig, BodyQualityLibrary},
    body_storage::{BodyActivation, BodyPropertyBits, BodyStorage},
    broadphase::{BroadphaseQuantization, HybridBroadphase},
    collision_body::{BodyCacheSettings, SweptBoundsStep},
    collision_queries::CollisionQueries,
    constraint_links::{ConstraintLink, ConstraintLinks},
    deactivation::DeactivationWorld,
    motion_storage::MotionStorage,
    materials::MaterialOwner,
    point_query::PointQuantization,
    shape_point_query::ShapePointContact,
    shape_registry::ShapeRegistry,
    signal::Signal,
    space_cells::SpaceCells,
    world_point_query::{WorldPointQuery, WorldPointRequest, WorldQueryShape},
};

pub(crate) struct WorldStorage {
    pub info: WorldInfo,
    pub bodies: BodyStorage,
    pub motions: MotionStorage,
    pub deactivation: DeactivationWorld,
    pub broadphase: HybridBroadphase,
    pub quantization: BroadphaseQuantization,
    pub qualities: BodyQualityLibrary,
    pub cells: SpaceCells,
    pub step: SweptBoundsStep,
    pub phase: u32,
    pub step_index: u32,
    pub properties: Vec<BodyPropertyBits>,
    pub materials: MaterialOwner<usize, usize>,
    /// Per-body event chains (world+9e8). The source's signal-head word is an
    /// owned typed signal here; no parallel body-to-subscriber map is needed.
    pub body_events: BodyProperties<
        Signal<crate::gameplay::component_state::entity::ComponentHandle, super::BodyEventCallback>,
    >,
    pub constraint_heads: Vec<u32>,
    pub constraints: Vec<ConstraintLink>,
    pub constraint_next: Vec<[u32; 2]>,
    registry: ShapeRegistry,
}

impl WorldStorage {
    pub fn new(info: WorldInfo, identity: usize, materials: MaterialOwner<usize, usize>) -> Result<Self, String> {
        if info.user_body_buffer.is_some() || info.user_motion_buffer.is_some() {
            return Err("external physics buffers require their actual owner".into());
        }
        if info.quality_library.is_some()
            || info.broad_phase_config.is_some()
            || info.broad_phase_type != 1
        {
            return Err(
                "custom physics quality/broadphase resources require their actual owner".into(),
            );
        }
        let qualities = BodyQualityLibrary::new(BodyQualityConfig {
            enable_welding_for_default_objects: info.enable_welding_for_default_objects,
            enable_welding_for_critical_objects: info.enable_welding_for_critical_objects,
            enable_contact_caching: info.enable_contact_caching,
        });
        let cells = if info.simulation_type == 1 {
            SpaceCells::dynamic(info.num_splitter_cells as usize)?
        } else {
            SpaceCells::Single
        };
        Ok(Self {
            bodies: BodyStorage::new(info.body_buffer_capacity as usize)?,
            motions: MotionStorage::new(info.motion_buffer_capacity as usize)?,
            deactivation: DeactivationWorld::for_motion_capacity(
                info.motion_buffer_capacity as usize,
            )?,
            broadphase: HybridBroadphase::with_body_capacity(info.body_buffer_capacity as usize),
            quantization: BroadphaseQuantization::new(info.broad_phase_aabb),
            registry: ShapeRegistry::new(identity),
            qualities,
            cells,
            step: SweptBoundsStep::initial(),
            phase: 0,
            step_index: 0,
            properties: Vec::new(),
            materials,
            body_events: BodyProperties::default(),
            constraint_heads: Vec::new(),
            constraints: Vec::new(),
            constraint_next: Vec::new(),
            info,
        })
    }

    /// c8e5b0 on the same buffers later read by WorldPointQuery. Required
    /// creation/activation signals resolve through the caller's live component
    /// owner, including changes made by observers during this call.
    pub fn create_body<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        info: &BodyInfo,
        addition: BodyAddition,
        activation: BodyActivation,
        modifier: Option<&mut dyn BodyInfoModifier>,
    ) -> Result<u32, String> {
        self.edit_bodies(geometry, events, |mut creation| {
            creation.create(info, addition, activation, modifier)
        })
    }

    /// c8e930 admission is separate from creation during component startup.
    pub fn add_bodies<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        ids: &[u32],
        addition: BodyAddition,
        activation: BodyActivation,
    ) -> Result<(), String> {
        self.edit_bodies(geometry, events, |creation| {
            creation
                .world
                .add(ids, addition, activation, creation.callbacks)
        })
    }

    pub fn set_material<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        id: u32,
        material: u16,
        mode: i32,
    ) -> Result<bool, String> {
        self.edit_bodies(geometry, events, |creation| {
            creation
                .world
                .set_material(id, material, mode, creation.callbacks)
        })
    }

    fn edit_bodies<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks, R>(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        edit: impl FnOnce(BodyCreation<'_, '_, '_, G, E>) -> Result<R, String>,
    ) -> Result<R, String> {
        let links = || ConstraintLinks {
            heads: &self.constraint_heads,
            constraints: &self.constraints,
            next: &self.constraint_next,
        };
        let tolerance = [self.info.collision_tolerance; 4];
        let mut callbacks = PublicationContext {
            geometry,
            events,
            cache: BodyCacheSettings {
                bounds_tolerance: tolerance,
                cache_margin: self.info.aabb_margin,
                lod: Some(self.info.lod),
                step: &self.step,
                packing: &self.quantization.packed,
            },
            qualities: &self.qualities,
            constraints: links(),
            enable_deactivation: self.info.enable_deactivation,
            phase: self.phase,
            step_index: self.step_index,
        };
        let mut world = BodyPublication {
            bodies: &mut self.bodies,
            motions: &mut self.motions,
            deactivation: &mut self.deactivation,
            broadphase: &mut self.broadphase,
            packing: &self.quantization.packed,
            qualities: &self.qualities,
            constraints: links(),
            tolerance,
            step: &self.step,
            enable_deactivation: self.info.enable_deactivation,
            phase: self.phase,
            step_index: self.step_index,
        };
        edit(BodyCreation {
            world: &mut world,
            callbacks: &mut callbacks,
            registry: &mut self.registry,
            lod: &self.info.lod,
            cells: &self.cells,
            properties: &mut self.properties,
        })
    }

    /// Resolve queued requests against these actual published bodies/trees.
    /// The resource callback resolves shape handles, never supplies contacts.
    pub fn resolve_point_queries<'a>(
        &self,
        executor: &mut WorldPointQuery,
        queries: &mut CollisionQueries<WorldPointRequest, ShapePointContact>,
        resolve: impl FnMut(usize) -> Result<WorldQueryShape<'a>, String>,
    ) -> Result<(), String> {
        executor.resolve_pending(
            queries,
            &self.bodies,
            &self.broadphase.query_trees(),
            PointQuantization {
                offset: self.quantization.offset[..3].try_into().unwrap(),
                scale: self.quantization.inverse_scale[..3].try_into().unwrap(),
            },
            resolve,
        )
    }

    /// Detach mutable-shape observers while the level's resources still live.
    pub fn release(self, geometry: &mut impl BodyCreationGeometry) -> Result<(), String> {
        self.registry.release(geometry)
    }
}

#[cfg(test)]
mod tests;
