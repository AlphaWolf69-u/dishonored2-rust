//! arkCpntPlayerController (vtable 0x141c72918): the input pass
//! FUN_1407f9ea0 (ProcessInput, slot 29), the DEFAULT think FUN_1407fb5c0,
//! ground-support state selection FUN_1407fbb20 and the decl states mapping
//! FUN_1407f8270.
use super::character_proxy::SupportState;
use super::states::{StateId, StateMachine};
use bevy_math::Vec3;

/// eActions_t ids (binding_tables.rs `ACTIONS`).
pub const ACTION_MOVE: i32 = 1;
pub const ACTION_MOVE_FORWARD: i32 = 2;
pub const ACTION_MOVE_BACKWARD: i32 = 3;
pub const ACTION_MOVE_LEFT: i32 = 4;
pub const ACTION_MOVE_RIGHT: i32 = 5;
pub const ACTION_LOOK: i32 = 6;
pub const ACTION_LOOK_UP: i32 = 7;
pub const ACTION_LOOK_DOWN: i32 = 8;
pub const ACTION_LOOK_LEFT: i32 = 9;
pub const ACTION_LOOK_RIGHT: i32 = 10;
pub const ACTION_JUMP: i32 = 14;
pub const ACTION_SPRINT: i32 = 15;
pub const ACTION_CROUCH: i32 = 16;
/// The DirectInput mouse-move key code.
pub const MOUSE_MOVE_KEY: u32 = 0xea;

/// eUpdatedAction_t (0x14 bytes): flags bit 0 down/active, bit 1 analog.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActionEvent {
    pub action: i32,
    pub key: u32,
    pub value: [f32; 2],
    pub flags: u32,
}

/// The cvars the controller reads (`g_*`, cvar float +0x2c / int +0x28).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookSettings {
    pub mouse_clamp: f32,
    pub mouse_sensitivity: f32,
    pub invert_mouse_y: bool,
    pub invert_look_y: bool,
    pub fix_mouse_sensibility: bool,
    pub look_stick_sensitivity: [f32; 2],
    pub sprint_toggle: bool,
}

impl Default for LookSettings {
    fn default() -> Self {
        Self {
            mouse_clamp: 360.0,
            mouse_sensitivity: 1.0,
            invert_mouse_y: false,
            invert_look_y: false,
            fix_mouse_sensibility: true,
            look_stick_sensitivity: [1.0, 1.0],
            sprint_toggle: true,
        }
    }
}

/// `m_usage` (arkDeclCpntPlayerController::buttonUsage_t, 0x14239e1c8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonUsage {
    Pressed = 0,
    Down = 1,
    Released = 2,
    ToggleDown = 3,
}

/// arkDeclCpntPlayerController::stateMapping_t.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateMapping {
    pub usage: ButtonUsage,
    pub action: i32,
    pub state: StateId,
    /// `m_priorityWhenAvailable`, -1 when unset (INTERACT priority picker).
    pub priority_when_available: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Controller {
    pub mapping: Vec<StateMapping>,
    allowed: u64,
    pressed: u64,
    released: u64,
    held: u64,
    toggled: u64,
    keyboard_move: bool,
    keyboard_look: bool,
    mouse_look: bool,
    raw_move: Vec3,
    raw_look: Vec3,
    /// +0xc4: x forward, y left.
    pub input_move: Vec3,
    /// +0xd0: x pitch (+down), y yaw (+left).
    pub input_look: Vec3,
    /// +0xdc.
    pub look_is_analog: bool,
    /// +0x68.
    pub support_state: SupportState,
    pub sneak_action: i32,
    pub sprinting_action: i32,
    sprinting_stopped: bool,
    sprinting_button_down_before_stop: bool,
    /// +0x170.
    pub must_sprint: bool,
}

fn bit(action: i32) -> u64 {
    if (0..64).contains(&action) { 1 << action } else { 0 }
}

impl Controller {
    pub fn new(mapping: Vec<StateMapping>) -> Self {
        // FUN_1407f9530: the actions of the SNEAKING and SPRINTING mappings.
        let action_of = |state| mapping.iter().find(|m| m.state == state).map_or(0, |m| m.action);
        let sneak_action = action_of(StateId::SNEAKING);
        let sprinting_action = action_of(StateId::SPRINTING);
        Self {
            mapping,
            allowed: !0,
            pressed: 0,
            released: 0,
            held: 0,
            toggled: 0,
            keyboard_move: false,
            keyboard_look: false,
            mouse_look: false,
            raw_move: Vec3::ZERO,
            raw_look: Vec3::ZERO,
            input_move: Vec3::ZERO,
            input_look: Vec3::ZERO,
            look_is_analog: false,
            support_state: SupportState::Unsupported,
            sneak_action,
            sprinting_action,
            sprinting_stopped: false,
            sprinting_button_down_before_stop: false,
            must_sprint: false,
        }
    }

    pub fn down(&self, action: i32) -> bool {
        self.held & self.allowed & bit(action) != 0
    }
    pub fn pressed(&self, action: i32) -> bool {
        self.pressed & self.allowed & bit(action) != 0
    }
    pub fn released(&self, action: i32) -> bool {
        self.released & self.allowed & bit(action) != 0
    }
    pub fn toggled(&self, action: i32) -> bool {
        self.toggled & bit(action) != 0
    }
    pub fn set_toggled(&mut self, action: i32, on: bool) {
        if on {
            self.toggled |= bit(action) & self.allowed;
        } else {
            self.toggled &= !bit(action);
        }
    }

    /// FUN_1407fa5e0 (Clear, slot 26).
    pub fn clear(&mut self) {
        self.raw_move = Vec3::ZERO;
        self.raw_look = Vec3::ZERO;
        self.pressed = 0;
        self.released = 0;
        self.held = 0;
    }

    /// FUN_1407fb2f0 "stop sprint".
    pub fn stop_sprint(&mut self) {
        let sprint = bit(self.sprinting_action);
        self.sprinting_stopped = true;
        self.sprinting_button_down_before_stop = self.held & self.allowed & sprint != 0;
        self.held &= !sprint;
    }

    /// FUN_1407f9ea0: one input pass over the frame's bound events.
    pub fn process_input(&mut self, events: &[ActionEvent], settings: &LookSettings) {
        self.pressed = 0;
        self.released = 0;
        let mut mouse_seen = false;
        let move_bits = bit(ACTION_MOVE_FORWARD) | bit(ACTION_MOVE_BACKWARD) | bit(ACTION_MOVE_LEFT) | bit(ACTION_MOVE_RIGHT);
        let look_bits = bit(ACTION_LOOK_UP) | bit(ACTION_LOOK_DOWN) | bit(ACTION_LOOK_LEFT) | bit(ACTION_LOOK_RIGHT);
        for event in events {
            let b = bit(event.action);
            let analog = event.flags & 2 != 0;
            // Analog triggers (19/20) keep their own hysteresis; not bound
            // to locomotion.
            if !(analog && matches!(event.action, 19 | 20)) {
                if event.flags & 1 != 0 {
                    self.pressed |= b;
                    self.held |= b;
                } else {
                    self.released |= b;
                    self.held &= !b;
                }
                if self.allowed & b == 0 {
                    continue;
                }
            }
            if !analog {
                let held = self.allowed & self.held;
                let x = (held & bit(ACTION_MOVE_FORWARD) != 0) as i32 - (held & bit(ACTION_MOVE_BACKWARD) != 0) as i32;
                let y = (held & bit(ACTION_MOVE_LEFT) != 0) as i32 - (held & bit(ACTION_MOVE_RIGHT) != 0) as i32;
                if x != 0 || y != 0 || self.released & self.allowed & move_bits != 0 {
                    self.keyboard_move = true;
                    self.raw_move = Vec3::new(x as f32, y as f32, 0.0);
                }
                if held & bit(ACTION_LOOK_UP) != 0 {
                    self.raw_look.x -= 1.0;
                }
                if held & bit(ACTION_LOOK_DOWN) != 0 {
                    self.raw_look.x += 1.0;
                }
                if held & bit(ACTION_LOOK_LEFT) != 0 {
                    self.raw_look.y -= 1.0;
                }
                if held & bit(ACTION_LOOK_RIGHT) != 0 {
                    self.raw_look.y += 1.0;
                }
                if self.released & self.allowed & look_bits != 0 && !self.mouse_look {
                    self.keyboard_look = true;
                    self.raw_look = Vec3::ZERO;
                }
            } else if event.action == ACTION_MOVE {
                self.keyboard_move = false;
                self.raw_move = Vec3::new(event.value[1], -event.value[0], 0.0);
            } else if event.action == ACTION_LOOK {
                self.keyboard_look = false;
                if event.key == MOUSE_MOVE_KEY {
                    self.mouse_look = true;
                    if !mouse_seen {
                        self.raw_look = Vec3::ZERO;
                        mouse_seen = true;
                    }
                    self.raw_look.y -= event.value[0] / settings.mouse_clamp;
                    let invert = if settings.invert_mouse_y { -1.0 } else { 1.0 };
                    self.raw_look.x += event.value[1] / settings.mouse_clamp * invert;
                } else {
                    self.mouse_look = false;
                    mouse_seen = false;
                    self.raw_look.y = -event.value[0];
                    self.raw_look.x = event.value[1] * if settings.invert_look_y { 1.0 } else { -1.0 };
                }
            }
        }
        self.toggled ^= self.allowed & self.pressed;
        if self.keyboard_look {
            self.raw_look /= self.raw_look.length_squared().max(f32::MIN_POSITIVE).sqrt();
        }
        if self.keyboard_move {
            self.raw_move /= self.raw_move.length_squared().max(f32::MIN_POSITIVE).sqrt();
        }
        if !mouse_seen && self.mouse_look {
            self.raw_look = Vec3::ZERO;
        }
    }

    /// FUN_1407fb5c0 up to the state requests. `frame_delta` is clock
    /// group 0's delta.
    pub fn think(&mut self, settings: &LookSettings, frame_delta: f32, physics_support: SupportState, ground_distance: f32, machine: &mut StateMachine) {
        let mut look = Vec3::new(self.raw_look.x, self.raw_look.y, 0.0);
        self.look_is_analog = !self.keyboard_look && !self.mouse_look;
        if !self.mouse_look {
            if look.length_squared() > 1.0 {
                look /= look.length_squared().max(f32::MIN_POSITIVE).sqrt();
            }
            look.x = look.x * look.x.abs() * settings.look_stick_sensitivity[1];
            look.y = look.y * look.y.abs() * settings.look_stick_sensitivity[0];
        } else if settings.fix_mouse_sensibility {
            // 1/60 (DAT_141c72cc4).
            look = if frame_delta <= f32::EPSILON {
                Vec3::ZERO
            } else {
                look * (0.016_666_668 / frame_delta) * settings.mouse_sensitivity
            };
        } else {
            look *= settings.mouse_sensitivity;
        }
        self.input_look = look;
        self.input_move = self.raw_move;
        if self.input_move.length_squared() > 1.0 {
            self.input_move = self.input_move.normalize();
        }
        if self.sprinting_stopped && self.released(self.sprinting_action) {
            self.sprinting_stopped = false;
            self.sprinting_button_down_before_stop = false;
        }
        self.update_support(physics_support, ground_distance, false, machine);
        self.apply_mapping(machine);
    }

    /// FUN_1407fbb20 (slipping is not ported).
    fn update_support(&mut self, support: SupportState, ground_distance: f32, force: bool, machine: &mut StateMachine) {
        if !force && support == self.support_state {
            return;
        }
        match support {
            SupportState::Unsupported => {
                if force || ground_distance == f32::MAX {
                    self.support_state = SupportState::Unsupported;
                    machine.request(StateId::FALLING);
                }
            }
            SupportState::Sliding | SupportState::Supported => {
                self.support_state = support;
                machine.request_landing();
            }
        }
    }

    /// FUN_1407f8270.
    fn apply_mapping(&self, machine: &mut StateMachine) {
        for mapping in &self.mapping {
            let fire = match mapping.usage {
                // Entries with a priority go through the INTERACT picker
                // FUN_1407f8040, which locomotion does not use.
                ButtonUsage::Pressed => mapping.priority_when_available < 0 && self.pressed(mapping.action),
                ButtonUsage::Down => self.down(mapping.action),
                ButtonUsage::Released => self.released(mapping.action),
                ButtonUsage::ToggleDown => self.toggled(mapping.action),
            };
            if fire {
                machine.request(mapping.state);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(action: i32, down: bool) -> ActionEvent {
        ActionEvent { action, key: 0, value: [0.0; 2], flags: down as u32 }
    }

    #[test]
    fn keyboard_move_is_the_normalized_sum_of_held_directions() {
        let mut controller = Controller::new(Vec::new());
        let settings = LookSettings::default();
        controller.process_input(&[key(ACTION_MOVE_FORWARD, true), key(ACTION_MOVE_LEFT, true)], &settings);
        assert!(controller.raw_move.distance(Vec3::new(0.70710677, 0.70710677, 0.0)) < 1e-6);
        controller.process_input(&[key(ACTION_MOVE_BACKWARD, true)], &settings);
        assert!(controller.raw_move.distance(Vec3::new(0.0, 1.0, 0.0)) < 1e-6);
        controller.process_input(&[key(ACTION_MOVE_LEFT, false)], &settings);
        assert_eq!(controller.raw_move, Vec3::ZERO);
        controller.process_input(&[], &settings);
        assert!(controller.down(ACTION_MOVE_FORWARD) && controller.down(ACTION_MOVE_BACKWARD));
    }

    #[test]
    fn mouse_counts_scale_by_clamp_sensitivity_and_frame_time() {
        let mut controller = Controller::new(Vec::new());
        let settings = LookSettings::default();
        let mouse = |dx: f32, dy: f32| ActionEvent { action: ACTION_LOOK, key: MOUSE_MOVE_KEY, value: [dx, dy], flags: 3 };
        controller.process_input(&[mouse(36.0, 0.0), mouse(36.0, -18.0)], &settings);
        assert_eq!(controller.raw_look, Vec3::new(-0.05, -0.2, 0.0));
        let mut machine = StateMachine::empty();
        controller.think(&settings, 1.0 / 120.0, SupportState::Supported, 0.0, &mut machine);
        assert!(controller.input_look.distance(Vec3::new(-0.1, -0.4, 0.0)) < 1e-6);
        assert!(!controller.look_is_analog);
        controller.process_input(&[], &settings);
        assert_eq!(controller.raw_look, Vec3::ZERO);
    }

    #[test]
    fn toggles_flip_on_each_press_edge() {
        let mut controller = Controller::new(Vec::new());
        let settings = LookSettings::default();
        controller.process_input(&[key(ACTION_CROUCH, true)], &settings);
        assert!(controller.toggled(ACTION_CROUCH) && controller.pressed(ACTION_CROUCH));
        controller.process_input(&[key(ACTION_CROUCH, false)], &settings);
        assert!(controller.toggled(ACTION_CROUCH) && !controller.pressed(ACTION_CROUCH));
        controller.process_input(&[key(ACTION_CROUCH, true)], &settings);
        assert!(!controller.toggled(ACTION_CROUCH));
    }
}
