//! Cached game-logic state8 and its real player-state82 handoff.
//! The UI worker owns the movie. Inventory, player, clock and audio keep
//! their existing owners; only one temporary open snapshot is shared here.
use super::{
    interaction_focus::{DiscoveredDocuments, ShortcutSelection},
    power_wheel_movie::{Command, Event, Notification, Operation},
};
use crate::gameplay::{
    game_clock_runtime::ClockRuntime,
    game_logic::{GameLogic, StateId},
    player::consumables::PotionKind,
    power_wheel::{Bend, Snapshot},
    powers::world_effects::{TimeLease, TimeRequest},
};
use bevy::{
    ecs::{message::MessageCursor, system::RunSystemOnce},
    input::{
        ButtonState,
        keyboard::KeyboardInput,
        mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel},
    },
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};
use std::sync::{
    Mutex,
    mpsc::{self, Receiver, Sender},
};

#[derive(Resource)]
pub(super) struct State {
    epoch: u64,
    prepared: bool,
    retired: bool,
    clear_requested: bool,
    ready: bool,
    open: bool,
    closing: bool,
    pending_pause: bool,
    in_flight: bool,
    input_owned: bool,
    error: Option<String>,
    lease: Option<TimeLease>,
    bend: Bend,
    snapshot: Option<Snapshot>,
    elapsed: f64,
    interval: f64,
    sender: Sender<Event>,
    receiver: Mutex<Receiver<Event>>,
    keys: MessageCursor<KeyboardInput>,
    buttons: MessageCursor<MouseButtonInput>,
    wheels: MessageCursor<MouseWheel>,
    cursor: Option<(f64, f64)>,
    restore: Vec<(Entity, CursorGrabMode, bool)>,
}
impl Default for State {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            epoch: 0,
            prepared: false,
            retired: false,
            clear_requested: false,
            ready: false,
            open: false,
            closing: false,
            pending_pause: false,
            in_flight: false,
            input_owned: false,
            error: None,
            lease: None,
            bend: Bend::default(),
            snapshot: None,
            elapsed: 0.,
            interval: 0.,
            sender,
            receiver: Mutex::new(receiver),
            keys: Default::default(),
            buttons: Default::default(),
            wheels: Default::default(),
            cursor: None,
            restore: Vec::new(),
        }
    }
}
impl State {
    pub(super) fn screen_fx_active(&self) -> bool {
        self.open && !self.closing
    }
    pub(super) fn blocks_input(&self) -> bool {
        self.open || self.input_owned || self.clear_requested || self.retired
    }
}

fn command(world: &World, epoch: u64, operation: Operation) -> Result<(), String> {
    world
        .get_non_send_resource::<super::pause_menu::PauseMenu>()
        .ok_or("wheel UI worker absent")?
        .power_wheel_command(Command { epoch, operation })
}
fn stop_time(world: &mut World, state: &mut State) -> Result<(), String> {
    if let Some(lease) = state.lease {
        world
            .get_resource_mut::<ClockRuntime>()
            .ok_or("wheel clock disappeared before its request retired")?
            .release_time_request(lease, state.bend.exit)?;
        state.lease = None;
    }
    Ok(())
}
fn screen(world: &mut World) -> (i32, i32) {
    world
        .query::<&Window>()
        .iter(world)
        .next()
        .map_or((1920, 1080), |window| {
            crate::gameplay::pause_state::screen_size(
                (window.physical_width(), window.physical_height()),
                (window.physical_width(), window.physical_height()),
            )
        })
}
fn ui_fx(world: &mut World, on: bool) {
    if let Some(mut fx) = world.get_resource_mut::<super::pause_fx::PauseScreenFx>() {
        fx.requested = Some(on);
    }
}
fn cursor(world: &mut World, state: &mut State, on: bool) {
    if on {
        state.restore.clear();
        for (entity, mut cursor) in world
            .query::<(Entity, &mut CursorOptions)>()
            .iter_mut(world)
        {
            state
                .restore
                .push((entity, cursor.grab_mode, cursor.visible));
            cursor.grab_mode = CursorGrabMode::None;
            cursor.visible = false;
        }
    } else {
        for (entity, grab, visible) in state.restore.drain(..) {
            if let Ok(mut entity) = world.get_entity_mut(entity) {
                if let Some(mut cursor) = entity.get_mut::<CursorOptions>() {
                    cursor.grab_mode = grab;
                    cursor.visible = visible;
                }
            }
        }
    }
}
fn finish(world: &mut World, state: &mut State) -> Result<(), String> {
    stop_time(world, state)?;
    ui_fx(world, false);
    cursor(world, state, false);
    if let Some(mut logic) = world.get_resource_mut::<GameLogic>() {
        if logic.current() == Some(StateId::POWER_WHEEL) {
            logic.request_finish_current();
            logic.commit_finish();
        }
    }
    if let Some(mut menu) = world.get_non_send_resource_mut::<super::pause_menu::PauseMenu>() {
        menu.set_power_wheel_open(false);
    }
    state.open = false;
    state.closing = false;
    state.in_flight = false;
    state.snapshot = None;
    state.elapsed = 0.;
    state.cursor = None;
    if let Some(mut frame) =
        world.get_resource_mut::<crate::rendering::bevy_backend::flash_ui::FlashUiFrame>()
    {
        frame.0 = None;
    }
    Ok(())
}

/// Retire before the existing world clock and game-logic owners disappear.
pub(super) fn retire(world: &mut World, catalog: Option<&Arc<crate::archive::Catalog>>) -> Result<bool, String> {
    if !world.contains_resource::<State>() {
        return Ok(true);
    }
    let catalog = catalog.ok_or("wheel retirement lost its actual catalog owner")?;
    world.resource_scope(|world, mut state: Mut<State>| {
        if state.retired {
            return Ok(true);
        }
        if state.open || state.lease.is_some() || !state.restore.is_empty() {
            finish(world, &mut state)?;
        }
        //EndHud may already have cleared the worker's sender/movie. This
        //barrier carries its own reply address through that same FIFO, so the
        //acknowledgement still follows actual old-movie/frame production.
        if !state.clear_requested {
            command(
                world,
                state.epoch,
                Operation::Clear {
                    events: state.sender.clone(),
                },
            )?;
            state.clear_requested = true;
        }
        world
            .run_system_once_with(super::resident_images::service_ui_images_for_catalog, catalog.clone())
            .map_err(|error| error.to_string())?;
        let mut acknowledged = false;
        loop {
            let event = state
                .receiver
                .get_mut()
                .map_err(|_| "wheel event receiver poisoned")?
                .try_recv();
            match event {
                Ok(Event {
                    epoch,
                    notification: Notification::Cleared,
                }) if epoch == state.epoch => {
                    acknowledged = true;
                }
                Ok(_) => {} //Drop stale frames and their actual resource leases.
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("wheel clear acknowledgement channel disconnected".into());
                }
            }
        }
        if !acknowledged {
            if world
                .get_non_send_resource::<super::pause_menu::PauseMenu>()
                .is_none_or(|menu| menu.ui_worker_failed())
            {
                return Err("wheel UI worker exited before its clear acknowledgement".into());
            }
            return Ok(false);
        }
        reset_map(world, &mut state);
        state.retired = true;
        //Keep the actual old HUD epoch. The incoming begin_level changes it;
        //a private increment can collide with that real next-map generation.
        Ok(true)
    })
}

fn reset_map(world: &World, state: &mut State) {
    state.prepared = false;
    state.clear_requested = false;
    state.ready = false;
    state.open = false;
    state.closing = false;
    state.pending_pause = false;
    state.in_flight = false;
    state.input_owned = false;
    state.error = None;
    state.lease = None;
    state.bend = Bend::default();
    state.snapshot = None;
    state.elapsed = 0.;
    state.interval = 0.;
    state.cursor = None;
    state.restore.clear();
    discard_input(world, state);
}

pub(super) fn before_ui(world: &mut World) -> Result<(), String> {
    phase(world, false)
}
pub(super) fn after_player(world: &mut World) -> Result<(), String> {
    phase(world, true)
}
fn phase(world: &mut World, after_player: bool) -> Result<(), String> {
    if !world.contains_resource::<State>() {
        world.init_resource::<State>();
    }
    let result = world.resource_scope(|world, mut state: Mut<State>| {
        update_state(world, &mut state, after_player)
    });
    if let Err(error) = &result {
        world.resource_mut::<State>().error = Some(error.clone());
    }
    result
}
pub(super) fn startup_pending(world: &World) -> bool {
    if !world
        .get_resource::<GameLogic>()
        .is_some_and(|logic| logic.declaration.states[8].is_some())
    {
        return false;
    }
    world
        .get_resource::<State>()
        .is_none_or(|state| !state.ready)
}
pub(super) fn startup_error(world: &World) -> Option<String> {
    world
        .get_resource::<State>()
        .and_then(|state| state.error.clone())
}
fn update_state(world: &mut World, state: &mut State, after_player: bool) -> Result<(), String> {
    if state.clear_requested {
        discard_input(world, state);
        return Ok(());
    }
    if world
        .get_resource::<super::loading_screen::ExitState>()
        .is_some_and(|exit| exit.requested())
    {
        discard_input(world, state);
        return Ok(());
    }
    // A release/click which closes Flash still belongs to that UI input pass.
    // Do not replay it as a weapon action later in the same engine frame.
    if !after_player {
        state.input_owned = state.open;
    }
    let Some(menu) = world.get_non_send_resource::<super::pause_menu::PauseMenu>() else {
        return Ok(());
    };
    let epoch = menu.hud_epoch();
    let loading = world
        .get_resource::<super::loading_screen::Acquisition>()
        .is_none_or(super::loading_screen::is_loading);
    if epoch != state.epoch {
        if state.open || state.lease.is_some() {
            finish(world, state)?;
        }
        state.epoch = epoch;
        reset_map(world, state);
        state.retired = false;
    }
    if state.retired {
        discard_input(world, state);
        return Ok(());
    }
    if !after_player && !state.prepared && state.error.is_none() {
        let path = {
            let Some(mut logic) = world.get_resource_mut::<GameLogic>() else {
                discard_input(world, state);
                return Ok(());
            };
            if logic.is_shutting_down() {
                return Ok(());
            }
            let Some(path) = logic.declaration.states[StateId::POWER_WHEEL.0 as usize].clone()
            else {
                return Ok(());
            };
            logic.register(StateId::POWER_WHEEL)?;
            path
        };
        let size = screen(world);
        command(
            world,
            state.epoch,
            Operation::Prepare {
                screen: size,
                path,
                events: state.sender.clone(),
            },
        )?;
        state.prepared = true;
    }
    drain_events(world, state, false)?;
    if loading {
        discard_input(world, state);
        return Ok(());
    }
    let logic_active = world
        .get_resource::<GameLogic>()
        .is_some_and(|logic| logic.current() == Some(StateId::POWER_WHEEL));
    let requested = {
        let Some(mut player) = world.get_resource_mut::<super::player::PlayerState>() else {
            return Ok(());
        };
        let Some(player) = player.player.as_mut() else {
            return Ok(());
        };
        player.set_power_wheel_status(state.ready, logic_active);
        after_player && player.take_power_wheel_request()
    };
    let mut opened = false;
    if requested && !state.open && state.ready {
        let snapshot = world.resource_scope(|world, mut inventory: Mut<DiscoveredDocuments>| {
            let player = world
                .resource::<super::player::PlayerState>()
                .player
                .as_ref()
                .ok_or("wheel player absent")?;
            let mut snapshot = inventory.wheel_snapshot(player);
            //6396f0 chooses the first owned upgrade whose actual ammo record
            //sets overridePowerWheelDesc (flags+61 bit5). No resource reread.
            for item in std::sync::Arc::make_mut(&mut snapshot.items) {
                let Some(ammo) = item.ammunition.as_ref() else {
                    continue;
                };
                let Some(actor) = super::equipment_identity::id(world, &item.equipment.path) else {
                    continue;
                };
                item.upgrade =
                    super::equipment_attachment_admission::upgrade_definitions(world, actor)
                        .and_then(|upgrades| {
                            upgrades.iter().find(|upgrade| {
                                upgrade.ammunition.iter().any(|candidate| {
                                    candidate.item == ammo.path.as_ref()
                                        && candidate.flags & (1 << 13) != 0
                                })
                            })
                        })
                        .cloned();
            }
            Ok::<_, String>(snapshot)
        })?;
        let mut logic = world.resource_mut::<GameLogic>();
        if logic.request(StateId::POWER_WHEEL) {
            logic
                .commit()?
                .ok_or("wheel game-logic request not committed")?;
            drop(logic);
            state.lease = Some(world.resource_mut::<ClockRuntime>().acquire_time_request(
                TimeRequest {
                    world: state.bend.world,
                    unbent: state.bend.unbent,
                    ease: state.bend.enter,
                },
            )?);
            state.snapshot = Some(snapshot.clone());
            state.open = true;
            opened = true;
            state.closing = false;
            state.pending_pause = false;
            world
                .non_send_resource_mut::<super::pause_menu::PauseMenu>()
                .set_power_wheel_open(true);
            if let Some(player) = world
                .resource_mut::<super::player::PlayerState>()
                .player
                .as_mut()
            {
                player.controller.clear();
                player.clear_power_inputs();
            }
            cursor(world, state, true);
            ui_fx(world, true);
            command(world, state.epoch, Operation::Open(snapshot))?;
        }
    }
    if state.open {
        if !state.closing
            && world
                .get_resource::<super::player::PlayerState>()
                .and_then(|state| state.player.as_ref())
                .is_some_and(|player| player.is_dead() || !player.power_wheel_state_active())
        {
            state.closing = true;
            stop_time(world, state)?;
            command(world, state.epoch, Operation::ForceExit)?;
        }
        if !after_player || opened {
            let input_changed = input(world, state)?;
            if !after_player {
                state.elapsed += world.resource::<Time>().delta_secs_f64();
            }
            if !state.in_flight && (opened || input_changed || state.elapsed >= state.interval) {
                let dt = std::mem::take(&mut state.elapsed);
                command(world, state.epoch, Operation::Tick(dt))?;
                state.in_flight = true;
            }
            drain_events(world, state, true)?;
        }
    } else if !after_player {
        discard_input(world, state);
    }
    Ok(())
}
fn drain_events(world: &mut World, state: &mut State, wait: bool) -> Result<(), String> {
    loop {
        if wait && state.in_flight {
            world
                .run_system_once(super::resident_images::service_ui_images)
                .map_err(|error| error.to_string())?;
            if world
                .get_non_send_resource::<super::pause_menu::PauseMenu>()
                .is_none_or(|menu| menu.ui_worker_failed())
            {
                return Err(
                    "power wheel UI worker exited before its real frame acknowledgement".into(),
                );
            }
        }
        let event = if wait && state.in_flight {
            match state
                .receiver
                .lock()
                .map_err(|_| "wheel event receiver poisoned")?
                .recv_timeout(std::time::Duration::from_millis(1))
            {
                Ok(event) => Some(event),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("wheel UI event channel disconnected".into());
                }
            }
        } else {
            state
                .receiver
                .lock()
                .map_err(|_| "wheel event receiver poisoned")?
                .try_recv()
                .ok()
        };
        let Some(event) = event else { break };
        if event.epoch != state.epoch {
            continue;
        }
        match event.notification {
            Notification::Cleared => {}
            Notification::Prepared { interval } => {
                state.interval = interval;
                state.ready = true;
            }
            Notification::Frame(frame) => {
                state.in_flight = false;
                if state.open {
                    if let Some(frame) = frame {
                        world
                            .resource_mut::<crate::rendering::bevy_backend::flash_ui::FlashUiFrame>(
                            )
                            .0 = Some(frame);
                    }
                }
            }
            Notification::Closing { selected, visible } => {
                state.closing = true;
                stop_time(world, state)?;
                ui_fx(world, false);
                if let Some(snapshot) = &state.snapshot {
                    let alive = world
                        .get_resource::<super::player::PlayerState>()
                        .and_then(|state| state.player.as_ref())
                        .is_some_and(|player| !player.is_dead());
                    if alive {
                        if let Some(item) =
                            snapshot.items.iter().find(|item| item.index() == selected)
                        {
                            select(world, item)?;
                        }
                    }
                    let mut inventory = world.resource_mut::<DiscoveredDocuments>();
                    for (item, visible) in snapshot.items.iter().zip(visible) {
                        inventory.wheel_visibility(item, visible);
                    }
                }
            }
            Notification::Disabled => {
                finish(world, state)?;
                if std::mem::take(&mut state.pending_pause) {
                    let mut logic = world.resource_mut::<GameLogic>();
                    if logic.request(StateId::PAUSE) {
                        logic.commit()?;
                        drop(logic);
                        let size = screen(world);
                        world
                            .non_send_resource_mut::<super::pause_menu::PauseMenu>()
                            .open_pause((size.0 as u32, size.1 as u32));
                    }
                }
            }
            Notification::Shortcut { slot, index } => {
                if slot < 14 {
                    let item = state.snapshot.as_ref().and_then(|snapshot| {
                        snapshot.items.iter().find(|item| item.index() == index)
                    });
                    world.resource_mut::<DiscoveredDocuments>().set_shortcut(
                        slot,
                        item.map(|item| item.equipment.path.as_ref()),
                        item.and_then(|item| {
                            item.ammunition.as_ref().map(|ammo| ammo.path.as_ref())
                        }),
                    );
                }
            }
            Notification::HealthPotion | Notification::ManaPotion => {
                let kind = if matches!(event.notification, Notification::HealthPotion) {
                    PotionKind::Health
                } else {
                    PotionKind::Mana
                };
                world
                    .run_system_once_with(potion, kind)
                    .map_err(|error| error.to_string())?;
                if let Some(player) = world
                    .resource::<super::player::PlayerState>()
                    .player
                    .as_ref()
                {
                    let refresh = world
                        .resource::<DiscoveredDocuments>()
                        .wheel_potions(player);
                    command(world, state.epoch, Operation::Refresh(refresh))?;
                }
            }
            Notification::Pause => {
                state.pending_pause = true;
                stop_time(world, state)?;
                command(world, state.epoch, Operation::ForceExit)?;
            }
            Notification::Sound(id) => {
                world.write_message(super::audio::SoundEvent(
                    crate::gameplay::sound::Request::Post {
                        event: id,
                        emitter: crate::gameplay::sound::Emitter::Menu,
                    },
                ));
            }
            Notification::Failed(error) => {
                finish(world, state)?;
                state.prepared = false;
                state.ready = false;
                return Err(format!("power wheel: {error}"));
            }
        }
    }
    Ok(())
}
fn select(world: &mut World, item: &crate::gameplay::power_wheel::Item) -> Result<(), String> {
    world.resource_scope(
        |world, mut actions: Mut<super::player_actions::PlayerActions>| {
            let player = world
                .resource::<super::player::PlayerState>()
                .player
                .as_ref()
                .ok_or("wheel selection player absent")?;
            actions.request_selection(
                player,
                ShortcutSelection {
                    equipment: item.equipment.path.to_string(),
                    ammunition: item.ammunition.as_ref().map(|ammo| ammo.path.to_string()),
                    equipment_entity: item.equipment.equipment.clone(),
                },
            );
            Ok(())
        },
    )
}
fn discard_input(world: &World, state: &mut State) {
    if let Some(messages) = world.get_resource::<Messages<KeyboardInput>>() {
        state.keys.clear(messages);
    }
    if let Some(messages) = world.get_resource::<Messages<MouseButtonInput>>() {
        state.buttons.clear(messages);
    }
    if let Some(messages) = world.get_resource::<Messages<MouseWheel>>() {
        state.wheels.clear(messages);
    }
}
fn input(world: &mut World, state: &mut State) -> Result<bool, String> {
    let mut changed = false;
    if let Some(messages) = world.get_resource::<Messages<KeyboardInput>>() {
        for key in state.keys.read(messages) {
            if key.repeat {
                continue;
            }
            if let Some(code) = super::engine_keys::engine_key(key.key_code) {
                command(
                    world,
                    state.epoch,
                    Operation::Key {
                        down: key.state == ButtonState::Pressed,
                        key: code,
                    },
                )?;
                changed = true;
            }
        }
    }
    if let Some(messages) = world.get_resource::<Messages<MouseButtonInput>>() {
        for button in state.buttons.read(messages) {
            let button_id = match button.button {
                MouseButton::Left => 0,
                MouseButton::Right => 1,
                MouseButton::Middle => 2,
                MouseButton::Back => 3,
                MouseButton::Forward => 4,
                MouseButton::Other(id) if id < 7 => id as u8,
                _ => continue,
            };
            command(
                world,
                state.epoch,
                Operation::Button {
                    down: button.state == ButtonState::Pressed,
                    button: button_id,
                },
            )?;
            changed = true;
        }
    }
    if let Some(messages) = world.get_resource::<Messages<MouseWheel>>() {
        for wheel in state.wheels.read(messages) {
            let delta = match wheel.unit {
                MouseScrollUnit::Line => wheel.y as i32,
                MouseScrollUnit::Pixel => (wheel.y / 120.) as i32,
            };
            if delta != 0 {
                command(world, state.epoch, Operation::Wheel(delta.signum()))?;
                changed = true;
            }
        }
    }
    let position = world
        .query::<&Window>()
        .iter(world)
        .next()
        .and_then(|window| {
            window.cursor_position().map(|cursor| {
                (
                    cursor.x as f64 * 1920. / window.width() as f64,
                    cursor.y as f64 * 1080. / window.height() as f64,
                )
            })
        });
    if let Some((x, y)) = position {
        if state.cursor != position {
            command(world, state.epoch, Operation::Cursor { x, y })?;
            state.cursor = position;
            changed = true;
        }
    }
    Ok(changed)
}
/// Trusted movie callbacks share the exact existing potion transaction,
///including native wheel-state sound suppression, difficulty and receipts.
fn potion(
    In(kind): In<PotionKind>,
    mut state: ResMut<super::player::PlayerState>,
    mut inventory: ResMut<DiscoveredDocuments>,
    mut actions: ResMut<super::player_actions::PlayerActions>,
    logic: Res<GameLogic>,
    mut sounds: MessageWriter<super::audio::SoundEvent>,
) {
    let super::player::PlayerState {
        player, difficulty, ..
    } = &mut *state;
    let Some(player) = player.as_mut() else {
        return;
    };
    let preset = difficulty
        .as_ref()
        .map_or(0, |difficulty| difficulty.current);
    let action = match kind {
        PotionKind::Health => 35,
        PotionKind::Mana => 36,
    };
    super::player_actions::dispatch_potions(
        player,
        &[crate::gameplay::player::controller::ActionEvent {
            action,
            key: 0,
            value: [0.; 2],
            flags: 1,
        }],
        &mut inventory,
        &mut actions,
        difficulty.as_ref(),
        preset,
        Some(&logic),
        &mut sounds,
    );
}
