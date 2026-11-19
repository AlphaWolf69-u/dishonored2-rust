//! Level-owned readable and usable component instances.
use crate::{
    archive::Catalog,
    entities::{self, Document},
    gameplay::{navigation, usable::Usable},
};
use bevy::prelude::*;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) struct Actor {
    pub entity: String,
    pub definition: Arc<entities::usable::DefinitionData>,
    pub state: Option<Usable>,
    pub frame: crate::gameplay::reference_frame::ReferenceFrame,
    pub fake_box: Option<navigation::ComponentBox>,
    pub started: bool,
    pub enabled: bool,
    pub last_instigator:Option<crate::gameplay::entity_lifetime::EntityId>,
    pub scheduling: crate::gameplay::component_state::declaration::DeclarationScheduling,
    last_delta: Option<[[f32; 4]; 3]>,
}

#[derive(Resource)]
pub(super) struct Interactions {
    pub catalog: Arc<Catalog>,
    pub(super) limits: Arc<crate::assets::inventory_item::ReadableLimits>,
    pub sensor: entities::usable::Sensor,
    pub actors: Vec<Actor>,
    pub readable: Vec<entities::usable::ReadableBinding>,
    pub carrying: Vec<entities::carrying::Binding>,
    pub actor_by_entity: HashMap<String, usize>,
    pub readable_by_entity: HashMap<String, usize>,
    pub carrying_by_entity: HashMap<String, usize>,
    pub casts: crate::gameplay::interaction_sensor::Scratch,
    pub(super) physics_queries: super::physics_scene::interaction_queries::InteractionQueries,
    pub(super) pending_pickup: Option<super::pickups::Pending>,
    pub pending: Vec<PendingChange>,
    selected: Option<(SelectionKind, usize, usize)>,
    grapple_prompt:Option<(crate::gameplay::entity_lifetime::EntityId,crate::gameplay::pickable_component::CheckedComponent)>,
    pending_read: Option<ReadResult>,
    pending_activation: Option<(
        ReadResult,
        Arc<crate::gameplay::player::read_note::Presentation>,
    )>,
    item_sources: crate::assets::inventory_item::PickableSourceLease,
    pub(super) acquisitions_closed: Arc<AtomicBool>,
    retire: Vec<PendingPickup>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionKind {
    Readable,
    Usable,
    Carry,
    CarryLive(crate::gameplay::entity_lifetime::EntityId),
    Pickup(crate::gameplay::entity_lifetime::EntityId),
    Openable(crate::gameplay::entity_lifetime::EntityId),
}

struct PendingPickup {
    entity: String,
    identity: (
        crate::gameplay::entity_lifetime::EntityId,
        crate::gameplay::pickable_component::CheckedComponent,
    ),
    step: u8,
}

struct ReadResult {
    entity: String,
    identity: (
        crate::gameplay::entity_lifetime::EntityId,
        crate::gameplay::pickable_component::CheckedComponent,
    ),
    path: String,
    item: Arc<crate::assets::inventory_item::PickableItem>,
}

impl Drop for Interactions {
    fn drop(&mut self) {
        self.close_acquisitions();
    }
}

pub(super) struct PendingChange {
    pub actor: usize,
    pub stage:Option<(usize,bool,Option<crate::gameplay::entity_lifetime::EntityId>)>,
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
    EffectSpawn {
        path: path.to_owned(),
        origin: step.effect_origin,
        axis: step.effect_axis,
        attached: step.effect_attached,
    }
}

impl Interactions {
    pub fn load(
        catalog: Arc<Catalog>,
        document: &Document,
        character: &str,
    ) -> Result<Self, String> {
        let read = |path: &str| -> Result<Option<String>, String> {
            Ok(catalog
                .read(path)
                .map_err(|e| e.to_string())?
                .map(entities::owned_declaration_text))
        };
        let sensor = entities::usable::player_sensor(&catalog, character)?;
        let bindings = entities::usable::bindings(document, |path| read(path))?;
        let readable = entities::usable::readable_bindings(&catalog, document)?;
        let mut carrying = entities::carrying::bindings(&catalog, document)?;
        // A component that offers both paths has one retained definition and
        // one pool slot; the use callback selects holding before inventory.
        for binding in &mut carrying {
            if let Some(readable) = readable.iter().find(|value| value.entity == binding.entity) {
                binding.definition = readable.component.clone();
            }
        }
        super::pause_menu::trace_readable_ui(|| {
            format!(
                "READ_NOTE declarations admitted={} character={character}",
                readable.len()
            )
        });
        let limits = Arc::new(crate::assets::inventory_item::ReadableLimits::load(
            &catalog, character,
        )?);
        let definitions: HashMap<&str, &entities::Definition> = document
            .definitions
            .iter()
            .filter(|d| d.kind == "entity")
            .map(|d| (d.id.as_str(), d))
            .collect();
        let mut actors = Vec::new();
        for binding in bindings {
            let entity = definitions
                .get(binding.entity.as_str())
                .ok_or("usable entity missing")?;
            let frame = match entities::spawn_frame(entity) {
                Ok(frame) => frame,
                Err(error) => {
                    bevy::log::warn!(
                        "Usable {} has no valid spawn frame: {error}",
                        binding.entity
                    );
                    continue;
                }
            };
            // An empty authored state list is valid but has no state to toggle.
            let state = if binding.definition.steps.is_empty() {
                None
            } else {
                match Usable::new(binding.definition.clone()) {
                    Ok(state) => Some(state),
                    Err(error) => {
                        bevy::log::warn!("Usable {}: {error}", binding.entity);
                        continue;
                    }
                }
            };
            let started = binding.definition.starts_on_spawn;
            let enabled=binding.definition.usable_by_player;
            actors.push(Actor {
                enabled,last_instigator:None,
                entity: binding.entity,
                definition: binding.definition,
                state,
                frame,
                fake_box: None,
                started,
                scheduling: binding.scheduling,
                last_delta: None,
            });
        }
        let actor_by_entity = actors
            .iter()
            .enumerate()
            .map(|(i, a)| (a.entity.clone(), i))
            .collect();
        let readable_by_entity = readable
            .iter()
            .enumerate()
            .map(|(i, a)| (a.entity.clone(), i))
            .collect();
        let carrying_by_entity = carrying
            .iter()
            .enumerate()
            .map(|(i, a)| (a.entity.clone(), i))
            .collect();
        let pending = actors
            .iter()
            .enumerate()
            .filter_map(|(actor, value)| {
                if !value.definition.starts_on_spawn {
                    return None;
                }
                let state = value.state.as_ref()?;
                let change = state.initial();
                change.start_effect.map(|effect| PendingChange {
                    actor,
                    stage:None,
                    animation: None,
                    stop_effect: change.stop_effect,
                    start_effect: Some(effect_spawn(state, effect)),
                })
            })
            .collect();
        let item_sources =
            crate::assets::inventory_item::PickableSourceLease::for_catalog(&catalog)?;
        Ok(Self {
            catalog,
            limits,
            sensor,
            actors,
            readable,
            carrying,
            actor_by_entity,
            readable_by_entity,
            carrying_by_entity,
            casts: Default::default(),
            pending,
            selected: None,
            grapple_prompt:None,
            pending_read: None,
            physics_queries: Default::default(),
            pending_pickup: None,
            pending_activation: None,
            item_sources,
            acquisitions_closed: Arc::default(),
            retire: Vec::new(),
        })
    }

    pub(super) fn close_acquisitions(&self) {
        self.acquisitions_closed.store(true, Ordering::Release);
    }

    pub(super) fn retire_acquisition(&mut self) -> bool {
        self.pending_read = None;
        true
    }

    pub(super) fn request_readable(
        &mut self,
        index: usize,
        identity: (
            crate::gameplay::entity_lifetime::EntityId,
            crate::gameplay::pickable_component::CheckedComponent,
        ),
        declaration: Arc<crate::assets::inventory_item::PickableItem>,
    ) {
        if self.acquisitions_closed.load(Ordering::Acquire)
            || self.pending_read.is_some()
            || self.pending_activation.is_some()
        {
            return;
        }
        let item = self.readable[index].item.clone();
        super::pause_menu::trace_readable_ui(|| format!("READ_NOTE item requested path={item}"));
        let entity = self.readable[index].entity.clone();
        // The admitted component already owns the exact item generation.
        // Reacquiring here could select a different editable generation.
        self.pending_read = Some(ReadResult {
            entity,
            identity,
            path: item,
            item: declaration,
        });
    }

    ///841500/83fe70 hand the actual same checked Pickable toREAD_NOTE52.
    ///The existing read owner retains the descriptor and owns its UI/inventory
    ///acknowledgements; no second pickup or source read is created.
    pub(in crate::app) fn request_loot_readable(&mut self,live:&crate::gameplay::entity_lifetime::LiveEntities,
        id:crate::gameplay::entity_lifetime::EntityId,reference:crate::gameplay::pickable_component::CheckedComponent,
        item:Arc<crate::assets::inventory_item::PickableItem>)->Result<bool,String> {
        let (name,_)=live.get(id).ok_or("Loot47 readable target generation is stale")?;
        if self.acquisitions_closed.load(Ordering::Acquire)||self.pending_read.is_some()||self.pending_activation.is_some(){return Ok(false);}
        if item.note_type==-1{return Err("Loot47 READ_NOTE target has no authored note type".into());}
        if item.readable.is_none(){return Err("Loot47 READ_NOTE item has no canonical presentation".into());}
        self.pending_read=Some(ReadResult{entity:name.to_owned(),identity:(id,reference),path:item.path.to_string(),item});
        Ok(true)
    }

    ///77aec0/77b3b0 authored action: mutate the one actual Usable state.
    pub(in crate::app) fn request_usable_stage(&mut self,entity:&str,stage:Option<usize>,instant:bool)->Result<bool,String>{
        let Some(&index)=self.actor_by_entity.get(entity) else{return Ok(false);};
        let actor=&mut self.actors[index];
        if !actor.started{return Ok(false);}
        let Some(state)=actor.state.as_mut() else{return Ok(false);};
        let changes=match stage{Some(stage)=>state.set_stage(stage,instant),None=>state.interact()};
        let animation=changes.animation;let stop_effect=changes.stop_effect;
        let notification=changes.stage_notification;
        let path=changes.start_effect.map(str::to_owned);
        let stage=notification.map(|started|(state.state,started,actor.last_instigator));
        if notification==Some(false){actor.last_instigator=None;}
        self.pending.push(PendingChange{actor:index,stage,animation,stop_effect,
            start_effect:path.as_deref().map(|path|effect_spawn(state,path))});
        self.selected=None;Ok(true)
    }

    pub(in crate::app) fn invalidate_selection(&mut self){self.selected=None;}

    pub fn limits(&self) -> Arc<crate::assets::inventory_item::ReadableLimits> {
        self.limits.clone()
    }

    /// The same declaration source used by async reads; called only by the
    /// level preparation worker when admitting a real Pickable component.
    pub(super) fn acquire_readable_item(
        &self,
        path: &str,
    ) -> Result<Arc<crate::assets::inventory_item::PickableItem>, String> {
        self.item_sources
            .0
            .lock()
            .map_err(|_| "readable item source is poisoned")?
            .acquire(&self.catalog, path)
    }

    /// Start the same fake bodies used by the original component. Their owner
    /// is the level collision world and the actor retains only its handle.
    pub fn start_boxes(&mut self, world: &mut navigation::CollisionWorld) {
        for actor in &mut self.actors {
            if !actor.started || !actor.definition.fake_collision || actor.fake_box.is_some() {
                continue;
            }
            let corners = entities::usable::fake_box_corners(&actor.frame, &actor.definition)
                .map(super::game_to_view);
            actor.fake_box = Some(world.add_interaction_box(&actor.entity, corners));
        }
    }

    ///77b3b0 reapplies the same state's animation/FX and readmits77af40's
    ///fake collision body on an actual component restart, without a level scan.
    pub(super) fn start_actor(&mut self, index: usize, preserve_state: bool,
        world: &mut navigation::CollisionWorld) -> Result<(), String> {
        let actor = self.actors.get_mut(index).ok_or("Usable Start actor absent")?;
        if actor.started { return Ok(()); }
        let change = actor.state.as_mut().map(|state| {
            let changes = state.start(preserve_state);
            let animation = changes.animation;
            let stop_effect = changes.stop_effect;
            let effect = changes.start_effect.map(str::to_owned);
            PendingChange { actor: index, stage:None, animation, stop_effect,
                start_effect: effect.as_deref().map(|path| effect_spawn(state, path)) }
        });
        actor.started = true;
        if !preserve_state{actor.enabled=actor.definition.usable_by_player;}
        if actor.definition.fake_collision && actor.fake_box.is_none() {
            let corners = entities::usable::fake_box_corners(&actor.frame, &actor.definition)
                .map(super::game_to_view);
            actor.fake_box = Some(world.add_interaction_box(&actor.entity, corners));
        }
        if let Some(change) = change { self.pending.push(change); }
        Ok(())
    }

    pub fn stop_box(&mut self, actor: usize, world: &mut navigation::CollisionWorld) {
        if let Some(handle) = self
            .actors
            .get_mut(actor)
            .and_then(|actor| actor.fake_box.take())
        {
            world.remove_interaction_box(handle);
        }
    }

    /// 77b610/4cd920 detach the same usable collision and animation owners.
    /// The published last pose remains with its render placement until that
    /// placement retires; stopping does not hide or destroy the entity.
    pub(super) fn stop_actor(
        &mut self,
        index: usize,
        collision: &mut navigation::CollisionWorld,
        animations: &mut super::usable_animation::UsableAnimations,
        effects: &mut super::usable_effects::UsableEffects,
    ) {
        let actor = &mut self.actors[index];
        if !actor.started {
            return;
        }
        actor.started = false;
        actor.enabled = false;
        if let Some(handle) = actor.fake_box.take() {
            collision.remove_interaction_box(handle);
        }
        self.pending.retain(|change| change.actor != index);
        animations.stop_actor(index);
        effects.stop_usable(index);
        self.selected = None;
    }
}

/// Called after render acknowledgement, before supporting level resources
/// disappear. No outgoing async read may reopen a note or restart a usable.
pub(super) fn stop_level(world: &mut World) {
    if !world.contains_resource::<Interactions>() {
        return;
    }
    world.resource_scope(|world, mut interactions: Mut<Interactions>| {
        interactions.close_acquisitions();
        interactions.pending_activation = None;
        interactions.retire.clear();
        interactions.selected = None;
        interactions.grapple_prompt = None;
        if let Some(mut focused) = world.get_resource_mut::<super::interaction_focus::FocusedInteraction>() {
            focused.current = None;
        }
        // Partially prepared levels may not have published these backends.
        for (index, actor) in interactions.actors.iter_mut().enumerate() {
            actor.started = false;
            if let Some(handle) = actor.fake_box.take() {
                if let Some(mut navigation) = world.get_resource_mut::<super::NavigationWorld>() {
                    navigation.0.remove_interaction_box(handle);
                }
            }
            if let Some(mut effects) =
                world.get_resource_mut::<super::usable_effects::UsableEffects>()
            {
                effects.stop_usable(index);
            }
        }
        if let Some(mut animations) =
            world.get_resource_mut::<super::usable_animation::UsableAnimations>()
        {
            animations.stop_all();
        }
        interactions.pending.clear();
    });
}

fn interaction_type(name: Option<&str>) -> i32 {
    match name.unwrap_or("ARK_UI_INTERACTION_INVALID") {
        "ARK_UI_INTERACTION_USE" => 0,
        "ARK_UI_INTERACTION_PICK_UP" => 1,
        "ARK_UI_INTERACTION_CARRY" => 2,
        "ARK_UI_INTERACTION_READ" => 3,
        "ARK_UI_INTERACTION_KEYHOLE" => 4,
        "ARK_UI_INTERACTION_OPEN_DOOR" => 5,
        "ARK_UI_INTERACTION_LOCKED_DOOR" => 6,
        "ARK_UI_INTERACTION_TALK" => 7,
        "ARK_UI_INTERACTION_ASSASSINATE" => 8,
        "ARK_UI_INTERACTION_CHOKE" => 9,
        "ARK_UI_INTERACTION_CARRY_BODY" => 10,
        "ARK_UI_INTERACTION_GRAB_CHAIN" => 11,
        "ARK_UI_INTERACTION_TRAVEL_DOOR" => 12,
        "ARK_UI_INTERACTION_BLOCKED_DOOR" => 13,
        "ARK_UI_INTERACTION_LOOT" => 14,
        "ARK_UI_INTERACTION_BUSY_TALK" => 15,
        _ => -1,
    }
}

fn pickable_prompt(live:&crate::gameplay::entity_lifetime::LiveEntities,id:crate::gameplay::entity_lifetime::EntityId,
    pickable:&crate::gameplay::pickable_component::PickableComponent)->Option<super::interaction_focus::InteractionPrompt> {
    let name = pickable.item.as_ref().map(|item| &item.name)
                        .filter(|name| !name.is_empty())
                        .unwrap_or(&pickable.definition.interaction_labels[0]);
    Some(super::interaction_focus::InteractionPrompt {
                        entity: live.get(id)?.0.to_owned(),
                        name: name.clone(),
                        usage_text: pickable.definition.interaction_labels[1].clone(),
                        interaction_type: pickable.definition.pickup_interaction.icon,
                        binding: "<GBA_INTERACT>".to_owned(),
                        name_postfix: String::new(),
                        hold: false,
                        locked: pickable
                            .definition
                            .pickup_interaction
                            .has(crate::gameplay::interaction_definition::LOCKED),
                        player_state: pickable.definition.interaction_state.clone(),
                    })
}

fn grapple_prompt(scene:&super::physics_scene::PhysicsScene,live:&crate::gameplay::entity_lifetime::LiveEntities,
    player:&crate::gameplay::player::Player,models:Option<&super::world_models::WorldModels>,sources:&super::shadow_updates::ModelShadowSources,
    placements:&super::RenderSourcePlacements)->Result<Option<(crate::gameplay::entity_lifetime::EntityId,crate::gameplay::pickable_component::CheckedComponent)>,String> {
    let Some((candidates,cosine))=player.grapple_catch_prompt_candidates() else{return Ok(None);};
    if candidates[0].is_some(){return Err("GrappleCatch ragdoll HUD requires the actual NPCInteraction owner".into());}
    let (eye,axis)=player.published_camera_frame();
    for id in candidates.into_iter().flatten() {
        if live.get(id).is_none(){continue;}
        if !scene.grapple_pickup_available(live,player,id)?{continue;}
        let Some(frame)=scene.entity_frame(live,id)? else{continue;};
        let bounds=scene.interaction_bounds(live,id,models,sources,placements)?.unwrap_or([frame.position;2]);
        if !crate::gameplay::player::grapple_catch::in_cone(bounds,eye,axis.x_axis,cosine){continue;}
        let Some((reference,_))=scene.grapple_pickable(live,id)? else{continue;};
        return Ok(Some((id,reference)));
    }
    Ok(None)
}

/// Consume the five sorted sensor casts in the order used by 1403b8430.
/// The subsequent occlusion query and state/actor admission are owned by the
/// same target; a candidate behind a blocking collision is not selected.
pub(super) fn focus_and_interact(
    interactions: Option<ResMut<Interactions>>,
    world: Option<Res<super::NavigationWorld>>,
    physics: Option<Res<super::physics_scene::PhysicsScene>>,
    models: Option<Res<super::world_models::WorldModels>>,
    mut effects: Option<ResMut<crate::rendering::bevy_backend::camera_scene::FxSimulation>>,
    sources: Res<super::shadow_updates::ModelShadowSources>,
    placements: Res<super::RenderSourcePlacements>,
    live: Option<Res<crate::gameplay::entity_lifetime::LiveEntities>>,
    camera: Query<&Transform, With<Camera3d>>,
    clock: Res<crate::gameplay::game_clock_runtime::ClockRuntime>,
    mut player: Option<ResMut<super::player::PlayerState>>,
    mut requests: Option<ResMut<super::readable_movie::ReadableRequests>>,
    mut inventory: ResMut<super::interaction_focus::DiscoveredDocuments>,
    mut focused: ResMut<super::interaction_focus::FocusedInteraction>,
    mut entity_events: ResMut<crate::gameplay::entity_events::EntityEvents>,
) {
    //Existing action-scenario trace only. A previous HUD prompt is not proof
    //that this frame's sensor retained a primary interaction (7fc420).
    let trace_input=player.as_deref().and_then(|state|state.player.as_ref())
        .is_some_and(|player|player.carrying.tracing_enabled()&&{
            let (_,press,release,_,_)=player.controller.action_input_snapshot(22);
            press||release
        });
    if trace_input {
        let actual=player.as_deref().and_then(|state|state.player.as_ref()).unwrap();
        eprintln!("ACTION_INTERACT_INPUT source=7f9ea0+7f8040 advanced={} raw=(allowed,pressed,released,held,enable_count){:?} debug_free_fly={} states={:?} previous_focus={:?} previous_selected={:?} catch_hint={:?} owners=(interactions,nav,physics,models,live){:?} camera_count={} clock=(marker,time0,time1,time3){:?} camera={:?} equipment_side0={:?} equipment_side1={:?} selected_power={:?} pending_read={} pending_read_activation={} pending_pickup={:?} request47=(land_busy,swim_busy,land_typed,swim_typed){:?} player_id={:?} dead={}",
            clock.advanced(),actual.controller.action_input_snapshot(22),
            player.as_deref().is_some_and(|state|state.debug_free_fly),actual.active_player_states(),focused.current,
            interactions.as_deref().and_then(|owner|owner.selected),interactions.as_deref().and_then(|owner|owner.grapple_prompt),
            (interactions.is_some(),world.is_some(),physics.is_some(),models.is_some(),live.is_some()),camera.iter().count(),
            clock.clock().map(|clock|(clock.frame_marker(),clock.time(0),clock.time(1),clock.time(3))),
            camera.single().map(|camera|(camera.translation,camera.forward())),
            actual.equipment_action_status(0),actual.equipment_action_status(1),actual.selected_power().map(|cast|cast.settings.id),
            interactions.as_deref().is_some_and(|owner|owner.pending_read.is_some()),
            interactions.as_deref().is_some_and(|owner|owner.pending_activation.is_some()),
            interactions.as_deref().and_then(|owner|owner.pending_pickup.as_ref()).map(|pending|(pending.entity,pending.phase)),
            actual.loot_request_snapshot(),actual.managed_entity(),actual.is_dead());
    }
    //7f9ea0 retains a press independently from a later release in the
    //same event batch.7f8040 reads that enabled press, not final held state.
    if let Some(player) = player.as_deref_mut().and_then(|state| state.player.as_mut()) {
        player.set_interaction_presence(false);
    }
    let pressed = clock.advanced()
        && player.as_deref().filter(|state| !state.debug_free_fly)
            .and_then(|state| state.player.as_ref())
            .is_some_and(|player| player.controller.pressed(22));
    if let Some(player)=player.as_deref_mut().and_then(|state|state.player.as_mut()) {
        if player.carrying.tracing_enabled() && player.controller.pressed(22) {
            let target=focused.current.as_ref().and_then(|prompt|live.as_deref()?.find(&prompt.entity));
            player.carrying.trace("frontend-input22",target,22,pressed);
        }
    }
    if pressed {
        super::pause_menu::trace_readable_ui(|| {
            format!(
                "INTERACT entry resource={} player_read_note={:?} focus={:?}",
                interactions.is_some(),
                player
                    .as_deref()
                    .and_then(|runtime| runtime.player.as_ref())
                    .and_then(|player| player.read_note_state_allowed()),
                focused
                    .current
                    .as_ref()
                    .map(|prompt| (&prompt.entity, prompt.locked))
            )
        });
    }
    let Some(mut interactions) = interactions else {
        if trace_input {eprintln!("ACTION_INTERACT_EARLY reason=interaction_owner_absent");}
        if focused.current.is_some() {
            focused.current = None;
        }
        return;
    };
    let (Some(world), Ok(camera)) = (world, camera.single()) else {
        if trace_input {eprintln!("ACTION_INTERACT_EARLY reason=navigation_or_single_camera_absent");}
        interactions.selected = None;
        interactions.grapple_prompt = None;
        if focused.current.is_some() {
            focused.current = None;
        }
        return;
    };
    // A UI request follows the real READ_NOTE acceptance; acquiring the item
    // does not admit inventory, hide the pickup, or open a movie by itself.
    if interactions.pending_activation.is_some() {
        let result = player
            .as_deref_mut()
            .and_then(|runtime| runtime.player.as_mut())
            .and_then(|player| player.take_read_note_result());
        if let Some(result) = result {
            let (read, expected) = interactions
                .pending_activation
                .take()
                .expect("retained read activation");
            if !read_is_live(&read, physics.as_deref(), live.as_deref()) {
                return;
            }
            if result.is_err() {
                warn!("READ_NOTE activation was rejected or cancelled");
            }
            if let Ok(presentation) = result {
                if !Arc::ptr_eq(&presentation, &expected) {
                    warn!("READ_NOTE activation descriptor differs");
                    return;
                }
                let Some(requests) = requests.as_deref_mut() else {
                    warn!("Reading UI request owner is absent");
                    return;
                };
                let maximum = interactions.limits.maximum(&read.path);
                use crate::assets::inventory_item::{PickableOtherKind, PickableReadable};
                let portable = match &read.item.readable {
                    Some(PickableReadable::Note(note) | PickableReadable::Other(note, _)) => {
                        note.can_be_picked_up
                    }
                    Some(PickableReadable::Map(map)) => map.can_be_picked_up,
                    None => false,
                };
                if portable {
                    let Some(scene) = physics.as_deref() else {
                        return;
                    };
                    let Some(live) = live.as_deref() else {
                        return;
                    };
                    let Some(&binding_index) = interactions.readable_by_entity.get(&read.entity)
                    else {
                        return;
                    };
                    let binding = &interactions.readable[binding_index];
                    let parent=match live.parent(read.identity.0){Ok(parent)=>parent,Err(error)=>{warn!("document pickup parent identity: {error}");return;}};
                    if parent.is_some()
                        || !binding.unsupported_pickup_components.is_empty()
                    {
                        warn!(
                            "document pickup requires actual bound/projectile callbacks: {} {:?}",
                            read.entity, binding.unsupported_pickup_components
                        );
                        return;
                    }
                    if let Err(error) = scene.preflight_document_pickup(
                        live,
                        read.identity.1,
                        &sources,
                        &placements,
                    ) {
                        warn!("document pickup preflight: {error}");
                        return;
                    }
                    if let Err(error) = interactions.retire.try_reserve(1) {
                        warn!("document pickup queue: {error}");
                        return;
                    }
                }
                // 844bf0 notifies pickedUp before applying the inventory read
                // result, including notes which remain in the world.
                let (Some(effects), Some(live)) = (effects.as_deref_mut(), live.as_deref()) else {
                    return;
                };
                match effects.dispatch_picked_up(read.identity.0, true, false, live) {
                    Ok(errors) if errors.is_empty() => {}
                    Ok(errors) => {
                        warn!("document pickedUp: {}", errors.join("; "));
                        return;
                    }
                    Err(error) => {
                        warn!("document pickedUp: {error}");
                        return;
                    }
                }
                let (accepted, portable) = match &read.item.readable {
                    Some(PickableReadable::Note(note)) => {
                        let id = if note.title.is_empty() {
                            0
                        } else {
                            crate::assets::lang::key_id(&note.title)
                        };
                        (
                            inventory.admit(read.path, id, note.kind, maximum, read.item.clone()),
                            note.can_be_picked_up,
                        )
                    }
                    Some(PickableReadable::Other(note, kind)) => {
                        let id = if note.title.is_empty() {
                            0
                        } else {
                            crate::assets::lang::key_id(&note.title)
                        };
                        let accepted = match kind {
                            PickableOtherKind::Map => {
                                inventory.admit_map(read.path, id, maximum, read.item.clone())
                            }
                            PickableOtherKind::Audiograph => inventory.admit_audiograph(
                                read.path,
                                id,
                                maximum,
                                read.item.clone(),
                            ),
                            PickableOtherKind::Item => {
                                inventory.add_with_modifiers(
                                    &read.path,
                                    1,
                                    maximum,
                                    &read.item.modifiers,
                                ) != 0
                            }
                        };
                        (accepted, note.can_be_picked_up)
                    }
                    Some(PickableReadable::Map(map)) => {
                        let name = if map.title.is_empty() {
                            &map.item_name
                        } else {
                            &map.title
                        };
                        let id = if name.is_empty() {
                            0
                        } else {
                            crate::assets::lang::key_id(name)
                        };
                        (
                            inventory.admit_map(read.path, id, maximum, read.item.clone()),
                            map.can_be_picked_up,
                        )
                    }
                    None => {
                        warn!("READ_NOTE item has no presentation");
                        return;
                    }
                };
                if accepted && portable {
                    interactions.retire.push(PendingPickup {
                        entity: read.entity,
                        identity: read.identity,
                        step: 0,
                    });
                }
                super::pause_menu::trace_readable_ui(|| {
                    format!(
                        "READ_NOTE activation accepted inventory={accepted} portable={portable}"
                    )
                });
                requests.open.push_back(presentation);
            }
        }
    }
    if let Some(read) = interactions.pending_read.take() {
        match super::pause_menu::readable_presentation(&read.item)
            .map(|presentation| (read, presentation))
        {
            Ok((read, presentation)) => {
                if !read_is_live(&read, physics.as_deref(), live.as_deref()) {
                    return;
                }
                super::pause_menu::trace_readable_ui(|| {
                    format!("READ_NOTE item acquired path={}", read.path)
                });
                if let Some(player) = player
                    .as_deref_mut()
                    .and_then(|runtime| runtime.player.as_mut())
                {
                    if player.read_note_state_allowed() == Some(true)
                        && player.request_read_note(presentation.clone())
                    {
                        super::pause_menu::trace_readable_ui(|| {
                            format!("READ_NOTE activation requested path={}", read.path)
                        });
                        interactions.pending_activation = Some((read, presentation));
                    }
                }
            }
            Err(error) => warn!("Readable item unavailable: {error}"),
        }
    }
    let eye = camera.translation;
    let forward = camera.forward().as_vec3();
    if player
        .as_deref()
        .and_then(|state| state.player.as_ref())
        .is_some_and(|player| player.is_dead())
    {
        if focused.current.is_some() {
            focused.current = None;
        }
        interactions.selected = None;
        return;
    }
    let swimming = player
        .as_deref()
        .and_then(|state| state.player.as_ref())
        .is_some_and(|player| player.is_swimming());
    let sensor = interactions.sensor;
    let live_breakable = |name: &str| {
        physics
            .as_deref()
            .zip(live.as_deref())
            .is_some_and(|(scene, live)| {
                live.find(name)
                    .is_some_and(|id| scene.is_breakable(live, id))
            })
    };
    crate::gameplay::interaction_sensor::cast(
        &world.0,
        eye,
        forward,
        sensor,
        &mut interactions.casts,
    );
    let mut physics_queries = std::mem::take(&mut interactions.physics_queries);
    if let (Some(scene), Some(live)) = (physics.as_deref(), live.as_deref()) {
        if let Err(error) = scene.cast_interaction_sensor(
            live,
            bevy_math::Vec3::from_array(super::view_to_game(eye)),
            bevy_math::Vec3::from_array(super::view_to_game(forward)),
            sensor,
            &mut physics_queries,
            &mut interactions.casts,
        ) {
            interactions.physics_queries = physics_queries;
            interactions.selected = None;
            interactions.grapple_prompt = None;
            if focused.current.is_some() {
                focused.current = None;
            }
            warn!("Live Pickable sensor query: {error}");
            return;
        }
    }
    let mut selected = None;
    // 3b8430's last two groups are for NPC targeting. An ordinary
    // interaction descriptor can be selected from the first three groups.
    'casts: for hits in &interactions.casts.hits[..3] {
        for hit in hits {
            let Some(entity) = hit
                .entity
                .and_then(|id| live.as_deref()?.get(id).map(|(name, _)| name))
                .or_else(|| world.0.owner_name(hit.owner))
            else {
                continue;
            };
            let Some(target) = hit.entity.or_else(|| live.as_deref()?.find(entity)) else {
                continue;
            };
            // 878c70 excludes committed context 4 before reading any ordinary
            // interaction descriptor, including a placed document or usable.
            if live.as_deref().and_then(|live| live.entity_context(target).ok())
                .is_none_or(|context| context.current == 4)
            {
                continue;
            }
            // 3b8430 skips a transparent breakable before requesting its
            // interaction, using the actual contacted physics material.
            if hit.entity.is_none()
                && live_breakable(entity)
                && world
                    .0
                    .physics_material(hit.key)
                    .is_some_and(|material| material.can_blink_through())
            {
                continue;
            }
            let readable = interactions.readable_by_entity.get(entity).copied();
            let actor = interactions.actor_by_entity.get(entity).copied();
            let carrying = interactions.carrying_by_entity.get(entity).copied();
            let openable=if let (Some(scene),Some(live))=(physics.as_deref(),live.as_deref()){
                if let Some(id)=hit.entity.or_else(||live.find(entity)){
                    match scene.interaction_openable(live,id){Ok(value)=>value.map(|value|(id,value)),
                        Err(error)=>{warn!("Openable sensor for {entity}: {error}");continue;}}
                }else{None}
            }else{None};
            let (kind, index, step, max_distance, shape, allowed_swimming) = if let Some((id,(reference,usage,definition,_)))=openable{
                use crate::gameplay::interaction_definition::{ALLOW_SWIMMING,CandidateContext,admits_candidate};
                let (Some(scene),Some(live))=(physics.as_deref(),live.as_deref()) else{continue;};
                let frame=match scene.entity_frame(live,id){Ok(Some(frame))=>frame,_=>continue};
                let bounds=match scene.interaction_bounds(live,id,models.as_deref(),&sources,&placements){
                    Ok(bounds)=>bounds.unwrap_or([frame.position;2]),Err(error)=>{warn!("Openable bounds: {error}");continue;}};
                let context=CandidateContext{interactable:reference,player_mode:i32::from(swimming),is_possessing:false,
                    camera:super::view_to_game(eye),point:super::view_to_game(hit.position),target_forward:frame.basis[0],
                    target_min_z:bounds[0][2],target_max_z:bounds[1][2]};
                match admits_candidate(definition,&context,|mode,state,_|Err(format!("Openable interaction requires hosted player mode{mode}/state{state}"))){
                    Ok(true)=>{},Ok(false)=>continue,Err(error)=>{warn!("Openable candidate: {error}");continue;}}
                let Ok(shape)=usize::try_from(definition.occlusion_shape) else{continue;};
                (SelectionKind::Openable(id),usize::from(reference.handle.slot),usage,definition.maximum_distance,shape,definition.has(ALLOW_SWIMMING))
            }else if let Some(index)=carrying{
                use crate::gameplay::interaction_definition::{
                    ALLOW_SWIMMING, CandidateContext, admits_candidate,
                };
                let binding = &interactions.carrying[index];
                let definition = &binding.definition.pickup_interaction;
                let (Some(scene), Some(live), Some(player)) = (
                    physics.as_deref(),
                    live.as_deref(),
                    player.as_deref().and_then(|state| state.player.as_ref()),
                ) else {
                    continue;
                };
                let context = match scene.carry_context(entity, live, &sources) {
                    Ok(Some((interactable, frame, bounds))) => CandidateContext {
                        interactable,
                        player_mode: if swimming { 1 } else { 0 },
                        is_possessing: false,
                        camera: super::view_to_game(eye),
                        point: super::view_to_game(hit.position),
                        target_forward: frame.basis[0],
                        target_min_z: bounds[0],
                        target_max_z: bounds[1],
                    },
                    Ok(None) => continue,
                    Err(error) => {
                        warn!("Carry interaction context: {error}");
                        continue;
                    }
                };
                match admits_candidate(definition, &context, |mode, state, _request| {
                    Ok((if state==47 {player.loot_interaction_allowed(mode,state)} else {player.carry_interaction_allowed(mode,state)}) == Some(true))
                }) {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(error) => {
                        warn!("Carry interaction admission: {error}");
                        continue;
                    }
                }
                let Ok(shape) = usize::try_from(definition.occlusion_shape) else {
                    continue;
                };
                (
                    SelectionKind::Carry,
                    index,
                    0,
                    definition.maximum_distance,
                    shape,
                    definition.has(ALLOW_SWIMMING),
                )
            } else if let Some(index) = readable {
                let (Some(scene),Some(live))=(physics.as_deref(),live.as_deref()) else{continue;};
                //76ec20(true) marks the collected component. The static map
                //query can still hit its retained geometry; that is not a
                //fresh live interaction or a reason to reopen the document.
                match scene.readable_identity(entity,live) {
                    Ok(Some(_))=>{},
                    Ok(None)=>continue,
                    Err(error)=>{warn!("Readable focus identity unavailable: {error}");continue;},
                }
                (
                    SelectionKind::Readable,
                    index,
                    0,
                    interactions.readable[index].maximum_distance,
                    interactions.readable[index].occlusion_shape,
                    interactions.readable[index].allowed_swimming,
                )
            } else if let Some(index) = actor {
                if interactions.actors[index].definition.static_definition.is_some() {
                    let (Some(scene),Some(live))=(physics.as_deref(),live.as_deref()) else {continue;};
                    let Some(id)=live.find(entity) else {continue;};
                    let Some((reference,definition))=scene.interaction_static(live,id).ok().flatten() else {continue;};
                    let Ok(Some(frame))=scene.entity_frame(live,id) else {continue;};
                    let Ok(bounds)=scene.interaction_bounds(live,id,models.as_deref(),&sources,&placements) else {continue;};
                    let bounds=bounds.unwrap_or([frame.position;2]);
                    let context=crate::gameplay::interaction_definition::CandidateContext {interactable:reference,
                        player_mode:i32::from(swimming),is_possessing:false,camera:super::view_to_game(eye),
                        point:super::view_to_game(hit.position),target_forward:frame.basis[0],target_min_z:bounds[0][2],target_max_z:bounds[1][2]};
                    if !crate::gameplay::interaction_definition::admits_candidate(definition,&context,|mode,state,_|
                        Err(format!("static usable interaction requires hosted mode{mode}/state{state}"))).unwrap_or(false){continue;}
                    let Ok(shape)=usize::try_from(definition.occlusion_shape) else {continue;};
                    (SelectionKind::Usable,index,0,definition.maximum_distance,shape,
                        definition.has(crate::gameplay::interaction_definition::ALLOW_SWIMMING))
                } else {
                let Some(state) = interactions.actors[index].state.as_ref() else {
                    continue;
                };
                if !interactions.actors[index].started||!interactions.actors[index].enabled {
                    continue;
                }
                let step = state.state;
                let entry = &state.definition.steps[step];
                (
                    SelectionKind::Usable,
                    index,
                    step,
                    if entry.overrides_interaction {
                        entry.maximum_distance
                    } else {
                        state.definition.maximum_distance
                    },
                    if entry.overrides_interaction {
                        entry.occlusion_shape
                    } else {
                        state.definition.occlusion_shape
                    },
                    if entry.overrides_interaction {
                        entry.allowed_swimming
                    } else {
                        state.definition.allowed_swimming
                    },
                )
                }
            } else {
                let id = target;
                use crate::gameplay::interaction_definition::{
                    ALLOW_SWIMMING, CandidateContext, admits_candidate,
                };
                let (Some(scene), Some(live), Some(player)) = (
                    physics.as_deref(),
                    live.as_deref(),
                    player.as_deref().and_then(|state| state.player.as_ref()),
                ) else {
                    continue;
                };
                let Ok(Some((reference, pickable))) = scene.interaction_pickable(live, id) else {
                    continue;
                };
                let carrying = pickable.definition.hold.is_some();
                if !carrying && pickable.item.as_ref().is_none_or(|item|
                    item.note_type == -1 && !super::pickups::inventory_family_available(item, player)) {
                    continue;
                }
                let Ok(Some(frame)) = scene.entity_frame(live, id) else {
                    continue;
                };
                let bounds = match scene.interaction_bounds(live, id, models.as_deref(), &sources, &placements) {
                    Ok(bounds) => bounds.unwrap_or([frame.position; 2]),
                    Err(error) => {
                        warn!("Pickable model bounds: {error}");
                        continue;
                    }
                };
                if trace_input {
                    eprintln!("ACTION_INTERACT_PICKABLE id={id:?} reference={reference:?} carrying={carrying} item={:?} bounds={bounds:?} frame={frame:?}",
                        pickable.item.as_ref().map(|item|(&item.path,item.note_type,super::pickups::inventory_family_available(item,player))));
                }
                let definition = &pickable.definition.pickup_interaction;
                let context = CandidateContext {
                    interactable: reference,
                    player_mode: if swimming { 1 } else { 0 },
                    is_possessing: false,
                    camera: super::view_to_game(eye),
                    point: super::view_to_game(hit.position),
                    target_forward: frame.basis[0],
                    target_min_z: bounds[0][2],
                    target_max_z: bounds[1][2],
                };
                let admission=admits_candidate(definition, &context, |mode, state, _| {
                    Ok(if carrying && state!=47 { player.carry_interaction_allowed(mode, state) }
                        else { player.loot_interaction_allowed(mode, state) } == Some(true))
                });
                if trace_input {
                    let mode=if definition.player_mode==14{context.player_mode}else{definition.player_mode};
                    let state_allowed=if carrying&&definition.player_state!=47{player.carry_interaction_allowed(mode,definition.player_state)}else{player.loot_interaction_allowed(mode,definition.player_state)};
                    eprintln!("ACTION_INTERACT_CANDIDATE id={id:?} reference={reference:?} carrying={carrying} primary={definition:?} secondary=not_exposed_by_host retained_component_record={:?} mode={} resolved_mode={mode} state_allowed={state_allowed:?} possessing={} camera={:?} point={:?} forward={:?} min_z={} max_z={} admitted={admission:?}",
                        pickable.interaction,context.player_mode,context.is_possessing,context.camera,context.point,context.target_forward,context.target_min_z,context.target_max_z);
                }
                match admission {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(error) => {
                        warn!("Pickup interaction admission: {error}");
                        continue;
                    }
                }
                let Ok(shape) = usize::try_from(definition.occlusion_shape) else {
                    continue;
                };
                (
                    if carrying { SelectionKind::CarryLive(id) } else { SelectionKind::Pickup(id) },
                    usize::from(reference.handle.slot),
                    0,
                    definition.maximum_distance,
                    shape,
                    definition.has(ALLOW_SWIMMING),
                )
            };
            if swimming && !allowed_swimming {
                continue;
            }
            if kind == SelectionKind::Readable
                && player
                    .as_deref()
                    .and_then(|runtime| runtime.player.as_ref())
                    .is_none_or(|player| player.read_note_state_allowed() != Some(true))
            {
                continue;
            }
            if eye.distance_squared(hit.position) > max_distance * max_distance {
                continue;
            }
            let (Some(scene), Some(live)) = (physics.as_deref(), live.as_deref()) else {
                continue;
            };
            for radius in std::iter::once(sensor.shape_radii[0]).chain(
                (shape > 0).then(|| sensor.shape_radii.get(shape).copied()).flatten(),
            ) {
                match scene.interaction_occluded(
                    live, target,
                    bevy_math::Vec3::from_array(super::view_to_game(eye)),
                    bevy_math::Vec3::from_array(super::view_to_game(hit.position)),
                    radius, &mut physics_queries,
                ) {
                    Ok(blocked) => {
                        if trace_input {
                            eprintln!("ACTION_INTERACT_OCCLUSION id={target:?} radius={radius} blocked={blocked} hit={:?}",hit.position);
                        }
                        // 3b8430 ends this group's sorted hit scan after the
                        // first eligible descriptor reaches occlusion.
                        if blocked { continue 'casts; }
                    }
                    Err(error) => {
                        warn!("Live pickup occlusion: {error}");
                        continue 'casts;
                    }
                }
            }
            selected = Some((kind, index, step));
            break 'casts;
        }
    }
    interactions.physics_queries = physics_queries;
    let catch_prompt=physics.as_deref().zip(live.as_deref()).zip(player.as_deref().and_then(|state|state.player.as_ref()))
        .map(|((scene,live),player)|grapple_prompt(scene,live,player,models.as_deref(),&sources,&placements)).transpose();
    let catch_prompt=match catch_prompt {Ok(value)=>value.flatten(),Err(error)=>{warn!("Grapple catch prompt: {error}");None}};
    if let Some((id,reference))=catch_prompt {
        //Immutable interaction metadata belongs to this actual component
        //generation. A stable catch hint does no per-frame string allocation.
        if interactions.grapple_prompt!=Some((id,reference))||focused.current.is_none() {
            focused.current=physics.as_deref().zip(live.as_deref()).and_then(|(scene,live)|
                scene.grapple_pickable(live,id).ok().flatten().and_then(|(_,pickable)|pickable_prompt(live,id,pickable)));
        }
        interactions.grapple_prompt=Some((id,reference));
        selected=None;interactions.selected=None;
    } else if interactions.grapple_prompt.take().is_some()
        || selected != interactions.selected
        || selected.is_some() != focused.current.is_some()
    {
        let prompt = selected
            .map(|(kind, index, step)| {
                if let SelectionKind::Openable(id)=kind{
                    let (Some(scene),Some(live))=(physics.as_deref(),live.as_deref()) else{return None;};
                    let (_,_,definition,labels)=scene.interaction_openable(live,id).ok().flatten()?;
                    return Some(super::interaction_focus::InteractionPrompt{entity:live.get(id)?.0.to_owned(),
                        name:labels[0].clone(),usage_text:labels[1].clone(),interaction_type:definition.icon,
                        binding:"<GBA_INTERACT>".to_owned(),name_postfix:String::new(),hold:false,
                        locked:definition.has(crate::gameplay::interaction_definition::LOCKED),player_state:String::new()});
                }
                if kind==SelectionKind::Usable&&interactions.actors[index].definition.static_definition.is_some(){
                    let (Some(scene),Some(live))=(physics.as_deref(),live.as_deref()) else{return None;};
                    let actor=&interactions.actors[index];let id=live.find(&actor.entity)?;
                    let (reference,definition)=scene.interaction_static(live,id).ok().flatten()?;
                    let labels=scene.static_usable_labels(reference.handle).ok()?;
                    return Some(super::interaction_focus::InteractionPrompt{entity:actor.entity.clone(),name:labels[0].clone(),
                        usage_text:labels[1].clone(),interaction_type:definition.icon,binding:"<GBA_INTERACT>".to_owned(),name_postfix:String::new(),hold:false,
                        locked:definition.has(crate::gameplay::interaction_definition::LOCKED),player_state:String::new()});
                }
                if let SelectionKind::Pickup(id) | SelectionKind::CarryLive(id) = kind {
                    let (Some(scene), Some(live)) = (physics.as_deref(), live.as_deref()) else {
                        return None;
                    };
                    let Ok(Some((_, pickable))) = scene.interaction_pickable(live, id) else {
                        return None;
                    };
                    return pickable_prompt(live,id,pickable);
                }
                if kind == SelectionKind::Carry {
                    let binding = &interactions.carrying[index];
                    return Some(super::interaction_focus::InteractionPrompt {
                        entity: binding.entity.clone(),
                        name: binding.interaction_name.clone(),
                        usage_text: binding.interaction_text.clone(),
                        interaction_type: binding.definition.pickup_interaction.icon,
                        binding: "<GBA_INTERACT>".to_owned(),
                        name_postfix: String::new(),
                        hold: false,
                        locked: binding
                            .definition
                            .pickup_interaction
                            .has(crate::gameplay::interaction_definition::LOCKED),
                        player_state: binding.player_state.clone(),
                    });
                }
                let (entity, name, text, icon, state, locked) = if kind == SelectionKind::Readable {
                    let item = &interactions.readable[index];
                    (
                        &item.entity,
                        item.interaction_name.as_deref(),
                        item.interaction_text.as_deref(),
                        item.interaction_icon.as_deref(),
                        Some("PLAYER_STATE_READ_NOTE"),
                        item.locked,
                    )
                } else {
                    let actor = &interactions.actors[index];
                    let def = &actor.definition;
                    let entry = def.steps.get(step);
                    if let Some(entry)=entry.filter(|entry|entry.overrides_interaction) {
                        (
                            &actor.entity,
                            entry.interaction_name.as_deref(),
                            entry.interaction_text.as_deref(),
                            entry.interaction_icon.as_deref(),
                            entry.player_state.as_deref(),
                            entry.locked,
                        )
                    } else {
                        (
                            &actor.entity,
                            def.interaction_name.as_deref(),
                            def.interaction_text.as_deref(),
                            def.interaction_icon.as_deref(),
                            def.player_state.as_deref(),
                            def.locked,
                        )
                    }
                };
                Some(super::interaction_focus::InteractionPrompt {
                    entity: entity.clone(),
                    name: name.unwrap_or("").to_owned(),
                    usage_text: text.unwrap_or("").to_owned(),
                    interaction_type: interaction_type(icon),
                    binding: "<GBA_INTERACT>".to_owned(),
                    name_postfix: String::new(),
                    hold: false,
                    locked,
                    player_state: state.unwrap_or("").to_owned(),
                })
            })
            .flatten();
        if focused.current != prompt {
            focused.current = prompt;
        }
        interactions.selected = selected;
    }
    if let Some(player) = player.as_deref_mut().and_then(|state| state.player.as_mut()) {
        //Use the sensor's accepted descriptor, not a HUD-only FarReach hint.
        //Locked interactions still own a descriptor in7fc420/846b50.
        player.set_interaction_presence(interactions.selected.is_some());
    }
    if trace_input {
        let actual=player.as_deref().and_then(|state|state.player.as_ref());
        eprintln!("ACTION_INTERACT_DECISION source=3b8430+7fc420 pressed={pressed} selected={selected:?} retained={:?} focus={:?} catch_hint={:?} pending_pickup={:?} pending47={:?} active47target={:?} mode47={:?} request47={:?}",
            interactions.selected,focused.current,interactions.grapple_prompt,
            interactions.pending_pickup.as_ref().map(|pending|(pending.entity,pending.phase)),
            actual.and_then(|p|p.pending_loot_object()),actual.and_then(|p|p.world_pickup_target()),actual.and_then(|p|p.world_pickup_mode()),actual.map(|p|p.loot_request_snapshot()));
        if let Some((SelectionKind::Openable(id),_,_))=selected {
            eprintln!("ACTION_INTERACT_OPENABLE id={id:?} primary={:?} secondary=not_hosted",
                physics.as_deref().zip(live.as_deref()).map(|(scene,live)|scene.interaction_openable(live,id)));
        }
        for (group,hits) in interactions.casts.hits.iter().enumerate(){
            eprintln!("ACTION_INTERACT_CAST group={group} count={} first={:?}",hits.len(),&hits[..hits.len().min(4)]);
        }
    }
    //74b6e0 still processes a locked door use (rattle/key/HUD result).
    //Its locked icon does not turn it into an unclickable pickup.
    let door=selected.is_some_and(|(kind,_,_)|matches!(kind,SelectionKind::Openable(_)));
    if !pressed || focused.current.as_ref().is_none_or(|prompt| prompt.locked&&!door) {
        return;
    }
    let Some((kind, index, _)) = selected else {
        return;
    };
    if let SelectionKind::Openable(id)=kind{
        let notify=physics.as_deref().zip(live.as_deref()).and_then(|(scene,live)|scene.interaction_openable(live,id).ok().flatten())
            .is_some_and(|(_,_,definition,_)|definition.has(crate::gameplay::interaction_definition::SEND_USE_NOTIFICATION));
        if trace_input {eprintln!("ACTION_INTERACT_OPENABLE_REQUEST id={id:?} send_use_notification={notify}");}
        if !notify{return;}
        let Some(instigator)=player.as_deref().and_then(|state|state.player.as_ref()).and_then(|player|player.managed_entity()) else{return;};
        entity_events.post(crate::gameplay::entity_events::Event{id:0x96,target:Some(id),
            payload:crate::gameplay::entity_events::Payload::Use{instigator:Some(instigator)}});
        interactions.selected=None;
        return;
    }
    if let SelectionKind::Pickup(id) = kind {
        if interactions.pending_pickup.is_none() {
            if let (Some(scene), Some(live), Some(player)) = (
                physics.as_deref(),
                live.as_deref(),
                player
                    .as_deref_mut()
                    .and_then(|state| state.player.as_mut()),
            ) {
                let current=scene.interaction_pickable(live,id);
                if trace_input {eprintln!("ACTION_INTERACT_REQUEST_TARGET id={id:?} current={:?}",
                    current.as_ref().map(|value|value.as_ref().map(|(reference,pickable)|(reference,pickable.item.as_ref().map(|item|&item.path)))));}
                if let Ok(Some((reference, pickable))) = current {
                    if let Some(item) = pickable.item.as_ref() {
                        let requested=player.request_loot_object(id);
                        if trace_input {eprintln!("ACTION_INTERACT_REQUEST id={id:?} state=47 requested={requested} item={:?}",item.path);}
                        if requested {
                            interactions.pending_pickup = Some(super::pickups::Pending {
                                entity: id,
                                reference,
                                definition: pickable.definition.clone(),
                                item: Some(item.clone()),
                                hold:None,
                                maximum: interactions.limits.maximum(&item.path),
                                phase: 0,
                                admitted: 0,
                                hud_notified: false,
                                from_grapple: false,
                                notice: Default::default(),
                                rejected_notice: false,
                                rejected_sound: None,
                            });
                        }
                    }
                }
            }
        }
    } else if let SelectionKind::CarryLive(id) = kind {
        if let (Some(scene), Some(live), Some(player)) = (
            physics.as_deref(), live.as_deref(),
            player.as_deref_mut().and_then(|state| state.player.as_mut()),
        ) {
            match scene.interaction_pickable(live, id) {
                Ok(Some((_, component))) => {
                    if component.definition.pickup_interaction.player_state==47 {
                        if let Err(error)=super::pickups::request_borrowed(&mut interactions,scene,live,player,id,false){warn!("Live hold pickup request: {error}");}
                } else {
                    let accepted=super::carrying::request(player, id, component.definition.clone());
                    player.carrying.trace("frontend-live-carry",Some(id),component.definition.pickup_interaction.player_state,accepted);
                }
                }
                Ok(None) => {}
                Err(error) => warn!("Live carry action identity: {error}"),
            }
        }
    } else if kind == SelectionKind::Carry {
        let (Some(scene), Some(live), Some(player)) = (
            physics.as_deref(),
            live.as_deref(),
            player
                .as_deref_mut()
                .and_then(|state| state.player.as_mut()),
        ) else {
            return;
        };
        match scene.carry_pickable(&interactions.carrying[index].entity, live) {
            Ok(Some((id, definition))) => {
                if definition.pickup_interaction.player_state==47 {
                    if let Err(error)=super::pickups::request_borrowed(&mut interactions,scene,live,player,id,false){warn!("Hold pickup request: {error}");}
                } else {
                    let state=definition.pickup_interaction.player_state;
                    let accepted=super::carrying::request(player, id, definition);
                    player.carrying.trace("frontend-placed-carry",Some(id),state,accepted);
                }
            }
            Ok(None) => {}
            Err(error) => warn!("Carry action identity: {error}"),
        }
    } else if kind == SelectionKind::Readable {
        let identity =
            physics
                .as_deref()
                .zip(live.as_deref())
                .and_then(|(scene, live)| {
                    match scene.readable_item(&interactions.readable[index].entity, live) {
                        Ok(value) => value,
                        Err(error) => {
                            warn!("Readable identity unavailable: {error}");
                            None
                        }
                    }
                });
        if let Some((identity, item)) = identity {
            interactions.request_readable(index, identity, item);
        }
    } else {
        //77b3b0/74cd90 register the same actual96 on Usable blockers.
        //This host executes the Usable directly below; publish its authored
        //use notification to the shared observers without a second executor.
        let actor=&interactions.actors[index];
        if actor.definition.static_definition.is_some(){
            let notify=physics.as_deref().zip(live.as_deref()).and_then(|(scene,live)|live.find(&actor.entity).and_then(|id|scene.interaction_static(live,id).ok().flatten()))
                .is_some_and(|(_,definition)|definition.has(crate::gameplay::interaction_definition::SEND_USE_NOTIFICATION));
            if notify{
                if let (Some(target),Some(instigator))=(live.as_deref().and_then(|live|live.find(&actor.entity)),
                    player.as_deref().and_then(|state|state.player.as_ref()).and_then(|player|player.managed_entity())){
                    entity_events.post(crate::gameplay::entity_events::Event{id:0x96,target:Some(target),
                        payload:crate::gameplay::entity_events::Payload::Use{instigator:Some(instigator)}});
                }
            }
            interactions.selected=None;
            return;
        }
        if let Some(state)=actor.state.as_ref(){
            let step=&state.definition.steps[state.state];
            let notify=if step.overrides_interaction{step.send_use_notification}else{state.definition.send_use_notification};
            if notify{
                if let (Some(target),Some(instigator))=(live.as_deref().and_then(|live|live.find(&actor.entity)),
                    player.as_deref().and_then(|state|state.player.as_ref()).and_then(|player|player.managed_entity())){
                    entity_events.post(crate::gameplay::entity_events::Event{id:0x96,target:Some(target),
                        payload:crate::gameplay::entity_events::Payload::Use{instigator:Some(instigator)}});
                }
            }
        }
        interactions.selected = None;
    }
}

fn read_is_live(
    read: &ReadResult,
    physics: Option<&super::physics_scene::PhysicsScene>,
    live: Option<&crate::gameplay::entity_lifetime::LiveEntities>,
) -> bool {
    physics.zip(live).is_some_and(|(scene, live)| {
        scene
            .interaction_pickable(live,read.identity.0)
            .is_ok_and(|current|current.is_some_and(|(reference,_)|reference==read.identity.1))
    })
}

///Bounded fixture report requested by the real-input action driver. All
///values are borrowed from production owners; no focus/pose/ACK is injected.
pub(super) fn describe_carry_fixture(world:&World,id:crate::gameplay::entity_lifetime::EntityId)->Result<String,String> {
    use std::fmt::Write;
    let live=world.get_resource::<crate::gameplay::entity_lifetime::LiveEntities>().ok_or("fixture LiveEntities absent")?;
    let scene=world.get_resource::<super::physics_scene::PhysicsScene>().ok_or("fixture PhysicsScene absent")?;
    let interactions=world.get_resource::<Interactions>().ok_or("fixture Interactions absent")?;
    let player=world.get_resource::<super::player::PlayerState>().and_then(|s|s.player.as_ref()).ok_or("fixture Player absent")?;
    let mut out=String::new();
    let (eye,axis)=player.published_camera_frame();
    writeln!(out,"player eye={eye:?} forward={:?} states={:?} swimming={} held={:?} pending47={:?} active47target={:?} mode47={:?} availableHold1={} availableHold2={}",axis.x_axis,player.active_player_states(),player.is_swimming(),player.carrying.held.as_ref().map(|t|t.entity),player.pending_loot_object(),player.world_pickup_target(),player.world_pickup_mode(),player.loot_hold_available(false),player.loot_hold_available(true)).unwrap();
    writeln!(out,"focused={:?} selected={:?} grapplePrompt={:?} closed={} pendingRead={} pendingActivation={} retiringReads={} pendingPickup={:?}",world.get_resource::<super::interaction_focus::FocusedInteraction>().and_then(|f|f.current.as_ref()),interactions.selected,interactions.grapple_prompt,interactions.acquisitions_closed.load(Ordering::Acquire),interactions.pending_read.is_some(),interactions.pending_activation.is_some(),interactions.retire.len(),interactions.pending_pickup.as_ref().map(|p|(p.entity,p.reference,p.phase,p.hold.is_some(),p.from_grapple))).unwrap();
    if let Some((name,_))=live.get(id) {
        writeln!(out,"binding placedCarry={:?} readable={:?} actor={:?}",interactions.carrying_by_entity.get(name),interactions.readable_by_entity.get(name),interactions.actor_by_entity.get(name)).unwrap();
        writeln!(out,"bounds={:?} carryContext={:?}",super::world_models::entity_bounds(world,live,id),world.get_resource::<super::shadow_updates::ModelShadowSources>().map(|sources|scene.carry_context(name,live,sources))).unwrap();
    }
    if let Some((reference,p))=scene.grapple_pickable(live,id)? {
        let def=&p.definition.pickup_interaction;
        let resolved_mode=if def.player_mode==14{i32::from(player.is_swimming())}else{def.player_mode};
        writeln!(out,"admission ref={reference:?} authoredState={} mode={} resolvedMode={resolved_mode} loot={:?} legacyCarry={:?} capability={:?}",def.player_state,def.player_mode,player.loot_interaction_allowed(resolved_mode,def.player_state),player.carry_interaction_allowed(resolved_mode,def.player_state),scene.grapple_pickup_available(live,player,id)).unwrap();
        if let Some(props)=&p.definition.hold {
            let tag=player.reference_tag_binding(&props.attach_tag).and_then(|(joint,offset,orientation)|player.reference_local_joint(joint).map(|frame|(joint,frame,offset,orientation)));
            writeln!(out,"carry_tag tag={} binding={tag:?} hold={:?} grab={:?} body_forward={:?}",props.attach_tag,player.carrying_hold_decl_for_hands(props.both_hands),player.carrying_grab_decl(),player.physics.axis().x_axis).unwrap();
        }
    }
    writeln!(out,"carry={}",super::physics_scene::carrying_host::describe_fixture_progress(world,id)).unwrap();
    player.carrying.write_trace(&mut out);
    for (group,hits) in interactions.casts.hits.iter().enumerate(){writeln!(out,"production_cast group={group} count={} hits={:?}",hits.len(),&hits[..hits.len().min(64)]).unwrap();}
    writeln!(out,"physics={}",scene.describe_interaction_fixture(live,id,eye,axis.x_axis,interactions.sensor)?).unwrap();
    Ok(out)
}

/// 14077be20 follows the owning entity pose. Unchanged owners do not touch the
/// mutable collision broad phase; stationary boxes remain at their last pose.
pub(super) fn sync_boxes(
    interactions: Option<ResMut<Interactions>>,
    world: Option<ResMut<super::NavigationWorld>>,
    bound: Option<Res<super::model_motion::BoundModels>>,
) {
    let (Some(mut interactions), Some(mut world), Some(bound)) = (interactions, world, bound)
    else {
        return;
    };
    for actor in &mut interactions.actors {
        let Some(handle) = actor.fake_box else {
            continue;
        };
        let Some(delta) = bound.entity_delta(&actor.entity).copied() else {
            continue;
        };
        if actor.last_delta == Some(delta) {
            continue;
        }
        let corners =
            entities::usable::fake_box_corners(&actor.frame, &actor.definition).map(|point| {
                let moved = std::array::from_fn(|row| {
                    delta[row][3]
                        + (0..3)
                            .map(|column| delta[row][column] * point[column])
                            .sum::<f32>()
                });
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
    navigation: Option<ResMut<super::NavigationWorld>>,
    physics: Option<ResMut<super::physics_scene::PhysicsScene>>,
    live: Option<ResMut<crate::gameplay::entity_lifetime::LiveEntities>>,
    mut entity_events: ResMut<crate::gameplay::entity_events::EntityEvents>,
    placements: Res<super::RenderSourcePlacements>,
    mut sources: ResMut<super::shadow_updates::ModelShadowSources>,
    mut retired: ResMut<super::interaction_focus::RetiredPlacements>,
    mut sounds: MessageWriter<super::audio::SoundEvent>,
) {
    let (Some(mut interactions), Some(mut navigation), Some(mut scene), Some(mut live)) =
        (interactions, navigation, physics, live)
    else {
        return;
    };
    if let Err(error) = drain_accepted_pickups(
        &mut interactions,
        &mut scene,
        &mut live,
        &mut entity_events,
        &mut navigation,
        &mut sources,
        &placements,
        &mut retired,
        |event| {
            sounds.write(event);
        },
    ) {
        warn!("document pickup callback pending: {error}");
    }
}

/// 844bf0 completes 76e2b0/76ec20 before returning. The host's deferred
/// callback must reach that same boundary before any component Stop, including
/// a map request which has already gated its normal PostUpdate consumer.
fn drain_accepted_pickups(
    interactions: &mut Interactions,
    scene: &mut super::physics_scene::PhysicsScene,
    live: &mut crate::gameplay::entity_lifetime::LiveEntities,
    entity_events: &mut crate::gameplay::entity_events::EntityEvents,
    navigation: &mut super::NavigationWorld,
    sources: &mut super::shadow_updates::ModelShadowSources,
    placements: &super::RenderSourcePlacements,
    hidden: &mut super::interaction_focus::RetiredPlacements,
    mut post: impl FnMut(super::audio::SoundEvent),
) -> Result<(), String> {
    while let Some(pending) = interactions.retire.first_mut() {
        if live.get(pending.identity.0).is_none() {
            return Err("document pickup identity retired before callback".into());
        }
        if let Some(sound) = scene.begin_document_pickup(
            live,
            entity_events,
            pending.identity.1,
            &mut pending.step,
        )? {
            // bevy_backend: PlayerBody is the existing player Audio host. Its
            // FIFO and ordinary retirement own the accepted sound request.
            post(super::audio::SoundEvent(
                crate::gameplay::sound::Request::Post {
                    event: sound.id,
                    emitter: crate::gameplay::sound::Emitter::PlayerBody,
                },
            ));
        }
        navigation
            .0
            .mask_placed_owner_filter(&pending.entity, 0x0820_001f);
        scene.finish_document_pickup(live, pending.identity.1, sources, placements, hidden)?;
        interactions.retire.remove(0);
    }
    Ok(())
}

/// Drain only already accepted work: never repeat pickedUp or inventory
/// admission, and retain every cursor/resource when a body callback fails.
pub(super) fn finish_accepted_pickups(world: &mut World) -> Result<(), String> {
    if world
        .get_resource::<Interactions>()
        .is_none_or(|owner| owner.retire.is_empty())
    {
        return Ok(());
    }
    let mut state = bevy::ecs::system::SystemState::<(
        ResMut<Interactions>,
        ResMut<super::physics_scene::PhysicsScene>,
        ResMut<crate::gameplay::entity_lifetime::LiveEntities>,
        ResMut<crate::gameplay::entity_events::EntityEvents>,
        ResMut<super::NavigationWorld>,
        ResMut<super::shadow_updates::ModelShadowSources>,
        Res<super::RenderSourcePlacements>,
        ResMut<super::interaction_focus::RetiredPlacements>,
        MessageWriter<super::audio::SoundEvent>,
    )>::new(world);
    let result = (|| {
        let (
            mut interactions,
            mut scene,
            mut live,
            mut entity_events,
            mut navigation,
            mut sources,
            placements,
            mut hidden,
            mut sounds,
        ) = state
            .get_mut(world)
            .map_err(|error| format!("accepted pickup retirement owner absent: {error}"))?;
        drain_accepted_pickups(
            &mut interactions,
            &mut scene,
            &mut live,
            &mut entity_events,
            &mut navigation,
            &mut sources,
            &placements,
            &mut hidden,
            |event| {
                sounds.write(event);
            },
        )
    })();
    state.apply(world);
    result
}

/// Real retained queue state for phase observers. This never admits a pickup
/// or manufactures a nonempty cursor for an otherwise idle world.
pub(super) fn accepted_pickup_progress(world: &World) -> Vec<(String, u8)> {
    world
        .get_resource::<Interactions>()
        .map(|owner| {
            owner
                .retire
                .iter()
                .map(|pending| (pending.entity.clone(), pending.step))
                .collect()
        })
        .unwrap_or_default()
}

/// Request the same action as a focused real readable. Phase observers use
/// installed bindings and admitted generations, then let ordinary App frames
/// perform READ_NOTE, pickedUp, inventory admission and movie publication.
pub(super) fn request_admitted_readable(
    world: &mut World,
) -> Result<Option<(usize, String)>, String> {
    if !world.contains_resource::<Interactions>() {
        return Ok(None);
    }
    world.resource_scope(|world, mut interactions: Mut<Interactions>| {
        if interactions.acquisitions_closed.load(Ordering::Acquire) {
            return Err("readable observation cannot admit into a closing level".into());
        }
        if interactions.pending_read.is_some()
            || interactions.pending_activation.is_some()
            || !interactions.retire.is_empty()
        {
            return Err("readable observation already has an unfinished action".into());
        }
        let scene = world
            .get_resource::<super::physics_scene::PhysicsScene>()
            .ok_or("readable observation has no admitted component owner")?;
        let live = world
            .get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
            .ok_or("readable observation has no live entity owner")?;
        let sources = world
            .get_resource::<super::shadow_updates::ModelShadowSources>()
            .ok_or("readable observation has no render source owner")?;
        let placements = world
            .get_resource::<super::RenderSourcePlacements>()
            .ok_or("readable observation has no model placements")?;
        let mut selected = None;
        for (index, binding) in interactions.readable.iter().enumerate() {
            // These are the same source-family exclusions as the normal
            // focused action, not a diagnostic-only admission path.
            if binding.locked
                || !binding.component.usable_by_player
                || live.find(&binding.entity).map(|id|live.parent(id)).transpose()?.flatten().is_some()
                || !binding.unsupported_pickup_components.is_empty()
            {
                continue;
            }
            let Some((identity, item)) = scene.readable_item(&binding.entity, live)? else {
                continue;
            };
            let portable = match &item.readable {
                Some(
                    crate::assets::inventory_item::PickableReadable::Note(note)
                    | crate::assets::inventory_item::PickableReadable::Other(note, _),
                ) => note.can_be_picked_up,
                Some(crate::assets::inventory_item::PickableReadable::Map(map)) => {
                    map.can_be_picked_up
                }
                None => false,
            };
            if !portable {
                continue;
            }
            if let Err(error) =
                scene.preflight_document_pickup(live, identity.1, sources, placements)
            {
                eprintln!(
                    "READABLE_ACTION_UNAVAILABLE entity={} reason={error}",
                    binding.entity
                );
                continue;
            }
            selected = Some((index, binding.entity.clone(), identity, item));
            break;
        }
        let Some((index, entity, identity, item)) = selected else {
            return Ok(None);
        };
        interactions.request_readable(index, identity, item);
        if !interactions
            .pending_read
            .as_ref()
            .is_some_and(|read| read.identity == identity)
        {
            return Err("the real readable action did not accept its request".into());
        }
        Ok(Some((index, entity)))
    })
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ReadActionProgress {
    pub pending_read: bool,
    pub pending_activation: bool,
    pub accepted_pickups: usize,
    pub acquisitions_closed: bool,
}

pub(super) fn read_action_progress(world: &World) -> Option<ReadActionProgress> {
    let owner = world.get_resource::<Interactions>()?;
    Some(ReadActionProgress {
        pending_read: owner.pending_read.is_some(),
        pending_activation: owner.pending_activation.is_some(),
        accepted_pickups: owner.retire.len(),
        acquisitions_closed: owner.acquisitions_closed.load(Ordering::Acquire),
    })
}

impl Interactions {
    ///The accepted ordinary sensor target, distinct from its HUD-only catch hint.
    ///No selection is reconstructed or changed; validate the actual live ID.
    pub(super) fn selected_interaction_target(&self,live:&crate::gameplay::entity_lifetime::LiveEntities)
        ->Option<crate::gameplay::entity_lifetime::EntityId>{
        let (kind,index,_)=self.selected?;
        let id=match kind {
            SelectionKind::Pickup(id)|SelectionKind::CarryLive(id)|SelectionKind::Openable(id)=>id,
            SelectionKind::Carry=>live.find(&self.carrying.get(index)?.entity)?,
            SelectionKind::Readable=>live.find(&self.readable.get(index)?.entity)?,
            SelectionKind::Usable=>live.find(&self.actors.get(index)?.entity)?,
        };
        live.get(id).map(|_|id)
    }
}

///Existing F8 callback only; selected is the actual sensor-owned descriptor.
pub(super) fn write_focus_diagnostics(world:&World,report:&mut String) {
    use std::fmt::Write;
    let _=writeln!(report,"\nInteraction focus: {:?}",world.get_resource::<super::interaction_focus::FocusedInteraction>().map(|owner|&owner.current));
    if let Some(owner)=world.get_resource::<Interactions>() {
        let _=writeln!(report,"  accepted_sensor_selection={:?} pending_changes={} pending_pickup={} pending_read={}",
            owner.selected,owner.pending.len(),owner.pending_pickup.is_some(),owner.pending_read.is_some());
    }else{let _=writeln!(report,"  actual interaction owner absent");}
}
