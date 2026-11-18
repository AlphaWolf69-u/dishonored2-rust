//!770990→60a480/60a8b0 material contacts feed the SAME whole-FX owner.
//!This module owns only map metadata and actual ground-demand admission.
//!No projected sound/group cache, audio objects, custom RNG or tail manager.
use crate::{
    archive::Catalog,
    assets::{
        declaration_texts,
        entities::{contacts::ContactDatabase, physics_material::resources::Definition},
    },
    gameplay::{entity_lifetime::EntityId, sound::Contact},
};
use bevy::prelude::*;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

#[derive(Message)]
pub(in crate::app) struct ContactSound {
    pub contact: Contact,
    pub owner: EntityId,
    pub source: Arc<Definition>,
    pub target: Arc<Definition>,
}
struct Definitions {
    catalog: Arc<Catalog>,
    texts: declaration_texts::DeclarationTexts,
    database: Arc<ContactDatabase>,
    //Existing read-only catalog-scope diagnostic only; production receives
    //actual Body/ground leases and never populates a second material cache.
    chains: HashMap<String, Arc<Definition>>,
}
impl Definitions {
    fn load(
        catalog: Arc<Catalog>,
        map: &str,
        seed: declaration_texts::PhysicsSeed,
    ) -> Result<Self, String> {
        let base = catalog.base();
        let database = ContactDatabase::acquire(&catalog)?;
        let language = crate::assets::lang::read_text_language(base)?;
        let texts = declaration_texts::acquire(
            base,
            &catalog,
            map.trim_end_matches(".entities"),
            language,
            seed,
        )?;
        Ok(Self {
            catalog,
            texts,
            database,
            chains: HashMap::new(),
        })
    }
    fn prepare_material_chain(&mut self, name: &str) -> Result<(), String> {
        if !self.chains.contains_key(name) {
            let definition = crate::assets::entities::physics_material::resources::acquire_with(
                &self.catalog,
                name,
                |path| declaration_texts::read(&self.catalog, &self.texts, path),
            )?;
            self.chains.insert(name.to_owned(), definition);
        }
        Ok(())
    }
}
struct GroundDemand {
    target: Arc<Definition>,
    sources: Arc<[Arc<Definition>]>,
}
struct Ground {
    demand: GroundDemand,
    scopes: Vec<Arc<str>>,
}
#[derive(Resource, Default)]
pub(in crate::app) struct ContactSounds {
    definitions: Option<Result<Definitions, String>>,
    ground: Option<Ground>,
    next_ground: Option<GroundDemand>,
    ground_dirty: bool,
    pending: VecDeque<ContactSound>,
    sequence: u64,
    closed: bool,
}
impl ContactSounds {
    pub(super) fn release_level_history(&self) {
        if let Some(Ok(definitions)) = &self.definitions {
            definitions
                .texts
                .release_level_history(&definitions.catalog);
        }
    }

    ///Pure handoff of the generation admitted by the map acquisition worker.
    ///None is pending publication; a recorded acquisition failure stays an error.
    pub(super) fn database(&self) -> Result<Option<&Arc<ContactDatabase>>, String> {
        match self.definitions.as_ref() {
            None => Ok(None),
            Some(Ok(definitions)) => Ok(Some(&definitions.database)),
            Some(Err(error)) => Err(error.clone()),
        }
    }
    pub(super) fn declarations(&self) -> Option<&declaration_texts::DeclarationTexts> {
        self.definitions
            .as_ref()?
            .as_ref()
            .ok()
            .map(|definitions| &definitions.texts)
    }
    pub(super) fn acquire() -> Self {
        let definitions = (|| {
            let base =
                crate::assets::installation::game_base().map_err(|error| error.to_string())?;
            let catalog = Catalog::shared(&base).map_err(|error| error.to_string())?;
            let map = crate::gameplay::map_info::current_map().unwrap_or_default();
            Definitions::load(catalog, &map, Default::default())
        })();
        Self {
            definitions: Some(definitions),
            ..Default::default()
        }
    }
    pub(super) fn acquire_with_seed(
        catalog: Arc<Catalog>,
        map: &str,
        seed: declaration_texts::PhysicsSeed,
    ) -> Self {
        let _span = crate::app::load_timing::Span::new("contact declaration acquisition");
        Self {
            definitions: Some(Definitions::load(catalog, map, seed)),
            ..Default::default()
        }
    }
}

impl ContactSounds {
    ///Called at the EXISTING actual player ground-query publication, using
    ///the same MaterialOwner/body declarations. Unchanged witness/bank does
    ///no allocation, archive read, cloned metadata or per-frame world scan.
    pub(in crate::app) fn observe_ground(
        &mut self,
        target: Option<&Arc<Definition>>,
        sources: &Arc<[Arc<Definition>]>,
    ) {
        if self.closed {
            return;
        }
        let current = if self.ground_dirty {
            self.next_ground.as_ref()
        } else {
            self.ground.as_ref().map(|ground| &ground.demand)
        };
        if match (current, target) {
            (None, None) => true,
            (Some(current), Some(target)) => {
                Arc::ptr_eq(&current.target, target) && Arc::ptr_eq(&current.sources, sources)
            }
            _ => false,
        } {
            return;
        }
        self.next_ground = target.map(|target| GroundDemand {
            target: target.clone(),
            sources: sources.clone(),
        });
        self.ground_dirty = true;
    }
    fn prepare_ground(&mut self, world: &mut World) -> Result<(), String> {
        if self.ground_dirty {
            if let Some(ground) = self.ground.take() {
                for scope in ground.scopes {
                    super::super::explosions::release_owned(world, &scope)?;
                }
            }
            self.ground = self.next_ground.take().map(|demand| Ground {
                demand,
                scopes: Vec::new(),
            });
            self.ground_dirty = false;
        }
        let Some(ground) = self.ground.as_mut() else {
            return Ok(());
        };
        if ground.scopes.len() == ground.demand.sources.len() {
            return Ok(());
        }
        let database = self
            .definitions
            .as_ref()
            .ok_or("contact metadata was not published")?
            .as_ref()
            .map_err(Clone::clone)?
            .database
            .clone();
        while let Some(source) = ground.demand.sources.get(ground.scopes.len()) {
            let next = self
                .sequence
                .checked_add(1)
                .ok_or("contact scope generation exhausted")?;
            let scope: Arc<str> = Arc::from(format!("body-ground-contact:{next}"));
            super::super::explosions::prepare_contact_pair_leases_owned(
                world,
                scope.clone(),
                source.clone(),
                ground.demand.target.clone(),
                database.clone(),
            )?;
            //Only THIS observed ground and the actual loaded source refs.
            //No all-world material Cartesian product or synthetic lookahead.
            if let Err(error) = super::super::explosions::prewarm_contact_pair_owned(world, &scope)
            {
                super::super::explosions::release_owned(world, &scope)?;
                return Err(error);
            }
            self.sequence = next;
            ground.scopes.push(scope);
        }
        Ok(())
    }
    fn collect(&mut self, world: &mut World) -> Result<(), String> {
        let mut messages = world.resource_mut::<Messages<ContactSound>>();
        if self.closed && !messages.is_empty() {
            return Err("contact producer posted after source retirement".into());
        }
        self.pending.extend(messages.drain());
        Ok(())
    }
    fn finish(&mut self, world: &mut World) -> Result<(), String> {
        while let Some(packet) = self.pending.front() {
            let database = self
                .definitions
                .as_ref()
                .ok_or("contact metadata was not published")?
                .as_ref()
                .map_err(Clone::clone)?
                .database
                .clone();
            let current = self
                .ground
                .as_ref()
                .filter(|ground| Arc::ptr_eq(&ground.demand.target, &packet.target))
                .and_then(|ground| {
                    ground
                        .demand
                        .sources
                        .iter()
                        .position(|source| Arc::ptr_eq(source, &packet.source))
                        .and_then(|index| ground.scopes.get(index))
                })
                .cloned();
            let temporary = current.is_none();
            let scope = if let Some(scope) = current {
                scope
            } else {
                let next = self
                    .sequence
                    .checked_add(1)
                    .ok_or("accepted contact generation exhausted")?;
                let scope: Arc<str> = Arc::from(format!("accepted-body-contact:{next}"));
                super::super::explosions::prepare_contact_pair_leases_owned(
                    world,
                    scope.clone(),
                    packet.source.clone(),
                    packet.target.clone(),
                    database,
                )?;
                self.sequence = next;
                scope
            };
            let contact = &packet.contact;
            if world.contains_resource::<super::observation::Observation>() {
                eprintln!("CONTACT_FX_PACKET source={:?} target={:?} owner={:?} speed={} mass={} stimulus={} scope={scope}",
                    contact.source,contact.target,packet.owner,contact.speed,contact.mass,contact.stimulus_enabled);
            }
            let context = crate::rendering::bevy_backend::world_effects::EntityEffectContext {
                owner: Some(packet.owner),
                culprit: None,
                cannot_incapacitate_player_and_allies: false,
                player_impact: false,
                velocities: [[0.; 3]; 2],
                contact_speed: contact.speed,
                contact_mass: contact.mass,
                contact_stimulus_enabled: Some(contact.stimulus_enabled),
            };
            let result = super::super::explosions::post_contact_pair_owned(
                world,
                &scope,
                super::super::physics_scene::explosion_effects::Contact {
                    source: packet.source.name.clone(),
                    target: packet.target.name.clone(),
                    target_definition: Some(packet.target.clone()),
                    position: contact.position,
                    normal: contact.normal,
                    direction: contact.direction,
                },
                context,
            );
            //Scope leases may close after whole-FX admission. The canonical
            //Owner retains the real ticket/context/FX source through all tails.
            if temporary {
                super::super::explosions::release_owned(world, &scope)?;
            }
            result?;
            self.pending.pop_front();
        }
        Ok(())
    }
}

///Windowless diagnostics keep the same metadata path; no output device needed.
pub(super) fn initialize(mut commands: Commands, state: Res<ContactSounds>) {
    if state.definitions.is_none() {
        commands.insert_resource(ContactSounds::acquire());
    }
}
///Only actual producer work needs the exclusive World borrow. Reservations
///already submitted here progress through the canonical FX owner independently.
pub(super) fn needs_update(owner: Res<ContactSounds>, messages: Res<Messages<ContactSound>>) -> bool {
    !messages.is_empty()
        || !owner.pending.is_empty()
        || owner.ground_dirty
        || owner.ground.as_ref().is_some_and(|ground| {
            ground.scopes.len() != ground.demand.sources.len()
        })
}

pub(super) fn update(world: &mut World) -> Result<(), bevy::ecs::error::BevyError> {
    world
        .resource_scope(|world, mut owner: Mut<ContactSounds>| {
            owner.collect(world)?;
            if !owner.closed {
                owner.prepare_ground(world)?;
                owner.finish(world)?;
            }
            Ok::<(), String>(())
        })
        .map_err(Into::into)
}
pub(super) fn finish_accepted(world: &mut World) -> Result<(), String> {
    if !world.contains_resource::<ContactSounds>() {
        return if world
            .get_resource::<Messages<ContactSound>>()
            .is_some_and(|messages| !messages.is_empty())
        {
            Err("accepted contacts have no actual ContactSystem metadata owner".into())
        } else {
            Ok(())
        };
    }
    world.resource_scope(|world, mut owner: Mut<ContactSounds>| {
        owner.collect(world)?;
        owner.finish(world)
    })
}
pub(super) fn close(world: &mut World) -> Result<(), String> {
    if !world.contains_resource::<ContactSounds>() {
        return finish_accepted(world);
    }
    world.resource_scope(|world, mut owner: Mut<ContactSounds>| {
        owner.collect(world)?;
        owner.finish(world)?;
        if let Some(ground) = owner.ground.take() {
            for scope in ground.scopes {
                super::super::explosions::release_owned(world, &scope)?;
            }
        }
        owner.next_ground = None;
        owner.ground_dirty = false;
        owner.closed = true;
        Ok(())
    })
}

/// Acquire actual contact inputs in two independent archive namespaces.
pub(in crate::app) fn observe_catalogs(base: &std::path::Path, map: &str) -> Result<(), String> {
    let left = Arc::new(Catalog::open(base).map_err(|error| error.to_string())?);
    let right = Arc::new(Catalog::open(base).map_err(|error| error.to_string())?);
    let level = crate::assets::level::LevelResources::resolve(&left, map)
        .map_err(|error| error.to_string())?;
    let language = crate::assets::lang::read_text_language(base)?;
    let manifest = crate::assets::map_resources::acquire_level_list(
        base,
        &left,
        level
            .entities
            .trim_start_matches("maps/")
            .trim_end_matches(".entities"),
        language,
    )?;
    let material = manifest
        .iter()
        .find(|entry| entry.kind == "physicsMaterial")
        .and_then(|entry| entry.path.strip_prefix("generated/decls/physicsmaterial/"))
        .and_then(|path| path.strip_suffix(".decl"))
        .ok_or("contact material is absent")?;
    let mut a = ContactSounds::acquire_with_seed(left.clone(), &level.entities, Default::default());
    let mut b =
        ContactSounds::acquire_with_seed(right.clone(), &level.entities, Default::default());
    let first = a
        .definitions
        .as_mut()
        .ok_or("first contact owner is absent")?
        .as_mut()
        .map_err(|error| error.clone())?;
    let second = b
        .definitions
        .as_mut()
        .ok_or("second contact owner is absent")?
        .as_mut()
        .map_err(|error| error.clone())?;
    if !Arc::ptr_eq(&first.catalog, &left) || !Arc::ptr_eq(&second.catalog, &right) {
        return Err("contact preparation used a different archive namespace".into());
    }
    first.prepare_material_chain(material)?;
    second.prepare_material_chain(material)?;
    let first_material = &first.chains[material];
    let second_material = &second.chains[material];
    if *first.database != *second.database
        || first_material.material() != second_material.material()
        || first_material.contact_chain() != second_material.contact_chain()
        || Arc::ptr_eq(first_material, second_material)
    {
        return Err("independent contact definitions or material namespaces differ".into());
    }
    let mut signature = blake3::Hasher::new();
    let mut records = 0;
    for entry in manifest.iter() {
        if entry.kind != "physicsMaterial"
            && entry.kind != "soundevent"
            && !(entry.kind == "fx" && entry.path.contains("/contactsystem/"))
        {
            continue;
        }
        let one = first.texts.get(&entry.path);
        let two = second.texts.get(&entry.path);
        if one != two {
            return Err(format!("contact source differs for {}", entry.path));
        }
        if let Some(text) = one {
            signature.update(entry.path.as_bytes());
            signature.update(text.as_bytes());
            records += 1;
        }
    }
    let weak_left = Arc::downgrade(&left);
    let weak_right = Arc::downgrade(&right);
    drop(manifest);
    drop(a);
    drop(left);
    if weak_left.strong_count() != 0 || weak_right.strong_count() != 2 {
        return Err("retiring one contact catalog affected the other".into());
    }
    drop(b);
    drop(right);
    if weak_right.strong_count() != 0 {
        return Err("contact resources retained their catalog".into());
    }
    eprintln!("CONTACT_CATALOG_SCOPES records={records} namespaces=true materials=true independent_release=true final_release=true signature={}", signature.finalize());
    Ok(())
}
