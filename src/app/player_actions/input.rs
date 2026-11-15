//! Native inventory shortcut and immediate player-action dispatch.
use super::*;

/// Single consumer of native equipment requests. Metadata demand and key
/// shortcuts never ask the model/material owners to preload an arsenal.
/// Schedule before equipment_admission::advance and the next player frame.
pub(in crate::app) fn advance_selection(world: &mut World) {
    let Some(mut actions) = world.remove_resource::<PlayerActions>() else {
        return;
    };
    let Some(mut inventory) = world.remove_resource::<DiscoveredDocuments>() else {
        world.insert_resource(actions);
        return;
    };
    let Some(mut state) = world.remove_resource::<super::super::player::PlayerState>() else {
        world.insert_resource(inventory);
        world.insert_resource(actions);
        return;
    };
    actions.configure_shortcuts(&mut inventory);
    if let Some(player) = state.player.as_mut() {
        actions.configure_controller(player);
    }
    if !actions.rehydrated && inventory.is_initialized() {
        // Inventory belongs to the session, while its actual spawned entity
        // and model belong to this loaded world. Reacquire those leases
        // without replaying additions, quantities, modifiers or autoequip.
        for item in inventory.owned_equipment_items() {
            actions.request_metadata(item, false);
        }
        for slot in 0..16 {
            if let Some(path) = inventory.equipped(slot) {
                // Host session-to-world handoff requests the retained
                // selection through the same real controller entry; it
                // neither forces an instant animation nor replays a pickup.
                if let Some(player) = state.player.as_ref() {
                    actions
                        .source_flags
                        .insert(path.to_owned(), player.shortcut_equipment_flags());
                }
                let entity = inventory
                    .declaration(path)
                    .and_then(|item| item.equipment.clone());
                actions.selection_requests.push(ShortcutSelection {
                    equipment: path.to_owned(),
                    ammunition: None,
                    equipment_entity: entity,
                });
            }
        }
        actions.rehydrated = true;
    }
    use super::super::interaction_focus::InventoryChange;
    for change in inventory.take_equipment_changes() {
        match change {
            InventoryChange::EquipmentNeeded { path } => {
                if let Some(item) = inventory.declaration(&path) {
                    actions.request_metadata(item, true);
                }
            }
            InventoryChange::EquipmentRemoved { path } => {
                actions.metadata.remove(&path);
                actions.equipment_admissions.remove(&path);
                actions.metadata_failed.remove(&path);
                actions.owned_prepared.remove(&path);
                actions.owned_targets.retain(|owned| owned != &path);
                actions.failed_targets.remove(&path);
                actions.power_activations.remove(&path);
                if let Some(reservation) = actions.power_reservations.remove(&path) {
                    let id = actions
                        .power_dependency_ids
                        .remove(&path)
                        .expect("owned power reservation has an ID");
                    actions
                        .retiring_power_dependencies
                        .push(PowerDependencyRetirement {
                            id,
                            reservation,
                            sources_released: false,
                        });
                }
                if let Some(pending) = actions.owned_loading.get(&path) {
                    pending.cancelled.store(true, Ordering::Release);
                }
                actions.shortcut_items.retain(|item| item != &path);
                if let Some(pending) = actions.metadata_loading.get(&path) {
                    pending.cancelled.store(true, Ordering::Release);
                }
                if let Some(pending) = actions.loading.get(&path) {
                    pending.cancelled.store(true, Ordering::Release);
                }
                if let Some(pending) = actions.power_loading.get(&path) {
                    pending.cancelled.store(true, Ordering::Release);
                }
                actions.removed_equipment.push(path);
            }
            InventoryChange::AutoEquipRequested { path, instant } => {
                let entity = inventory
                    .declaration(&path)
                    .and_then(|item| item.equipment.clone());
                // 619760's message9d carries bit2; 7f8d60 maps it to
                // switch bit100, with instant selecting30 instead of20.
                actions
                    .source_flags
                    .insert(path.clone(), if instant { 0x130 } else { 0x120 });
                actions.selection_requests.push(ShortcutSelection {
                    equipment: path,
                    ammunition: None,
                    equipment_entity: entity,
                });
            }
            InventoryChange::Unequipped { path, .. } => {
                actions.removed_equipment.push(path);
            }
            InventoryChange::Equipped { .. } => {} // Produced by the admitted live component transaction.
            _ => unreachable!("filtered equipment message"),
        }
    }
    if let Some(player) = state.player.as_mut() {
        if let Err(error) = retire_power_dependencies(&mut actions, player, world) {
            eprintln!("Power dependency retirement: {error}");
            actions.startup_error.get_or_insert(error);
            world.insert_resource(state);
            world.insert_resource(inventory);
            world.insert_resource(actions);
            return;
        }
    }
    actions
        .shortcut_items
        .extend(inventory.take_shortcut_items());
    actions.finish_metadata(&mut inventory);
    actions.finish_owned(&inventory);
    if let Some(player) = state.player.as_mut() {
        // Actual owned entities are prepared in source order, one live clip
        // bank transaction at a time. Selection then reuses these entities.
        while let Some(path) = actions.owned_targets.front() {
            if player.has_equipment_target(path)
                || inventory.quantity(path) == 0
                || actions.failed_targets.contains(path)
            {
                actions.owned_targets.pop_front();
                continue;
            }
            if !actions.loading.is_empty()
                || !actions.power_loading.is_empty()
                || !actions.ready.is_empty()
                || !actions.power_ready.is_empty()
                || !actions.power_targets.is_empty()
            {
                break;
            }
            let path = path.clone();
            let (Some(item), Some(metadata)) = (
                inventory.declaration(&path).cloned(),
                actions.metadata.get(&path).cloned(),
            ) else {
                break;
            };
            let result = if metadata.equipment.archetype == 5 {
                let definitions = world
                    .get_resource::<super::super::powers::Powers>()
                    .map(|powers| powers.definitions.clone());
                if let Some((definitions, index)) = definitions.and_then(|definitions| {
                    definitions
                        .0
                        .iter()
                        .position(|power| power.item.eq_ignore_ascii_case(&path))
                        .map(|index| (definitions, index))
                }) {
                    actions.request_power(
                        item,
                        definitions,
                        index,
                        metadata.equipment.clone(),
                        player,
                    )
                } else {
                    Err(format!(
                        "owned power {path} is absent from the player's declared power system"
                    ))
                }
            } else {
                actions.request(item, player)
            };
            if let Err(error) = result {
                eprintln!("Owned equipment actions {path}: {error}");
                actions
                    .startup_error
                    .get_or_insert_with(|| format!("Owned equipment actions {path}: {error}"));
                actions.failed_targets.insert(path);
            }
            break;
        }
        let mut waiting = Vec::new();
        for selection in std::mem::take(&mut actions.selection_requests) {
            let Some(item) = inventory.declaration(&selection.equipment).cloned() else {
                continue;
            };
            if inventory.quantity(&selection.equipment) == 0 {
                continue;
            }
            let Some(metadata) = actions.metadata.get(&selection.equipment).cloned() else {
                if actions.metadata_failed.contains(&selection.equipment) {
                    continue;
                }
                actions.request_metadata(&item, false);
                waiting.push(selection);
                continue;
            };
            let slot = metadata.equipment.slot;
            let live = if metadata.equipment.archetype == 5 {
                actions.active_power.as_deref() == Some(selection.equipment.as_str())
            } else {
                let side = match slot {
                    1 => Some(0),
                    0 => Some(1),
                    _ => None,
                };
                side.is_some_and(|side| {
                    actions
                        .active(side)
                        .is_some_and(|active| active.entity == metadata.entity)
                })
            };
            if live && inventory.equipped(slot) == Some(selection.equipment.as_str()) {
                if let Some(ranged) = &metadata.ranged {
                    if let Err(error) = install_ranged(
                        player,
                        &selection.equipment,
                        Arc::new(ranged.clone()),
                        selection.ammunition.as_deref(),
                        &mut inventory,
                    ) {
                        eprintln!("Ammunition selection {}: {error}", selection.equipment);
                    }
                }
                actions
                    .selected_ammunition
                    .insert(selection.equipment, selection.ammunition);
                continue;
            }
            actions.wanted[slot as usize] = Some(selection.equipment.clone());
            actions
                .selected_ammunition
                .insert(selection.equipment.clone(), selection.ammunition.clone());
            if player.has_equipment_target(&selection.equipment) {
                if let Some(flags) = actions.request_flags(&selection.equipment) {
                    if let Err(error) =
                        player.request_equipment_selection_by_identity(&selection.equipment, flags)
                    {
                        eprintln!("Equipment selection {}: {error}", selection.equipment);
                    }
                } else {
                    waiting.push(selection);
                }
                continue;
            }
            if actions.failed_targets.contains(&selection.equipment) {
                continue;
            }
            if !actions.loading.is_empty()
                || !actions.power_loading.is_empty()
                || !actions.ready.is_empty()
                || !actions.power_ready.is_empty()
                || !actions.power_targets.is_empty()
            {
                waiting.push(selection);
                continue;
            }
            if metadata.equipment.archetype == 5 {
                let definitions = world
                    .get_resource::<super::super::powers::Powers>()
                    .map(|powers| powers.definitions.clone());
                let Some(definitions) = definitions else {
                    waiting.push(selection);
                    continue;
                };
                let Some(index) = definitions
                    .0
                    .iter()
                    .position(|power| power.item.eq_ignore_ascii_case(&selection.equipment))
                else {
                    continue;
                };
                if let Err(error) = actions.request_power(
                    item,
                    definitions,
                    index,
                    metadata.equipment.clone(),
                    player,
                ) {
                    eprintln!("Power selection {}: {error}", selection.equipment);
                }
            } else if let Err(error) = actions.request(item, player) {
                eprintln!("Equipment selection {}: {error}", selection.equipment);
            }
        }
        actions.selection_requests.extend(waiting);
        for mut ready in actions.take_ready_powers() {
            if inventory.quantity(&ready.item.path) == 0 {
                continue;
            }
            if !ready.can_apply(player) {
                actions.requeue_power(ready);
                continue;
            }
            if !ready.is_current(player) {
                let entity = ready.item.equipment.clone();
                actions.selection_requests.push(ShortcutSelection {
                    equipment: ready.item.path.to_string(),
                    ammunition: None,
                    equipment_entity: entity,
                });
                continue;
            }
            match ready.publish_dependencies(world) {
                Ok(true) => {}
                Ok(false) => {
                    actions.requeue_power(ready);
                    continue;
                }
                Err(error) => {
                    let path = ready.item.path.to_string();
                    eprintln!("Owned power dependencies {path}: {error}");
                    actions
                        .startup_error
                        .get_or_insert_with(|| format!("Owned power dependencies {path}: {error}"));
                    actions.failed_targets.insert(path);
                    continue;
                }
            }
            let path = ready.item.path.to_string();
            match ready.stage(player) {
                Ok((target, activation)) => {
                    actions.power_activations.insert(path, activation);
                    actions.power_targets.push(target);
                }
                Err(error) => eprintln!("Power selection {path}: {error}"),
            }
        }
    }
    world.insert_resource(state);
    world.insert_resource(inventory);
    world.insert_resource(actions);
}

fn retire_power_dependencies(
    actions: &mut PlayerActions,
    player: &mut Player,
    world: &mut World,
) -> Result<(), String> {
    if actions.retiring_power_dependencies.is_empty() {
        return Ok(());
    }
    let blink_active = actions
        .blink
        .as_ref()
        .is_some_and(|blink| blink.has_activity())
        || actions
            .active_blink
            .as_ref()
            .is_some_and(|blink| blink.has_activity());
    let far_reach_active = actions
        .far_reach
        .as_ref()
        .is_some_and(|runtime| runtime.has_activity())
        || actions
            .active_far_reach
            .as_ref()
            .is_some_and(|runtime| runtime.has_activity());
    let mut failure = None;
    actions.retiring_power_dependencies.retain_mut(|retired| {
        if failure.is_some() { return true; }
        let result = (|| -> Result<bool, String> {
            if !retired.sources_released {
                // Stop the old generation before a newly admitted copy can
                // absorb sources into this same family owner.
                match retired.id {
                    1 => {
                        if !world.contains_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
                            || !world.contains_resource::<crate::gameplay::entity_events::EntityEvents>() {
                            return Err("Bend Time source retirement lost its clock/event owners".into());
                        }
                        world.resource_scope(|world, mut clock: Mut<crate::gameplay::game_clock_runtime::ClockRuntime>| {
                            world.resource_scope(|world, mut events: Mut<crate::gameplay::entity_events::EntityEvents>| {
                                world.get_resource_mut::<super::super::power_world::PowerWorld>()
                                    .ok_or("Bend Time source owner retired too early")?
                                    .release_owned_sources(&mut clock, &mut events, player.feet())
                            })
                        })?;
                    }
                    10 => world.get_resource_mut::<super::super::windblast::Windblasts>()
                        .ok_or("Windblast source owner retired too early")?.release_owned_sources(),
                    _ => {},
                }
                player.remove_power(retired.id);
                actions.cached_blinks.remove(&retired.reservation);
                actions.cached_far_reaches.remove(&retired.reservation);
                retired.sources_released = true;
            }
            let pending = match retired.id {
                2 => blink_active,
                6 => far_reach_active,
                10 => world.get_resource::<super::super::windblast::Windblasts>().is_some_and(|owner| owner.has_active()),
                _ => false,
            };
            if pending { return Ok(true); }
            world.get_resource_mut::<super::super::usable_effects::EffectRequests>()
                .ok_or("power FX reservation owner retired too early")?
                .release_reservation(retired.reservation.clone());
            Ok(false)
        })();
        match result { Ok(pending) => pending, Err(error) => { failure = Some(error); true } }
    });
    failure.map_or(Ok(()), Err)
}

pub(in crate::app) fn dispatch_shortcuts(
    player: &Player,
    events: &[player::controller::ActionEvent],
    inventory: &mut DiscoveredDocuments,
    actions: &mut PlayerActions,
) {
    actions.configure_shortcuts(inventory);
    for event in events {
        if event.flags & 3 != 1 || !matches!(event.action, 23..=34 | 37..=40) {
            continue;
        }
        if let Some(selection) = inventory.shortcut_selection(event.action) {
            actions.source_flags.insert(
                selection.equipment.clone(),
                player.shortcut_equipment_flags(),
            );
            actions.selection_requests.push(selection);
        }
    }
}

struct Ammunition<'a>(&'a mut DiscoveredDocuments);
impl player::weapons::AmmunitionInventory for Ammunition<'_> {
    fn quantity(&self, item: &str) -> u32 {
        self.0.quantity(item)
    }
    fn consume(&mut self, item: &str, count: u32) -> u32 {
        self.0.consume(item, count)
    }
}

pub(in crate::app) fn install_ranged(
    player: &mut Player,
    identity: &str,
    settings: Arc<player::weapons::Ranged>,
    ammunition: Option<&str>,
    inventory: &mut DiscoveredDocuments,
) -> Result<(), String> {
    let inventory = Ammunition(inventory);
    player.equip_ranged(identity, settings.clone(), &inventory)?;
    if let Some(item) = ammunition {
        let index = settings
            .ammunition
            .iter()
            .position(|ammo| ammo.item == item)
            .ok_or("shortcut ammunition is not used by the selected weapon")?;
        let capacity = settings.capacity(index, &player.attributes);
        if let Some(weapon) = player.weapon.as_mut() {
            if weapon.ammunition().item != item {
                weapon.select_ammunition(item, capacity, &inventory)?;
            }
        }
    }
    Ok(())
}

pub(in crate::app) fn register_ranged(
    player: &mut Player,
    identity: &str,
    settings: Arc<player::weapons::Ranged>,
    inventory: &mut DiscoveredDocuments,
) -> Result<(), String> {
    player.register_ranged(identity, settings, &Ammunition(inventory))
}

pub(in crate::app) fn player_frame(
    player: &mut Player,
    world: &dyn player::character_proxy::PlayerWorld,
    events: &[player::controller::ActionEvent],
    look: &player::controller::LookSettings,
    dt: f32,
    frame_delta: f32,
    now: f64,
    fov: f32,
    inventory: &mut DiscoveredDocuments,
) {
    player.update_power_inputs(events);
    player.frame_with_inventory(
        world,
        events,
        look,
        dt,
        frame_delta,
        now,
        fov,
        &mut Ammunition(inventory),
    );
}

/// 7194a0/745b20 consume on the bound action's press, never on a draw
/// frame or key repeat. Ownership/admission is rechecked by the player.
pub(in crate::app) fn dispatch_potions(
    player: &mut Player,
    events: &[player::controller::ActionEvent],
    inventory: &mut DiscoveredDocuments,
    actions: &mut PlayerActions,
    difficulty: Option<&crate::gameplay::options::DifficultyManager>,
    preset: i32,
    logic: Option<&crate::gameplay::game_logic::GameLogic>,
    sounds: &mut MessageWriter<super::super::audio::SoundEvent>,
) {
    for event in events {
        if event.flags & 3 != 1 {
            continue;
        }
        let kind = match event.action {
            35 => player::consumables::PotionKind::Health,
            36 => player::consumables::PotionKind::Mana,
            _ => continue,
        };
        apply_potion(
            player, kind, inventory, actions, difficulty, preset, logic, sounds,
        );
    }
}

/// 856160 requests the same 745b20 transaction after failed power admission.
/// Consuming the request never retries or invents a successful cast.
pub(in crate::app) fn dispatch_automatic_potion(
    player: &mut Player,
    inventory: &mut DiscoveredDocuments,
    actions: &mut PlayerActions,
    difficulty: Option<&crate::gameplay::options::DifficultyManager>,
    preset: i32,
    logic: Option<&crate::gameplay::game_logic::GameLogic>,
    sounds: &mut MessageWriter<super::super::audio::SoundEvent>,
) {
    if player.take_power_mana_potion_request() {
        apply_potion(
            player,
            player::consumables::PotionKind::Mana,
            inventory,
            actions,
            difficulty,
            preset,
            logic,
            sounds,
        );
    }
}

fn apply_potion(
    player: &mut Player,
    kind: player::consumables::PotionKind,
    inventory: &mut DiscoveredDocuments,
    actions: &mut PlayerActions,
    difficulty: Option<&crate::gameplay::options::DifficultyManager>,
    preset: i32,
    logic: Option<&crate::gameplay::game_logic::GameLogic>,
    sounds: &mut MessageWriter<super::super::audio::SoundEvent>,
) {
    let settings = player::consumables::PotionSettings {
        health_sound: actions.potion_sounds[0].clone(),
        mana_sound: actions.potion_sounds[1].clone(),
        health_effect: difficulty.map_or(1.0, |manager| manager.health_potion_factor(preset)),
        mana_effect: difficulty.map_or(1.0, |manager| manager.mana_potion_factor(preset)),
        mana_speed: difficulty.map_or(1.0, |manager| manager.mana_heal_factor(preset)),
        suppress_sound: logic.and_then(|logic| logic.current())
            == Some(crate::gameplay::game_logic::StateId(8)),
    };
    match player.use_potion(kind, inventory, &settings) {
        Ok(Some(mut result)) => {
            if let Some(sound) = result.sound.take() {
                sounds.write(super::super::audio::SoundEvent(
                    crate::gameplay::sound::Request::Post {
                        event: sound.id,
                        emitter: crate::gameplay::sound::Emitter::PlayerBody,
                    },
                ));
            }
            actions.potion_results.push(result);
        }
        Ok(None) => {}
        Err(error) => eprintln!("Potion action: {error}"),
    }
}
