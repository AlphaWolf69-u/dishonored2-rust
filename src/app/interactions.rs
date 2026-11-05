//! Level-owned readable and usable component instances.
use std::{collections::{HashMap, HashSet}, sync::Arc};
use bevy::prelude::*;
use crate::{archive::Catalog, entities::{self, Document}, gameplay::{navigation, usable::Usable}};

pub(super) struct Actor {
    pub entity: String,
    pub definition: Arc<entities::usable::DefinitionData>,
    pub state: Option<Usable>,
    pub frame: crate::gameplay::reference_frame::ReferenceFrame,
    pub fake_box: Option<navigation::ComponentBox>,
    last_delta: Option<[[f32; 4]; 3]>,
}

#[derive(Resource)]
pub(super) struct Interactions {
    pub catalog: Arc<Catalog>,
    limits: Arc<crate::assets::inventory_item::ReadableLimits>,
    pub sensor: entities::usable::Sensor,
    pub actors: Vec<Actor>,
    pub readable: Vec<entities::usable::ReadableBinding>,
    pub actor_by_entity: HashMap<String, usize>,
    pub readable_by_entity: HashMap<String, usize>,
    pub casts: crate::gameplay::interaction_sensor::Scratch,
    breakable: HashSet<String>,
    parent: HashMap<String, String>,
    pub pending: Vec<PendingChange>,
    selected: Option<(bool, usize, usize)>,
    pending_read: Option<std::sync::Mutex<bevy::tasks::Task<Result<ReadResult, String>>>>,
    pub retire: Vec<String>,
}

enum ReadTarget {
    Note(crate::assets::inventory_item::Readable),
    Other(crate::assets::inventory_item::Readable, OtherKind),
    Map(crate::assets::interactive_map::Map),
}
enum OtherKind { Map, Audiograph, Item }
struct ReadResult {
    entity: String,
    path: String,
    target: Option<ReadTarget>,
    modifiers: crate::assets::inventory_item::ItemModifiers,
}

pub(super) struct PendingChange {
    pub actor: usize,
    pub animation: Option<crate::gameplay::player::animation::AnimBasicCommand>,
    pub stop_effect: bool,
    pub start_effect: Option<EffectSpawn>,
}

#[derive(Clone)]
pub(super) struct EffectSpawn {
    pub path: String,
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
    pub attached: bool,
}

pub(super) fn effect_spawn(state: &Usable, path: &str) -> EffectSpawn {
    let step = &state.definition.steps[state.state];
    EffectSpawn { path: path.to_owned(), origin: step.effect_origin,
        axis: step.effect_axis, attached: step.effect_attached }
}

impl Interactions {
    pub fn load(catalog: Arc<Catalog>, document: &Document, character: &str,
        loadout: &[(String, u32)], list: &crate::assets::map_resources::LevelList) -> Result<Self, String> {
        let read = |path: &str| -> Result<Option<String>, String> {
            Ok(catalog.read(path).map_err(|e| e.to_string())?
                .map(|bytes| entities::declaration_text(&bytes).into_owned()))
        };
        let sensor = entities::usable::player_sensor(&catalog, character)?;
        let bindings = entities::usable::bindings(document, |path| read(path))?;
        let readable = entities::usable::readable_bindings(document, |path| read(path))?;
        let limits = Arc::new(crate::assets::inventory_item::ReadableLimits::load(&catalog, character,
            readable.iter().map(|binding| binding.item.clone()).chain(loadout.iter().map(|(path, _)| path.clone()))
                .chain(list.iter().filter_map(super::interaction_focus::registered_item_path).map(str::to_owned)))?);
        let definitions: HashMap<&str, &entities::Definition> = document.definitions.iter()
            .filter(|d| d.kind == "entity").map(|d| (d.id.as_str(), d)).collect();
        let mut actors = Vec::new();
        for binding in bindings {
            let entity = definitions.get(binding.entity.as_str()).ok_or("usable entity missing")?;
            let frame = match entities::spawn_frame(entity) {
                Ok(frame) => frame,
                Err(error) => { bevy::log::warn!("Usable {} has no valid spawn frame: {error}", binding.entity); continue; }
            };
            // An empty authored state list is valid but has no state to toggle.
            let state = if binding.definition.steps.is_empty() { None }
                else { match Usable::new(binding.definition.clone()) {
                    Ok(state) => Some(state),
                    Err(error) => { bevy::log::warn!("Usable {}: {error}", binding.entity); continue; }
                } };
            actors.push(Actor { entity: binding.entity, definition: binding.definition, state, frame, fake_box: None, last_delta: None });
        }
        let actor_by_entity = actors.iter().enumerate().map(|(i, a)| (a.entity.clone(), i)).collect();
        let readable_by_entity = readable.iter().enumerate().map(|(i, a)| (a.entity.clone(), i)).collect();
        let breakable = entities::usable::breakable_entities(document)?;
        let parent = entities::entity_binds(document).into_iter().map(|bind| (bind.entity_id, bind.parent)).collect();
        let pending = actors.iter().enumerate().filter_map(|(actor, value)| {
            if !value.definition.starts_on_spawn { return None; }
            let state = value.state.as_ref()?;
            let change = state.initial();
            change.start_effect.map(|effect| PendingChange { actor, animation: None,
                stop_effect: change.stop_effect, start_effect: Some(effect_spawn(state, effect)) })
        }).collect();
        Ok(Self { catalog, limits, sensor, actors, readable, actor_by_entity, readable_by_entity,
            casts: Default::default(), breakable, parent, pending, selected: None, pending_read: None, retire: Vec::new() })
    }

    pub fn limits(&self) -> Arc<crate::assets::inventory_item::ReadableLimits> { self.limits.clone() }

    /// Start the same fake bodies used by the original component. Their owner
    /// is the level collision world and the actor retains only its handle.
    pub fn start_boxes(&mut self, world: &mut navigation::CollisionWorld) {
        for actor in &mut self.actors {
            if !actor.definition.starts_on_spawn || !actor.definition.fake_collision { continue; }
            let corners = entities::usable::fake_box_corners(&actor.frame, &actor.definition).map(super::game_to_view);
            actor.fake_box = Some(world.add_interaction_box(&actor.entity, corners));
        }
    }

    pub fn stop_box(&mut self, actor: usize, world: &mut navigation::CollisionWorld) {
        if let Some(handle) = self.actors.get_mut(actor).and_then(|actor| actor.fake_box.take()) {
            world.remove_interaction_box(handle);
        }
    }
}

fn interaction_type(name: Option<&str>) -> i32 {
    match name.unwrap_or("ARK_UI_INTERACTION_INVALID") {
        "ARK_UI_INTERACTION_USE" => 0, "ARK_UI_INTERACTION_PICK_UP" => 1,
        "ARK_UI_INTERACTION_CARRY" => 2, "ARK_UI_INTERACTION_READ" => 3,
        "ARK_UI_INTERACTION_KEYHOLE" => 4, "ARK_UI_INTERACTION_OPEN_DOOR" => 5,
        "ARK_UI_INTERACTION_LOCKED_DOOR" => 6, "ARK_UI_INTERACTION_TALK" => 7,
        "ARK_UI_INTERACTION_ASSASSINATE" => 8, "ARK_UI_INTERACTION_CHOKE" => 9,
        "ARK_UI_INTERACTION_CARRY_BODY" => 10, "ARK_UI_INTERACTION_GRAB_CHAIN" => 11,
        "ARK_UI_INTERACTION_TRAVEL_DOOR" => 12, "ARK_UI_INTERACTION_BLOCKED_DOOR" => 13,
        "ARK_UI_INTERACTION_LOOT" => 14, "ARK_UI_INTERACTION_BUSY_TALK" => 15,
        _ => -1,
    }
}

/// Consume the five sorted sensor casts in the order used by 1403b8430.
/// The subsequent occlusion query and state/actor admission are owned by the
/// same target; a candidate behind a blocking collision is not selected.
pub(super) fn focus_and_interact(
    interactions: Option<ResMut<Interactions>>,
    world: Option<Res<super::NavigationWorld>>,
    camera: Query<&Transform, With<Camera3d>>,
    input: Res<super::player::PlayerInput>,
    player: Option<Res<super::player::PlayerState>>,
    mut requests: Option<ResMut<super::readable_movie::ReadableRequests>>,
    mut inventory: ResMut<super::interaction_focus::DiscoveredDocuments>,
    mut focused: ResMut<super::interaction_focus::FocusedInteraction>,
    mut was_held: Local<bool>,
    mut occlusion: Local<crate::gameplay::interaction_sensor::OcclusionScratch>,
) {
    let held = input.held.contains(&22);
    let pressed = held && !*was_held;
    *was_held = held;
    let Some(mut interactions) = interactions else {
        if focused.current.is_some() { focused.current = None; }
        return;
    };
    let (Some(world), Ok(camera)) = (world, camera.single()) else {
        if focused.current.is_some() { focused.current = None; }
        return;
    };
    if let Some(task) = interactions.pending_read.as_mut() {
        let task = task.get_mut().unwrap_or_else(|error| error.into_inner());
        if let Some(result) = bevy::tasks::block_on(bevy::tasks::futures_lite::future::poll_once(task)) {
            interactions.pending_read = None;
            match result {
                Ok(ReadResult { entity, path, target: Some(ReadTarget::Note(readable)), modifiers }) => {
                    let maximum = interactions.limits.maximum(&path);
                    let id = if readable.title.is_empty() { 0 } else { crate::assets::lang::key_id(&readable.title) };
                    if inventory.admit(path, id, readable.kind, maximum, &modifiers) && readable.can_be_picked_up {
                        interactions.retire.push(entity);
                    }
                    if let Some(requests) = requests.as_deref_mut() {
                        requests.open.push_back(super::readable_movie::Request { title: readable.title, text: readable.text, note_type: readable.note_type });
                    }
                },
                Ok(ReadResult { entity, path, target: Some(ReadTarget::Other(readable, kind)), modifiers }) => {
                    let maximum = interactions.limits.maximum(&path);
                    let id = if readable.title.is_empty() { 0 } else { crate::assets::lang::key_id(&readable.title) };
                    let accepted = match kind {
                        OtherKind::Map => inventory.admit_map(path, id, maximum, &modifiers),
                        OtherKind::Audiograph => inventory.admit_audiograph(path, id, maximum, &modifiers),
                        OtherKind::Item => inventory.add_with_modifiers(&path, 1, maximum, &modifiers) != 0,
                    };
                    if accepted && readable.can_be_picked_up {
                        interactions.retire.push(entity);
                    }
                    if let Some(requests) = requests.as_deref_mut() {
                        requests.open.push_back(super::readable_movie::Request { title: readable.title, text: readable.text, note_type: readable.note_type });
                    }
                },
                Ok(ReadResult { entity, path, target: Some(ReadTarget::Map(map)), modifiers }) => {
                    let maximum = interactions.limits.maximum(&path);
                    let key = if map.title.is_empty() { &map.item_name } else { &map.title };
                    let id = if key.is_empty() { 0 } else { crate::assets::lang::key_id(key) };
                    if inventory.admit_map(path, id, maximum, &modifiers) && map.can_be_picked_up {
                        interactions.retire.push(entity);
                    }
                    if let Some(requests) = requests.as_deref_mut() {
                        requests.map_open.push_back(super::readable_movie::MapRequest {
                            title: map.title, image: map.image, legends: map.legends, map_type: map.map_type,
                        });
                    }
                },
                Ok(ReadResult { path, target: None, .. }) => bevy::log::warn!("Readable interaction has an unsupported item type: {path}"),
                Err(error) => bevy::log::warn!("Readable item unavailable: {error}"),
            }
        }
    }
    let eye = camera.translation;
    let forward = camera.forward().as_vec3();
    if player.as_deref().and_then(|state| state.player.as_ref()).is_some_and(|player| player.is_dead()) {
        if focused.current.is_some() { focused.current = None; }
        interactions.selected = None;
        return;
    }
    let swimming = player.as_deref().and_then(|state| state.player.as_ref()).is_some_and(|player| player.is_swimming());
    let sensor = interactions.sensor;
    crate::gameplay::interaction_sensor::cast(&world.0, eye, forward, sensor, &mut interactions.casts);
    let mut selected = None;
    'casts: for hits in &interactions.casts.hits {
        for hit in hits {
            let Some(entity) = world.0.owner_name(hit.owner) else { continue };
            let readable = interactions.readable_by_entity.get(entity).copied();
            let actor = interactions.actor_by_entity.get(entity).copied();
            let (is_readable, index, step, max_distance, shape, allowed_swimming) = if let Some(index) = readable {
                (true, index, 0, interactions.readable[index].maximum_distance, interactions.readable[index].occlusion_shape,
                    interactions.readable[index].allowed_swimming)
            } else if let Some(index) = actor {
                let Some(state) = interactions.actors[index].state.as_ref() else { continue };
                if !state.definition.starts_on_spawn { continue; }
                let step = state.state;
                let entry = &state.definition.steps[step];
                (false, index, step, if entry.overrides_interaction { entry.maximum_distance } else { state.definition.maximum_distance },
                    if entry.overrides_interaction { entry.occlusion_shape } else { state.definition.occlusion_shape },
                    if entry.overrides_interaction { entry.allowed_swimming } else { state.definition.allowed_swimming })
            } else { continue };
            if swimming && !allowed_swimming { continue; }
            if eye.distance_squared(hit.position) > max_distance * max_distance { continue; }
            let classify = |key, owner| {
                use crate::gameplay::interaction_sensor::OcclusionHit;
                let owner = world.0.owner_name(owner);
                if owner == Some(entity) || owner.and_then(|name| interactions.parent.get(name)).is_some_and(|parent| parent == entity) {
                    OcclusionHit::Target
                } else if owner.is_some_and(|name| interactions.breakable.contains(name))
                    && world.0.physics_material(key).is_some_and(|material| material.can_blink_through()) {
                    OcclusionHit::Ignore
                } else { OcclusionHit::Block }
            };
            if crate::gameplay::interaction_sensor::occluded(&world.0, eye, hit.position, sensor.shape_radii[0], &classify, &mut occlusion)
                || (shape > 0 && sensor.shape_radii.get(shape).is_some_and(|&radius|
                    crate::gameplay::interaction_sensor::occluded(&world.0, eye, hit.position, radius, &classify, &mut occlusion))) { continue; }
            selected = Some((is_readable, index, step));
            break 'casts;
        }
    }
    if selected != interactions.selected {
        let prompt = selected.map(|(is_readable, index, step)| {
            let (entity, name, text, icon, state, locked) = if is_readable {
                let item = &interactions.readable[index];
                (&item.entity, item.interaction_name.as_deref(), item.interaction_text.as_deref(), item.interaction_icon.as_deref(), Some("PLAYER_STATE_READ_NOTE"), item.locked)
            } else {
                let actor = &interactions.actors[index];
                let def = &actor.definition;
                let entry = &def.steps[step];
                if entry.overrides_interaction {
                    (&actor.entity, entry.interaction_name.as_deref(), entry.interaction_text.as_deref(), entry.interaction_icon.as_deref(), entry.player_state.as_deref(), entry.locked)
                } else {
                    (&actor.entity, def.interaction_name.as_deref(), def.interaction_text.as_deref(), def.interaction_icon.as_deref(), def.player_state.as_deref(), def.locked)
                }
            };
            super::interaction_focus::InteractionPrompt {
                entity: entity.clone(), name: name.unwrap_or("").to_owned(), usage_text: text.unwrap_or("").to_owned(),
                interaction_type: interaction_type(icon), binding: "<GBA_INTERACT>".to_owned(),
                name_postfix: String::new(), hold: false, locked, player_state: state.unwrap_or("").to_owned(),
            }
        });
        if focused.current != prompt { focused.current = prompt; }
        interactions.selected = selected;
    }
    if !pressed || focused.current.as_ref().is_none_or(|prompt| prompt.locked) { return; }
    let Some((is_readable, index, _)) = selected else { return };
    if is_readable {
        if interactions.pending_read.is_none() {
            let item = interactions.readable[index].item.clone();
            let entity = interactions.readable[index].entity.clone();
            let catalog = interactions.catalog.clone();
            interactions.pending_read = Some(std::sync::Mutex::new(bevy::tasks::IoTaskPool::get().spawn(async move {
                let fields = crate::assets::inventory_item::item_fields(&catalog, &item)?;
                let modifiers = crate::assets::inventory_item::modifiers_fields(&item, &fields)?;
                let target = if let Some(note) = crate::assets::inventory_item::readable_fields(&fields)? {
                    Some(ReadTarget::Note(note))
                } else {
                    match crate::assets::interactive_map::resolve_fields(&catalog, &item, &fields)? {
                        Some(map) => Some(ReadTarget::Map(map)),
                        None => crate::assets::inventory_item::other_readable_fields(&fields)?.map(|(readable, item_type)| {
                            let kind = match item_type { "TYPE_INTERACTIVE_MAP" => OtherKind::Map,
                                "TYPE_AUDIOGRAPH" => OtherKind::Audiograph, _ => OtherKind::Item };
                            ReadTarget::Other(readable, kind)
                        }),
                    }
                };
                Ok(ReadResult { entity, path: item, target, modifiers })
            })));
        }
    } else {
        let changes = {
            let actor = &mut interactions.actors[index];
            let Some(state) = actor.state.as_mut() else { return };
            let changes = state.interact();
            let animation = changes.animation;
            let stop_effect = changes.stop_effect;
            let path = changes.start_effect.map(str::to_owned);
            PendingChange { actor: index, animation, stop_effect,
                start_effect: path.as_deref().map(|path| effect_spawn(state, path)) }
        };
        interactions.pending.push(changes);
        // A state may change the authored prompt without a camera movement.
        interactions.selected = None;
    }
}

/// 14077be20 follows the owning entity pose. Unchanged owners do not touch the
/// mutable collision broad phase; stationary boxes remain at their last pose.
pub(super) fn sync_boxes(
    interactions: Option<ResMut<Interactions>>,
    world: Option<ResMut<super::NavigationWorld>>,
    bound: Option<Res<super::model_motion::BoundModels>>,
) {
    let (Some(mut interactions), Some(mut world), Some(bound)) = (interactions, world, bound) else { return };
    for actor in &mut interactions.actors {
        let Some(handle) = actor.fake_box else { continue };
        let Some(delta) = bound.entity_delta(&actor.entity).copied() else { continue };
        if actor.last_delta == Some(delta) { continue; }
        let corners = entities::usable::fake_box_corners(&actor.frame, &actor.definition).map(|point| {
            let moved = std::array::from_fn(|row| delta[row][3] + (0..3).map(|column| delta[row][column] * point[column]).sum::<f32>());
            super::game_to_view(moved)
        });
        world.0.update_interaction_box(handle, corners);
        actor.last_delta = Some(delta);
    }
}

/// 140844bf0 retires the world pickup only when inventory accepted it and
/// the item's m_readableCanBePickedUp is true. A note that remains in the
/// world stays readable even when a duplicate can no longer enter inventory.
pub(super) fn retire_admitted_pickups(
    interactions: Option<ResMut<Interactions>>,
    world: Option<ResMut<super::NavigationWorld>>,
    placements: Res<super::RenderSourcePlacements>,
    mut retired: ResMut<super::interaction_focus::RetiredPlacements>,
) {
    let (Some(mut interactions), Some(mut world)) = (interactions, world) else { return };
    if interactions.retire.is_empty() { return; }
    for entity in interactions.retire.drain(..) {
        world.0.set_owner_active(&entity, false);
        for (index, placement) in placements.0.iter().enumerate() {
            if placement.id == entity { retired.0.insert(index); }
        }
    }
}
