//! FlashSurface4b0740 and InteractiveMap39efb0, with actual pooled identities.
use super::*;
use crate::{
    app::flash_surfaces::Key,
    assets::flash_surface::{Definition, MapDefinition},
    gameplay::{
        component_state::ComponentInstance,
        entity_events::{EntityEvents, Event, Listener, Payload},
        entity_lifetime::LiveEntities,
    },
};

pub(super) struct Surface {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    started: bool,
    requested: bool,
    loaded: bool,
    playing: bool,
    requires_map: bool,
    configured: bool,
    rendered: bool,
    reported: bool,
    static_released: bool,
    capture_remaining: f32,
    pending_map: Option<Arc<MapDefinition>>,
}
impl Surface {
    fn new(definition: Arc<Definition>) -> Self {
        Self {
            owner: Default::default(),
            capture_remaining: definition.capture_delay,
            definition,
            started: false,
            requested: false,
            loaded: false,
            playing: false,
            requires_map: false,
            configured: false,
            rendered: false,
            reported: false,
            static_released: false,
            pending_map: None,
        }
    }
}
pub(super) struct Map {
    pub owner: ComponentOwner,
    pub definition: Arc<MapDefinition>,
    listening: bool,
}
macro_rules! instance {
    ($t:ty) => {
        impl ComponentInstance for $t {
            fn owner(&self) -> &ComponentOwner {
                &self.owner
            }
            fn owner_mut(&mut self) -> &mut ComponentOwner {
                &mut self.owner
            }
        }
    };
}
instance!(Surface);
instance!(Map);

impl PhysicsScene {
    ///Direct construction rollback reaches model destruction before the
    ///ordinary entity batch. Finish the same source7e7c00 Stop/ACK first;
    ///never remove a movie behind a still-started component or steal another
    ///accepted scoped release cursor.
    pub(in crate::app) fn prepare_flash_model_retirement(
        &mut self,
        world: &mut bevy::prelude::World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: crate::gameplay::entity_lifetime::EntityId,
    ) -> Result<bool, String> {
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(true);
        };
        if live.get(id).is_none() {
            return Err("FlashSurface pre-render retirement generation stale".into());
        }
        let pending = self.entities[index]
            .components
            .entries
            .values()
            .any(|&packed| {
                ComponentHandle::from_packed(packed)
                    .ok()
                    .filter(|h| h.class_id == self.classes[52])
                    .and_then(|h| self.flash_surfaces.get(h.slot))
                    .is_some_and(|s| s.requested)
            });
        if !pending {
            return Ok(true);
        }
        if !self
            .simulation
            .as_ref()
            .is_some_and(|driver| driver.is_idle())
        {
            return Ok(false);
        }
        let registry = self
            .registry
            .as_ref()
            .ok_or("FlashSurface pre-render retirement registry borrowed")?;
        if registry.scoped_release.is_some() || registry.scoped_release_running {
            return Ok(false);
        }
        let mut registry = self
            .registry
            .take()
            .ok_or("FlashSurface pre-render retirement registry absent")?;
        let result = (|| {
            //The actual entity component map is stable during queued Stop;
            //no second per-entity queue or retained handle snapshot is needed.
            let mut cursor = 0;
            loop {
                let handle = self.entities[index]
                    .components
                    .entries
                    .values()
                    .filter_map(|&packed| ComponentHandle::from_packed(packed).ok())
                    .filter(|h| h.class_id == self.classes[52])
                    .nth(cursor);
                let Some(handle) = handle else { break };
                cursor += 1;
                registry.request_stop(
                    handle,
                    &mut interaction_components::Startup {
                        world: Some(&mut *world),
                        scene: &mut *self,
                        live: &mut *live,
                        events: &mut *events,
                    },
                )?;
            }
            Ok::<(), String>(())
        })();
        self.registry = Some(registry);
        result?;
        if self.gameplay_component_drain.is_none() {
            self.gameplay_component_drain = Some(lifetime::Retirement::default());
        }
        if !lifetime::finish_gameplay_drain(world, self, live, events)? {
            return Ok(false);
        }
        Ok(!self.entities[index]
            .components
            .entries
            .values()
            .any(|&packed| {
                ComponentHandle::from_packed(packed)
                    .ok()
                    .filter(|h| h.class_id == self.classes[52])
                    .and_then(|h| self.flash_surfaces.get(h.slot))
                    .is_some_and(|s| s.requested)
            }))
    }
    pub(in crate::app) fn write_flash_surface_diagnostics(&self, report: &mut String) {
        use std::fmt::Write;
        let _ = writeln!(
            report,
            "\nWorld FlashSurface owners={} maps={} ready={}",
            self.flash_surfaces.roster.used,
            self.interactive_maps.roster.used,
            self.flash_surfaces_ready()
        );
        for position in 0..self.flash_surfaces.roster.used as usize {
            let slot = self.flash_surfaces.roster.indices[position];
            let Some(surface) = self.flash_surfaces.get(slot) else {
                continue;
            };
            let entity = surface
                .owner
                .entity
                .and_then(|index| self.entities.get(index));
            let _ = writeln!(
                report,
                "  entity={:?} name={:?} slot={} generation={} flags={:#x} iggy_info={} started={} requested={} loaded={} playing={} requires_map={} configured={} rendered={} static={} capture_remaining={} static_released={} pending_map={}",
                entity.map(|e| e.shared),
                entity.map(|e| e.name.as_str()),
                slot,
                surface.owner.generation,
                surface.owner.flags,
                surface.definition.iggy_info,
                surface.started,
                surface.requested,
                surface.loaded,
                surface.playing,
                surface.requires_map,
                surface.configured,
                surface.rendered,
                surface.definition.is_static_texture,
                surface.capture_remaining,
                surface.static_released,
                surface.pending_map.is_some()
            );
        }
    }
    pub(super) fn admit_placed_flash_components(
        &mut self,
        catalog: &archive::Catalog,
        document: &entities::Document,
        live: &mut LiveEntities,
        registry: &mut ComponentRegistry,
        constructor_tag: crate::gameplay::name_pool::NameId,
    ) -> Result<(), String> {
        use entities::scene_motion::{atom, child};
        let declarations = document
            .definitions
            .iter()
            .filter(|d| d.kind == "component")
            .map(|d| (d.id.as_str(), d))
            .collect::<HashMap<_, _>>();
        let mut cache = HashMap::new();
        let mut surfaces = HashMap::<String, Arc<Definition>>::new();
        let mut maps = HashMap::<String, Arc<MapDefinition>>::new();
        let names = self
            .types
            .classes()
            .hierarchy()
            .registration_order()
            .iter()
            .map(|&index| {
                let metadata = self
                    .types
                    .classes()
                    .metadata(index)
                    .ok_or("world Flash entity metadata absent")?;
                Ok((
                    self.types
                        .classes()
                        .name_text(metadata.name)
                        .ok_or("world Flash entity class name absent")?
                        .to_vec(),
                    index,
                ))
            })
            .collect::<Result<HashMap<_, _>, String>>()?;
        let mut read =
            |path: &str| crate::assets::declaration_texts::read_common_or_catalog(catalog, path);
        let mut source = 0;
        for authored in document.definitions.iter().filter(|d| d.kind == "entity") {
            for (slot, value) in child(child(&authored.fields, "edit"), "m_componentDecls")
                .iter()
                .filter(|(slot, _)| slot.starts_with("item_add["))
            {
                let Some(entry) = entities::scope(Some(value)) else {
                    continue;
                };
                for (kind, factory) in [("cpntflashsurface", 52), ("cpntinteractivemap", 53)] {
                    let Some(fields) = entities::components::entry_fields(
                        entry,
                        kind,
                        &declarations,
                        &mut cache,
                        &mut read,
                    )?
                    else {
                        continue;
                    };
                    let index = if let Some(index) = self
                        .entities
                        .iter()
                        .position(|e| !e.retired && e.name == authored.id)
                    {
                        index
                    } else {
                        let class = atom(&authored.fields, "class")
                            .ok_or("world Flash entity class absent")?;
                        let class = *names
                            .get(class.as_bytes())
                            .ok_or("world Flash entity class unregistered")?;
                        let pose = entities::spawn_frame(authored)?;
                        let index = self.entities.len();
                        let shared = live.allocate_managed(
                            Arc::from(authored.id.as_str()),
                            class,
                            None,
                            0,
                            constructor_tag,
                        )?;
                        self.entities.push(PhysicsEntity {
                            retired: false,
                            external: false,
                            name: authored.id.clone(),
                            shared,
                            components: EntityComponents::new(index),
                            class,
                            environment: None,
                            flags: 0,
                            entity_flags: 0x001e_8008,
                            entity_flags_d7: 0,
                            priority: 0,
                            frame: crate::gameplay::reference_frame::PublishedFrame {
                                pose,
                                ..crate::gameplay::reference_frame::ReferenceFrameCache::default()
                                    .published()
                            },
                            timespace: 0,
                        });
                        self.entity_flags_d4.push(0);
                        index
                    };
                    let scheduling = entities::components::scheduling(
                        &fields,
                        DeclarationScheduling::from_base_flags(0),
                    )?;
                    let declaration =
                        self.types
                            .declaration(self.factories[factory], source, scheduling)?;
                    source += 1;
                    let immediate = declaration.scheduling.starts_on_spawn();
                    //Same immutable declaration identity, not one retained
                    //copy of its strings/legend vector for every instance.
                    let key = if let Some(name) = atom(entry, "mapComponentDecl") {
                        format!("map:{name}")
                    } else if entities::field(entry, "inlineDecl").is_none() {
                        format!("decl:{}", atom(entry, "inheritedDecl").unwrap_or(""))
                    } else {
                        format!("{}:{slot}", authored.id)
                    };
                    let handle = if factory == 52 {
                        let definition = match surfaces.get(&key) {
                            Some(value) => value.clone(),
                            None => {
                                let value = Arc::new(Definition::read(&fields)?);
                                surfaces.insert(key, value.clone());
                                value
                            }
                        };
                        self.create_flash_surface(
                            index,
                            definition,
                            &declaration,
                            registry,
                            immediate,
                        )?
                    } else {
                        let definition = match maps.get(&key) {
                            Some(value) => value.clone(),
                            None => {
                                let value = Arc::new(MapDefinition::read(&fields)?);
                                maps.insert(key, value.clone());
                                value
                            }
                        };
                        self.create_interactive_map(
                            index,
                            definition,
                            &declaration,
                            registry,
                            immediate,
                        )?
                    };
                    if let Some(handle) = handle {
                        self.declarations.insert(handle.packed(), declaration);
                    }
                }
            }
        }
        Ok(())
    }
    pub(in crate::app) fn has_flash_surfaces(&self) -> bool {
        self.flash_surfaces.roster.used != 0
    }
    pub(in crate::app) fn flash_surfaces_ready(&self) -> bool {
        (0..self.flash_surfaces.roster.used as usize).all(|position| {
            self.flash_surfaces
                .get(self.flash_surfaces.roster.indices[position])
                .is_some_and(|s| {
                    !s.started
                        || (s.loaded
                            && s.playing
                            && s.rendered
                            && s.pending_map.is_none()
                            && (!s.requires_map || s.configured))
                })
        })
    }
    pub(in crate::app) fn flash_update_key(&self, position: usize) -> Result<Option<Key>, String> {
        if position >= self.flash_surfaces.roster.used as usize {
            return Ok(None);
        }
        self.flash_key(ComponentHandle {
            class_id: self.classes[52],
            slot: self.flash_surfaces.roster.indices[position],
        })
        .map(Some)
    }
    fn checked_surface(&self, key: Key) -> Result<&Surface, String> {
        if key.handle.class_id != self.classes[52] {
            return Err("world movie command targeted another component class".into());
        }
        if self.flash_key(key.handle)? != key {
            return Err("world FlashSurface command generation differs".into());
        }
        self.flash_surfaces
            .get(key.handle.slot)
            .ok_or("world FlashSurface payload absent".into())
    }
    fn checked_surface_mut(&mut self, key: Key) -> Result<&mut Surface, String> {
        self.checked_surface(key)?;
        self.flash_surfaces
            .get_mut(key.handle.slot)
            .ok_or("world FlashSurface payload absent".into())
    }
    //None: source stopped. Some(None): same generation already requested.
    pub(in crate::app) fn flash_request(
        &self,
        key: Key,
    ) -> Result<Option<Option<(Arc<Definition>, bool)>>, String> {
        let s = self.checked_surface(key)?;
        if !s.started || s.owner.flags & 12 == 12 {
            return Ok(None);
        }
        if s.requested {
            return Ok(Some(None));
        }
        if s.definition.iggy_info.is_empty() {
            return Err("FlashSurface Start has no IggyInfo".into());
        }
        Ok(Some(Some((s.definition.clone(), s.requires_map))))
    }
    pub(in crate::app) fn flash_requested(&mut self, key: Key) -> Result<(), String> {
        self.checked_surface_mut(key)?.requested = true;
        Ok(())
    }
    pub(in crate::app) fn flash_static_released(&mut self, key: Key) -> Result<(), String> {
        self.checked_surface_mut(key)?.static_released = true;
        Ok(())
    }
    pub(in crate::app) fn flash_pending_map(
        &self,
        key: Key,
    ) -> Result<Option<Arc<MapDefinition>>, String> {
        let s = self.checked_surface(key)?;
        Ok((s.started && s.owner.flags & 12 != 12)
            .then(|| s.pending_map.clone())
            .flatten())
    }
    pub(in crate::app) fn flash_map_submitted(&mut self, key: Key) -> Result<(), String> {
        self.checked_surface_mut(key)?.pending_map = None;
        Ok(())
    }
    pub(super) fn is_flash_class(&self, class: i32) -> bool {
        self.classes[52..54].contains(&class)
    }
    pub(super) fn create_flash_surface(
        &mut self,
        index: usize,
        definition: Arc<Definition>,
        declaration: &ComponentDeclaration,
        registry: &mut ComponentRegistry,
        immediate: bool,
    ) -> Result<Option<ComponentHandle>, String> {
        ComponentFactory {
            declaration,
            pool: &mut self.flash_surfaces,
            registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut self.entities[index].components,
            immediate,
            |_, _, _| Ok(Surface::new(definition)),
            |_, _, _| Ok(()),
        )
    }
    pub(super) fn create_interactive_map(
        &mut self,
        index: usize,
        definition: Arc<MapDefinition>,
        declaration: &ComponentDeclaration,
        registry: &mut ComponentRegistry,
        immediate: bool,
    ) -> Result<Option<ComponentHandle>, String> {
        ComponentFactory {
            declaration,
            pool: &mut self.interactive_maps,
            registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut self.entities[index].components,
            immediate,
            |_, _, _| {
                Ok(Map {
                    owner: Default::default(),
                    definition,
                    listening: false,
                })
            },
            |_, _, _| Ok(()),
        )
    }
    fn flash_key(&self, h: ComponentHandle) -> Result<Key, String> {
        let owner = self.owner(h)?;
        let index = owner.entity.ok_or("FlashSurface entity absent")?;
        let entity = self
            .entities
            .get(index)
            .filter(|e| !e.retired)
            .ok_or("FlashSurface entity retired")?
            .shared;
        Ok(Key {
            entity,
            handle: h,
            generation: owner.generation,
        })
    }
    fn map_listener(&self, h: ComponentHandle) -> Result<Listener, String> {
        Ok(Listener::Component {
            owner: self.owner_id,
            handle: h,
            generation: self.owner(h)?.generation,
            handler: 0x199,
        })
    }
    pub(super) fn initialize_flash_component(
        &mut self,
        live: &LiveEntities,
        events: &mut EntityEvents,
        h: ComponentHandle,
    ) -> Result<(), String> {
        let key = self.flash_key(h)?;
        if live.get(key.entity).is_none() {
            return Err("FlashSurface initialization entity generation stale".into());
        }
        if h.class_id == self.classes[53] {
            if !self
                .interactive_maps
                .get(h.slot)
                .ok_or("InteractiveMap payload absent")?
                .listening
            {
                events.register(0x199, Some(key.entity), self.map_listener(h)?);
                self.interactive_maps.get_mut(h.slot).unwrap().listening = true;
            }
            let index = self.owner(h)?.entity.ok_or("InteractiveMap owner absent")?;
            if let Some(surface) = self.entities[index].components.find_component(
                [self.classes[52]; 2],
                [0, 0],
                |h| Ok(Some(self.owner(h)?.flags)),
            )? {
                self.flash_surfaces
                    .get_mut(surface.slot)
                    .ok_or("InteractiveMap FlashSurface absent")?
                    .requires_map = true;
            }
        }
        self.owner_mut(h)?.mark_initialized();
        Ok(())
    }
    pub(super) fn start_flash_component(&mut self, h: ComponentHandle) -> Result<(), String> {
        if h.class_id == self.classes[52] {
            let surface = self
                .flash_surfaces
                .get_mut(h.slot)
                .ok_or("FlashSurface payload absent")?;
            if surface.requested {
                return Err("FlashSurface restart precedes previous movie retirement".into());
            }
            surface.started = true;
            surface.capture_remaining = surface.definition.capture_delay;
        }
        Ok(())
    }
    pub(super) fn dispatch_interactive_map(
        &mut self,
        live: &LiveEntities,
        listener: Listener,
        event: &Event,
    ) -> Result<(), String> {
        let Listener::Component {
            owner,
            handle,
            generation,
            handler: 0x199,
        } = listener
        else {
            return Err("InteractiveMap callback differs".into());
        };
        let key = self.flash_key(handle)?;
        if owner != self.owner_id
            || generation != key.generation
            || event.id != 0x199
            || event.target != Some(key.entity)
            || live.get(key.entity).is_none()
        {
            return Err("InteractiveMap callback generation differs".into());
        }
        let index = self
            .owner(handle)?
            .entity
            .ok_or("InteractiveMap entity absent")?;
        let range = [self.classes[52]; 2];
        let surface = self.entities[index]
            .components
            .find_component(range, [0x28, 0], |h| Ok(Some(self.owner(h)?.flags)))?;
        if let Some(surface) = surface {
            let definition = self
                .interactive_maps
                .get(handle.slot)
                .ok_or("InteractiveMap payload absent")?
                .definition
                .clone();
            self.flash_surfaces
                .get_mut(surface.slot)
                .ok_or("InteractiveMap FlashSurface payload absent")?
                .pending_map = Some(definition);
        }
        Ok(())
    }
    pub(super) fn stop_flash_component(
        &mut self,
        world: Option<&mut bevy::prelude::World>,
        events: &mut EntityEvents,
        h: ComponentHandle,
    ) -> Result<(), String> {
        let key = self.flash_key(h)?;
        if h.class_id == self.classes[52] {
            let surface = self
                .flash_surfaces
                .get_mut(h.slot)
                .ok_or("FlashSurface payload absent")?;
            surface.started = false;
            surface.pending_map = None;
            if surface.requested {
                super::super::flash_surface_movie::remove(
                    world.ok_or("FlashSurface movie retirement requires published world")?,
                    key,
                )?;
            }
        }
        let _ = events;
        Ok(())
    }
    pub(super) fn deinitialize_flash_component(
        &mut self,
        events: &mut EntityEvents,
        h: ComponentHandle,
    ) -> Result<(), String> {
        let key = self.flash_key(h)?;
        if h.class_id == self.classes[53] {
            if self
                .interactive_maps
                .get(h.slot)
                .ok_or("InteractiveMap payload absent")?
                .listening
            {
                events.unregister(0x199, Some(key.entity), self.map_listener(h)?);
                self.interactive_maps.get_mut(h.slot).unwrap().listening = false;
            }
        }
        Ok(())
    }
    pub(super) fn flash_stop_complete(
        &mut self,
        world: Option<&mut bevy::prelude::World>,
        h: ComponentHandle,
    ) -> Result<bool, String> {
        if h.class_id != self.classes[52] {
            return Ok(true);
        }
        let key = self.flash_key(h)?;
        if !self
            .flash_surfaces
            .get(h.slot)
            .ok_or("FlashSurface payload absent")?
            .requested
        {
            return Ok(true);
        }
        let world = world.ok_or("FlashSurface retirement owner absent")?;
        super::super::flash_surface_movie::poll(world)?;
        let retired =
            super::super::flash_surface_movie::status(world, key)?.is_some_and(|s| s.retired);
        if retired {
            super::super::flash_surface_movie::forget(world, key)?;
            let s = self.flash_surfaces.get_mut(h.slot).unwrap();
            s.requested = false;
            s.loaded = false;
            s.playing = false;
            s.configured = false;
            s.rendered = false;
            s.reported = false;
            s.static_released = false;
        }
        Ok(retired)
    }
    pub(in crate::app) fn observe_flash_status(
        &mut self,
        key: Key,
        status: super::super::flash_surface_movie::Status,
        dt: f32,
        events: &mut EntityEvents,
    ) -> Result<bool, String> {
        let s = self.checked_surface_mut(key)?;
        if !s.started {
            return Ok(false);
        }
        if status.loaded && !s.loaded {
            s.loaded = true;
            events.post(Event {
                id: 0x198,
                target: Some(key.entity),
                payload: Payload::None,
            });
        }
        if status.playing && !s.playing {
            s.playing = true;
            events.post(Event {
                id: 0x199,
                target: Some(key.entity),
                payload: Payload::None,
            });
        }
        s.configured = status.configured;
        s.rendered |= status.captured;
        if !s.reported && s.loaded && s.playing && s.rendered && (!s.requires_map || s.configured) {
            s.reported = true;
            static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if *TRACE.get_or_init(|| std::env::var_os("DH2_DEBUG_WORLD_FLASH").is_some()) {
                eprintln!(
                    "WORLD_FLASH_SURFACE_READY entity={:?} handle={} generation={} iggy_info={} configured={} rendered={} static={}",
                    key.entity,
                    key.handle.packed(),
                    key.generation,
                    s.definition.iggy_info,
                    s.configured,
                    s.rendered,
                    s.definition.is_static_texture
                );
            }
        }
        //4b0f40 releases only static playback; the captured image remains
        //bound until Stop's actual target/renderer acknowledgement.
        if s.definition.is_static_texture && s.playing && !s.static_released {
            s.capture_remaining -= dt;
            return Ok(s.capture_remaining <= 0. && status.captured);
        }
        Ok(false)
    }
}
