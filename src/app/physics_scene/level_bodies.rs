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

/// The level preparation owner transfers its one already-read source lease.
pub(in crate::app) struct PreparedLevelBodies {
    pub path: String,
    pub source: Arc<ShapeSource>,
    pub materials: Vec<Arc<entities::physics_material::resources::Definition>>,
}

impl PhysicsScene {
    /// 882300's real worldspawn header and 852100's material table precede
    /// 852560's ordinary shared component initialization/body publication.
    pub(super) fn admit_level_bodies(
        &mut self,
        document: &entities::Document,
        prepared: &PreparedLevelBodies,
        live: &mut crate::gameplay::entity_lifetime::LiveEntities,
        registry: &mut ComponentRegistry,
        constructor_tag: crate::gameplay::name_pool::NameId,
    ) -> Result<(), String> {
        let mut worldspawns = document.definitions.iter().filter(|definition| definition.kind == "entity"
            && definition.fields.iter().any(|(key,value)| key == "class"
                && matches!(value, entities::Value::Atom(value) if value.trim_matches('"') == "idWorldspawn")));
        let definition = worldspawns
            .next()
            .ok_or("level physics worldspawn absent")?;
        if worldspawns.next().is_some() {
            return Err("multiple physics worldspawns".into());
        }
        let class = self
            .types
            .classes()
            .hierarchy()
            .registration_order()
            .iter()
            .copied()
            .find(|&index| {
                self.types
                    .classes()
                    .metadata(index)
                    .is_some_and(|metadata| {
                        self.types.classes().name_text(metadata.name)
                            == Some(b"idWorldspawn".as_slice())
                    })
            })
            .ok_or("worldspawn class metadata absent")?;
        let index = if let Some(index) = self
            .entities
            .iter()
            .position(|entity| entity.name == definition.id)
        {
            if self.entities[index].class != class {
                return Err("worldspawn admitted with a different class".into());
            }
            index
        } else {
            let index = self.entities.len();
            let shared = live.allocate_managed(
                Arc::from(definition.id.as_str()),
                class,
                None,
                0,
                constructor_tag,
            )?;
            self.entities.push(PhysicsEntity {
                retired: false,
                external: false,
                name: definition.id.clone(),
                shared,
                components: EntityComponents::new(index),
                class,
                environment: None,
                flags: 0,
                entity_flags: 0x001e_8008,
                entity_flags_d7: 0,
                priority: 0,
                frame: crate::gameplay::reference_frame::ReferenceFrameCache::default().published(),
                timespace: 0,
            });
            self.entity_flags_d4.push(4); // 882300 marks the actual worldspawn.
            index
        };
        self.entity_flags_d4[index] |= 4;
        let mut table = Vec::with_capacity(prepared.source.physics.material_map.len());
        {
            let mut world = self
                .world
                .write()
                .map_err(|_| "level physics owner poisoned")?;
            let mut ids = Vec::with_capacity(prepared.materials.len());
            for (name, definition) in prepared
                .source
                .physics
                .materials
                .iter()
                .zip(&prepared.materials)
            {
                let handle = world
                    .storage
                    .materials
                    .admit_definition(name, definition.clone())?;
                ids.push(world.storage.materials.resolve(handle, |_, _, _, _| {
                    panic!("material subscriber before level initialization")
                })?);
            }
            for &entry in &prepared.source.physics.material_map {
                table.push(
                    *ids.get(entry as usize)
                        .ok_or("level physics material map exceeds declarations")?,
                );
            }
        }
        let declaration = self.types.declaration(
            self.factories[14],
            0,
            DeclarationScheduling::from_base_flags(0),
        )?;
        let handle = ComponentFactory {
            declaration: &declaration,
            pool: &mut self.level_bodies,
            registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut self.entities[index].components,
            declaration.scheduling.starts_on_spawn(),
            |_, _, _| {
                PhysicsWorldComponent::new(prepared.path.clone(), prepared.source.clone(), table)
            },
            |_, _, _| Ok(()),
        )?
        .ok_or("physics world component creation disabled")?;
        self.declarations.insert(handle.packed(), declaration);
        Ok(())
    }
}

pub(super) struct PhysicsWorldComponent {
    pub owner: ComponentOwner,
    pub source: Option<Arc<ShapeSource>>,
    pub path: String,
    /// 852100's +40 remap table, retained by each +10 collision-data owner.
    pub materials: Arc<[u16]>,
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
            materials: materials.into(),
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
                world.begin_disabled_component_body(handle, self.owner.generation, &info)?;
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
            //851760/694c20's +10 table belongs to this actual component;
            //WorldStorage keeps the same Arc under full body generation.
            world.register_body_materials(body, self.materials.clone())?;
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
        self.materials = Arc::from([]);
        self.creation.clear();
        Ok(())
    }
}
