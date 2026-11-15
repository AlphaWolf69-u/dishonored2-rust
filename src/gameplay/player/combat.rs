//! Shared player-action and ranged-state decisions, from 140854dc0,
//! 140855230/360/5a0 and 140866fe0/140867330. Animation completion comes
//! from Midnight; these owners never synthesize an animation-duration timer.
use crate::entities::{scene_motion::atom, Value};

type Fields = [(String, Value)];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActionType {
    #[default]
    Press,
    Release,
    Message,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActionDecl {
    pub kind: ActionType,
    pub togglable: bool,
    pub cooldown: f32,
    pub max_hold_duration: f32,
    pub midnight: Option<String>,
    pub wait_for_midnight: bool,
    pub wait_before_execute: bool,
    pub wait_wall_uncrouch: bool,
}

impl Default for ActionDecl {
    fn default() -> Self {
        // 140854dc0: +88 = 1, all other action fields zero/null.
        Self { kind: ActionType::Press, togglable: false, cooldown: 0.0,
            max_hold_duration: 0.0, midnight: None, wait_for_midnight: true,
            wait_before_execute: false, wait_wall_uncrouch: false }
    }
}

fn number(fields: &Fields, key: &str, fallback: f32) -> Result<f32, String> {
    atom(fields, key).map_or(Ok(fallback), |text| text.trim_end_matches('f').parse::<f32>()
        .ok().filter(|value| value.is_finite()).ok_or_else(|| format!("invalid {key}={text}")))
}
fn boolean(fields: &Fields, key: &str, fallback: bool) -> Result<bool, String> {
    match atom(fields, key) {
        None => Ok(fallback), Some("true" | "1") => Ok(true), Some("false" | "0") => Ok(false),
        Some(value) => Err(format!("invalid {key}={value}")),
    }
}
fn path(fields: &Fields, key: &str) -> Option<String> {
    atom(fields, key).filter(|value| !value.is_empty() && *value != "NULL").map(str::to_owned)
}
impl ActionDecl {
    pub fn read(fields: &Fields) -> Result<Self, String> {
        Ok(Self {
            kind: match atom(fields, "m_type") {
                None | Some("ACTION_ONPRESS" | "0") => ActionType::Press,
                Some("ACTION_ONRELEASE" | "1") => ActionType::Release,
                Some("ACTION_ONMSG" | "2") => ActionType::Message,
                Some(value) => return Err(format!("unknown arkStateActionType_t {value}")),
            },
            togglable: boolean(fields, "m_togglable", false)?,
            cooldown: number(fields, "m_cooldown", 0.0)?,
            max_hold_duration: number(fields, "m_maxHoldDuration", 0.0)?,
            midnight: path(fields, "m_midnight"),
            wait_for_midnight: boolean(fields, "m_waitForMidnight", true)?,
            wait_before_execute: boolean(fields, "m_waitForMidnightBeforeExecute", false)?,
            wait_wall_uncrouch: boolean(fields, "m_waitWallUncrouch", false)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActionStep { #[default] None, Execute, Quit }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionRuntime {
    pub scene: u32,
    elapsed: f32,
    last_deactivated: f64,
    waiting_scene: bool,
    waiting_execute: bool,
    waiting_uncrouch: bool,
    execute_requested: bool,
}
impl ActionRuntime {
    /// 140855300: cooldown uses the owning entity's unbent time.
    pub fn can_activate(&self, now: f64, decl: &ActionDecl) -> bool {
        decl.cooldown <= 0.0 || (now - self.last_deactivated) as f32 >= decl.cooldown
    }
    /// The return value requests the derived class's actual execute callback.
    /// A wall-crouch transition may defer that callback, as 140854e50 does.
    pub fn activate(&mut self, decl: &ActionDecl, wall_uncrouch_busy: bool) -> ActionStep {
        self.elapsed = 0.0;
        self.scene = 0;
        self.waiting_scene = false;
        self.waiting_execute = false;
        self.waiting_uncrouch = decl.wait_wall_uncrouch && wall_uncrouch_busy;
        if self.waiting_uncrouch { ActionStep::None } else { ActionStep::Execute }
    }
    /// 140855360 runs after the derived class starts its real scene.
    pub fn after_scene(&mut self, handle: u32, decl: &ActionDecl) {
        self.scene = handle;
        self.waiting_scene = decl.wait_for_midnight && handle != 0;
        self.waiting_execute = decl.wait_before_execute && handle != 0;
    }
    /// 140855570: a repeated accepted request toggles a togglable action.
    pub fn requested_again(&mut self, decl: &ActionDecl, active: bool) {
        self.execute_requested = decl.togglable && active;
    }
    pub fn update(&mut self, dt: f32, decl: &ActionDecl, button_down: bool,
        scene_finished: bool, wall_uncrouch_busy: bool) -> ActionStep {
        if self.waiting_uncrouch {
            if wall_uncrouch_busy { return ActionStep::None; }
            self.waiting_uncrouch = false;
            return ActionStep::Execute;
        }
        let result = if self.waiting_scene {
            if scene_finished { ActionStep::Quit } else { ActionStep::None }
        } else {
            self.execute_requested |= decl.kind == ActionType::Press
                || decl.kind == ActionType::Release && !button_down
                || decl.max_hold_duration > 0.0 && self.elapsed >= decl.max_hold_duration;
            if self.execute_requested && (!self.waiting_execute || scene_finished) {
                ActionStep::Execute
            } else { ActionStep::None }
        };
        self.elapsed += dt;
        result
    }
    /// Message 0xf6, 140855750: only the matching scene executes ONMSG.
    pub fn message(&self, decl: &ActionDecl, handle: u32) -> ActionStep {
        if decl.kind == ActionType::Message && self.scene == handle { ActionStep::Execute }
        else { ActionStep::None }
    }
    /// 1408550d0 clears the scene identity without stopping its animation.
    pub fn deactivate(&mut self, now: f64) {
        self.last_deactivated = now;
        self.elapsed = 0.0;
        self.scene = 0;
        self.waiting_scene = false;
        self.waiting_execute = false;
        self.waiting_uncrouch = false;
        self.execute_requested = false;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FireDecl {
    pub action: ActionDecl,
    pub firelock_activation_delay: f32,
    pub charged_shot_hold_duration: f32,
    pub target_marked_sound: Option<String>,
    pub target_lost_sound: Option<String>,
}
impl FireDecl {
    pub fn read(fields: &Fields) -> Result<Self, String> {
        // 140866440 +90=.5, +a8=1; reflected at 141781710.
        Ok(Self { action: ActionDecl::read(fields)?,
            firelock_activation_delay: number(fields, "m_firelockActivationDelay", 0.5)?,
            charged_shot_hold_duration: number(fields, "m_chargedShotHoldDuration", 1.0)?,
            target_marked_sound: path(fields, "m_targetMarkedSoundEvent"),
            target_lost_sound: path(fields, "m_targetLostSoundEvent"),
        })
    }
}

/// Values are the original state field +78, not a replacement timing model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FirePhase {
    #[default] Inactive, TargetDelay, Targeting, TargetAcquired, Charging, Firing, OutOfAmmo,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FireRuntime {
    pub action: ActionRuntime,
    pub phase: FirePhase,
    pub target_delay: f32,
    pub charge_time: f32,
}
impl FireRuntime {
    pub fn begin(&mut self, decl: &FireDecl, can_fire: bool, targeting: bool, charged: bool) {
        self.phase = if !can_fire { FirePhase::OutOfAmmo }
            else if targeting { FirePhase::TargetDelay }
            else if charged { FirePhase::Charging }
            else { FirePhase::Firing };
        self.target_delay = decl.firelock_activation_delay;
        self.charge_time = 0.0;
    }
    /// 140867330: charging accrues only while a charge scene exists; the
    /// same update fires on release, independent of the charged threshold.
    pub fn released(&mut self, button_down: bool) -> bool {
        if !button_down && matches!(self.phase, FirePhase::TargetDelay | FirePhase::Targeting
            | FirePhase::TargetAcquired | FirePhase::Charging) {
            self.phase = FirePhase::Firing;
            true
        } else { false }
    }
    pub fn advance_target_delay(&mut self, dt: f32) -> bool {
        if self.phase != FirePhase::TargetDelay { return false; }
        self.target_delay -= dt;
        if self.target_delay <= 0.0 { self.phase = FirePhase::Targeting; true } else { false }
    }
}
