//! Dedicated static acoustic world created by 1400b7960/1400b8840.
use crate::{
    assets::{
        physics_material::MaterialConstants,
        physworld::{
            body::BodyInfo, mass::PrincipalMassProperties, query_geometry::QueryGeometry,
            shape::ShapeHeader,
        },
        sound_propagation::Propagation,
    },
    gameplay::{
        body_activation::ActivationOwners,
        body_storage::{BodyActivation, BodyCreationAssets},
        materials::MaterialOwner,
        motion_info::MotionInfo,
        physics_world::{
            body_creation::{BodyCreationCallbacks, BodyCreationGeometry},
            storage::WorldStorage,
            BodyAddition, MotionCreationCallbacks, PublicationEvents, PublicationGeometry,
            WorldInfo,
        },
        shape_point_query::ShapePointContact,
        shape_registry::{ShapeSignalStore, ShapeSignals},
        shape_transform::ShapeMatrix,
        world_point_query::{WorldPointQuery, WorldPointRequest, WorldQueryShape},
    },
};
use std::{path::Path, sync::Arc};

pub struct AcousticWorld {
    pub graph: Propagation,
    world: WorldStorage,
    query: WorldPointQuery,
    contacts: Vec<ShapePointContact>,
    signals: Vec<ShapeSignals>,
}

impl AcousticWorld {
    pub fn new(graph: Propagation, game_base: &Path) -> Result<Self, String> {
        // ca4dc0 defaults with exactly the overrides in 1400b7960. This
        // world has one permanent static motion and no dynamic motions.
        let info = WorldInfo {
            body_buffer_capacity: u32::try_from(graph.zones.len())
                .map_err(|_| "too many acoustic zones")?
                + 1,
            motion_buffer_capacity: 1,
            constraint_buffer_capacity: 1,
            simulation_type: 0,
            ..WorldInfo::default()
        };
        let materials = MaterialOwner::new(Arc::new(MaterialConstants::load(game_base)?), 0)?;
        let mut world = WorldStorage::new(info, 0, materials)?;
        let mut signals = (0..graph.zones.len())
            .map(|_| ShapeSignals::default())
            .collect::<Vec<_>>();
        let mut geometry = Geometry {
            graph: &graph,
            signals: &mut signals,
        };
        let mut events = Events;
        for index in 0..graph.zones.len() {
            let info = BodyInfo {
                shape: index,
                user_data: index as u64 + 1,
                ..BodyInfo::default()
            };
            world.create_body(
                &mut geometry,
                &mut events,
                &info,
                BodyAddition::Queued,
                BodyActivation::Activate,
                None,
            )?;
        }
        world.flush_additions(&mut geometry, &mut events)?;
        Ok(Self {
            graph,
            world,
            query: WorldPointQuery::new(u32::MAX)?,
            contacts: Vec::new(),
            signals,
        })
    }

    /// 1400b92e0 overwrites the stored hit and zeroes the distance threshold,
    /// but leaves collector+10 (early-out) false. 141288af0 keeps traversing
    /// zero-distance candidates, so the last projection supplies the zone.
    pub fn zone_at(&mut self, point: [f32; 3]) -> Result<Option<usize>, String> {
        let mut request = WorldPointRequest::environment(point);
        request.filter = None;
        self.contacts.clear();
        self.world.point_query(
            &mut self.query,
            &request,
            |index| {
                let zone = self
                    .graph
                    .zones
                    .get(index)
                    .ok_or("missing acoustic query geometry")?;
                Ok(WorldQueryShape {
                    shapes: &zone.geometry,
                    materials: None,
                })
            },
            &mut self.contacts,
        )?;
        self.contacts
            .last()
            .map(|hit| {
                let index = hit
                    .target
                    .user_data
                    .checked_sub(1)
                    .ok_or("null acoustic volume identity")? as usize;
                self.graph
                    .zones
                    .get(index)
                    .ok_or("acoustic hit references missing zone")?;
                Ok(index)
            })
            .transpose()
    }

    pub fn release(mut self) -> Result<(), String> {
        self.world.release(&mut Geometry {
            graph: &self.graph,
            signals: &mut self.signals,
        })
    }
}

struct Geometry<'a> {
    graph: &'a Propagation,
    signals: &'a mut [ShapeSignals],
}
impl Geometry<'_> {
    fn root(
        &self,
        index: usize,
    ) -> Result<&crate::assets::physworld::query_geometry::QueryShape, String> {
        let zone = self
            .graph
            .zones
            .get(index)
            .ok_or("missing acoustic shape")?;
        zone.geometry.get(zone.geometry.root())
    }
}
impl PublicationGeometry for Geometry<'_> {
    fn is_null(&mut self, shape: usize) -> Result<bool, String> {
        self.root(shape)?;
        Ok(false)
    }
    fn bounds(&mut self, shape: usize, frame: ShapeMatrix) -> Result<[[f32; 4]; 2], String> {
        let zone = self
            .graph
            .zones
            .get(shape)
            .ok_or("missing acoustic bounds shape")?;
        zone.geometry.bounds(zone.geometry.root(), frame)
    }
}
impl ShapeSignalStore for Geometry<'_> {
    fn signals_mut(&mut self, shape: usize) -> Result<&mut ShapeSignals, String> {
        self.signals
            .get_mut(shape)
            .ok_or_else(|| "missing acoustic shape signals".into())
    }
}
impl BodyCreationGeometry for Geometry<'_> {
    fn creation_assets(&self, info: &BodyInfo) -> Result<BodyCreationAssets<'_>, String> {
        Ok(BodyCreationAssets {
            shape: self.header(info.shape)?,
            wrapped_shapes: None,
            name: None,
            mass_distribution: None,
        })
    }
    fn header(&self, shape: usize) -> Result<ShapeHeader, String> {
        Ok(self.root(shape)?.header)
    }
    fn is_mutable(&self, shape: usize) -> Result<bool, String> {
        Ok(match &self.root(shape)?.geometry {
            QueryGeometry::Compound(shape) => shape.is_mutable,
            QueryGeometry::Polytope(_) => false,
        })
    }
    fn rotation_limit_measure(&self, _: usize) -> Result<f32, String> {
        Err("acoustic world has no dynamic bodies".into())
    }
    fn cached_mass(&self, _: usize) -> Result<Option<PrincipalMassProperties>, String> {
        Err("acoustic world has no dynamic bodies".into())
    }
    fn calculate_mass(&self, _: usize, _: [u32; 3]) -> Result<PrincipalMassProperties, String> {
        Err("acoustic world has no dynamic bodies".into())
    }
}

// This world's creation/addition signals have no subscribers. The native
// owner subscribes only to body removal, after world construction.
struct Events;
impl MotionCreationCallbacks for Events {
    fn motion_capacity_requested(&mut self, _: ActivationOwners<'_>) -> Result<(), String> {
        Err("acoustic world unexpectedly requested a dynamic motion".into())
    }
    fn motion_created(
        &mut self,
        _: ActivationOwners<'_>,
        _: &MotionInfo,
        _: u32,
    ) -> Result<(), String> {
        Err("acoustic world unexpectedly created a dynamic motion".into())
    }
}
impl BodyCreationCallbacks for Events {
    fn body_capacity_requested(&mut self, _: ActivationOwners<'_>) -> Result<(), String> {
        Err("acoustic body capacity did not include every authored zone".into())
    }
    fn body_created(
        &mut self,
        _: ActivationOwners<'_>,
        _: &BodyInfo,
        _: u32,
    ) -> Result<(), String> {
        Ok(())
    }
}
impl PublicationEvents for Events {
    fn activation(&mut self, _: ActivationOwners<'_>, _: &[u32], _: bool) -> Result<(), String> {
        Ok(())
    }
    fn added(&mut self, _: ActivationOwners<'_>, _: &[u32]) -> Result<(), String> {
        Ok(())
    }
    fn transform_changed(
        &mut self,
        _: ActivationOwners<'_>,
        _: u32,
        _: bool,
    ) -> Result<(), String> {
        Ok(())
    }
}
