//! GameLocal rumble7b5fa0/7b67a0 and input-device policy03b580/03b3e0.
//! The shared FX owner supplies its completed numeric channels; this module
//! owns only the same global impulse mixer and the host controller output.
use bevy::{input::{gamepad::{Gamepad, GamepadAxisChangedEvent, GamepadButtonChangedEvent, GamepadRumbleIntensity},
    keyboard::KeyboardInput, mouse::{AccumulatedMouseMotion, MouseButtonInput, MouseWheel}}, prelude::*};
use crate::gameplay::{controller_feedback::Mixer, entity_events::{EntityEvents, Event, Listener, Payload}, game_clock_runtime::{ClockRuntime, GamePaused}};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}};

pub(super) struct Shared {
    pub generation: u64,
    mixer: Mutex<Mixer>,
    zero: AtomicBool,
}
impl Shared {
    fn new() -> Arc<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Arc::new(Self { generation: NEXT.fetch_add(1, Ordering::Relaxed), mixer: Default::default(), zero: AtomicBool::new(false) })
    }
    pub(super) fn message(&self, generation: u64, event: &Event, clock: Option<&ClockRuntime>) -> Result<(), String> {
        if generation != self.generation { return Err("rumble receiver generation differs".into()); }
        let Payload::Bytes(bytes) = &event.payload else { return Err("rumble event has no source byte packet".into()); };
        let mut mixer = self.mixer.lock().map_err(|_| "rumble mixer poisoned")?;
        match event.id {
            0x11a => {
                let bytes: &[u8; 12] = bytes.as_slice().try_into().map_err(|_| "rumble impulse packet size differs")?;
                let values: [f32; 3] = std::array::from_fn(|i| f32::from_le_bytes(bytes[i*4..i*4+4].try_into().unwrap()));
                let time = clock.ok_or("rumble impulse clock absent")?.clock().map_err(str::to_owned)?.time(1).ok_or("rumble clock group1 absent")?;
                let duration = crate::platform::console::shared().read().cvars.float("ark_rumble_impulse_duration").unwrap_or(0.1);
                mixer.impulse(time, values[0], values[1], values[2], duration);
            }
            0x11b => {
                let flag = *bytes.first().ok_or("rumble clear packet empty")?;
                mixer.clear();
                if flag != 0 { self.zero.store(true, Ordering::Release); }
            }
            _ => return Err("rumble receiver event differs".into()),
        }
        Ok(())
    }
}

#[derive(Resource)]
struct Feedback {
    shared: Arc<Shared>,
    pad: Option<Entity>,
    published: Option<(Entity, [f32; 2])>,
    lease: Arc<()>,
    closed: bool,
    enabled: bool,
}
impl Default for Feedback {
    fn default() -> Self { Self { shared: Shared::new(), pad: None, published: None, lease: Arc::new(()), closed: false, enabled: false } }
}
pub(super) fn install(app: &mut App) {
    app.init_resource::<Feedback>()
        .add_systems(PreUpdate, sample.after(bevy::input::InputSystems))
        .add_systems(PostUpdate, publish
            .after(crate::rendering::bevy_backend::camera_scene::simulate_effects)
            .before(bevy::gilrs::RumbleSystems));
}
///03b580: any selected-controller activity wins over concurrent keyboard or
///mouse activity. These independent readers do not consume gameplay input.
fn sample(mut keys: MessageReader<KeyboardInput>, mut buttons: MessageReader<MouseButtonInput>,
    mut wheel: MessageReader<MouseWheel>, motion: Res<AccumulatedMouseMotion>,
    mut pad_buttons: MessageReader<GamepadButtonChangedEvent>, mut axes: MessageReader<GamepadAxisChangedEvent>,
    mut owner: ResMut<Feedback>) {
    let keyboard = keys.read().count() != 0;
    let button_activity = buttons.read().count() != 0;
    let wheel_activity = wheel.read().count() != 0;
    let mouse = button_activity || wheel_activity || motion.delta != Vec2::ZERO;
    let pad = pad_buttons.read().map(|event| event.entity).chain(axes.read().map(|event| event.entity)).last();
    if let Some(pad) = pad { owner.pad = Some(pad); }
    else if keyboard || mouse { owner.pad = None; }
}
fn eligible(world: &World, pad: Option<Entity>) -> bool {
    pad.is_some_and(|id| world.get::<Gamepad>(id).is_some())
        && world.get_resource::<super::loading_screen::Acquisition>().is_some_and(|state| !super::loading_screen::is_loading(state))
        && world.get_resource::<GamePaused>().is_some_and(|paused| !paused.0)
        && crate::platform::console::shared().read().cvars.int("ark_rumble_enable").is_some_and(|value| value != 0)
}
///7b64e0's normal11a/11b subscriptions belong to the real scene lifetime.
pub(super) fn prepare(world: &mut World) {
    if !world.contains_resource::<Feedback>() || !world.contains_resource::<EntityEvents>() { return; }
    let Some(scene) = world.get_resource::<super::physics_scene::PhysicsScene>() else { return; };
    if scene.controller_feedback.is_none() {
        let shared = Shared::new();
        {
            let mut owner = world.resource_mut::<Feedback>();
            owner.shared = shared.clone(); owner.lease = Arc::new(()); owner.closed = false; owner.enabled = false;
        }
        let listener = Listener::ControllerFeedback { generation: shared.generation };
        let mut events = world.resource_mut::<EntityEvents>();
        events.register(0x11a, None, listener); events.register(0x11b, None, listener);
        world.resource_mut::<super::physics_scene::PhysicsScene>().controller_feedback = Some(shared);
    }
    world.resource_scope(|world, mut owner: Mut<Feedback>| { sync_policy(world, &mut owner); });
}
fn sync_policy(world: &World, owner: &mut Feedback) {
    let enabled = !owner.closed && eligible(world, owner.pad);
    if enabled != owner.enabled {
        owner.shared.mixer.lock().unwrap_or_else(|error| error.into_inner()).enabled(enabled);
        owner.enabled = enabled;
    }
}
fn send(world: &mut World, owner: &Feedback, pad: Entity, values: Option<[f32; 2]>) {
    //Gilrs startup can genuinely fail or hardware can be absent. No output
    //owner then exists; retaining numeric FX/impulse semantics is sufficient.
    if let Some(mut messages) = world.get_resource_mut::<Messages<bevy::gilrs::ConstantRumble>>() {
        messages.write(bevy::gilrs::ConstantRumble { gamepad: pad,
            intensity: values.map(|values| GamepadRumbleIntensity { strong_motor: values[0], weak_motor: values[1] }),
            owner: Arc::downgrade(&owner.lease) });
    }
}
fn publish(world: &mut World) {
    if !world.contains_resource::<Feedback>() { return; }
    world.resource_scope(|world, mut owner: Mut<Feedback>| {
        sync_policy(world, &mut owner);
        let pad = owner.pad.filter(|_| owner.enabled);
        if owner.published.is_some_and(|(previous, _)| Some(previous) != pad) {
            let (previous, _) = owner.published.take().unwrap();
            send(world, &owner, previous, None);
        }
        let Some(pad) = pad else { return; };
        let time = world.get_resource::<ClockRuntime>().and_then(|clock| clock.clock().ok()).and_then(|clock| clock.time(1));
        let frame = world.get_resource::<crate::rendering::bevy_backend::camera_scene::FxSimulation>().and_then(|fx| fx.observe_rumble());
        let values = match (time, frame) {
            (Some(time), Some((_, values))) => owner.shared.mixer.lock().unwrap_or_else(|error| error.into_inner()).output(time, values),
            _ => [0.; 4],
        };
        let values = if owner.shared.zero.swap(false, Ordering::AcqRel) { [0.; 2] } else { [values[0], values[1]] };
        if owner.published != Some((pad, values)) {
            send(world, &owner, pad, Some(values)); owner.published = Some((pad, values));
        }
    });
}
///7b66b0 unregisters both callbacks and explicitly zeroes output; the backend
///also holds a weak lease, so owner drop and disconnected devices release FF.
pub(super) fn retire(world: &mut World) {
    if !world.contains_resource::<Feedback>() { return; }
    world.resource_scope(|world, mut owner: Mut<Feedback>| {
        if owner.closed { return; }
        owner.closed = true;
        owner.enabled = false;
        let listener = Listener::ControllerFeedback { generation: owner.shared.generation };
        if let Some(mut events) = world.get_resource_mut::<EntityEvents>() {
            events.unregister(0x11a, None, listener); events.unregister(0x11b, None, listener);
        }
        owner.shared.mixer.lock().unwrap_or_else(|error| error.into_inner()).enabled(false);
        if let Some((pad, _)) = owner.published.take() { send(world, &owner, pad, None); }
    });
}
