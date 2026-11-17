//! A demanded equipment consumer of the existing runtime material owner.
//! Acquisition/mesh construction runs in MaterialWork, with its AssetBatch,
//! material snapshot, image leases and ordinary publication/retirement.
use super::*;
use crate::assets::equipment::Descriptor;
use crate::assets::equipment::attachment::Descriptor as AttachmentDescriptor;
use crate::gameplay::entity_lifetime::EntityId;

pub(in crate::app) struct EquipmentRequest {
    pub entity: EntityId,
    pub descriptor: Arc<Descriptor>,
}

pub(in crate::app) struct AttachmentRequest {
    pub entity: EntityId,
    pub descriptor: Arc<AttachmentDescriptor>,
}

pub(in crate::app) struct PreparedAttachment {
    pub entity: EntityId,
    pub model: super::super::player_arms::attachment::PreparedModel,
}

#[derive(Default)]
pub(super) struct EquipmentAdmissions {
    requested: HashMap<EntityId, std::sync::Weak<Descriptor>>,
    live: Vec<std::sync::Weak<Descriptor>>,
    failed: HashMap<EntityId, String>,
    staged: HashMap<String, Arc<Descriptor>>,
    selected: HashMap<usize, String>,
    owned: HashMap<String, Arc<Descriptor>>,
    metadata: HashMap<String, Arc<crate::assets::equipment::Metadata>>,
    stopping: HashMap<String, EntityId>,
    load_error: Option<String>,
    pose_dirty: bool,
    auto_admissions: bool,
    upgrade_starts: Vec<(EntityId, Arc<Descriptor>)>,
    ammo_starts: Vec<(
        EntityId,
        Arc<Descriptor>,
        String,
        Option<String>,
        Option<String>,
    )>,
    attachments: HashMap<EntityId, std::sync::Weak<AttachmentDescriptor>>,
    attachment_live: Vec<std::sync::Weak<AttachmentDescriptor>>,
    attachment_failed: HashMap<EntityId, String>,
}

pub(in crate::app) fn request_attachment(
    world: &mut World,
    id: EntityId,
    descriptor: Arc<AttachmentDescriptor>,
) -> Result<(), String> {
    let mut state = world
        .get_resource_mut::<RuntimeMaterialCoordinator>()
        .ok_or("attachment has no shared material coordinator")?;
    if let Some(error) = state.equipment.attachment_failed.get(&id) {
        return Err(error.clone());
    }
    let lease = Arc::downgrade(&descriptor);
    if state
        .equipment
        .attachments
        .get(&id)
        .is_some_and(|old| old.ptr_eq(&lease))
    {
        return Ok(());
    }
    state.equipment.attachments.insert(id, lease.clone());
    if !state
        .equipment
        .attachment_live
        .iter()
        .any(|old| old.ptr_eq(&lease))
    {
        state.equipment.attachment_live.push(lease);
    }
    state.revision = None;
    state.published_generation = None;
    Ok(())
}

pub(in crate::app) fn withdraw_attachment(world: &mut World, id: EntityId) {
    if let Some(mut state) = world.get_resource_mut::<RuntimeMaterialCoordinator>() {
        state.equipment.attachments.remove(&id);
        state.equipment.attachment_failed.remove(&id);
        state.revision = None;
        state.published_generation = None;
    }
}

pub(in crate::app) fn attachment_failure(world: &World, id: EntityId) -> Option<String> {
    world
        .get_resource::<RuntimeMaterialCoordinator>()
        .and_then(|state| state.equipment.attachment_failed.get(&id))
        .cloned()
}

fn attachment_admitted(world: &World, id: EntityId) -> bool {
    world
        .get_resource::<super::super::player_arms::PlayerArms>()
        .is_some_and(|arms| arms.attachments.has(id))
}

pub(super) fn requested_attachments(
    world: &World,
    state: &RuntimeMaterialCoordinator,
) -> Vec<AttachmentRequest> {
    state
        .equipment
        .attachments
        .iter()
        .filter(|(id, _)| {
            !state.equipment.attachment_failed.contains_key(*id)
                && !attachment_admitted(world, **id)
        })
        .filter_map(|(id, lease)| {
            lease.upgrade().map(|descriptor| AttachmentRequest {
                entity: *id,
                descriptor,
            })
        })
        .collect()
}

pub(super) fn add_attachment_leases(
    state: &EquipmentAdmissions,
    consumers: &HashSet<MaterialConsumer>,
    leases: &mut Vec<std::sync::Weak<AttachmentDescriptor>>,
) {
    for consumer in consumers {
        let MaterialConsumer::Attachment(id) = consumer else {
            continue;
        };
        if let Some(lease) = state.attachments.get(id) {
            if !leases.iter().any(|old| old.ptr_eq(lease)) {
                leases.push(lease.clone());
            }
        }
    }
}

/// Called only for the actual completed inventory demand, before installing
/// its gameplay tree. No archive/program/mesh decoding occurs on this thread.
pub(in crate::app) fn request(
    world: &mut World,
    entity: EntityId,
    descriptor: Arc<Descriptor>,
) -> Result<(), String> {
    // Equipment without a render model has no surfaces to admit.
    if descriptor.visual.is_none() {
        return Ok(());
    }
    let mut coordinator = world
        .get_resource_mut::<RuntimeMaterialCoordinator>()
        .ok_or("runtime material coordinator is absent")?;
    let key = entity;
    if let Some(error) = coordinator.equipment.failed.get(&key) {
        return Err(error.clone());
    }
    let lease = Arc::downgrade(&descriptor);
    if coordinator
        .equipment
        .requested
        .get(&key)
        .is_some_and(|old| old.ptr_eq(&lease))
    {
        return Ok(());
    }
    coordinator.equipment.requested.insert(key, lease.clone());
    coordinator.equipment.live.push(lease);
    coordinator.revision = None;
    coordinator.published_generation = None;
    Ok(())
}

/// CPU asset publication and the shared image binding gate completed. The
/// existing Bevy renderer still performs its ordinary GPU readiness checks.
pub(in crate::app) fn admitted(world: &World, entity: EntityId) -> bool {
    world
        .get_resource::<super::super::player_arms::PlayerArms>()
        .is_some_and(|arms| arms.has_equipment_model(entity))
}

pub(in crate::app) fn failure(world: &World, entity: EntityId) -> Option<String> {
    world
        .get_resource::<RuntimeMaterialCoordinator>()
        .and_then(|owner| owner.equipment.failed.get(&entity))
        .cloned()
}

fn withdraw_model(
    world: &mut World,
    entity: EntityId,
) -> Option<super::super::player_arms::RetiredModel> {
    if let Some(mut owner) = world.get_resource_mut::<RuntimeMaterialCoordinator>() {
        owner.equipment.requested.remove(&entity);
        owner.equipment.failed.remove(&entity);
        owner.revision = None;
        owner.published_generation = None;
    }
    let mut arms = world.remove_resource::<super::super::player_arms::PlayerArms>()?;
    let mut commands = super::super::setup_commands::SetupCommands::prepared(world);
    let receipt = arms.remove_equipment_model(entity, &mut commands);
    commands.publish(world);
    world.insert_resource(arms);
    receipt
}

/// Source EntityStop: children withdraw before the parent. Keep CPU leases and
/// the exact managed generation reachable until every fallible detach accepted
/// its real renderer/audio retirement cursor.
fn retire_owned(world: &mut World, identity: &str) -> Result<bool, String> {
    let id = world
        .get_resource::<RuntimeMaterialCoordinator>()
        .and_then(|owner| owner.equipment.stopping.get(identity).copied())
        .or_else(|| super::super::equipment_identity::id(world, identity));
    if let Some(id) = id {
        world
            .resource_mut::<RuntimeMaterialCoordinator>()
            .equipment
            .stopping
            .insert(identity.to_owned(), id);
    }
    if let Some(id) = id {
        if super::super::player_animation_events::blocks_retirement(world, id)
            || super::super::animation_effects::blocks_retirement(world, id) { return Ok(false); }
        if !super::super::weapon_firing::retired(world, identity) { return Ok(false); }
        super::super::weapon_throwing::remove_owned(world, identity);
        super::super::melee::remove_owned(world, identity);
        if !super::super::weapon_throwing::retired(world, identity)
            || !super::super::melee::retired(world, identity) { return Ok(false); }
        if !super::super::heart::retire(world, id)? { return Ok(false); }
        super::super::equipment_attachment_admission::remove(world, id)?;
    }
    let receipt = id.and_then(|entity| withdraw_model(world, entity));
    if let Some(id) = id {
        match receipt {
            Some(receipt) => {
                super::super::equipment_identity::await_render_retirement(world, id, receipt)?
            }
            None => super::super::equipment_identity::await_unrendered_retirement(world, id)?,
        }
    }
    if let Some(mut owner) = world.get_resource_mut::<RuntimeMaterialCoordinator>() {
        owner.equipment.owned.remove(identity);
        owner.equipment.staged.remove(identity);
        owner.equipment.metadata.remove(identity);
        owner.equipment.stopping.remove(identity);
        if let Some(id) = id {
            owner.equipment.ammo_starts.retain(|entry| entry.0 != id);
            owner.equipment.upgrade_starts.retain(|entry| entry.0 != id);
        }
    }
    Ok(true)
}

/// Withdraw only the actual inventory actor; declarations/models may be shared
/// by other admitted generations and are never used as instance identities.
fn withdraw(world: &mut World, identity: &str) {
    if let Err(error) = retire_owned(world, identity) {
        world
            .resource_mut::<RuntimeMaterialCoordinator>()
            .equipment
            .load_error
            .get_or_insert(error);
    }
}

/// Stop while scene, audio and render owners remain alive. Each call advances
/// existing worker/consumer receipts once; native animation time is irrelevant.
pub(in crate::app) fn shutdown(world: &mut World) -> Result<bool, String> {
    if let Some(mut owner) = world.get_resource_mut::<RuntimeMaterialCoordinator>() {
        if !owner.retire_acquisition() {
            return Ok(false);
        }
    }
    if let Some(mut arms) = world.get_resource_mut::<super::super::player_arms::PlayerArms>() {
        arms.retire_completed_models();
    }
    if !super::super::equipment_attachment_admission::shutdown(world)? {
        return Ok(false);
    }
    let identities = world
        .get_resource::<RuntimeMaterialCoordinator>()
        .map(|owner| {
            owner
                .equipment
                .metadata
                .keys()
                .chain(owner.equipment.owned.keys())
                .chain(owner.equipment.staged.keys())
                .chain(owner.equipment.stopping.keys())
                .cloned()
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    for identity in identities {
        if let Some(mut state) = world.get_resource_mut::<super::super::player::PlayerState>() {
            if let Some(player) = state.player.as_mut() {
                player.retire_equipment(&identity);
            }
        }
        if !retire_owned(world, &identity)? { return Ok(false); }
    }
    if let Some(mut owner) = world.get_resource_mut::<RuntimeMaterialCoordinator>() {
        owner.equipment.ammo_starts.clear();
        owner.equipment.upgrade_starts.clear();
        owner.equipment.selected.clear();
    }
    super::super::equipment_identity::shutdown(world)
}

pub(super) fn needs_update(state: &RuntimeMaterialCoordinator) -> bool {
    state
        .equipment
        .live
        .iter()
        .any(|lease| lease.strong_count() == 0)
        || state
            .equipment
            .attachment_live
            .iter()
            .any(|lease| lease.strong_count() == 0)
}

pub(super) fn retire_expired(state: &mut RuntimeMaterialCoordinator) {
    let before = state.equipment.live.len();
    let attachments_before = state.equipment.attachment_live.len();
    state
        .equipment
        .attachment_live
        .retain(|lease| lease.strong_count() != 0);
    state
        .equipment
        .attachments
        .retain(|_, lease| lease.strong_count() != 0);
    state
        .equipment
        .live
        .retain(|lease| lease.strong_count() != 0);
    state
        .equipment
        .requested
        .retain(|_, lease| lease.strong_count() != 0);
    if state.equipment.live.len() != before
        || state.equipment.attachment_live.len() != attachments_before
    {
        state.revision = None;
    }
}

pub(super) fn requested(
    world: &World,
    state: &RuntimeMaterialCoordinator,
) -> Vec<EquipmentRequest> {
    state
        .equipment
        .requested
        .iter()
        .filter(|(key, _)| !state.equipment.failed.contains_key(*key) && !admitted(world, **key))
        .filter_map(|(entity, lease)| {
            lease.upgrade().map(|descriptor| EquipmentRequest {
                entity: *entity,
                descriptor,
            })
        })
        .collect()
}

pub(super) fn model_entities(
    world: &World,
    state: &EquipmentAdmissions,
    models: &[super::super::player_arms::PreparedEquipmentModel],
) -> Result<Vec<EntityId>, String> {
    models
        .iter()
        .map(|model| {
            let id = model.entity();
            if !state
                .requested
                .get(&id)
                .is_some_and(|lease| lease.ptr_eq(&Arc::downgrade(model.descriptor())))
            {
                return Err("prepared equipment request generation changed".into());
            }
            let retained = state.owned.iter().any(|(identity, descriptor)| {
                Arc::ptr_eq(descriptor, model.descriptor())
                    && super::super::equipment_identity::id(world, identity) == Some(id)
            });
            if !retained {
                return Err("prepared equipment has no actual owned inventory actor".into());
            }
            Ok(id)
        })
        .collect()
}

pub(super) fn add_material_leases(
    state: &EquipmentAdmissions,
    consumers: &HashSet<MaterialConsumer>,
    leases: &mut Vec<std::sync::Weak<Descriptor>>,
) {
    for consumer in consumers {
        let MaterialConsumer::Equipment(entity) = consumer else {
            continue;
        };
        let Some(lease) = state.requested.get(entity) else {
            continue;
        };
        if !leases.iter().any(|old| old.ptr_eq(lease)) {
            leases.push(lease.clone());
        }
    }
}

pub(super) fn failed(world: &World, state: &mut RuntimeMaterialCoordinator, error: &str) {
    for id in state.equipment.attachments.keys() {
        if !attachment_admitted(world, *id) {
            state
                .equipment
                .load_error
                .get_or_insert_with(|| error.to_owned());
            state
                .equipment
                .attachment_failed
                .insert(*id, error.to_owned());
        }
    }
    for name in state.equipment.requested.keys() {
        if !admitted(world, *name) {
            state
                .equipment
                .load_error
                .get_or_insert_with(|| error.to_owned());
            state.equipment.failed.insert(*name, error.to_owned());
        }
    }
}

/// The loading gate waits only for actual owned material/mesh/image publication.
/// Native staged draws advance after continue on the normal game clock.
pub(in crate::app) fn startup_pending(world: &World) -> bool {
    super::super::equipment_attachment_admission::startup_pending(world)
        || super::super::weapon_throwing::startup_pending(world)
        || super::super::melee::startup_pending(world)
        || super::super::heart::startup_pending(world)
        || world
            .get_resource::<RuntimeMaterialCoordinator>()
            .is_some_and(|owner| {
                owner.equipment.requested.iter().any(|(entity, lease)| {
                    lease.strong_count() != 0
                        && !owner.equipment.failed.contains_key(entity)
                        && !admitted(world, *entity)
                }) || owner.equipment.attachments.iter().any(|(id, lease)| {
                    lease.strong_count() != 0
                        && !owner.equipment.attachment_failed.contains_key(id)
                        && !attachment_admitted(world, *id)
                })
            })
}
///Read only from the existing opt-in loading diagnostic; ordinary gameplay
///does not format owner state or retain a second readiness certificate.
pub(in crate::app) fn debug_startup_pending(world: &World) -> String {
    let coordinator = world.get_resource::<RuntimeMaterialCoordinator>();
    let models = coordinator.map_or(0, |owner|owner.equipment.requested.iter().filter(|(id,lease)|
        lease.strong_count()!=0&&!admitted(world,**id)).count());
    let attachments = coordinator.map_or(0, |owner|owner.equipment.attachments.iter().filter(|(id,lease)|
        lease.strong_count()!=0&&!attachment_admitted(world,**id)).count());
    let mut report=format!("attachments={} throwable={} melee={} Heart={} models_pending={} attachment_models_pending={}",
        super::super::equipment_attachment_admission::startup_pending(world),
        super::super::weapon_throwing::startup_pending(world),
        super::super::melee::startup_pending(world),
        super::super::heart::startup_pending(world),models,attachments);
    if let (Some(actions),Some(player))=(world.get_resource::<super::super::player_actions::PlayerActions>(),
        world.get_resource::<super::super::player::PlayerState>().and_then(|state|state.player.as_ref())) {
        report.push_str("\n  ");report.push_str(&actions.debug_ready_targets(player));
    }
    for details in [super::super::equipment_attachment_admission::debug_startup_pending(world),
        super::super::weapon_throwing::debug_startup_pending(world),super::super::melee::debug_startup_pending(world),
        super::super::fx_gameplay::debug_pending(world)] {
        report.push('\n');report.push_str(&details);
    }
    report
}

pub(in crate::app) fn startup_error(world: &World) -> Option<String> {
    world
        .get_resource::<RuntimeMaterialCoordinator>()
        .and_then(|owner| {
            owner
                .equipment
                .load_error
                .as_ref()
                .or_else(|| owner.equipment.failed.values().next())
        })
        .cloned()
        .or_else(|| super::super::equipment_attachment_admission::startup_error(world))
        .or_else(|| super::super::weapon_throwing::startup_error(world).map(str::to_owned))
        .or_else(|| super::super::melee::startup_error(world).map(str::to_owned))
        .or_else(|| super::super::heart::startup_error(world))
}

fn take_staged(world: &mut World, identity: &str) -> Option<Arc<Descriptor>> {
    let mut owner = world.get_resource_mut::<RuntimeMaterialCoordinator>()?;
    owner
        .equipment
        .staged
        .remove(identity)
        .or_else(|| owner.equipment.owned.get(identity).cloned())
}

fn retain_metadata(
    world: &mut World,
    identity: &str,
    metadata: Arc<crate::assets::equipment::Metadata>,
) -> Result<EntityId, String> {
    let entity = super::super::equipment_identity::admit_equipment(
        world,
        identity,
        &metadata.entity,
        &metadata.class_name,
        metadata.ai_tampering_declared,
    )?;
    // Own the declaration before fallible component registration, so Stop can
    // unwind the exact managed generation even if audio registration fails.
    world
        .resource_mut::<RuntimeMaterialCoordinator>()
        .equipment
        .metadata
        .insert(identity.to_owned(), metadata.clone());
    // 619380 copies the spawning player's entity clock selector once. Keep
    // that real generation in the canonical clock owner, so later managed
    // overrides need not be guessed from the currently selected hand/player.
    {
        let mut clock = world
            .get_resource_mut::<crate::gameplay::game_clock_runtime::ClockRuntime>()
            .ok_or("equipment has no canonical entity clock")?;
        let timespace = clock.player_timespace().map_err(str::to_owned)?;
        clock.register_entity_time(entity, timespace)?;
    }
    if metadata.sound_settings.is_some() {
        let part = match metadata.equipment.slot {
            1 => 8,
            0 => 16,
            slot => {
                return Err(format!(
                    "equipment audio has no player cast for slot {slot}"
                ));
            }
        };
        super::super::equipment_identity::bind_audio(world, entity, part)?;
    }
    if let Some(definition) = &metadata.heart {
        super::super::heart::prepare(world, entity, definition.clone())?;
    }
    if let Some(definition) = &metadata.heart_barks {
        super::super::heart::prepare_barks(world, entity, definition.clone())?;
    }
    Ok(entity)
}

/// 6170b0/619380 and ranged639f00 initialize the actual owned entity before
/// selection. Only its default selected ammo child starts, not all variants.
fn retain_owned(
    world: &mut World,
    identity: &str,
    descriptor: Arc<Descriptor>,
    player: &mut crate::gameplay::player::Player,
    inventory: &mut super::super::interaction_focus::DiscoveredDocuments,
) -> Result<EntityId, String> {
    let entity = retain_metadata(world, identity, descriptor.metadata.clone())?;
    let existed = world
        .resource::<RuntimeMaterialCoordinator>()
        .equipment
        .owned
        .contains_key(identity);
    world
        .resource_mut::<RuntimeMaterialCoordinator>()
        .equipment
        .owned
        .insert(identity.to_owned(), descriptor.clone());
    request(world, entity, descriptor.clone())?;
    if !existed {
        super::super::melee::request_owned(world, Arc::from(identity), descriptor.clone())?;
        if let Some(definition) = &descriptor.throwable {
            for ammunition in &definition.ammunition {
                inventory.bind_ammunition(identity, &ammunition.item);
            }
            player.register_throwable(identity, entity, definition.clone(), inventory)?;
            super::super::weapon_throwing::request_owned(world, Arc::from(identity), descriptor.clone())?;
        }
        world
            .resource_mut::<RuntimeMaterialCoordinator>()
            .equipment
            .upgrade_starts
            .push((entity, descriptor.clone()));
        if let Some(settings) = &descriptor.ranged {
            super::super::player_actions::register_ranged(
                player,
                identity,
                Arc::new(settings.clone()),
                inventory,
            )?;
            player.bind_ranged_entity(identity, entity)?;
            super::super::weapon_firing::request_owned(world, Arc::from(identity), descriptor.clone())?;
            let component = player
                .registered_ranged(identity)
                .ok_or("owned ranged Start did not retain its runtime")?;
            let ammo = component.ammunition();
            world
                .resource_mut::<RuntimeMaterialCoordinator>()
                .equipment
                .ammo_starts
                .push((
                    entity,
                    descriptor.clone(),
                    ammo.item.clone(),
                    ammo.attachment.clone(),
                    settings.projectile_attachment_tag.clone(),
                ));
        }
    }
    world
        .resource_mut::<RuntimeMaterialCoordinator>()
        .equipment
        .owned
        .insert(identity.to_owned(), descriptor);
    Ok(entity)
}

/// Selection/metadata arbitration remains PlayerActions' input owner. This
/// adapter publishes prepared banks/targets, then commits quantity, sound,
/// ranged and render lifetime changes only from actual equipment callbacks.
pub(in crate::app) fn advance(world: &mut World) {
    use super::super::{
        interaction_focus::DiscoveredDocuments, player::PlayerState, player_actions::PlayerActions,
    };
    let Some(mut actions) = world.remove_resource::<PlayerActions>() else {
        return;
    };
    let Some(mut state) = world.remove_resource::<PlayerState>() else {
        world.insert_resource(actions);
        return;
    };
    let Some(mut inventory) = world.remove_resource::<DiscoveredDocuments>() else {
        world.insert_resource(state);
        world.insert_resource(actions);
        return;
    };
    if let Some(player) = state.player.as_mut() {
        for (identity, metadata) in std::mem::take(&mut actions.prepared_owned_metadata) {
            if inventory.quantity(&identity) == 0 {
                continue;
            }
            match retain_metadata(world, &identity, metadata.clone()) {
                Ok(_) => {
                    if actions.take_equipment_admission(&identity) {
                        if inventory.equipment_available(&identity, &metadata.equipment) {
                            world
                                .resource_mut::<RuntimeMaterialCoordinator>()
                                .equipment
                                .auto_admissions = true;
                        }
                    }
                }
                Err(error) => {
                    world
                        .resource_mut::<RuntimeMaterialCoordinator>()
                        .equipment
                        .load_error
                        .get_or_insert(error.clone());
                    bevy::log::warn!("Equipment {identity} identity admission failed: {error}");
                }
            }
        }

        // 619760 owns the constructor's first-frame instant flag; the host
        // supplies no guessed flag while arbitrating actual admitted actors.
        let arbitrate = world
            .resource::<RuntimeMaterialCoordinator>()
            .equipment
            .auto_admissions
            && !actions.metadata_admissions_pending();
        if arbitrate {
            inventory.arbitrate_auto_equip();
            world
                .resource_mut::<RuntimeMaterialCoordinator>()
                .equipment
                .auto_admissions = false;
        }

        let mut removals = std::mem::take(&mut actions.removed_equipment);
        for identity in world.resource::<RuntimeMaterialCoordinator>().equipment.stopping.keys() {
            if !removals.contains(identity) { removals.push(identity.clone()); }
        }
        for path in removals {
            //Collect already accepted runtime messages before removing the
            //runtime. Normalpublish drains them after PlayerState is restored.
            if let Err(error) = super::super::weapon_firing::begin_retirement(world, player, &path) {
                actions.removed_equipment.push(path);
                world.resource_mut::<RuntimeMaterialCoordinator>().equipment.load_error.get_or_insert(error);
                continue;
            }
            //Remove the player's presentation immediately, as before. The
            //retire_owned fence below retains the actual actor/children and
            //prepared scopes through accepted callbacks and their tails.
            player.retire_equipment(&path);
            player.retire_ranged(&path);
            player.retire_throwable(&path);
            for side in 0..2 {
                let removed = world
                    .resource::<RuntimeMaterialCoordinator>()
                    .equipment
                    .selected
                    .get(&side)
                    .is_some_and(|selected| selected == &path);
                if removed {
                    actions.remove_active(side);
                    world
                        .resource_mut::<RuntimeMaterialCoordinator>()
                        .equipment
                        .selected
                        .remove(&side);
                }
            }
            if let Err(error) = retire_owned(world, &path) {
                // Preserve the Stop request and source leases for a later retry.
                actions.removed_equipment.push(path);
                world
                    .resource_mut::<RuntimeMaterialCoordinator>()
                    .equipment
                    .load_error
                    .get_or_insert(error.clone());
                bevy::log::warn!("Equipment Stop failed: {error}");
            }
        }
        for target in std::mem::take(&mut actions.power_targets) {
            let identity = target.identity.clone();
            let slot = target.equipment.slot;
            if inventory.quantity(&identity) == 0 {
                actions.cancel_power_selection(&identity);
                continue;
            }
            let changed_pose = target.tree.is_some();
            match player.register_equipment_target(target) {
                Ok(_) => {
                    if changed_pose {
                        world
                            .resource_mut::<RuntimeMaterialCoordinator>()
                            .equipment
                            .pose_dirty = true;
                    }
                    if actions.wanted(slot, &identity) {
                        if let Some(flags) = actions.request_flags(&identity) {
                            if let Err(error) =
                                player.request_equipment_selection_by_identity(&identity, flags)
                            {
                                bevy::log::warn!("Power {identity} selection failed: {error}");
                            }
                        }
                    }
                }
                Err(error) => {
                    actions.cancel_power_selection(&identity);
                    world
                        .resource_mut::<RuntimeMaterialCoordinator>()
                        .equipment
                        .load_error
                        .get_or_insert(error.clone());
                    bevy::log::warn!("Power {identity} registration failed: {error}");
                }
            }
        }
        for (identity, descriptor) in std::mem::take(&mut actions.prepared_owned_equipment) {
            if inventory.quantity(&identity) == 0 {
                continue;
            }
            if let Err(error) = retain_owned(world, &identity, descriptor, player, &mut inventory) {
                world
                    .resource_mut::<RuntimeMaterialCoordinator>()
                    .equipment
                    .load_error
                    .get_or_insert(error.clone());
                bevy::log::warn!("Owned equipment {identity} Start failed: {error}");
            }
        }
        for mut ready in actions.take_ready() {
            if inventory.quantity(&ready.item.path) > 0 {
                if let Err(error) = retain_owned(
                    world,
                    &ready.item.path,
                    ready.descriptor.clone(),
                    player,
                    &mut inventory,
                ) {
                    world
                        .resource_mut::<RuntimeMaterialCoordinator>()
                        .equipment
                        .load_error
                        .get_or_insert(error.clone());
                    bevy::log::warn!("Owned equipment {} Start failed: {error}", ready.item.path);
                    if let Err(error)=ready.cancel_animation_audio(world){record_error(world,error);}
                    continue;
                }
            }
            if !ready.can_apply(player) {
                actions.requeue(ready);
                continue;
            }
            if !ready.is_current(player) {
                if let Err(error)=ready.cancel_animation_audio(world){record_error(world,error);}
                if !actions
                    .active(ready.side())
                    .is_some_and(|active| Arc::ptr_eq(active, &ready.descriptor))
                {
                    if inventory.quantity(&ready.item.path) == 0 {
                        withdraw(world, &ready.item.path);
                    }
                }
                if let Err(error) = actions.request(ready.item, player) {
                    world
                        .resource_mut::<RuntimeMaterialCoordinator>()
                        .equipment
                        .load_error
                        .get_or_insert(error.clone());
                    bevy::log::warn!("Equipment reacquisition failed: {error}");
                }
                continue;
            }
            let identity = ready.item.path.to_string();
            if inventory.quantity(&identity) == 0 {
                if let Err(error)=ready.cancel_animation_audio(world){record_error(world,error);}
                withdraw(world, &ready.item.path);
                continue;
            }
            let Some(entity) = super::super::equipment_identity::id(world, &identity) else {
                actions.requeue(ready);
                continue;
            };
            if let Some(error) = failure(world, entity) {
                if let Err(error)=ready.cancel_animation_audio(world){record_error(world,error);}
                bevy::log::warn!("Equipment {identity} material admission failed: {error}");
                withdraw(world, &ready.item.path);
                continue;
            }
            if ready.descriptor.visual.is_some() && !admitted(world, entity) {
                match request(world, entity, ready.descriptor.clone()) {
                    Ok(()) => actions.requeue(ready),
                    Err(error) => {
                        world
                            .resource_mut::<RuntimeMaterialCoordinator>()
                            .equipment
                            .load_error
                            .get_or_insert(error.clone());
                        bevy::log::warn!("Equipment {identity} admission failed: {error}");
                        if let Err(error)=ready.cancel_animation_audio(world){record_error(world,error);}
                    }
                }
                continue;
            }
            let descriptor = ready.descriptor.clone();
            let dependencies_ready = (|| {
                if !ready.prepare_animation_audio(world,entity)?{return Ok(false)}
                if descriptor.throwable.is_some() {
                    if let Some(item) = actions.selected_ammunition.get(&identity).and_then(Option::as_deref) {
                        super::super::weapon_throwing::request_ammunition(world, &identity, item)?;
                    }
                }
                if descriptor.heart.is_some() && !super::super::heart::ready(world, entity)? { return Ok(false); }
                if descriptor.melee.is_some() && !super::super::melee::ready_owned(world, &identity)? { return Ok(false); }
                if descriptor.throwable.is_some() && !super::super::weapon_throwing::ready_owned(world, &identity)? { return Ok(false); }
                Ok::<_, String>(true)
            })();
            match dependencies_ready {
                Ok(true) => {},
                Ok(false) => { actions.requeue(ready); continue; },
                Err(error) => {
                    if let Err(release)=ready.cancel_animation_audio(world){record_error(world,release);}
                    record_error(world, error); continue;
                },
            }
            let slot = descriptor.equipment.slot;
            actions.animation_audio.transfer(ready.take_animation_audio());
            match ready.stage(player) {
                Ok(mut target) => {
                    if let Some(tree) = target.tree.as_mut() {
                        tree.managed_entity =
                            super::super::equipment_identity::id(world, &identity);
                    }
                    match player.register_equipment_target(target) {
                        Ok(_) => {
                            world
                                .resource_mut::<RuntimeMaterialCoordinator>()
                                .equipment
                                .pose_dirty = true;
                            if actions.wanted(slot, &identity) {
                                if let Some(flags) = actions.request_flags(&identity) {
                                    match player
                                        .request_equipment_selection_by_identity(&identity, flags)
                                    {
                                        Ok(_) => {
                                            world
                                                .resource_mut::<RuntimeMaterialCoordinator>()
                                                .equipment
                                                .staged
                                                .insert(identity, descriptor);
                                        }
                                        Err(error) => bevy::log::warn!(
                                            "Equipment {identity} selection failed: {error}"
                                        ),
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            world
                                .resource_mut::<RuntimeMaterialCoordinator>()
                                .equipment
                                .load_error
                                .get_or_insert(error.clone());
                            bevy::log::warn!("Equipment {identity} registration failed: {error}")
                        }
                    }
                }
                Err(error) => {
                    world
                        .resource_mut::<RuntimeMaterialCoordinator>()
                        .equipment
                        .load_error
                        .get_or_insert(error.clone());
                    bevy::log::warn!("Equipment {identity} bank publication failed: {error}")
                }
            }
            if let Some(id)=super::super::player_lifetime::id(world){
                if let Err(error)=actions.animation_audio.prepare(world,id,player){record_error(world,error);}
            }
        }
    }
    world.insert_resource(inventory);
    world.insert_resource(state);
    world.insert_resource(actions);
    refresh_loading_pose(world);
    if let Err(error) = publish_entity_frames(world) {
        record_error(world, error);
    }
    let upgrades = std::mem::take(
        &mut world
            .resource_mut::<RuntimeMaterialCoordinator>()
            .equipment
            .upgrade_starts,
    );
    for (entity, descriptor) in upgrades {
        let result = super::super::equipment_attachment_admission::default_upgrades(
            world,
            entity,
            descriptor.clone(),
            &descriptor.equipment.default_upgrades,
        );
        if let Err(error) = result {
            record_error(world, error);
        }
    }
    let starts = std::mem::take(
        &mut world
            .resource_mut::<RuntimeMaterialCoordinator>()
            .equipment
            .ammo_starts,
    );
    for (entity, descriptor, ammunition, attachment, tag) in starts {
        if let Err(error) = super::super::equipment_attachment_admission::select(
            world,
            entity,
            descriptor,
            &ammunition,
            attachment.as_deref(),
            tag.as_deref(),
            true,
        ) {
            world
                .resource_mut::<RuntimeMaterialCoordinator>()
                .equipment
                .load_error
                .get_or_insert(error.clone());
            bevy::log::warn!("Owned ammunition child Start failed: {error}");
        }
    }
    if let Err(error) = super::super::equipment_identity::poll_retirements(world) {
        world
            .resource_mut::<RuntimeMaterialCoordinator>()
            .equipment
            .load_error
            .get_or_insert(error.clone());
        bevy::log::warn!("Equipment retirement failed: {error}");
    }
    if let Err(error) = super::super::equipment_attachment_admission::advance(world) {
        world
            .resource_mut::<RuntimeMaterialCoordinator>()
            .equipment
            .load_error
            .get_or_insert(error.clone());
        bevy::log::warn!("Ammunition attachment admission failed: {error}");
    }
}

/// Consume actual equipment callbacks immediately after Player::frame, before
/// firing/power execution. This performs no source/material preparation.
pub(in crate::app) fn publish_callbacks(world: &mut World) {
    use super::super::{
        interaction_focus::DiscoveredDocuments,
        player::PlayerState,
        player_actions::{self, PlayerActions},
        powers::Powers,
    };
    use crate::gameplay::player::equipment_states::Callback;
    let Some(mut actions) = world.remove_resource::<PlayerActions>() else {
        return;
    };
    let Some(mut state) = world.remove_resource::<PlayerState>() else {
        world.insert_resource(actions);
        return;
    };
    let Some(mut inventory) = world.remove_resource::<DiscoveredDocuments>() else {
        world.insert_resource(state);
        world.insert_resource(actions);
        return;
    };
    let mut powers = world.remove_resource::<Powers>();
    if let Some(player) = state.player.as_mut() {
        for callback in player.take_equipment_callbacks() {
            match callback {
                Callback::Equipped { identity, side } => {
                    world
                        .resource_mut::<RuntimeMaterialCoordinator>()
                        .equipment
                        .selected
                        .insert(side, identity.clone());
                    if let Some(descriptor) = take_staged(world, &identity) {
                        if side == 0 {
                            if let Some(settings) = &descriptor.ranged {
                                if let Err(error) = player_actions::install_ranged(
                                    player,
                                    &identity,
                                    Arc::new(settings.clone()),
                                    actions
                                        .selected_ammunition
                                        .get(&identity)
                                        .and_then(Option::as_deref),
                                    &mut inventory,
                                ) {
                                    bevy::log::warn!(
                                        "Equipment {identity} ranged activation failed: {error}"
                                    );
                                }
                            } else {
                                player.unequip_ranged();
                            }
                            if descriptor.throwable.is_some() {
                                if let Some(item) = actions.selected_ammunition.get(&identity).and_then(Option::as_deref) {
                                    if let Err(error) = player.select_throwable_ammunition(&identity, item, &inventory) {
                                        bevy::log::error!("Equipment {identity} ammunition selection failed: {error}");
                                    }
                                }
                                if let Err(error) = player.equip_throwable(&identity) {
                                    bevy::log::error!("Equipment {identity} throwable activation failed: {error}");
                                }
                            } else { player.unequip_throwable(); }
                        }
                        if descriptor.heart.is_some() {
                            if let Some(id) = super::super::equipment_identity::id(world, &identity) {
                                player.bind_heart_entity(Some(id));
                                if let Err(error) = super::super::heart::equip(world, id) {
                                    bevy::log::error!("Heart {identity} activation failed: {error}");
                                }
                            }
                            player.equip_heart_state(true);
                        }
                        inventory.equip(&identity, descriptor.equipment.slot, false);
                        player.set_equipped_archetype(side, Some(descriptor.equipment.archetype));
                        actions.accepted(side, descriptor);
                    } else {
                        match actions.activate_power(&identity, player) {
                            Ok(Some((id, equipment))) => {
                                inventory.equip(&identity, equipment.slot, true);
                                player.set_equipped_archetype(side, Some(equipment.archetype));
                                if let Some(powers) = powers.as_mut() {
                                    powers.select(id, &inventory);
                                }
                            }
                            Ok(None) => bevy::log::warn!(
                                "Equipment callback has no prepared target: {identity}"
                            ),
                            Err(error) => {
                                bevy::log::warn!("Power {identity} activation failed: {error}")
                            }
                        }
                    }
                }
                Callback::Unequipped { identity, side } => {
                    let selected = world
                        .resource::<RuntimeMaterialCoordinator>()
                        .equipment
                        .selected
                        .get(&side)
                        .is_some_and(|selected| selected == &identity);
                    if selected {
                        world
                            .resource_mut::<RuntimeMaterialCoordinator>()
                            .equipment
                            .selected
                            .remove(&side);
                    }
                    if let Some(descriptor) =
                        selected.then(|| actions.remove_active(side)).flatten()
                    {
                        if descriptor.throwable.is_some() { player.unequip_throwable(); }
                        if descriptor.heart.is_some() {
                            player.bind_heart_entity(None);
                            if let Some(id) = super::super::equipment_identity::id(world, &identity) {
                                if let Err(error) = super::super::heart::unequip(world, id) {
                                    bevy::log::error!("Heart {identity} deactivation failed: {error}");
                                }
                            }
                            player.equip_heart_state(false);
                        }
                        if descriptor.ranged.is_some() {
                            if inventory.quantity(&identity) == 0 {
                                player.retire_ranged(&identity);
                            } else {
                                player.unequip_ranged();
                            }
                        }
                        if inventory.equipped(descriptor.equipment.slot) == Some(identity.as_str())
                        {
                            inventory.unequip(descriptor.equipment.slot);
                        }
                        if inventory.quantity(&identity) == 0 {
                            withdraw(world, &identity);
                        }
                    }
                    let slot = match side {
                        0 => Some(1),
                        1 => Some(0),
                        _ => None,
                    };
                    if let Some(slot) = slot {
                        if inventory.equipped(slot) == Some(identity.as_str()) {
                            inventory.unequip(slot);
                        }
                    }
                    if actions.deactivate_power(&identity, player) {
                        if let Some(powers) = powers.as_mut() {
                            powers.clear_selected();
                        }
                    }
                    if selected {
                        player.set_equipped_archetype(side, None);
                    }
                }
                Callback::Cancelled { identity, side } => {
                    if let Some(descriptor) = take_staged(world, &identity) {
                        if !actions
                            .active(side)
                            .is_some_and(|active| Arc::ptr_eq(active, &descriptor))
                        {
                            if inventory.quantity(&identity) == 0 {
                                withdraw(world, &identity);
                            }
                        }
                    }
                    actions.cancel_power_selection(&identity);
                }
            }
        }
    }
    if let Some(powers) = powers {
        world.insert_resource(powers);
    }
    world.insert_resource(inventory);
    world.insert_resource(state);
    world.insert_resource(actions);
    refresh_loading_pose(world);
    if let Err(error) = publish_entity_frames(world) {
        record_error(world, error);
    }
}

fn record_error(world: &mut World, error: String) {
    if let Some(mut owner) = world.get_resource_mut::<RuntimeMaterialCoordinator>() {
        owner.equipment.load_error.get_or_insert(error.clone());
    }
    bevy::log::warn!("Equipment publication failed: {error}");
}

/// Loading pauses gameplay Think, but newly registered owned trees still need
/// their actual body/arm binding pose before child source admission can finish.
/// Sample only after an admitted target changed; never advance physics/time.
fn refresh_loading_pose(world: &mut World) {
    let dirty = world
        .get_resource::<RuntimeMaterialCoordinator>()
        .is_some_and(|owner| owner.equipment.pose_dirty);
    let paused = world
        .get_resource::<crate::gameplay::game_clock_runtime::GamePaused>()
        .is_some_and(|paused| paused.0);
    if !dirty || !paused {
        return;
    }
    let refreshed = world
        .get_resource_mut::<super::super::player::PlayerState>()
        .and_then(|mut state| {
            state.player.as_mut().map(|player| {
                player.view();
            })
        })
        .is_some();
    if refreshed {
        world
            .resource_mut::<RuntimeMaterialCoordinator>()
            .equipment
            .pose_dirty = false;
    }
}

/// Publish the actual equipment entity transform before its rig root offset.
/// Its managed entity clock retains the selector copied during619380 spawn,
/// including any subsequent override applied through the canonical clock owner.
fn publish_entity_frames(world: &mut World) -> Result<(), String> {
    use crate::gameplay::{
        entity_lifetime::LiveEntities, game_clock_runtime::ClockRuntime,
        reference_frame::ReferenceFrame,
    };
    let Some(state) = world.remove_resource::<super::super::player::PlayerState>() else {
        return Ok(());
    };
    let mut failure = None;
    if let Some(player) = state.player.as_ref() {
        player.visit_equipment_entity_frames(|id, position, basis| {
            // A withdrawn generation may remain in the previous render cache
            // until the next view sample. It must not receive fresh publication.
            let linked = world
                .get_resource::<LiveEntities>()
                .is_some_and(|live| live.parent(id).ok().flatten().is_some());
            if !linked || failure.is_some() {
                return;
            }
            let rate = match world
                .get_resource::<ClockRuntime>()
                .ok_or("equipment clock owner absent")
                .and_then(|clock| clock.entity_rates(id))
            {
                Ok(rates) => rates[1],
                Err(error) => {
                    failure = Some(error.to_owned());
                    return;
                }
            };
            if let Err(error) = super::super::equipment_identity::publish_frame_using(
                world,
                id,
                ReferenceFrame {
                    position: position.to_array(),
                    basis: basis.to_cols_array_2d(),
                },
                rate,
                Some(player),
            ) {
                failure = Some(error);
            }
        });
    }
    world.insert_resource(state);
    failure.map_or(Ok(()), Err)
}
