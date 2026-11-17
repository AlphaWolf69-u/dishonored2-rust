//! Managed identities for the actual player equipment and selected ammo child.
//! 619380 marks player equipment +d4 bit3 before 8aecc0/657ae0 registration;
//! 44dd00/62b240 spawn the ammo child normally, then set bit4 and bind it.
//! Model leases/fences remain with PlayerArms, not with this identity table.
use crate::gameplay::{
    component_state::types::ComponentTypes,
    entity_lifetime::{EntityId, LiveEntities, Stage},
};
use bevy::prelude::*;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Hash, PartialEq, Eq)]
enum Role { Equipment, Attachment, Arm }
#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    parent: EntityId,
    identity: String,
    declaration: String,
    role: Role,
}
struct Entry {
    key: Key,
    linked: bool,
    runtime_flags: u8,
    frame: Option<crate::gameplay::reference_frame::ReferenceFrame>,
    time_scale: f32,
    audio: Option<crate::gameplay::sound::Emitter>,
    audio_acknowledged: bool,
    rendered: bool,
    identity_retired: bool,
    graph_retired: bool,
    reference_retirement: Option<Box<super::physics_scene::lifetime::Retirement>>,
    movable: Option<super::runtime_entities::actor_class::Movable>,
}
struct PendingRetirement {
    id: EntityId,
    render: Option<super::player_arms::RetiredModel>,
}
#[derive(Resource)]
pub(in crate::app) struct Identities {
    types: Arc<ComponentTypes>,
    player: Option<EntityId>,
    entries: HashMap<EntityId, Entry>,
    pending: Vec<PendingRetirement>,
}

impl Identities {
    pub(in crate::app) fn published_frame(
        &self,
        id: EntityId,
    ) -> Option<(crate::gameplay::reference_frame::ReferenceFrame, f32)> {
        let entry = self.entries.get(&id)?;
        entry.linked.then_some((entry.frame?, entry.time_scale))
    }
}

/// Setup publication calls this before LevelLifetime captures its resources.
pub(super) fn initialize(world: &mut World) -> Result<(), String> {
    if world.contains_resource::<Identities>() {
        return Ok(());
    }
    let catalog = world
        .get_resource::<super::SceneData>()
        .and_then(|scene| scene.catalog.as_ref())
        .ok_or("equipment identity has no class metadata source")?;
    let types = ComponentTypes::acquire(catalog)?;
    let player = super::player_lifetime::id(world);
    world.insert_resource(Identities {
        types,
        player,
        entries: HashMap::new(),
        pending: Vec::new(),
    });
    admit_arms(world)?;
    Ok(())
}

///770560/8ae9b0 create two actual source-declared arm actors. Their existing
///Player rigs, controllers, sound objects and render surfaces remain owners.
fn admit_arms(world: &mut World) -> Result<(), String> {
    let Some(mut state)=world.remove_resource::<super::player::PlayerState>() else {
        return Err("arm admission has no actual PlayerState".into());
    };
    let result=(||{
        let Some(player)=state.player.as_mut() else{return Ok(());};
        let parent=player.managed_entity().ok_or("arm admission player identity absent")?;
        let timespace=world.resource::<crate::gameplay::game_clock_runtime::ClockRuntime>().entity_time_state(parent).map_err(str::to_owned)?.0;
        super::runtime_entities::with_scene(world,|_,scene,live,events|
            scene.admit_player_reference(player,live,events,timespace))?;
        for side in 0..player.body.arms.len() {
            let arm=&player.body.arms[side];
            if arm.managed_entity.is_some(){continue;}
            if arm.entity_class!="idEntity" {return Err(format!("arm {} requires actual constructor {}",arm.entity_declaration,arm.entity_class));}
            let id=admit(world,&format!("player-arm:{side}"),&arm.entity_declaration,&arm.entity_declaration,
                &arm.entity_class,parent,Role::Arm,arm.ai_tampering_declared)?;
            player.body.arms[side].managed_entity=Some(id);
            world.resource_mut::<crate::gameplay::game_clock_runtime::ClockRuntime>().register_entity_time(id,0)?;
            super::runtime_entities::with_scene(world,|_,scene,live,events|
                scene.bind_to_player(live,id,player,&player.body.arms[side].bind_tag,[0.;3],[0.,0.,0.,1.],events))?;
            if let Some(mut renders)=world.get_resource_mut::<super::player_arms::PlayerArms>() {
                renders.bind_arm_identity(&player.body.arms[side].model,id)?;
                drop(renders);
                mark_rendered(world,id)?;
            }
        }
        Ok(())
    })();
    world.insert_resource(state);
    result
}

/// Only a real admitted player can own equipment (619380). This function is
/// called after the existing equipment component/runtime and model prepare.
pub(super) fn admit_equipment(
    world: &mut World,
    identity: &str,
    declaration: &str,
    class: &str,
    ai_tampering_declared: bool,
) -> Result<EntityId, String> {
    // The native callback host temporarily borrows PlayerState out of World;
    // its constructed actor and this same managed generation remain alive.
    let parent = world
        .get_resource::<Identities>()
        .and_then(|owner| owner.player)
        .ok_or("equipment has no live player owner")?;
    if world
        .get_resource::<LiveEntities>()
        .and_then(|live| live.stage(parent))
        != Some(Stage::Spawned)
    {
        return Err("equipment player generation is stale".into());
    }
    admit(
        world,
        identity,
        declaration,
        declaration,
        class,
        parent,
        Role::Equipment,
        ai_tampering_declared,
    )
}

/// The actual weapon generation is the parent; never bind a bolt to a stand-in
/// player or use model readiness as a fabricated child identity.
pub(super) fn admit_ammunition(
    world: &mut World,
    descriptor: &crate::assets::equipment::attachment::Descriptor,
    weapon: EntityId,
) -> Result<EntityId, String> {
    admit_attachment(world, &descriptor.entity, descriptor, weapon)
}

pub(super) fn admit_attachment(
    world: &mut World,
    identity: &str,
    descriptor: &crate::assets::equipment::attachment::Descriptor,
    weapon: EntityId,
) -> Result<EntityId, String> {
    let identities = world
        .get_resource::<Identities>()
        .ok_or("equipment identity owner is absent")?;
    if !identities
        .entries
        .get(&weapon)
        .is_some_and(|entry| entry.key.role == Role::Equipment && entry.linked)
    {
        return Err("ammunition parent is not an admitted weapon generation".into());
    }
    // 8ae9b0 receives no clock-source entity for ammo/upgrade children.
    // Their idEntity874c50 selector +2ac starts at zero, independently of
    // the weapon selector copied by619380. Validate before identity admission.
    world
        .get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
        .ok_or("attachment has no canonical clock owner")?
        .clock()
        .map_err(str::to_owned)?;
    let id = admit(
        world,
        identity,
        &descriptor.entity,
        &descriptor.name_source,
        &descriptor.class,
        weapon,
        Role::Attachment,
        //The canonical attachment reader admits only an actual AnimBasic
        //component or a source component-less static-model constructor.
        false,
    )?;
    world
        .resource_mut::<crate::gameplay::game_clock_runtime::ClockRuntime>()
        .register_entity_time(id, 0)?;
    Ok(id)
}

fn admit(
    world: &mut World,
    identity: &str,
    declaration: &str,
    name_source: &str,
    class: &str,
    parent: EntityId,
    role: Role,
    ai_tampering_declared: bool,
) -> Result<EntityId, String> {
    let key = Key {
        parent,
        identity: identity.to_owned(),
        declaration: declaration.to_owned(),
        role: role.clone(),
    };
    if let Some((&id, _entry)) = world.get_resource::<Identities>().and_then(|owner| {
        owner
            .entries
            .iter()
            .find(|(_, entry)| entry.linked && entry.key == key)
    }) {
        let live = world
            .get_resource::<LiveEntities>()
            .ok_or("equipment managed owner is absent")?;
        if live.stage(id) != Some(Stage::Spawned) || live.parent(id)? != Some(parent) {
            return Err("equipment identity is retiring or stale".into());
        }
        return Ok(id);
    }
    if !world.contains_resource::<Identities>() {
        return Err("equipment identity was not initialized during level publication".into());
    }
    let types = world.resource::<Identities>().types.clone();
    //619380 and44dd00/62b240 copy the actual parent's current transform
    //before registration. This is the same frame, not a camera/root fallback.
    let birth_frame = world.get_resource::<super::physics_scene::PhysicsScene>()
        .ok_or("equipment reference scene is absent")?
        .equipment_parent_frame(world.resource::<LiveEntities>(), parent)?;
    let timespace = if role != Role::Equipment { 0 } else {
        world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
            .ok_or("equipment reference clock is absent")?.entity_time_state(parent)
            .map_err(str::to_owned)?.0
    };
    let classes = types.classes();
    let class_id = classes
        .hierarchy()
        .registration_order()
        .iter()
        .copied()
        .find(|&index| {
            classes
                .metadata(index)
                .and_then(|metadata| classes.name_text(metadata.name))
                == Some(class.as_bytes())
        })
        .ok_or_else(|| format!("equipment class is not registered: {class}"))?;
    let tag = world
        .get_resource::<LiveEntities>()
        .ok_or("equipment managed owner is absent")?
        .spawn_tag(parent)?;
    let mut live = world.resource_mut::<LiveEntities>();
    // idEntity constructor uses MAIN. Equipment619380 copies the actual
    // owning player's tag before registration; ammo8ae9b0 leaves MAIN intact.
    let main = types.intern_name(Some(b"MAIN"))?;
    let tag = if role == Role::Attachment { main } else { tag };
    if tag != main {
        return Err("equipment player spawn tag has no admitted spelling".into());
    }
    //4e65a0's arkGrenade constructor precedes619380 equipment admission.
    let constructor_flags = if class == "arkGrenade" { 0x20 } else { 0 };
    let runtime_flags = constructor_flags | if role == Role::Equipment { 8 } else { 0 };
    let id = live.allocate_managed_auto_name(name_source, class_id, runtime_flags, tag)?;
    let publication = live
        //619380 writes the actual inventory owner's ID at equipment+ac
        //before8aecc0 registration. Ammo8ae9b0 leaves its own owner unset.
        .set_owner(id, (role != Role::Attachment).then_some(parent))
        .and_then(|()| live.initialized(id))
        .and_then(|()| live.spawned(id))
        .and_then(|()| live.publish_spawn_tag(id, "MAIN"));
    if let Err(error) = publication {
        if live.retire_for_world_shutdown(id).is_none() {
            return Err(format!(
                "equipment publication failed: {error}; rollback retained {id:?}"
            ));
        }
        return Err(error);
    }
    drop(live);
    // Retain the true generation before entering fallible reference-actor
    //construction. Its ordinary scoped retirement owns any partial payload.
    world.resource_mut::<Identities>().entries.insert(
        id,
        Entry {
            key,
            linked: false,
            runtime_flags: if role == Role::Attachment { constructor_flags | 0x10 } else { runtime_flags },
            frame: Some(birth_frame),
            time_scale: 1.0,
            audio: None,
            audio_acknowledged: false,
            rendered: false,
            identity_retired: false,
            graph_retired: false,
            reference_retirement: None,
            movable: (class == "arkGrenade").then(super::runtime_entities::actor_class::Movable::new),
        },
    );
    let admitted = super::runtime_entities::with_scene(world, |_, scene, live, events| {
        scene.admit_equipment_reference(live, events, id, birth_frame, timespace)?;
        scene.publish_external_tampering_presence(live, id, ai_tampering_declared)?;
        scene.publish_equipment_runtime_flags(live, id, if role == Role::Attachment { constructor_flags | 0x10 } else { runtime_flags })?;
        scene.bind_equipment_reference(live, events, id, parent)
    });
    if let Err(error) = admitted {
        if let Err(cleanup) = await_unrendered_retirement(world, id) {
            return Err(format!("equipment {declaration} admission failed: {error}; retained rollback {id:?}: {cleanup}"));
        }
        return Err(format!("equipment {declaration} admission failed: {error}"));
    }
    world.resource_mut::<Identities>().entries.get_mut(&id)
        .ok_or("equipment admission journal disappeared")?.linked = true;
    Ok(id)
}

pub(super) fn item_identity(world: &World, id: EntityId) -> Option<&str> {
    let entry = world.get_resource::<Identities>()?.entries.get(&id)?;
    (entry.key.role == Role::Equipment && entry.linked).then_some(entry.key.identity.as_str())
}

pub(super) fn runtime_flags(world: &World, id: EntityId) -> Option<u8> {
    world
        .get_resource::<Identities>()?
        .entries
        .get(&id)
        .map(|entry| entry.runtime_flags)
}
pub(super) fn mark_movable_picked_up(world: &mut World, id: EntityId) -> Result<bool, String> {
    let Some(mut owner) = world.get_resource_mut::<Identities>() else { return Ok(false); };
    let Some(entry) = owner.entries.get_mut(&id) else { return Ok(false); };
    let Some(state) = entry.movable.as_mut() else { return Ok(false); };
    state.picked_up(true);
    entry.runtime_flags |= 4;
    Ok(true)
}
pub(super) fn movable_linked_entity(world:&World,id:EntityId)->Option<Option<EntityId>> {
    world.get_resource::<Identities>()?.entries.get(&id)?.movable.as_ref().map(|state|state.linked_entity())
}

pub(super) fn equipment(world: &World, identity: &str) -> Option<EntityId> {
    let owner = world.get_resource::<Identities>()?;
    owner.entries.iter().find_map(|(&id, entry)| {
        (entry.key.role == Role::Equipment && entry.linked && entry.key.identity == identity).then_some(id)
    })
}

pub(super) fn id(world: &World, identity: &str) -> Option<EntityId> {
    equipment(world, identity)
}

/// Bind only after the actual equipment declaration prepared cpntAudio.
/// `part` is the source's captured equipment casting bit (8/16).
pub(super) fn bind_audio(world: &mut World, id: EntityId, part: u8) -> Result<(), String> {
    if part != 8 && part != 16 {
        return Err("equipment audio has no native casting part".into());
    }
    let mut owner = world.resource_mut::<Identities>();
    let entry = owner
        .entries
        .get_mut(&id)
        .ok_or("equipment audio generation is absent")?;
    if entry.key.role != Role::Equipment || !entry.linked {
        return Err("equipment audio has no live component actor".into());
    }
    let emitter = crate::gameplay::sound::Emitter::Equipment { entity: id, part };
    if entry.audio.is_some_and(|old| old != emitter) {
        return Err("equipment audio owner changed casting part".into());
    }
    entry.audio = Some(emitter);
    Ok(())
}

pub(super) fn mark_rendered(world: &mut World, id: EntityId) -> Result<(), String> {
    let mut owner = world.resource_mut::<Identities>();
    let entry = owner
        .entries
        .get_mut(&id)
        .ok_or("equipment render generation is absent")?;
    if !entry.linked {
        return Err("cannot publish a detached equipment model".into());
    }
    entry.rendered = true;
    Ok(())
}

/// The real equipment animation/binding owner supplies its entity frame and
/// clock scale. No arm-slot alias or inferred model root becomes an identity.
pub(super) fn publish_frame(
    world: &mut World,
    id: EntityId,
    frame: crate::gameplay::reference_frame::ReferenceFrame,
    time_scale: f32,
) -> Result<(), String> {
    publish_frame_using(world, id, frame, time_scale, None)
}
pub(super) fn publish_frame_using(
    world: &mut World, id: EntityId, frame: crate::gameplay::reference_frame::ReferenceFrame,
    time_scale: f32, player: Option<&crate::gameplay::player::Player>,
) -> Result<(), String> {
    if !time_scale.is_finite() || time_scale < 0.0 {
        return Err("equipment time scale is invalid".into());
    }
    let mut owner = world
        .get_resource_mut::<Identities>()
        .ok_or("equipment identity owner is absent")?;
    let entry = owner
        .entries
        .get_mut(&id)
        .ok_or("equipment frame targeted a stale generation")?;
    if !entry.linked {
        return Err("equipment frame targeted a detached generation".into());
    }
    entry.frame = Some(frame);
    entry.time_scale = time_scale;
    drop(owner);
    super::runtime_entities::with_scene(world, |world, scene, live, _| {
        let clock = world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
            .ok_or("equipment publication clock is absent")?;
        let timespace = clock.entity_time_state(id).map_err(str::to_owned)?.0;
        scene.publish_equipment_reference(live, id, frame, timespace, clock.clock().map_err(str::to_owned)?, player)
    })
}

pub(super) fn frame(
    world: &World,
    id: EntityId,
) -> Option<(crate::gameplay::reference_frame::ReferenceFrame, f32)> {
    let owner = world.get_resource::<Identities>()?;
    let entry = owner.entries.get(&id)?;
    entry.linked.then_some((entry.frame?, entry.time_scale))
}

/// Withdraw the actual hierarchy binding when the render owner withdraws its
/// surfaces. Keep the generation until that owner's real GPU fence completes.
pub(super) fn detach(world: &mut World, id: EntityId) -> Result<(), String> {
    let owner = world
        .get_resource::<Identities>()
        .ok_or("equipment identity owner is absent")?;
    let entry = owner
        .entries
        .get(&id)
        .ok_or("equipment generation is not owned")?;
    let _ = entry;
    // Also handles partial birth: an entered canonical binding may exist
    //before the equipment journal marks the accepted linkage complete.
    super::runtime_entities::with_scene(world, |_, scene, live, _| {
        scene.detach_equipment_reference(live, id)
    })?;
    let mut owner = world.resource_mut::<Identities>();
    owner
        .entries
        .get_mut(&id)
        .ok_or("equipment owner disappeared")?
        .linked = false;
    Ok(())
}

/// Called only after the existing PlayerArms submitted-work acknowledgement.
/// Source/model leases stay with the retired render payload through that ack.
pub(super) fn retire_acknowledged(world: &mut World, id: EntityId) -> Result<bool, String> {
    if super::animation_effects::blocks_retirement(world, id) { return Ok(false); }
    let owner = world
        .get_resource::<Identities>()
        .ok_or("equipment identity owner is absent")?;
    let Some(entry) = owner.entries.get(&id) else {
        return Ok(true);
    };
    if entry.linked {
        return Err("equipment retirement preceded hierarchy detach".into());
    }
    if entry.audio.is_some() && !entry.audio_acknowledged {
        return Err("equipment identity retirement preceded its audio acknowledgement".into());
    }
    if entry.rendered
        && !owner.pending.iter().any(|pending| {
            pending.id == id
                && pending
                    .render
                    .as_ref()
                    .is_some_and(|receipt| receipt.complete())
        })
    {
        return Err("rendered equipment retirement has no completed GPU receipt".into());
    }
    let identity_retired = entry.identity_retired;
    let audio = entry.audio;
    retire_graph(world, id)?;
    let mut state = world.resource_mut::<Identities>().entries.get_mut(&id)
        .ok_or("equipment reference retirement journal disappeared")?
        .reference_retirement.take().unwrap_or_default();
    let retired = super::runtime_entities::with_scene(world, |world, scene, live, events| {
        scene.retire_equipment_reference(world, live, events, id, &mut state)
    });
    world.resource_mut::<Identities>().entries.get_mut(&id)
        .ok_or("equipment reference retirement journal disappeared")?.reference_retirement = Some(state);
    if !retired? { return Ok(false); }
    //80e050 must still be able to resolve the actual actor headers and roots.
    if !identity_retired && world.contains_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>() {
        super::time_sync::retire_entity_time(world, id)?;
    }
    if !identity_retired && world.resource_mut::<LiveEntities>().retire(id).is_none() {
        return Err("equipment identity still owns children or managed references".into());
    }
    world
        .resource_mut::<Identities>()
        .entries
        .get_mut(&id)
        .ok_or("equipment retirement record disappeared")?
        .identity_retired = true;
    super::runtime_entities::with_scene(world, |_, scene, live, _| {
        scene.finish_equipment_reference(live, id)
    })?;
    if let Some(emitter) = audio {
        super::audio::forget_retired_component_object(world, emitter)?;
    }
    world.resource_mut::<Identities>().entries.remove(&id);
    Ok(true)
}

/// Accept the exact renderer receipt after draw withdrawal; never make a
/// frame-count or acquisition-complete event stand in for this GPU fence.
pub(super) fn await_render_retirement(
    world: &mut World,
    id: EntityId,
    receipt: super::player_arms::RetiredModel,
) -> Result<(), String> {
    let mut owner = world.resource_mut::<Identities>();
    if !owner.entries.contains_key(&id) {
        return Err("render retirement has no owned equipment generation".into());
    }
    if !owner.pending.iter().any(|pending| pending.id == id) {
        owner.pending.push(PendingRetirement {
            id,
            render: Some(receipt),
        });
    }
    drop(owner);
    // Retain the real receipt before a fallible hierarchy callback. A retry
    // must never mistake already withdrawn surfaces for an unrendered child.
    detach(world, id)?;
    retire_graph(world, id)
}

fn retire_graph(world: &mut World, id: EntityId) -> Result<(), String> {
    let done = world
        .resource::<Identities>()
        .entries
        .get(&id)
        .ok_or("equipment graph retirement record disappeared")?
        .graph_retired;
    if !done {
        super::entity_graphs::retire(world, id)?;
        world
            .resource_mut::<Identities>()
            .entries
            .get_mut(&id)
            .ok_or("equipment graph retirement record disappeared")?
            .graph_retired = true;
    }
    Ok(())
}

pub(super) fn retirement_pending(world: &World, id: EntityId) -> bool {
    world
        .get_resource::<Identities>()
        .is_some_and(|owner| owner.pending.iter().any(|pending| pending.id == id))
}

/// No submitted receipt is required only when this actor never published a
/// draw model. Preserve any earlier real receipt on a withdrawal retry.
pub(super) fn await_unrendered_retirement(world: &mut World, id: EntityId) -> Result<(), String> {
    let mut owner = world.resource_mut::<Identities>();
    if !owner.entries.contains_key(&id) {
        return Err("unrendered equipment generation is absent".into());
    }
    if !owner.pending.iter().any(|pending| pending.id == id) {
        if owner.entries.get(&id).is_some_and(|entry| entry.rendered) {
            return Err("rendered equipment lost its real GPU retirement receipt".into());
        }
        owner.pending.push(PendingRetirement { id, render: None });
    }
    drop(owner);
    detach(world, id)?;
    retire_graph(world, id)
}

/// Ordinary per-frame completion polling by the existing admission owner.
/// Retire children first, even when one submission acknowledged both models.
pub(super) fn poll_retirements(world: &mut World) -> Result<bool, String> {
    // Queue every independent object's Stop before consuming the canonical
    // FIFO; a per-object frame barrier would serialize ordinary map teardown.
    let count = world
        .get_resource::<Identities>()
        .map_or(0, |owner| owner.pending.len());
    let mut have_audio = false;
    for index in 0..count {
        let audio = world.get_resource::<Identities>().and_then(|owner| {
            let pending = owner.pending.get(index)?;
            let entry = owner.entries.get(&pending.id)?;
            (!entry.audio_acknowledged).then_some((pending.id, entry.audio?))
        });
        if let Some((id, emitter)) = audio {
            have_audio = true;
            let acknowledged = super::audio::retire_component_object(world, emitter)?;
            if acknowledged {
                world
                    .resource_mut::<Identities>()
                    .entries
                    .get_mut(&id)
                    .ok_or("equipment audio retirement record disappeared")?
                    .audio_acknowledged = true;
            }
        }
    }
    if have_audio {
        super::audio::poll_object_retirements(world)?;
    }
    loop {
        let Some(owner) = world.get_resource::<Identities>() else {
            return Ok(true);
        };
        let next = owner.pending.iter().find_map(|pending| {
            (pending
                .render
                .as_ref()
                .is_none_or(|receipt| receipt.complete())
                && owner.entries.get(&pending.id).is_some_and(|entry| {
                    !entry.linked && (entry.audio.is_none() || entry.audio_acknowledged)
                })
                && !owner
                    .entries
                    .values()
                    .any(|entry| entry.key.parent == pending.id))
            .then_some(pending.id)
        });
        let Some(id) = next else {
            return Ok(owner.pending.is_empty());
        };
        if !retire_acknowledged(world, id)? { return Ok(false); }
        let mut owner = world.resource_mut::<Identities>();
        let index = owner
            .pending
            .iter()
            .position(|pending| pending.id == id)
            .ok_or("equipment retirement receipt disappeared")?;
        owner.pending.swap_remove(index);
    }
}

/// After all actual equipment/attachment renderer withdrawals, wait for their
/// receipts before allowing the player generation and level owner to retire.
pub(super) fn shutdown(world: &mut World) -> Result<bool, String> {
    //The same per-model GPU receipt owns first-person arms as equipment;
    //withdraw after producers stop, before their real identities are freed.
    let arms=world.get_resource::<Identities>().map(|owner|owner.entries.iter()
        .filter_map(|(&id,entry)|(entry.key.role==Role::Arm&&entry.linked).then_some(id)).collect::<Vec<_>>()).unwrap_or_default();
    for id in arms {
        let mut commands=super::setup_commands::SetupCommands::prepared(world);
        let receipt=world.get_resource_mut::<super::player_arms::PlayerArms>()
            .and_then(|mut models|models.remove_arm_model(id,&mut commands));
        commands.publish(world);
        if let Some(receipt)=receipt {await_render_retirement(world,id,receipt)?;}
        else {await_unrendered_retirement(world,id)?;}
    }
    if !poll_retirements(world)? {
        return Ok(false);
    }
    if world
        .get_resource::<Identities>()
        .is_some_and(|owner| !owner.entries.is_empty())
    {
        return Err("equipment shutdown still owns active entity generations".into());
    }
    world.remove_resource::<Identities>();
    Ok(true)
}
