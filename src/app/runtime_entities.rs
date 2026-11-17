//! Runtime entity admission8aecc0/87a4b0. Source prototypes prepare without
//! managed IDs; only accepted Spawn allocates the actual entity/component set.
use crate::{
    assets::{
        equipment::attachment::Geometry,
        runtime_entity::{Definition, Kind},
    },
    gameplay::{entity_lifetime::EntityId, reference_frame::ReferenceFrame},
};
use bevy::{
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, futures_lite::future::poll_once},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

mod placed;
pub(super) mod actor_class;
pub(in crate::app) use placed::SoundVolume as PreparedSoundVolume;
pub(in crate::app) use placed::{Child as PreparedBreakableChild, Prepared as PreparedBreakable};

#[derive(Clone, Copy)]
pub(super) struct SpawnOptions {
    pub frame: ReferenceFrame,
    pub scale: Option<[f32; 3]>,
    pub hidden_disabled: bool,
}
pub(super) struct BodyDraft {
    pub component: usize,
    pub variant: u8,
    pub draft: Option<Arc<super::rigid_body::RigidBodyDraft>>,
}
pub(super) struct BodyResources {
    pub component: usize,
    pub variant: u8,
    pub resources: Option<Arc<super::rigid_body::RigidBodyResources>>,
}
struct SourceWorker {
    shapes: crate::physworld::source::ShapeSources,
    texts: crate::assets::declaration_texts::PhysicsSeed,
}
///08c330 keys shape resources by the actual entity scale, not by an
///unscaled model plus a rendering-only multiplier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PrototypeKey {
    definition: usize,
    scale: [u32; 3],
}
pub(super) fn set_shader_clock(
    world: &mut World,
    entity: EntityId,
    group: u8,
    offset: f32,
) -> Result<(), String> {
    super::model_motion::set_shader_clock(world, entity, group, offset)
}
pub(super) fn entity_scale(definition: &Definition, override_scale: Option<[f32; 3]>) -> [f32; 3] {
    override_scale.unwrap_or_else(|| {
        definition
            .model
            .as_ref()
            .map_or([1.; 3], |model| model.placement.scale)
    })
}
fn scaled_key(definition: &Definition, scale: Option<[f32; 3]>) -> PrototypeKey {
    PrototypeKey {
        definition: definition as *const Definition as usize,
        scale: entity_scale(definition, scale).map(f32::to_bits),
    }
}
struct Prototype {
    definition: Weak<Definition>,
    model: Option<Arc<crate::assets::equipment::attachment::Descriptor>>,
    stick_model: Option<Arc<crate::assets::equipment::attachment::Descriptor>>,
    resources: Option<Arc<Vec<BodyResources>>>,
    error: Option<String>,
    fx_key: Arc<str>,
    contact_key: Arc<str>,
    owners: std::collections::HashSet<Arc<str>>,
}
struct PreparedDependencies {
    fired_sound: Option<Arc<crate::assets::wwise::SoundEventDeclaration>>,
    sounds: Vec<Arc<crate::assets::wwise::SoundEventDeclaration>>,
    bodies: Vec<BodyDraft>,
    items: HashMap<usize, Arc<crate::assets::inventory_item::PickableItem>>,
}
struct Pending {
    key: PrototypeKey,
    task: Mutex<Task<Result<PreparedDependencies, String>>>,
}
struct Actor {
    // Retain source definitions until actual component/backend acknowledgements.
    // Placed actors' definitions remain with the ordinary scene source owner.
    definition: Option<Arc<Definition>>,
    prototype: Option<PrototypeKey>,
    removing: bool,
    rollback: bool,
    rollback_queued: bool,
    children: Vec<EntityId>,
    release: Option<super::physics_scene::runtime_entities::Retirement>,
    stick: Option<Box<StickAdmission>>,
    dynamic: Option<actor_class::Dynamic>,
    breakable_class: Option<actor_class::Breakable>,
    movable: Option<actor_class::Movable>,
    grenade_birth: Option<GrenadeBirth>,
    cue: Option<actor_class::Cue>,
    ///Host destruction edge for actors created by actual manager Initialize;
    ///their typed source manager array remains the lifetime owner.
    startup_parent: Option<EntityId>,
}
struct GrenadeBirth { held: EntityId, target_flags: u32, transferred: bool, targeted: bool }
#[derive(Default)]
struct StickAdmission {
    chunks: Vec<(usize, usize, usize, EntityId)>,
    component_started: bool,
    complete: bool,
    scope: Option<Arc<str>>,
}
#[derive(Resource)]
pub(super) struct RuntimeEntities {
    catalog: Arc<crate::archive::Catalog>,
    source: Arc<Mutex<SourceWorker>>,
    fired_sounds: HashMap<PrototypeKey, Arc<crate::assets::wwise::SoundEventDeclaration>>,
    prototypes: HashMap<PrototypeKey, Prototype>,
    items: HashMap<
        PrototypeKey,
        Arc<HashMap<usize, Arc<crate::assets::inventory_item::PickableItem>>>,
    >,
    queue: VecDeque<PrototypeKey>,
    pending: Option<Pending>,
    actors: HashMap<EntityId, Actor>,
    cancelled: Arc<AtomicBool>,
    active_retirement: Option<EntityId>,
    placed_pending: VecDeque<PreparedBreakable>,
    placed_sound_volumes: VecDeque<PreparedSoundVolume>,
    placed_leases: HashMap<EntityId, Vec<placed::Lease>>,
    source_error: Option<String>,
}
fn key(definition: &Definition) -> PrototypeKey {
    scaled_key(definition, None)
}
pub(in crate::app) fn spawn_class(world: &mut World, id: EntityId) -> Result<bool, String> {
    actor_class::spawn(world, id)
}
///875290 calls the concrete +128 slot, then posts normal ARK_MSG_ACTIVATED C5.
pub(super) fn activate(world: &mut World, id: EntityId) -> Result<(), String> {
    with_scene(world, |world, scene, live, events| {
        scene.activate_runtime_actor(world, live, events, id)
    })
}
pub(in crate::app) fn activate_class(world: &mut World, id: EntityId) -> Result<bool, String> {
    actor_class::activate(world, id)
}
///The actual idEntity/arkProjectile/idDynamicEntity/arkBreakable event tables contain no pickedUp301
///handler (8cf620/914fb0). Other concrete classes need their own proof.
pub(super) fn picked_up_builtin_absent(world: &World, id: EntityId) -> Result<bool, String> {
    let live = world
        .get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
        .ok_or("pickedUp requires the actual identity owner")?;
    let (_, class) = live.get(id).ok_or("pickedUp entity generation is stale")?;
    let owner = world
        .get_resource::<RuntimeEntities>()
        .ok_or("pickedUp requires its canonical source owner")?;
    let definition = owner
        .actors
        .get(&id)
        .and_then(|actor| actor.definition.as_ref());
    if definition.is_some_and(|definition| class != definition.class_index) {
        return Err("pickedUp source and live class differ".into());
    }
    // Placed ammunition has no dynamic birth journal. Its live class uses
    // the same already resident executable metadata as the level's factories.
    // This lookup neither acquires a resource nor infers a class from a name.
    let types = owner
        .catalog
        .component_types
        .lock()
        .map_err(|_| "pickedUp class metadata owner is poisoned")?
        .clone()
        .ok_or("pickedUp executable class metadata is not resident")?;
    let metadata = types
        .classes()
        .metadata(class)
        .ok_or("pickedUp source class metadata absent")?;
    let actual = types
        .classes()
        .name_text(metadata.name)
        .ok_or("pickedUp source class name absent")?;
    if definition.is_some_and(|definition| actual != definition.class.as_bytes()) {
        return Err("pickedUp source class spelling differs from actual metadata".into());
    }
    match actual {
        b"idEntity" | b"arkProjectile" | b"idDynamicEntity" | b"arkBreakable" => Ok(true),
        b"arkGrenade" => Ok(false),
        _ => Err(format!(
            "pickedUp builtin dispatch is not verified for {}",
            String::from_utf8_lossy(actual)
        )),
    }
}
///8e7870 case12d calls the actual arkMovable783980 builtin before the
///normal named-event graph receiver. A failed pickup has no class side effect.
pub(super) fn picked_up_builtin(world: &mut World, id: EntityId, successful: bool) -> Result<(), String> {
    if picked_up_builtin_absent(world, id)? || !successful { return Ok(()); }
    let dynamic = world.get_resource_mut::<RuntimeEntities>().and_then(|mut owner| {
        owner.actors.get_mut(&id).and_then(|actor| actor.movable.as_mut())
            .map(|state| state.picked_up(true))
    });
    if dynamic.is_none() && !super::equipment_identity::mark_movable_picked_up(world, id)? {
        //Placed actual arkGrenade headers use the same ordinary retirement
        //journal. Project its ctor members only when this class consumes them.
        let mut owner = world.resource_mut::<RuntimeEntities>();
        let actor = owner.actors.entry(id).or_insert(Actor {
            definition: None, prototype: None, removing: false, rollback: false,
            rollback_queued: false, children: Vec::new(), release: None, stick: None,
            dynamic: None, breakable_class: None, movable: Some(actor_class::Movable::new()), grenade_birth: None, cue: None, startup_parent: None,
        });
        actor.movable.as_mut().ok_or("arkGrenade constructor state is absent")?.picked_up(true);
    }
    with_scene(world, |world, scene, live, _| {
        scene.mark_movable_picked_up(live, id)?;
        super::model_motion::set_entity_shadow_casting(world, live, id, false)
    })
}
///43f9b0 reads actual arkMovable+360, not idEntity's bind parent or culprit.
pub(in crate::app) fn movable_linked_entity(world:&World,id:EntityId)->Result<Option<EntityId>,String> {
    let live=world.get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
        .ok_or("Movable link requires the actual identity owner")?;
    let (_,class)=live.get(id).ok_or("Movable link target generation is stale")?;
    let owner=world.get_resource::<RuntimeEntities>().ok_or("Movable constructor owner absent")?;
    let types=owner.catalog.component_types.lock().map_err(|_|"Movable class metadata poisoned")?
        .clone().ok_or("Movable executable class metadata absent")?;
    let metadata=types.classes().metadata(class).ok_or("Movable class metadata absent")?;
    let name=types.classes().name_text(metadata.name).ok_or("Movable class name absent")?;
    if !matches!(name,b"arkMovable"|b"arkGrenade") {
        return Err(format!("Movable link constructor is not hosted for {}",String::from_utf8_lossy(name)));
    }
    if let Some(definition)=owner.actors.get(&id).and_then(|actor|actor.definition.as_ref()) {
        if definition.class_index!=class || definition.class.as_bytes()!=name {
            return Err("Movable constructor and live class differ".into());
        }
        if owner.actors[&id].movable.is_none() {
            return Err("runtime Movable constructor payload absent".into());
        }
    }
    let link=owner.actors.get(&id).and_then(|actor|actor.movable.as_ref())
        .map(|state|state.linked_entity()).or_else(||super::equipment_identity::movable_linked_entity(world,id));
    //Placed headers project ctor state on demand.783940 sets the managed
    //link NULL;4e65a0 preserves it. No admitted class setter7839c0 currently
    //changes that member. A future setter must retain it in this same journal.
    let linked=link.flatten();
    Ok(linked.filter(|linked|live.get(*linked).is_some()))
}
pub(super) fn bounds(world: &World, id: EntityId) -> Result<Option<[[f32; 3]; 2]>, String> {
    if let Some(bounds) = super::world_models::bounds(world, id) {
        return Ok(Some(bounds));
    }
    let live = world
        .get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
        .ok_or("entity bounds require the actual borrowed identity owner")?;
    bounds_for(world, live, id)
}
pub(super) fn bounds_for(
    world: &World,
    live: &crate::gameplay::entity_lifetime::LiveEntities,
    id: EntityId,
) -> Result<Option<[[f32; 3]; 2]>, String> {
    super::world_models::entity_bounds(world, live, id)
}

pub(super) fn initialize(world: &mut World) -> Result<(), String> {
    if world.contains_resource::<RuntimeEntities>() {
        return Err("runtime entity owner already exists".into());
    }
    let catalog = world
        .resource::<super::SceneData>()
        .catalog
        .clone()
        .ok_or("runtime entity catalog is absent")?;
    // Scene preparation has transferred the preload seed into ContactSounds.
    // Runtime acquisition starts after that handoff and borrows the canonical
    // common text snapshot. Typed declaration/shape owners still coalesce
    // resident resources; do not duplicate or recover the consumed seed.
    let language = crate::assets::lang::read_text_language(catalog.base())?;
    let texts = crate::assets::declaration_texts::PhysicsSeed::new(&catalog, language);
    let source = Arc::new(Mutex::new(SourceWorker {
        shapes: crate::physworld::source::ShapeSources::new(&catalog),
        texts,
    }));
    let placed = world
        .resource_mut::<super::physics_scene::PhysicsScene>()
        .take_prepared_breakables();
    let volumes = world
        .resource_mut::<super::physics_scene::PhysicsScene>()
        .take_prepared_sound_volumes();
    world.insert_resource(RuntimeEntities {
        catalog,
        source,
        fired_sounds: HashMap::new(),
        prototypes: HashMap::new(),
        items: HashMap::new(),
        queue: VecDeque::new(),
        pending: None,
        actors: HashMap::new(),
        cancelled: Arc::default(),
        active_retirement: None,
        placed_pending: placed.into(),
        placed_sound_volumes: volumes.into(),
        placed_leases: HashMap::new(),
        source_error: None,
    });
    Ok(())
}
fn effect_paths(definition: &Definition) -> Vec<String> {
    let mut paths = Vec::new();
    for component in definition
        .components
        .iter()
        .chain(&definition.stick_components)
    {
        match &component.kind {
            Kind::Projectile(value) => paths.extend(value.effect_paths().map(str::to_owned)),
            Kind::Breakable(value) => paths.extend(value.fx_paths().map(str::to_owned)),
            Kind::DamageZone(value)=>paths.extend(value.effect_paths().map(str::to_owned)),
            Kind::Grenade(value)=>paths.extend(value.fx_paths().map(str::to_owned)),
            _ => {}
        }
    }
    paths.sort_unstable();
    paths.dedup();
    paths
}
/// Explicit demand is idempotent per owner, including shared ammunition and
/// preconstructed chunk definitions. No arsenal-wide strong prototype cache.
pub(super) fn request_for(
    world: &mut World,
    scope: Arc<str>,
    definition: Arc<Definition>,
) -> Result<(), String> {
    let mut admitted = Vec::new();
    let result = request_inner(world, scope, definition, None, &mut admitted);
    if result.is_err() {
        if let Some(mut owner) = world.get_resource_mut::<RuntimeEntities>() {
            for (identity, scope) in admitted {
                if let Some(prototype) = owner.prototypes.get_mut(&identity) {
                    prototype.owners.remove(&scope);
                }
            }
        }
    }
    result
}
pub(super) fn request_scaled_for(
    world: &mut World,
    scope: Arc<str>,
    definition: Arc<Definition>,
    scale: [f32; 3],
) -> Result<(), String> {
    let mut admitted = Vec::new();
    let result = request_inner(world, scope, definition, Some(scale), &mut admitted);
    if result.is_err() {
        if let Some(mut owner) = world.get_resource_mut::<RuntimeEntities>() {
            for (identity, scope) in admitted {
                if let Some(prototype) = owner.prototypes.get_mut(&identity) {
                    prototype.owners.remove(&scope);
                }
            }
        }
    }
    result
}
fn request_inner(
    world: &mut World,
    scope: Arc<str>,
    definition: Arc<Definition>,
    override_scale: Option<[f32; 3]>,
    admitted: &mut Vec<(PrototypeKey, Arc<str>)>,
) -> Result<(), String> {
    if !matches!(definition.class.as_str(), "idEntity" | "arkProjectile" | "idDynamicEntity" | "arkBreakable" | "arkGrenade" | "arkAIWorld" | "arkAICue") {
        return Err(format!(
            "runtime actor {} class {} has no actual constructor adapter",
            definition.entity, definition.class
        ));
    }
    let scale = entity_scale(&definition, override_scale);
    if scale.iter().any(|value| !value.is_finite()) {
        return Err("runtime entity scale is nonfinite".into());
    }
    let identity = scaled_key(&definition, Some(scale));
    if let Some(mut owner) = world.get_resource_mut::<RuntimeEntities>() {
        if owner.cancelled.load(Ordering::Acquire) {
            return Err("runtime entity acquisition is closed".into());
        }
        if let Some(prototype) = owner.prototypes.get_mut(&identity) {
            if let Some(error) = &prototype.error {
                return Err(error.clone());
            }
            if prototype.owners.insert(scope.clone()) {
                admitted.push((identity, scope.clone()));
            }
            drop(owner);
            for chunk in definition.chunks.iter().chain(&definition.stick_chunks) {
                request_inner(
                    world,
                    scope.clone(),
                    chunk.definition.clone(),
                    Some(scale),
                    admitted,
                )?;
            }
            for child in &definition.startup_children {
                request_inner(world, scope.clone(), child.clone(), None, admitted)?;
            }
            return Ok(());
        }
    } else {
        return Err("runtime entity preparation owner is absent".into());
    }
    let fx_key: Arc<str> = Arc::from(format!(
        "entity-source:{:x}:{:08x}:{:08x}:{:08x}",
        identity.definition, identity.scale[0], identity.scale[1], identity.scale[2]
    ));
    if !world.contains_resource::<super::usable_effects::EffectRequests>() {
        return Err("runtime entity FX resource owner is absent".into());
    }
    if let Some(model) = &definition.model {
        if let Err(error) = super::world_models::prewarm_for(world, fx_key.clone(), model.clone()) {
            super::world_models::release_prewarm_for(world, &fx_key, model)?;
            return Err(error);
        }
    }
    if let Some(model) = &definition.stick_model {
        if let Err(error) = super::world_models::prewarm_for(world, fx_key.clone(), model.clone()) {
            if let Some(base) = &definition.model {
                super::world_models::release_prewarm_for(world, &fx_key, base)?;
            }
            super::world_models::release_prewarm_for(world, &fx_key, model)?;
            return Err(error);
        }
    }
    world
        .get_resource_mut::<super::usable_effects::EffectRequests>()
        .ok_or("runtime entity FX resource owner is absent")?
        .reserve(fx_key.clone(), effect_paths(&definition));
    let mut owner = world.resource_mut::<RuntimeEntities>();
    owner.prototypes.insert(
        identity,
        Prototype {
            definition: Arc::downgrade(&definition),
            model: definition.model.clone(),
            stick_model: definition.stick_model.clone(),
            resources: None,
            error: None,
            contact_key: Arc::from(format!("{fx_key}:contact")),
            fx_key,
            owners: std::collections::HashSet::from([scope.clone()]),
        },
    );
    owner.queue.push_back(identity);
    admitted.push((identity, scope.clone()));
    let contact_key = owner.prototypes[&identity].contact_key.clone();
    drop(owner);
    if let Some(material) = definition
        .projectile
        .as_ref()
        .and_then(|projectile| projectile.material.as_deref())
    {
        super::explosions::prepare_contact_owned(world, contact_key, material)?;
    }
    for chunk in definition.chunks.iter().chain(&definition.stick_chunks) {
        request_inner(
            world,
            scope.clone(),
            chunk.definition.clone(),
            Some(scale),
            admitted,
        )?;
    }
    for child in &definition.startup_children {
        request_inner(world, scope.clone(), child.clone(), None, admitted)?;
    }
    Ok(())
}
pub(super) fn ready(world: &World, definition: &Definition) -> Result<bool, String> {
    ready_inner(world, definition, None)
}
pub(super) fn debug_readiness(world:&World,definition:&Definition)->String {
    let Some(owner)=world.get_resource::<RuntimeEntities>()else{return format!("entity[{}] owner=absent",definition.entity);};
    let key=scaled_key(definition,Some(entity_scale(definition,None)));
    let Some(prototype)=owner.prototypes.get(&key)else{return format!("entity[{}] demand=absent",definition.entity);};
    format!("entity[{}] ready={:?} resources={} task={} queued={} error={:?} audio={:?} model={:?} stick_model={:?} children={:?}\n    {}\n    {}",
        definition.entity,ready(world,definition),prototype.resources.is_some(),owner.pending.as_ref().is_some_and(|pending|pending.key==key),owner.queue.contains(&key),
        prototype.error,super::audio::events_ready(world,&prototype.fx_key),
        prototype.model.as_ref().map(|model|super::world_models::prototype_ready(world,model)),
        prototype.stick_model.as_ref().map(|model|super::world_models::prototype_ready(world,model)),
        definition.chunks.iter().chain(&definition.stick_chunks).map(|chunk|(&chunk.definition.entity,ready_inner(world,&chunk.definition,Some(entity_scale(definition,None))))).collect::<Vec<_>>(),
        super::usable_effects::debug_reservation(world,&prototype.fx_key),
        super::explosions::debug_owned(world,&prototype.contact_key))
}
pub(super) fn ready_scaled(
    world: &World,
    definition: &Definition,
    scale: [f32; 3],
) -> Result<bool, String> {
    ready_inner(world, definition, Some(scale))
}
fn ready_inner(
    world: &World,
    definition: &Definition,
    override_scale: Option<[f32; 3]>,
) -> Result<bool, String> {
    let scale = entity_scale(definition, override_scale);
    let Some(prototype) = world
        .get_resource::<RuntimeEntities>()
        .and_then(|owner| owner.prototypes.get(&scaled_key(definition, Some(scale))))
    else {
        return Ok(false);
    };
    if let Some(error) = &prototype.error {
        return Err(error.clone());
    }
    if prototype.resources.is_none()
        || !super::audio::events_ready(world, &prototype.fx_key)?
        || !super::usable_effects::reservation_ready(world, &prototype.fx_key)?
    {
        return Ok(false);
    }
    if let Some(material) = definition
        .projectile
        .as_ref()
        .and_then(|projectile| projectile.material.as_deref())
    {
        if !super::explosions::ready_contact_owned(world, &prototype.contact_key, material)? {
            return Ok(false);
        }
    }
    if let Some(model) = &prototype.model {
        if !super::world_models::prototype_ready(world, model)? {
            return Ok(false);
        }
    }
    if let Some(model) = &prototype.stick_model {
        if !super::world_models::prototype_ready(world, model)? {
            return Ok(false);
        }
    }
    for chunk in definition.chunks.iter().chain(&definition.stick_chunks) {
        if !ready_inner(world, &chunk.definition, Some(scale))? {
            return Ok(false);
        }
    }
    for child in &definition.startup_children {
        if !ready_inner(world, child, None)? { return Ok(false); }
    }
    Ok(true)
}
fn startup_readiness(world: &World) -> Result<bool, String> {
    let Some(owner) = world.get_resource::<RuntimeEntities>() else {
        return Ok(true);
    };
    if let Some(error) = &owner.source_error {
        return Err(error.clone());
    }
    for (identity, prototype) in &owner.prototypes {
        if prototype.owners.is_empty() {
            continue;
        }
        if let Some(error) = &prototype.error {
            return Err(error.clone());
        }
        let definition = prototype
            .definition
            .upgrade()
            .ok_or("demanded runtime entity source generation expired")?;
        if !ready_inner(world, &definition, Some(identity.scale.map(f32::from_bits)))? {
            return Ok(false);
        }
    }
    Ok(owner.placed_pending.is_empty() && owner.placed_sound_volumes.is_empty())
}
pub(super) fn startup_pending(world: &World) -> bool {
    !matches!(startup_readiness(world), Ok(true))
}
pub(super) fn startup_error(world: &World) -> Option<String> {
    startup_readiness(world).err()
}
pub(super) fn contact_scope(world: &World, definition: &Definition) -> Option<Arc<str>> {
    Some(
        world
            .get_resource::<RuntimeEntities>()?
            .prototypes
            .get(&key(definition))?
            .contact_key
            .clone(),
    )
}
///The actual born actor retains this prototype until component Stop/Deinit
///and its FX tickets retire. Its already acknowledged dependency reservation
///can therefore serve that same Grenade without another asynchronous Reserve.
pub(super) fn grenade_dependency_scope(world: &World, id: EntityId,
    grenade: &Arc<crate::assets::grenades::Definition>) -> Result<Option<Arc<str>>, String> {
    let Some(owner) = world.get_resource::<RuntimeEntities>() else { return Ok(None); };
    let Some(actor) = owner.actors.get(&id) else { return Ok(None); };
    //Placed actors can have a removal-only journal with no runtime prototype.
    //Their actual component still owns its ordinary dependency reservation.
    if actor.prototype.is_none() && actor.definition.is_none() { return Ok(None); }
    let definition = actor.definition.as_ref().ok_or("Grenade actor has no actual source definition")?;
    if !definition.components.iter().chain(&definition.stick_components)
        .any(|component|matches!(&component.kind, Kind::Grenade(source) if Arc::ptr_eq(source, grenade))) {
        return Err("Grenade payload differs from its actual runtime prototype source".into());
    }
    let key = actor.prototype.ok_or("Grenade actor has no retained source prototype")?;
    let prototype = owner.prototypes.get(&key).ok_or("Grenade actor's retained source prototype is absent")?;
    Ok(Some(prototype.fx_key.clone()))
}
pub(super) fn fired_sound(
    world: &World,
    definition: &Definition,
) -> Option<Arc<crate::assets::wwise::SoundEventDeclaration>> {
    world
        .get_resource::<RuntimeEntities>()?
        .fired_sounds
        .get(&key(definition))
        .cloned()
}
pub(super) fn release_for(
    world: &mut World,
    scope: &str,
    definition: &Definition,
) -> Result<(), String> {
    if let Some(mut owner) = world.get_resource_mut::<RuntimeEntities>() {
        for (identity, prototype) in owner.prototypes.iter_mut() {
            if identity.definition == definition as *const Definition as usize {
                prototype.owners.remove(scope);
            }
        }
    }
    for chunk in definition.chunks.iter().chain(&definition.stick_chunks) {
        release_for(world, scope, &chunk.definition)?;
    }
    for child in &definition.startup_children { release_for(world, scope, child)?; }
    Ok(())
}
/// Complete canonical dependency workers during loading and play. Requests
/// queued by equipment/world consumers do not run their own preparation loop.
pub(super) fn advance_frame(world: &mut World) {
    if let Err(error) = advance(world) {
        error!("Runtime entity dependency preparation failed: {error}");
        crate::platform::client_diagnostics::record(&format!(
            "Runtime entity dependency preparation failed: {error}"
        ));
        world.write_message(AppExit::error());
    }
}
pub(super) fn advance(world: &mut World) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<RuntimeEntities>() else {
        return Ok(());
    };
    let result: Result<(), String> = (|| {
        if let Some(pending) = owner.pending.as_mut() {
            let result = block_on(poll_once(
                pending
                    .task
                    .get_mut()
                    .unwrap_or_else(|error| error.into_inner()),
            ));
            if let Some(result) = result {
                let identity = pending.key;
                owner.pending = None;
                let result = result.and_then(|prepared| {
                    let resources = world
                        .resource_mut::<super::physics_scene::PhysicsScene>()
                        .publish_runtime_resources(prepared.bodies)?;
                    let scope = owner
                        .prototypes
                        .get(&identity)
                        .ok_or("prepared entity demand disappeared")?
                        .fx_key
                        .clone();
                    super::audio::reserve_events(world, scope, prepared.sounds.into_iter())?;
                    if let Some(sound) = prepared.fired_sound {
                        owner.fired_sounds.insert(identity, sound);
                    }
                    owner.items.insert(identity, Arc::new(prepared.items));
                    Ok(resources)
                });
                if let Some(prototype) = owner.prototypes.get_mut(&identity) {
                    match result {
                        Ok(resources) => prototype.resources = Some(Arc::new(resources)),
                        Err(error) => prototype.error = Some(error),
                    }
                }
            }
        }
        if owner.pending.is_none() && !owner.cancelled.load(Ordering::Acquire) {
            while let Some(identity) = owner.queue.pop_front() {
                let Some(definition) = owner
                    .prototypes
                    .get(&identity)
                    .and_then(|prototype| prototype.definition.upgrade())
                else {
                    continue;
                };
                let materials = definition
                    .model
                    .iter()
                    .chain(definition.stick_model.iter())
                    .flat_map(|model| model.material_requests())
                    .map(|name| (name, super::material_loading::MaterialGeometry::Model))
                    .collect::<Vec<_>>();
                let snapshot = super::runtime_materials::MaterialSnapshot::capture(
                    world.resource::<super::runtime_materials::RuntimeMaterialOwner>(),
                    world.resource::<super::SceneRenderQueue>(),
                    &materials,
                    world.resource::<Assets<super::ProbeMaterial>>(),
                    world.resource::<Assets<super::DecalMaterial>>(),
                )?;
                let source = owner.source.clone();
                let catalog = owner.catalog.clone();
                let cancelled = owner.cancelled.clone();
                let task = IoTaskPool::get().spawn(async move {
                    let mut source = source
                        .lock()
                        .map_err(|_| "runtime physics source owner is poisoned")?;
                    let mut materials = snapshot.cache;
                    let mut drafts = Vec::new();
                    let mut items = HashMap::new();
                    let mut sounds = Vec::new();
                    let inventory =
                        crate::assets::inventory_item::PickableSourceLease::for_catalog(&catalog)?;
                    for (index, component) in definition
                        .components
                        .iter()
                        .chain(&definition.stick_components)
                        .enumerate()
                    {
                        if cancelled.load(Ordering::Acquire) {
                            return Err("runtime physics preparation cancelled".into());
                        }
                        if let Kind::Audio(definition) = &component.kind {
                            sounds.extend(definition.events.iter().flatten().cloned());
                        }
                        if let Kind::Pickable(definition) = &component.kind {
                            sounds.extend(definition.begin_pickup_sound.iter().cloned());
                            if !definition.item.is_empty() {
                                items.insert(
                                    index,
                                    inventory
                                        .0
                                        .lock()
                                        .map_err(|_| "runtime inventory source owner is poisoned")?
                                        .acquire(&catalog, &definition.item)?,
                                );
                            }
                        }
                        let settings = match &component.kind {
                            Kind::Rigid(definition) => &definition.settings,
                            Kind::Throwable(definition) => &definition.rigid.settings,
                            _ => continue,
                        };
                        let mut models = vec![(0, definition.model.as_ref())];
                        if index >= definition.components.len() {
                            if let Some(model) = &definition.stick_model {
                                models.push((1, Some(model)));
                            }
                        }
                        for (variant, model) in models {
                            let Some(model) = model else {
                                //808ef9 shares the actual virtual10 self-unlink
                                //path with NoPhysicsShape. Keep the real component
                                //through that Init boundary; do not invent a body.
                                drafts.push(BodyDraft {
                                    component: index,
                                    variant,
                                    draft: None,
                                });
                                continue;
                            };
                            let render = match &model.geometry {
                                Geometry::Rigid(model) => {
                                    Some(super::rigid_body::MaterialSurfaces::Rigid(&***model))
                                }
                                Geometry::Animated { model, .. } => {
                                    Some(super::rigid_body::MaterialSurfaces::Animated(model))
                                }
                            };
                            let SourceWorker { shapes, texts } = &mut *source;
                            let mut placement = model.placement.clone();
                            placement.scale = identity.scale.map(f32::from_bits);
                            let draft = super::rigid_body::prepare_draft_with_surfaces(
                                &catalog,
                                shapes,
                                texts,
                                settings,
                                &placement,
                                render,
                                Some(&mut materials),
                            )?;
                            drafts.push(BodyDraft {
                                component: index,
                                variant,
                                draft: draft.map(Arc::new),
                            });
                        }
                    }
                    let fired_sound = definition
                        .projectile
                        .as_ref()
                        .and_then(|value| value.effects.fired_sound.as_deref())
                        .map(|name| {
                            crate::assets::wwise::event_resources::acquire_with(
                                &catalog,
                                name,
                                |path| {
                                    crate::assets::declaration_texts::read_common_or_catalog(
                                        &catalog, path,
                                    )
                                },
                            )
                        })
                        .transpose()?;
                    sounds.extend(fired_sound.iter().cloned());
                    Ok(PreparedDependencies {
                        fired_sound,
                        sounds,
                        bodies: drafts,
                        items,
                    })
                });
                owner.pending = Some(Pending {
                    key: identity,
                    task: Mutex::new(task),
                });
                break;
            }
        }
        let dead = owner
            .prototypes
            .iter()
            .filter_map(|(&identity, prototype)| {
                (prototype.owners.is_empty()
                    && !owner
                        .actors
                        .values()
                        .any(|actor| actor.prototype == Some(identity))
                    && owner
                        .pending
                        .as_ref()
                        .is_none_or(|pending| pending.key != identity))
                .then_some(identity)
            })
            .collect::<Vec<_>>();
        for identity in dead {
            let scope = owner.prototypes[&identity].fx_key.clone();
            super::audio::release_events(world, &scope)?;
            super::explosions::release_owned(world, &owner.prototypes[&identity].contact_key)?;
            if let Some(model) = owner.prototypes[&identity].model.as_ref() {
                super::world_models::release_prewarm_for(world, &scope, model)?;
            }
            if let Some(model) = owner.prototypes[&identity].stick_model.as_ref() {
                super::world_models::release_prewarm_for(world, &scope, model)?;
            }
            world
                .resource_mut::<super::usable_effects::EffectRequests>()
                .release_reservation(scope);
            owner.items.remove(&identity);
            owner.fired_sounds.remove(&identity);
            owner.prototypes.remove(&identity);
        }
        Ok(())
    })();
    world.insert_resource(owner);
    result?;
    if let Err(error) = placed::advance(world) {
        world.resource_mut::<RuntimeEntities>().source_error = Some(error.clone());
        return Err(error);
    }
    let failed = world
        .resource::<RuntimeEntities>()
        .actors
        .iter()
        .filter_map(|(&id, actor)| (actor.rollback && !actor.rollback_queued).then_some(id))
        .collect::<Vec<_>>();
    let abort = failed.iter().copied().filter(|id|world.resource::<RuntimeEntities>().actors[id]
        .release.as_ref().is_none_or(|release|!release.rollback_done)).collect::<Vec<_>>();
    if !abort.is_empty() {
        //Keep source parent/child state available to callbacks while the one
        //registry withdraws this failed constructor closure in one cursor edit.
        if !with_scene(world, |world,scene,live,events|
            scene.prepare_runtime_rollbacks(world,live,events,&abort))? { return Ok(()); }
        let mut owner = world.resource_mut::<RuntimeEntities>();
        for id in abort {
            owner.actors.get_mut(&id).ok_or("runtime rollback journal absent")?
                .release.get_or_insert_with(Default::default).rollback_done = true;
        }
    }
    for id in failed {
        // An accepted managed identity retires through the one ordinary FIFO.
        // Do not cancel another removal request or bypass save-name preparation.
        world
            .resource_mut::<crate::gameplay::entity_lifetime::LiveEntities>()
            .request_removal(id)?;
        world
            .resource_mut::<RuntimeEntities>()
            .actors
            .get_mut(&id)
            .ok_or("runtime rollback journal absent")?
            .rollback_queued = true;
    }
    Ok(())
}
/// Borrow the sole gameplay owners together; a missing later owner never
/// drops owners already removed from World on an error path.
pub(super) fn with_scene<T>(
    world: &mut World,
    callback: impl FnOnce(
        &mut World,
        &mut super::physics_scene::PhysicsScene,
        &mut crate::gameplay::entity_lifetime::LiveEntities,
        &mut crate::gameplay::entity_events::EntityEvents,
    ) -> Result<T, String>,
) -> Result<T, String> {
    let mut scene = world
        .remove_resource::<super::physics_scene::PhysicsScene>()
        .ok_or("runtime physics scene is absent")?;
    let Some(mut live) = world.remove_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
    else {
        world.insert_resource(scene);
        return Err("runtime managed entity owner is absent".into());
    };
    let Some(mut events) = world.remove_resource::<crate::gameplay::entity_events::EntityEvents>()
    else {
        world.insert_resource(live);
        world.insert_resource(scene);
        return Err("runtime entity event owner is absent".into());
    };
    let result = callback(world, &mut scene, &mut live, &mut events);
    world.insert_resource(events);
    world.insert_resource(live);
    world.insert_resource(scene);
    result
}
pub(super) fn spawn(
    world: &mut World,
    definition: Arc<Definition>,
    options: SpawnOptions,
) -> Result<EntityId, String> {
    spawn_inner(world, definition, options, None, None)
}
pub(super) fn spawn_projectile(
    world: &mut World,
    definition: Arc<Definition>,
    options: SpawnOptions,
    request: crate::gameplay::projectiles::SpawnRequest,
) -> Result<EntityId, String> {
    spawn_inner(world, definition, options, Some(request), None)
}
///639970 configures accepted throw momentum before component Initialize.
pub(super) fn spawn_grenade(world: &mut World, definition: Arc<Definition>, options: SpawnOptions,
    attacker: EntityId, linear: bevy::math::Vec3, angular: bevy::math::Vec3,
    held: EntityId, target_flags: u32) -> Result<EntityId, String> {
    spawn_inner(world, definition, options, None, Some((attacker, linear.to_array(), angular.to_array(), held, target_flags)))
}
///632f70 transfers the same armed component before GameLocal consumes Start.
///Keep that source boundary across the backend's real FX readiness wait.
pub(super) fn finish_grenade_birth(world: &mut World, id: EntityId) -> Result<bool, String> {
    let (held, flags, transferred, targeted) = {
        let actor = world.resource::<RuntimeEntities>().actors.get(&id).ok_or("grenade birth generation absent")?;
        let Some(birth) = &actor.grenade_birth else { return Ok(true); };
        (birth.held, birth.target_flags, birth.transferred, birth.targeted)
    };
    let ready = with_scene(world, |world, scene, live, events| {
        if !transferred {
            let component = scene.grenade_component(live, id)?.ok_or("grenade birth component absent")?;
            scene.prepare_grenade_resources(world, component.handle)?;
            if !scene.grenade_resources_ready(world, component.handle)? { return Ok(false); }
            scene.grenade_transfer(world, live, events, id, held)?;
            world.resource_mut::<RuntimeEntities>().actors.get_mut(&id).unwrap().grenade_birth.as_mut().unwrap().transferred = true;
        }
        if !targeted {
            scene.grenade_set_targeted(live, id, flags & 4 != 0)?;
            world.resource_mut::<RuntimeEntities>().actors.get_mut(&id).unwrap().grenade_birth.as_mut().unwrap().targeted = true;
        }
        scene.finish_runtime_birth(world, live, events, id)?;
        Ok(true)
    })?;
    if !ready { return Ok(false); }
    world.resource_mut::<RuntimeEntities>().actors.get_mut(&id).ok_or("grenade birth journal absent")?.grenade_birth = None;
    Ok(true)
}
fn spawn_inner(
    world: &mut World,
    definition: Arc<Definition>,
    options: SpawnOptions,
    projectile: Option<crate::gameplay::projectiles::SpawnRequest>,
    grenade: Option<(EntityId, [f32; 3], [f32; 3], EntityId, u32)>,
) -> Result<EntityId, String> {
    spawn_named_inner(world, definition, options, projectile, grenade, None, None)
}
pub(super) fn spawn_named(world: &mut World, definition: Arc<Definition>, options: SpawnOptions,
    name: Arc<str>) -> Result<EntityId, String> {
    spawn_named_inner(world, definition, options, None, None, Some(name), None)
}
pub(super) struct AIWorldBirth {
    pub area: usize,
    pub section: Option<(EntityId, [[f32; 3]; 2])>,
}
pub(super) fn spawn_ai_world(world: &mut World, definition: Arc<Definition>, name: Arc<str>,
    birth: AIWorldBirth) -> Result<EntityId, String> {
    if definition.class != "arkAIWorld" { return Err("AIWorld source definition has a different actual class".into()); }
    spawn_named_inner(world, definition, SpawnOptions { frame: ReferenceFrame::default(), scale: None,
        hidden_disabled: false }, None, None, Some(name), Some(birth))
}
fn spawn_named_inner(world: &mut World, definition: Arc<Definition>, options: SpawnOptions,
    projectile: Option<crate::gameplay::projectiles::SpawnRequest>,
    grenade: Option<(EntityId, [f32; 3], [f32; 3], EntityId, u32)>, authored_name: Option<Arc<str>>,
    ai_world: Option<AIWorldBirth>) -> Result<EntityId, String> {
    spawn_constructed(world, definition, options, projectile, grenade, authored_name, ai_world, None, &mut Construction::World)
}
enum Construction<'a> {
    World,
    Callback {
        scene: &'a mut super::physics_scene::PhysicsScene,
        live: &'a mut crate::gameplay::entity_lifetime::LiveEntities,
        events: &'a mut crate::gameplay::entity_events::EntityEvents,
        registry: &'a mut crate::gameplay::component_state::ComponentRegistry,
    },
}
impl Construction<'_> {
    fn visit<R>(&mut self, world: &mut World, callback: impl FnOnce(&mut World,
        &mut super::physics_scene::PhysicsScene, &mut crate::gameplay::entity_lifetime::LiveEntities,
        &mut crate::gameplay::entity_events::EntityEvents,
        Option<&mut crate::gameplay::component_state::ComponentRegistry>) -> Result<R, String>) -> Result<R, String> {
        match self {
            Self::World => with_scene(world, |world, scene, live, events| callback(world, scene, live, events, None)),
            Self::Callback { scene, live, events, registry } => callback(world, scene, live, events, Some(registry)),
        }
    }
}
///The same source factory may append child construction during Initialize.
///Its caller supplies the actual borrowed registry; no second queue is made.
pub(super) fn spawn_in_registry(world: &mut World, scene: &mut super::physics_scene::PhysicsScene,
    live: &mut crate::gameplay::entity_lifetime::LiveEntities, events: &mut crate::gameplay::entity_events::EntityEvents,
    registry: &mut crate::gameplay::component_state::ComponentRegistry,
    definition: Arc<Definition>, options: SpawnOptions, name: Option<Arc<str>>, parent: EntityId) -> Result<EntityId, String> {
    let expected = prepared_child(world, parent, &definition.entity)?;
    if !Arc::ptr_eq(&expected, &definition) { return Err("startup child belongs to another prepared source generation".into()); }
    spawn_constructed(world, definition, options, None, None, name, None, Some(parent),
        &mut Construction::Callback { scene, live, events, registry })
}
pub(super) fn prepared_child(world: &World, parent: EntityId, source: &str) -> Result<Arc<Definition>, String> {
    let definition = world.get_resource::<RuntimeEntities>().and_then(|owner| owner.actors.get(&parent))
        .and_then(|actor| actor.definition.as_ref()).ok_or("startup child has no retained parent definition")?;
    definition.startup_children.iter().find(|child| child.entity.eq_ignore_ascii_case(source)).cloned()
        .ok_or_else(|| format!("{}: startup child {source} was not prepared by its source owner", definition.entity))
}
pub(super) fn set_ai_cue_category(world: &mut World, live: &crate::gameplay::entity_lifetime::LiveEntities,
    id: EntityId, category: i32) -> Result<(), String> {
    let (_, class) = live.get(id).ok_or("Cue category generation is stale")?;
    let mut owner = world.get_resource_mut::<RuntimeEntities>().ok_or("Cue constructor owner absent")?;
    let actor = owner.actors.get_mut(&id).ok_or("Cue constructor generation absent")?;
    let definition = actor.definition.as_ref().ok_or("Cue source definition absent")?;
    if definition.class != "arkAICue" || definition.class_index != class {
        return Err("Cue category source and actual class differ".into());
    }
    actor.cue.as_mut().ok_or("Cue constructor payload absent")?.category = category;
    Ok(())
}
pub(super) fn with_ai_cue<R>(world: &World, live: &crate::gameplay::entity_lifetime::LiveEntities,
    id: EntityId, callback: impl FnOnce(&actor_class::Cue) -> Result<R, String>) -> Result<R, String> {
    let (_, class) = live.get(id).ok_or("Cue generation is stale")?;
    let actor = world.get_resource::<RuntimeEntities>().and_then(|owner| owner.actors.get(&id))
        .ok_or("Cue constructor generation absent")?;
    if actor.definition.as_ref().is_none_or(|definition| definition.class != "arkAICue" || definition.class_index != class) {
        return Err("Cue source and actual class differ".into());
    }
    callback(actor.cue.as_ref().ok_or("Cue constructor payload absent")?)
}
///None means a placed/externally hosted source, whose existing model owner
///must answer. Some(false) is the actual runtime declaration's null model.
pub(super) fn runtime_model_declared(world: &World, live: &crate::gameplay::entity_lifetime::LiveEntities,
    id: EntityId) -> Result<Option<bool>, String> {
    let (_, class) = live.get(id).ok_or("runtime model query generation is stale")?;
    let Some(definition) = world.get_resource::<RuntimeEntities>().and_then(|owner|owner.actors.get(&id))
        .and_then(|actor|actor.definition.as_ref()) else { return Ok(None); };
    if definition.class_index != class { return Err("runtime model source and actual class differ".into()); }
    Ok(Some(definition.model.is_some()))
}
pub(super) fn with_ai_cue_mut<R>(world: &mut World, live: &crate::gameplay::entity_lifetime::LiveEntities,
    id: EntityId, callback: impl FnOnce(&mut actor_class::Cue) -> Result<R, String>) -> Result<R, String> {
    let (_, class) = live.get(id).ok_or("Cue generation is stale")?;
    let mut owner = world.get_resource_mut::<RuntimeEntities>().ok_or("Cue constructor owner absent")?;
    let actor = owner.actors.get_mut(&id).ok_or("Cue constructor generation absent")?;
    if actor.definition.as_ref().is_none_or(|definition| definition.class != "arkAICue" || definition.class_index != class) {
        return Err("Cue source and actual class differ".into());
    }
    callback(actor.cue.as_mut().ok_or("Cue constructor payload absent")?)
}
fn spawn_constructed(world: &mut World, definition: Arc<Definition>, options: SpawnOptions,
    projectile: Option<crate::gameplay::projectiles::SpawnRequest>,
    grenade: Option<(EntityId, [f32; 3], [f32; 3], EntityId, u32)>, authored_name: Option<Arc<str>>,
    ai_world: Option<AIWorldBirth>, startup_parent: Option<EntityId>, construction: &mut Construction<'_>) -> Result<EntityId, String> {
    if (definition.class == "arkAIWorld") != ai_world.is_some() {
        return Err("actual AIWorld construction requires its source world-list admission".into());
    }
    let scale = entity_scale(&definition, options.scale);
    let prototype_key = scaled_key(&definition, Some(scale));
    if !ready_inner(world, &definition, Some(scale))? {
        return Err("runtime entity source dependencies are not ready".into());
    }
    let owner = world.resource::<RuntimeEntities>();
    let resources = owner.prototypes[&prototype_key]
        .resources
        .clone()
        .ok_or("runtime body resources are absent")?;
    let model = owner.prototypes[&prototype_key].model.clone();
    let items = owner
        .items
        .get(&prototype_key)
        .cloned()
        .ok_or("runtime item source preparation is absent")?;
    let id = construction.visit(world, |_, scene, live, _, registry| {
        scene.allocate_runtime_actor(live, &definition, options, authored_name, registry.is_some())
    })?;
    // Journal before any fallible backend/constructor callback. Failure retains
    // the actual identity, source leases and entered bodies until rollback ack.
    world.resource_mut::<RuntimeEntities>().actors.insert(
        id,
        Actor {
            definition: Some(definition.clone()),
            prototype: Some(prototype_key),
            removing: false,
            rollback: true,
            rollback_queued: false,
            children: Vec::new(),
            release: None,
            stick: None,
            dynamic: (definition.class == "idDynamicEntity")
                .then(|| actor_class::Dynamic::new(definition.hide_on_spawn)),
            breakable_class: (definition.class == "arkBreakable")
                .then(|| actor_class::Breakable::new(definition.navmesh_static_cutting)),
            movable: (definition.class == "arkGrenade").then(actor_class::Movable::new),
            grenade_birth: grenade.map(|(_, _, _, held, target_flags)| GrenadeBirth { held, target_flags, transferred: false, targeted: false }),
            cue: (definition.class == "arkAICue").then(actor_class::Cue::new),
            startup_parent,
        },
    );
    let result: Result<(), String> = (|| {
        if let Some(birth) = ai_world {
            use crate::gameplay::ai_world::Worlds;
            if birth.area == 0 {
                if birth.section.is_some() || world.contains_resource::<Worlds>() {
                    return Err("default AIWorld constructor list is already owned or has section bounds".into());
                }
                world.insert_resource(Worlds::new(id));
            } else {
                let (owner, bounds) = birth.section.ok_or("section AIWorld has no actual bounds owner")?;
                let area = world.get_resource_mut::<Worlds>().ok_or("section AIWorld has no constructed default world")?
                    .add_section(id, owner, bounds);
                if area != birth.area { return Err("section AIWorld constructor order differs from source area index".into()); }
            }
            world.resource_mut::<crate::gameplay::entity_lifetime::LiveEntities>().set_ai_world_index(id,
                u32::try_from(birth.area).map_err(|_| "AIWorld area index overflows")?)?;
        }
        if let Some(mut clock) =
            world.get_resource_mut::<crate::gameplay::game_clock_runtime::ClockRuntime>()
        {
            clock.register_entity_time(id, 0)?;
        }
        if let Some(model) = &model {
            construction.visit(world, |world, _, live, _, _| {
                super::world_models::request_with_live(world, live, id, model.clone(), options.frame)
            })?;
            super::world_models::set_scale(world, id, scale)?;
            super::world_models::update(world, id, options.frame, false)?;
        }
        let mut chunks = Vec::new();
        chunks
            .try_reserve_exact(definition.chunks.len())
            .map_err(|error| format!("runtime chunk instances: {error}"))?;
        for chunk in &definition.chunks {
            let child = spawn_constructed(
                world,
                chunk.definition.clone(),
                SpawnOptions {
                    scale: Some(scale),
                    hidden_disabled: true,
                    ..options
                },
                None,
                None,
                None,
                None,
                startup_parent.map(|_| id),
                construction,
            )?;
            world
                .resource_mut::<RuntimeEntities>()
                .actors
                .get_mut(&id)
                .ok_or("runtime constructor journal disappeared")?
                .children
                .push(child);
            chunks.push((chunk.component, chunk.step, chunk.chunk, child));
        }
        construction.visit(world, |world, scene, live, events, registry| {
            if let Some(registry) = registry {
                scene.admit_runtime_actor_with_registry(world, live, events, id, definition.clone(),
                    &resources, &items, &chunks, projectile, grenade, false, false, registry, false)
            } else {
                scene.admit_runtime_actor(world, live, events, id, definition.clone(), &resources,
                    &items, &chunks, projectile, grenade, false, false)
            }
        })?;
        Ok(())
    })();
    if let Err(error) = result {
        let mut owner = world.resource_mut::<RuntimeEntities>();
        let mut pending = vec![id];
        let mut visited = HashSet::new();
        while let Some(failed) = pending.pop() {
            if !visited.insert(failed) { continue; }
            //Constructor callbacks register child actors directly into the
            //outer Init queue. Their typed manager owns the IDs even though
            //their own spawn call returned before outer initialization failed.
            pending.extend(owner.actors.iter().filter_map(|(&child,actor)|
                (actor.startup_parent == Some(failed)).then_some(child)));
            if let Some(actor) = owner.actors.get_mut(&failed) {
                actor.rollback = true;
                pending.extend(actor.children.iter().copied());
            }
        }
        return Err(format!("runtime entity construction {} class {} {id:?}: {error}",
            definition.entity, definition.class));
    }
    if model.is_some() {
        let visible = construction.visit(world, |_, scene, live, _, _| scene.runtime_actor_visible(live, id))?;
        super::world_models::update(world, id, options.frame, visible)?;
    }
    let mut owner = world.resource_mut::<RuntimeEntities>();
    let actor = owner
        .actors
        .get_mut(&id)
        .ok_or("runtime actor journal disappeared")?;
    actor.rollback = false;
    // Breakable owns the actual chunk array after accepted initialization.
    // The construction-only rollback journal must not duplicate it for life.
    actor.children = Vec::new();
    Ok(id)
}
///627f60 appends its source declarations through875d50 on the SAME live
///actor after the real Throwable stick handoff. No second entity/header exists.
pub(super) fn append_projectile_stick(
    world: &mut World,
    id: EntityId,
    definition: &Arc<Definition>,
) -> Result<bool, String> {
    let actor = world
        .get_resource::<RuntimeEntities>()
        .and_then(|owner| owner.actors.get(&id))
        .ok_or("stick append has no actual source actor journal")?;
    if actor.stick.as_ref().is_some_and(|state| state.complete) {
        return Ok(true);
    }
    if actor
        .stick
        .as_ref()
        .is_some_and(|state| state.component_started)
    {
        return Err("stick component append awaits its failed constructor retirement".into());
    }
    let frame = with_scene(world, |_, scene, live, _| {
        if !scene.runtime_admission_idle() {
            return Ok(None);
        }
        Ok(Some(scene.published_entity_frame(live, id)?))
    })?;
    let Some(frame) = frame else { return Ok(false) };
    if !ready_inner(world, definition, Some(frame.scale))? {
        let scope = {
            let mut owner = world.resource_mut::<RuntimeEntities>();
            let state = owner
                .actors
                .get_mut(&id)
                .ok_or("stick source actor journal disappeared")?
                .stick
                .get_or_insert_with(Default::default);
            state
                .scope
                .get_or_insert_with(|| Arc::from(format!("actor-components:{id:?}")))
                .clone()
        };
        request_scaled_for(world, scope, definition.clone(), frame.scale)?;
        return Ok(false);
    }
    let prototype = scaled_key(definition, Some(frame.scale));
    let (resources, items) = {
        let owner = world.resource::<RuntimeEntities>();
        (
            owner.prototypes[&prototype]
                .resources
                .clone()
                .ok_or("stick body preparation absent")?,
            owner
                .items
                .get(&prototype)
                .cloned()
                .ok_or("stick item preparation absent")?,
        )
    };
    {
        world
            .resource_mut::<RuntimeEntities>()
            .actors
            .get_mut(&id)
            .ok_or("stick source actor journal disappeared")?
            .stick
            .get_or_insert_with(Default::default);
    }
    loop {
        let next = world.resource::<RuntimeEntities>().actors[&id]
            .stick
            .as_ref()
            .unwrap()
            .chunks
            .len();
        let Some(chunk) = definition.stick_chunks.get(next) else {
            break;
        };
        let child = match spawn_inner(
            world,
            chunk.definition.clone(),
            SpawnOptions {
                frame: frame.pose,
                scale: Some(frame.scale),
                hidden_disabled: true,
            },
            None,
            None,
        ) {
            Ok(child) => child,
            Err(error) => {
                let mut owner = world.resource_mut::<RuntimeEntities>();
                let children = owner.actors[&id]
                    .stick
                    .as_ref()
                    .unwrap()
                    .chunks
                    .iter()
                    .map(|entry| entry.3)
                    .collect::<Vec<_>>();
                owner.actors.get_mut(&id).unwrap().rollback = true;
                for child in children {
                    if let Some(actor) = owner.actors.get_mut(&child) {
                        actor.rollback = true;
                    }
                }
                return Err(error);
            }
        };
        world
            .resource_mut::<RuntimeEntities>()
            .actors
            .get_mut(&id)
            .unwrap()
            .stick
            .as_mut()
            .unwrap()
            .chunks
            .push((chunk.component, chunk.step, chunk.chunk, child));
    }
    let model_changed = definition.stick_model.as_ref().is_some_and(|model| {
        super::world_models::descriptor(world, id).is_some_and(|actual| Arc::ptr_eq(model, &actual))
    });
    let chunks = {
        let mut owner = world.resource_mut::<RuntimeEntities>();
        let state = owner.actors.get_mut(&id).unwrap().stick.as_mut().unwrap();
        state.component_started = true;
        state.chunks.clone()
    };
    let result = with_scene(world, |world, scene, live, events| {
        scene.admit_runtime_actor(
            world,
            live,
            events,
            id,
            definition.clone(),
            &resources,
            &items,
            &chunks,
            None,
            None,
            true,
            model_changed,
        )
    });
    if let Err(error) = result {
        let mut owner = world.resource_mut::<RuntimeEntities>();
        if let Some(actor) = owner.actors.get_mut(&id) {
            actor.rollback = true;
        }
        for &(_, _, _, child) in &chunks {
            if let Some(actor) = owner.actors.get_mut(&child) {
                actor.rollback = true;
            }
        }
        return Err(error);
    }
    let mut owner = world.resource_mut::<RuntimeEntities>();
    let state = owner.actors.get_mut(&id).unwrap().stick.as_mut().unwrap();
    state.complete = true;
    state.chunks = Vec::new(); //The actual attached Breakable now owns this array.
    Ok(true)
}

/// Drain real component, graph, audio and renderer owners. The canonical
/// GameLocal removal batch owns save-name preparation and final Live ID retire.
pub(super) fn retire_components(world: &mut World, id: EntityId) -> Result<bool, String> {
    if super::pickups::blocks_retirement(world, id) {
        return Ok(false);
    }
    let whole_world = world
        .get_resource::<RuntimeEntities>()
        .is_some_and(|owner| owner.cancelled.load(Ordering::Acquire));
    if !whole_world
        && world
            .get_resource::<super::physics_scene::PhysicsScene>()
            .is_some_and(|scene| scene.entity_water_impact_pending(id))
    {
        return Ok(false);
    }
    super::breakables::begin_entity_retirement(world, id)?;
    if !super::breakables::entity_retired(world, id)? {
        return Ok(false);
    }
    let removing = {
        let mut owner = world.get_resource_mut::<RuntimeEntities>().ok_or("runtime entity lifetime owner is absent")?;
        // Placed actors share the exact same component destructor; absence
        // from the dynamic allocation journal is not proof of retirement.
        let actor = owner.actors.entry(id).or_insert(Actor {
            definition: None,
            prototype: None,
            removing: false,
            rollback: false,
            rollback_queued: false,
            children: Vec::new(),
            release: None,
            stick: None,
            dynamic: None,
            breakable_class: None,
            movable: None,
            grenade_birth: None,
            cue: None,
            startup_parent: None,
        });
        actor.removing
    };
    if !removing {
            super::projectiles::begin_retirement(world, id);
            let live = world
                .remove_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
                .ok_or("actor renderer retirement identity owner absent")?;
            let removed = super::world_models::remove_entity(world, &live, id);
            world.insert_resource(live);
            removed?;
            super::entity_graphs::retire(world, id)?;
            world.resource_mut::<RuntimeEntities>().actors.get_mut(&id).ok_or("runtime retirement actor vanished")?.removing = true;
    }
        if !super::projectiles::retired(world, id)
            || !super::world_models::retired_entity(world, id)
        {
            return Ok(false);
        }
        //The shared component registry owns one resumable entity destructor.
        //Independent renderer withdrawals precede that lease: claiming it
        //before GPU acknowledgement serializes world exit by one frame/actor.
    let (rollback, mut release) = {
        let mut owner = world.resource_mut::<RuntimeEntities>();
        if owner.active_retirement.is_some_and(|active| active != id) {
            return Ok(false);
        }
        let resumed = owner.active_retirement == Some(id);
        owner.active_retirement = Some(id);
        let actor = owner.actors.get_mut(&id).ok_or("runtime retirement actor vanished")?;
        if resumed && actor.release.is_none() { return Err("runtime retirement cursor is already borrowed by its callback".into()); }
        (actor.rollback, actor.release.take().unwrap_or_default())
    };
    //Only the active release cursor is borrowed. Actual parent/child class
    //state remains in its canonical owner for Stop/Deinit callbacks.
    let result = with_scene(world, |world, scene, live, events| {
        scene.retire_runtime_actor(world, live, events, id, &mut release, rollback)
    });
    world.resource_mut::<RuntimeEntities>().actors.get_mut(&id)
        .ok_or("runtime retirement callback removed its actual actor journal")?.release = Some(release);
    result
}
fn finish_retirement(world: &mut World, id: EntityId, mode: RetireMode) -> Result<bool, String> {
    //The whole-world component drain may already have returned this identity
    //while preserving its GameLocal source journal for the late owner phase.
    let alive = world.resource::<crate::gameplay::entity_lifetime::LiveEntities>().get(id).is_some();
    if alive && !retire_components(world, id)? {
        return Ok(false);
    }
    if !alive && (!super::world_models::retired_entity(world,id) || !super::projectiles::retired(world,id)) { return Ok(false); }
    if world.contains_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>() {
        super::time_sync::retire_entity_time(world, id)?;
    }
    with_scene(world, |_, scene, live, _| {
        if live.get(id).is_some() {
            let retired = match mode {
                RetireMode::Ordinary => live.retire_after_release(id),
                RetireMode::World => live.retire_for_world_shutdown(id),
            };
            if retired.is_none() {
                return Err(
                    "managed entity final retirement lacks completed release/attachment ownership"
                        .into(),
                );
            }
        }
        scene.finish_runtime_retirement(live, id)
    })?;
    let mut owner = world.resource_mut::<RuntimeEntities>();
    let actor = owner.actors.remove(&id);
    owner.active_retirement = None;
    drop(owner);
    placed::release(world, id)?;
    if let Some(actor) = actor {
        if let (Some(definition), Some(scope)) =
            (actor.definition, actor.stick.and_then(|state| state.scope))
        {
            release_for(world, &scope, &definition)?;
        }
    }
    Ok(true)
}
#[derive(Clone, Copy)]
enum RetireMode {
    Ordinary,
    World,
}
pub(super) fn retire(world: &mut World, id: EntityId) -> Result<bool, String> {
    finish_retirement(world, id, RetireMode::Ordinary)
}
pub(super) fn begin_retirement(world: &mut World) -> Result<bool, String> {
    let Some(mut owner) = world.get_resource_mut::<RuntimeEntities>() else {
        return Ok(true);
    };
    owner.cancelled.store(true, Ordering::Release);
    owner.queue.clear();
    drop(owner);
    //A failed AIWorld constructor can own the current Init cursor even though
    //its actual actor/children intentionally retire after the global drain.
    //Withdraw that failed cursor before any independent component teardown.
    advance(world)?;
    let owner = world.resource::<RuntimeEntities>();
    if owner.actors.values().any(|actor|actor.rollback && !actor.rollback_queued) {
        return Ok(false);
    }
    let mut ids = owner.actors.iter().filter_map(|(&id, actor)|
        (actor.startup_parent.is_none() && actor.definition.as_ref().is_none_or(|definition|definition.class != "arkAIWorld"))
            .then_some(id)).collect::<Vec<_>>();
    drop(owner);
    if let Some(entered) = world
        .resource::<super::physics_scene::PhysicsScene>()
        .runtime_startup_owner()?
    {
        if let Some(index) = ids.iter().position(|id| *id == entered) {
            ids.swap(0, index);
        }
    }
    let mut complete = true;
    for id in ids {
        complete &= finish_retirement(world, id, RetireMode::World)?;
    }
    if !complete {
        return Ok(false);
    }
    if world.resource::<RuntimeEntities>().pending.is_some() { return Ok(false); }
    if !world.resource::<RuntimeEntities>().actors.is_empty() { return Ok(true); }
    finish_preparation_retirement(world)
}
///After the shared whole-world component/attachment drain, release the actual
///GameLocal manager identities and their constructor-owned descendants.
pub(super) fn retire_ai_worlds(world: &mut World) -> Result<bool, String> {
    let ids = world.get_resource::<RuntimeEntities>().map(|owner|owner.actors.keys().copied().collect::<Vec<_>>()).unwrap_or_default();
    let mut complete = true;
    for id in ids { complete &= finish_retirement(world,id,RetireMode::World)?; }
    if !complete { return Ok(false); }
    finish_preparation_retirement(world)
}
fn finish_preparation_retirement(world: &mut World) -> Result<bool, String> {
    if !world.contains_resource::<RuntimeEntities>() { return Ok(true); }
    placed::shutdown(world)?;
    advance(world)?;
    let owner = world.resource::<RuntimeEntities>();
    if owner.pending.is_some() {
        return Ok(false);
    }
    let identities = owner.prototypes.keys().copied().collect::<Vec<_>>();
    drop(owner);
    for identity in identities {
        let scope = world.resource::<RuntimeEntities>().prototypes[&identity]
            .fx_key
            .clone();
        super::audio::release_events(world, &scope)?;
        let contact_key = world.resource::<RuntimeEntities>().prototypes[&identity]
            .contact_key
            .clone();
        super::explosions::release_owned(world, &contact_key)?;
        let model = world.resource::<RuntimeEntities>().prototypes[&identity]
            .model
            .clone();
        let stick_model = world.resource::<RuntimeEntities>().prototypes[&identity]
            .stick_model
            .clone();
        if let Some(model) = stick_model {
            super::world_models::release_prewarm_for(world, &scope, &model)?;
        }
        if let Some(model) = model {
            super::world_models::release_prewarm_for(world, &scope, &model)?;
        }
        world
            .resource_mut::<super::usable_effects::EffectRequests>()
            .release_reservation(scope);
        let mut owner = world.resource_mut::<RuntimeEntities>();
        owner.prototypes.remove(&identity);
        owner.items.remove(&identity);
        owner.fired_sounds.remove(&identity);
    }
    Ok(true)
}
