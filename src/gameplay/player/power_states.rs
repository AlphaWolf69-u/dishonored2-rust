//! Selected power and real UsePower/Blink state instances. The app executes
//! family callbacks on input/release; these states own arm scenes and motion.
use super::{states::{Frame, Scenes, StateId}, Player};
use crate::gameplay::powers::{self, Cast, Phase, Admission, AdmissionContext, action::{Action, Scenes as SceneHost},
    traversal::{BlinkMotion, BlinkContact, BlinkStep}};
use bevy_math::Vec3;
use super::combat::{ActionDecl, ActionRuntime, ActionStep, ActionType};
use crate::{assets::entities::Value, gameplay::options::decl::atom};

#[derive(Clone, Debug, PartialEq)]
pub struct UsePowerDecl {
    pub action: ActionDecl,
    pub out_of_mana: Option<String>,
    pub out_of_mana_slide: Option<String>,
}
impl UsePowerDecl {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let path = |name| atom(fields, name).filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("NULL")).map(str::to_owned);
        let mut action = ActionDecl::read(fields)?;
        // 1408560c0 writes 1 at decl+0x70: ACTION_ONRELEASE, overriding
        // arkDeclPlayerStateAction's ONPRESS default when not authored.
        if atom(fields, "m_type").is_none() { action.kind = ActionType::Release; }
        Ok(Self { action, out_of_mana: path("m_outOfManaMidnight"), out_of_mana_slide: path("m_outOfManaMidnightSlide") })
    }
}

#[derive(Default)]
pub struct Runtime {
    pub selected: Option<Cast>,
    scenes: Scenes,
    action: Action,
    pub now: f64,
    owned: bool,
    family_available: bool,
    health_overcast: bool,
    cost_factor: f32,
    pub active: bool,
    pub cancel: bool,
    pub blink: Option<BlinkMotion>,
    pub blink_pending: bool,
    contacts: Vec<BlinkContact>,
    previous_posture: usize,
    pub blink_finished: bool,
    pub sound_events: Vec<u32>,
    camera_modifier_applied: bool,
    camera_gamepad: bool,
    action_state: ActionRuntime,
    action_button: i32,
    execution_requested: bool,
    pub blink_mantle_sensor: Option<super::sensors::MantleSensor>,
    pub mantle_sensor_swapped: bool,
    pub blink_from_sprinting: bool,
    pub blink_from_sliding: bool,
    pub blink_from_falling: bool,
    pub blink_aborted: bool,
    input_pressure: f32,
    input_analog: bool,
    cooldown_bypass: bool,
    lifecycle: Vec<PowerLifecycle>,
}

#[derive(Clone, Copy, Debug)]
pub struct PowerInputs { pub held: bool, pub pressure: f32, pub analog: bool }
#[derive(Clone, Copy, Debug)]
pub enum PowerLifecycle { Began(i32), Finished { id: i32, committed: bool } }

struct PreparedScenes<'a, 'b> {
    scenes: &'a Scenes,
    fallback: &'a Scenes,
    midnight: &'a mut super::midnight::Midnight,
    md6: [&'b str; super::midnight::TARGETS],
    animations: super::midnight::Cast<'b>,
    sounds: &'a mut Vec<u32>,
}
impl SceneHost for PreparedScenes<'_, '_> {
    type Error = String;
    fn start(&mut self, path: &str) -> Result<u32, String> {
        let scene = self.scenes.get(path).or_else(|| self.fallback.get(path)).ok_or_else(|| format!("power scene was not prepared: {path}"))?;
        let parts = self.md6.iter().enumerate().filter(|(_, model)| !model.is_empty())
            .fold(0u8, |mask, (index, _)| mask | (1 << index)) & 0xf;
        self.midnight.start_with_parts(scene, self.md6, 1.0, self.animations, parts)
    }
    fn stop(&mut self, handle: u32) { self.midnight.stop(handle); }
    fn finished(&self, handle: u32) -> bool { self.midnight.finished(handle) }
    fn cancel_sound(&mut self, path: &str) {
        if let Some(name) = path.rsplit('/').next() { self.sounds.push(crate::assets::wwise::event_id(name)); }
    }
}

pub fn can_use(frame: &Frame<'_>, decl: &UsePowerDecl) -> bool {
    let runtime = &*frame.powers;
    runtime.selected.as_ref().is_some_and(|cast| runtime.owned && runtime.family_available
        && cast.phase() == Phase::Idle && cast.cooldown_ready(runtime.now, !runtime.cooldown_bypass)
        && runtime.action_state.can_activate(runtime.now, &decl.action))
}
pub fn activate_use(frame: &mut Frame<'_>, sliding: bool, decl: &UsePowerDecl) -> bool {
    let runtime = &mut *frame.powers;
    let Some(cast) = runtime.selected.as_mut() else { return false };
    let mut scenes = PreparedScenes { scenes: &runtime.scenes, fallback: frame.scenes, midnight: frame.midnight, md6: frame.md6, animations: frame.animations, sounds: &mut runtime.sound_events };
    let admission = cast.admission(runtime.now, frame.mana, frame.attributes, frame.mana_replenishes,
        &AdmissionContext { owned: runtime.owned, family_available: runtime.family_available,
            check_recharge_target: false, cost_factor: runtime.cost_factor,
            health_overcast: runtime.health_overcast, health: frame.health });
    if admission != Admission::Allowed {
        let failed = if sliding { decl.out_of_mana_slide.as_deref().or(decl.out_of_mana.as_deref()) } else { decl.out_of_mana.as_deref() };
        if let Some(path) = failed { if let Err(error) = scenes.start(path) { scenes.midnight.last_start_error = Some(error); } }
        return false;
    }
    match runtime.action.begin(cast, sliding, &mut scenes) {
        Ok(true) => {
            runtime.camera_gamepad = !frame.keyboard;
            runtime.action_state.activate(&decl.action, false);
            // Base state+0x24 is separate from UsePower's +0x50/+0x54.
            runtime.action_state.after_scene(0, &decl.action);
            runtime.active = true; runtime.cancel = false; runtime.execution_requested = false;
            runtime.lifecycle.push(PowerLifecycle::Began(cast.settings.id)); true
        },
        Ok(false) => false,
        Err(error) => { scenes.midnight.last_start_error = Some(error); false },
    }
}
pub fn update_use(frame: &mut Frame<'_>, sliding: bool, decl: &UsePowerDecl) -> bool {
    let runtime = &mut *frame.powers;
    let Some(cast) = runtime.selected.as_ref() else { return false };
    if runtime.cancel || !runtime.owned || !runtime.family_available { return false; }
    if cast.phase() == Phase::Cast && !runtime.action_state.executed()
        && runtime.action_state.finish_execution(&decl.action) == ActionStep::Quit { return false; }
    if cast.admission(runtime.now, frame.mana, frame.attributes, frame.mana_replenishes,
        &AdmissionContext { owned: runtime.owned, family_available: runtime.family_available,
            check_recharge_target: false, cost_factor: runtime.cost_factor,
            health_overcast: runtime.health_overcast, health: frame.health }) != Admission::Allowed { return false; }
    let mut scenes = PreparedScenes { scenes: &runtime.scenes, fallback: frame.scenes, midnight: frame.midnight, md6: frame.md6, animations: frame.animations, sounds: &mut runtime.sound_events };
    let step = runtime.action_state.update(frame.dt, &decl.action, frame.controller.down(runtime.action_button),
        scenes.finished(runtime.action_state.scene), false);
    match step { ActionStep::Execute if cast.phase() == Phase::Aiming => runtime.execution_requested = true,
        ActionStep::Quit => return false, _ => {} }
    match runtime.action.update(cast, sliding, &mut scenes) {
        Ok(()) => true,
        Err(error) => { scenes.midnight.last_start_error = Some(error); false },
    }
}
pub fn deactivate_use(frame: &mut Frame<'_>, sliding: bool) {
    let runtime = &mut *frame.powers;
    if let Some(cast) = runtime.selected.as_mut() {
        if runtime.active { runtime.lifecycle.push(PowerLifecycle::Finished { id: cast.settings.id, committed: cast.phase() == Phase::Cast }); }
        let mut scenes = PreparedScenes { scenes: &runtime.scenes, fallback: frame.scenes, midnight: frame.midnight, md6: frame.md6, animations: frame.animations, sounds: &mut runtime.sound_events };
        if let Err(error) = runtime.action.finish(cast, sliding, &mut scenes) {
            scenes.midnight.last_start_error = Some(error);
            runtime.action.stop(&mut scenes); cast.finish();
        }
    }
    runtime.active = false; runtime.cancel = false;
    runtime.action_state.deactivate(runtime.now); runtime.execution_requested = false;
}
pub fn activate_blink(frame: &mut Frame<'_>) {
    let runtime = &mut *frame.powers;
    runtime.blink_pending = false; runtime.blink_finished = false; runtime.blink_aborted = false;
    runtime.previous_posture = frame.physics.posture;
    frame.controller.disable_actions(0x4000);
    frame.physics.instant_motion();
    frame.physics.disable_gravity(true);
    frame.physics.set_gravity_velocity(Vec3::ZERO);
    frame.physics.set_posture(4, 0.0);
    if let Some(motion) = &runtime.blink {
        if motion.request.camera_source != Vec3::ZERO { frame.physics.proxy.position = motion.request.camera_source; }
    }
    frame.requests.push_angular(0, Vec3::ZERO);
}
pub fn update_blink(frame: &mut Frame<'_>) -> Option<StateId> {
    let runtime = &mut *frame.powers;
    let motion = runtime.blink.as_mut()?;
    // bevy_backend: movement requests are rebuilt per frame here, whereas
    // the native owner retains its angular-zero request and instant limits.
    // Republish those same values after underlying navigation updates.
    frame.physics.set_motion_limits(super::physics::UNSET, super::physics::UNSET,
        super::physics::UNSET, super::physics::UNSET);
    frame.requests.push_angular(0, Vec3::ZERO);
    match motion.step(frame.physics.position(), frame.physics.velocity, frame.dt, &runtime.contacts) {
        Ok(BlinkStep::Velocity(velocity)) => { frame.requests.push_linear(0, velocity); None },
        Ok(BlinkStep::Finish) => {
            frame.requests.push_linear(0, Vec3::ZERO);
            frame.physics.proxy.position = motion.request.final_position;
            runtime.blink_finished = true;
            Some(StateId(motion.request.exit_state))
        },
        Err(_) => { runtime.blink_finished = true; runtime.blink_aborted = true; Some(StateId::NULL) },
    }
}
pub fn deactivate_blink(frame: &mut Frame<'_>) {
    let runtime = &mut *frame.powers;
    frame.physics.disable_gravity(false);
    frame.controller.enable_actions(0x4000);
    runtime.blink_finished = true;
    runtime.blink_pending = false;
}

impl Player {
    /// Move the single selected Cast into the player. Shared immutable
    /// prepared scenes retain their existing resource owner leases.
    pub fn equip_power(&mut self, cast: Cast, scenes: Scenes) -> bool {
        if self.powers.active { return false; }
        self.powers.selected = Some(cast); self.powers.scenes = scenes;
        self.powers.action_button = self.controller.mapping.iter().find(|mapping| mapping.state == StateId::USE_POWER).map_or(0, |mapping| mapping.action);
        true
    }
    /// Actual equipment Deactivate/Stop callback (14061c1a0/14061c2a0),
    /// after the outgoing equipment scene has reached its native boundary.
    pub fn unequip_power(&mut self) {
        if let Some(cast) = self.powers.selected.as_mut() {
            if self.powers.active { self.powers.lifecycle.push(PowerLifecycle::Finished { id: cast.settings.id, committed: cast.phase() == Phase::Cast }); }
            let mut scenes = PreparedScenes { scenes: &self.powers.scenes, fallback: &self.scenes,
                midnight: &mut self.body.midnight, md6: super::body::cast_md6(&self.md6, &self.body.arms),
                animations: super::body::cast(&self.body.animations, &self.body.arms), sounds: &mut self.powers.sound_events };
            self.powers.action.stop(&mut scenes); cast.finish();
        }
        self.powers.active = false; self.powers.cancel = false;
        self.powers.execution_requested = false;
        self.powers.action_state.deactivate(self.powers.now);
        self.machine.quit(StateId::USE_POWER);
        self.update_power_camera_modifier();
        self.powers.selected = None; self.powers.scenes.clear();
    }
    pub fn selected_power(&self) -> Option<&Cast> { self.powers.selected.as_ref() }
    pub fn selected_power_mut(&mut self) -> Option<&mut Cast> { self.powers.selected.as_mut() }
    pub fn power_cast_active(&self, id: i32) -> bool {
        self.powers.active && self.powers.selected.as_ref().is_some_and(|cast| cast.settings.id == id && cast.phase() != Phase::Idle)
    }
    pub fn take_power_lifecycle_events(&mut self) -> Vec<PowerLifecycle> { std::mem::take(&mut self.powers.lifecycle) }
    pub fn set_power_admission(&mut self, now: f64, owned: bool, family_available: bool, health_overcast: bool, cost_factor: f32) {
        self.powers.now = now; self.powers.owned = owned; self.powers.family_available = family_available;
        self.powers.health_overcast = health_overcast; self.powers.cost_factor = cost_factor;
    }
    pub fn set_power_cooldown_bypass(&mut self, bypass: bool) { self.powers.cooldown_bypass = bypass; }
    /// Capture the authored eAction's analog value alongside the controller's
    /// existing held latch; the host forwards the same input events once.
    pub fn update_power_inputs(&mut self, events: &[super::controller::ActionEvent]) {
        for event in events.iter().filter(|event| event.action == self.powers.action_button) {
            self.powers.input_analog = event.flags & 2 != 0;
            self.powers.input_pressure = if event.flags & 1 == 0 { 0.0 }
                else if self.powers.input_analog { event.value[0] } else { 1.0 };
        }
    }
    pub fn power_inputs(&self) -> PowerInputs {
        let held = self.controller.down(self.powers.action_button);
        PowerInputs { held, pressure: if self.powers.input_analog { self.powers.input_pressure } else if held { 1.0 } else { 0.0 }, analog: self.powers.input_analog }
    }
    pub fn request_power_use(&mut self) -> bool {
        if self.powers.selected.is_none() || self.powers.active { return false; }
        self.machine.request(StateId::USE_POWER); true
    }
    pub fn power_release_ready(&self) -> bool {
        self.powers.active && self.powers.selected.as_ref().is_some_and(|cast| cast.phase() == Phase::Aiming)
    }
    pub fn take_power_execution_request(&mut self) -> bool { std::mem::take(&mut self.powers.execution_requested) && self.power_release_ready() }
    pub fn power_action_message(&mut self, handle: u32) {
        if let Some(state) = self.machine.state(StateId::USE_POWER) {
            if let super::states::Kind::UsePower(decl) = &state.decl.kind {
                self.powers.execution_requested |= self.powers.action_state.message(&decl.action, handle) == ActionStep::Execute;
            }
        }
    }
    pub fn cancel_power_use(&mut self) { self.powers.cancel = true; self.machine.quit(StateId::USE_POWER); }
    /// 14061bc20/14061be50. Run immediately after state transitions, before
    /// camera update: DAT141b9b5d0=-100 turns (1-factor) into a percentage.
    pub fn update_power_camera_modifier(&mut self) {
        let active = self.powers.active;
        if active == self.powers.camera_modifier_applied { return; }
        let factor = if self.powers.camera_gamepad { self.powers.selected.as_ref().map_or(1.0, |cast| cast.settings.camera_speed_factor) } else { 1.0 };
        self.apply_attribute_modifier("player-power-camera", 0, "ARK_ATTRIBUTE_CAMERA_ROTATION_FACTOR",
            0.0, (1.0 - factor) * -100.0, 0.0, active, self.powers.now);
        self.powers.camera_modifier_applied = active;
    }
    /// Family-specific callback only: failed targeting/spawn must never call
    /// this. Native RNG is shared with the player's other random consumers.
    pub fn commit_power_cast(&mut self, now: f64, cost_factor: f32) -> bool {
        if !self.power_release_ready() { return false; }
        let random = &mut self.random;
        let factor = powers::attribute_random_cost_factor(&self.attributes, || {
            *random = random.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f); *random
        });
        let Some(deficit) = self.powers.selected.as_mut().and_then(|cast| cast.commit(now, &mut self.mana, &self.attributes,
            cost_factor, factor, self.mana_replenishes)) else { return false };
        if self.powers.health_overcast && deficit > 0.0 { self.health.damage(deficit, &self.attributes); }
        true
    }
    pub fn begin_blink(&mut self, motion: BlinkMotion) -> bool {
        if self.machine.state(StateId::BLINKING).is_none() || self.powers.blink_pending
            || self.machine.is_active(StateId::BLINKING) { return false; }
        self.powers.blink = Some(motion); self.powers.blink_pending = true;
        self.machine.request(StateId::BLINKING); true
    }
    pub fn set_blink_mantle(&mut self, sensor: super::sensors::MantleSensor) { self.powers.blink_mantle_sensor = Some(sensor); }
    pub fn take_blink_mantle(&mut self) -> Option<super::sensors::MantleSensor> {
        if self.machine.is_active(StateId::BLINKING) || self.machine.is_active(StateId::MANTLE) || self.powers.mantle_sensor_swapped { None }
        else { self.powers.blink_mantle_sensor.take() }
    }
    pub fn set_blink_contacts(&mut self, contacts: &[BlinkContact]) {
        self.powers.contacts.clear(); self.powers.contacts.extend_from_slice(contacts);
    }
    pub fn blink_finished(&self) -> bool { self.powers.blink_finished }
    pub fn take_finished_blink(&mut self) -> Option<BlinkMotion> {
        if self.powers.blink_finished && !self.machine.is_active(StateId::BLINKING) { self.powers.blink_finished = false; self.powers.blink.take() }
        else { None }
    }
}
