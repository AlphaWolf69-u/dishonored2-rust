//! Body footstep distance clock (FUN_140776310) and navigation's authored
//! material/stride settings. Contact emission is supplied by the body owner.

/// arkPlayerFootStep_t: material, optional explicit sound and step length.
/// Navigation's constructor (FUN_1408684c0) initializes all three settings
/// blocks with null material/sound and a 2-metre step length.
#[derive(Clone, Debug, PartialEq)]
pub struct Footstep {
    pub material: Option<String>,
    pub sound: Option<u32>,
    pub step_length: f32,
}

impl Default for Footstep {
    fn default() -> Self { Self { material: None, sound: None, step_length: 2.0 } }
}

#[derive(Clone, Debug)]
pub struct FootstepSettings {
    pub min_speed: f32,
    pub min_interval: f32,
    /// Off, on (arkDeclCpntPlayerBody +0x1a0 / +0x198).
    pub silent_events: [Option<u32>; 2],
}

impl Default for FootstepSettings {
    /// arkDeclCpntPlayerBody constructor 14076f960.
    fn default() -> Self { Self { min_speed: 0.1, min_interval: 0.1, silent_events: [None; 2] } }
}

/// Body fields +0xc78..+0xc90. Configuration and initialization are owned
/// by the body; disabling leaves progress intact, matching the update routine.
#[derive(Clone, Debug, PartialEq)]
pub struct Footsteps {
    pub state: super::states::StateId,
    pub enabled: bool,
    pub stride: f32,
    pub distance: f32,
    pub second_half: bool,
    pub last_time: f64,
    pub buffer: f32,
    pub silent: bool,
}

impl Default for Footsteps {
    /// FUN_14076f430: +0xc78/+0xc7c zero, +0xc84 false, +0xc85 true,
    /// +0xc88 zero double and +0xc90 zero float.
    fn default() -> Self {
        Self { state: super::states::StateId::NULL, enabled: true, stride: 0.0, distance: 0.0, second_half: false, last_time: 0.0, buffer: 0.0, silent: false }
    }
}

impl Footsteps {
    /// SetFootStep (FUN_140774620) stores twice the declared step length:
    /// each half of this distance cycle produces one footstep.
    pub fn configure(&mut self, step: &Footstep, state: super::states::StateId) {
        self.stride = step.step_length * 2.0;
        self.state = state;
    }

    /// The callback returns the contact emission result (FUN_140770990).
    /// An attempted step changes the half-cycle even if emission fails;
    /// only successful emission updates the minimum-interval clock.
    pub fn update(&mut self, dt: f32, now: f64, speed: f32, min_speed: f32, min_interval: f32, mut emit: impl FnMut() -> bool) {
        if self.enabled && self.stride > 0.0 {
            if speed <= min_speed {
                self.distance = 0.0;
                self.second_half = false;
            } else {
                self.distance = (dt * speed + self.distance) % self.stride;
                if min_interval <= (now - self.last_time) as f32 {
                    let fraction = self.distance / self.stride;
                    let edge = if self.second_half { fraction < 0.5 } else { fraction >= 0.5 };
                    if edge {
                        self.second_half = !self.second_half;
                        if emit() { self.last_time = now; }
                    }
                }
            }
        }
        if self.buffer > 0.0 { self.buffer -= dt; }
    }
}

/// Navigation Update (FUN_1408695c0). The slope input is the ground normal
/// dotted with the player's forward axis. Stairs inherit each missing
/// material/sound independently from the ordinary step configuration.
pub fn navigation_step(decl: &super::states::NavigationDecl, forward_dot: f32, speed: f32) -> Footstep {
    let stairs = if forward_dot > 0.34 { Some(&decl.footstep_downstairs) }
        else if forward_dot < -0.34 { Some(&decl.footstep_upstairs) } else { None };
    let mut step = decl.footstep.clone();
    if let Some(stairs) = stairs {
        if stairs.material.is_some() { step.material = stairs.material.clone(); }
        if stairs.sound.is_some() { step.sound = stairs.sound; }
        step.step_length = stairs.step_length;
    }
    let max_speed = decl.max_forward_speed.max(decl.max_backward_speed).max(decl.max_strafe_speed);
    if max_speed > 0.0 {
        let fraction = speed.min(max_speed) / max_speed;
        let ratio = decl.progressive_footstep_ratio;
        let ratio = if ratio.abs() <= 1e-18 { 0.0 } else { ratio };
        step.step_length *= (1.0 - fraction) * ratio + fraction;
    }
    step
}
