//! arkCpntPhysicsWorld's actual map body batch (852100/852560).
//! The map loader supplies the already-read physworld lease; bodies share the
//! ordinary scene WorldStorage, shape registry and lifecycle callbacks.
use super::*;
use crate::{
    assets::physworld::{body::BodyInfo, source::ShapeSource},
    gameplay::{
        body_storage::BodyActivation,
        collision_user_data::CollisionUserData,
        component_state::ComponentInstance,
        physics_world::{body_creation::BodyCreationCallbacks, BodyAddition, PublicationEvents},
    },
};

pub(super) struct PhysicsWorldComponent {
    pub owner: ComponentOwner,
    pub source: Option<Arc<ShapeSource>>,
    pub path: String,
    /// 852100's +40 remap table, retained by each +10 collision-data owner.
    pub materials: Vec<u16>,
    pub bodies: Vec<u32>,
    shapes: Vec<usize>,
    creation: Vec<BodyInfo>,
    created: usize,
    creation_entered: bool,
    creation_owner: Option<ComponentHandle>,
    registered: usize,
    published: bool,
    destroyed: bool,
    retired_shapes: usize,
}
impl ComponentInstance for PhysicsWorldComponent {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}
impl PhysicsWorldComponent {
    pub(super) fn new(
        path: String,
        source: Arc<ShapeSource>,
        materials: Vec<u16>,
    ) -> Result<Self, String> {
        let creation = source.physics.body_infos(&source.bytes)?;
        // Source 852560 creates with motionId zero: the installed level system
        // owns static bodies. A dynamic entry needs its own authored motions.
        if creation.iter().any(|body| body.motion_type != 0) {
            return Err("level physworld contains a non-static body creation request".into());
        }
        let count = creation.len();
        Ok(Self {
            owner: Default::default(),
            source: Some(source),
            path,
            materials,
            bodies: Vec::with_capacity(count),
            shapes: Vec::with_capacity(count),
            creation,
            created: 0,
            creation_entered: false,
            creation_owner: None,
            registered: 0,
            published: false,
            destroyed: false,
            retired_shapes: 0,
        })
    }
    pub(super) fn initialize<E: PublicationEvents + BodyCreationCallbacks>(
        &mut self,
        handle: ComponentHandle,
        world: &mut WorldStorage,
        geometry: &mut TriggerAssets,
        events: &mut E,
    ) -> Result<(), String> {
        if self.destroyed {
            return Err("level physics component is retired".into());
        }
        let source = self.source.as_ref().ok_or("level physics source absent")?;
        if self.creation_owner.is_some_and(|owner| owner != handle) {
            return Err("level physics initialization belongs to another component".into());
        }
        self.creation_owner = Some(handle);
        // All fallible allocation happens before body admission. The cursor
        // keeps prior real IDs after callbacks fail, so retry never duplicates
        // a published body or its source reference.
        while self.registered < self.creation.len() {
            let index = self.registered;
            let shape = geometry.register_world_shape(
                &self.path,
                source.clone(),
                self.creation[index].shape,
            )?;
            self.creation[index].shape = shape;
            self.shapes.push(shape);
            self.registered += 1;
        }
        while self.created < self.creation.len() {
            let mut info = self.creation[self.created].clone();
            // 851760 -> 694c20 -> 18cc90: real PhysicsWorld component owner,
            // never the static motion anchor or an artificial rigid component.
            info.user_data = CollisionUserData { component: handle }.encode();
            if !self.creation_entered {
                world.begin_disabled_component_body(handle, self.owner.generation, info)?;
                self.creation_entered = true;
            }
            let body = world.resume_disabled_component_body(
                geometry,
                events,
                handle,
                self.owner.generation,
            )?;
            self.bodies.push(body);
            self.created += 1;
            self.creation_entered = false;
        }
        if !self.published {
            world.add_bodies(
                geometry,
                events,
                &self.bodies,
                BodyAddition::Immediate,
                BodyActivation::Activate,
            )?;
            self.published = true;
        }
        self.owner.mark_initialized();
        // Body creation has copied names and all per-body fields. Geometry's
        // retained source still owns compressed tables and its resource credit.
        self.creation.clear();
        self.creation.shrink_to_fit();
        Ok(())
    }
    pub(super) fn retire<E: PublicationEvents + BodyCreationCallbacks>(
        &mut self,
        world: &mut WorldStorage,
        geometry: &mut TriggerAssets,
        events: &mut E,
    ) -> Result<(), String> {
        if self.creation_entered {
            world.abort_disabled_component_body(
                geometry,
                events,
                self.creation_owner
                    .ok_or("level body creation owner absent")?,
                self.owner.generation,
            )?;
            self.creation_entered = false;
        }
        if !self.destroyed {
            // 852d60 -> c8f6b0 destroys bodies through actual observers, registry and
            // motion owners before their shape/resource references can leave.
            world.destroy_bodies(geometry, events, &self.bodies, 0)?;
            self.destroyed = true;
            self.bodies.clear();
        }
        while self.retired_shapes < self.shapes.len() {
            geometry.retire_world_shape(self.shapes[self.retired_shapes])?;
            self.retired_shapes += 1;
        }
        self.shapes.clear();
        self.source = None;
        self.materials.clear();
        self.creation.clear();
        Ok(())
    }
}
