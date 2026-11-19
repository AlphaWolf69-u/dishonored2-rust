//! The shared arkCpntPower cast lifecycle (14061bc20..14061c2a0).
//! Family effects remain owned by their world components. Starting an arm
//! animation never creates an effect or spends mana. Family callbacks
//! (including UsePower release, 1408566f0) execute and pay for the effect.
use crate::{assets::entities::Value, gameplay::{options::decl::atom, player::{mana::Mana, states::Attributes}}};
pub mod windblast;
pub mod blast;
pub mod action;
pub mod world_effects;
pub mod dark_vision;
pub mod traversal;
pub mod far_reach;

#[derive(Clone, Debug)]
pub struct Settings {
    pub id: i32,
    pub cooldown: f32,
    pub mana_cost: f32,
    pub mana_replenish: f32,
    pub camera_speed_factor: f32,
    pub in_midnight: Option<String>,
    pub loop_midnight: Option<String>,
    pub out_midnight: Option<String>,
    pub in_midnight_slide: Option<String>,
    pub loop_midnight_slide: Option<String>,
    pub out_midnight_slide: Option<String>,
    pub cancel_midnight: Option<String>,
    pub cancel_sound: Option<String>,
    pub cancel_sound_event: Option<std::sync::Arc<crate::assets::wwise::SoundEventDeclaration>>,
    pub windblast: Option<windblast::Settings>,
}
impl Settings {
    pub fn read(id: i32, fields: &[(String, Value)]) -> Result<Self, String> {
        let number = |name, default| match atom(fields, name) {
            None => Ok(default), Some(value) => value.trim_end_matches('f').parse::<f32>()
                .ok().filter(|v| v.is_finite()).ok_or_else(|| format!("invalid power {name}: {value}")),
        };
        let path = |name| atom(fields, name).filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("NULL")).map(str::to_owned);
        // 14061d660 overrides the base constructor's zero cost for Blink.
        let default_mana = if id == 2 { 20.0 } else { 0.0 };
        Ok(Self { id, cooldown: number("m_cooldown", 0.0)?, mana_cost: number("m_manaCost", default_mana)?,
            mana_replenish: number("m_manaReplenish", default_mana)?, camera_speed_factor: number("m_cameraSpeedFactor", 1.0)?,
            in_midnight: path("m_inMidnight"), loop_midnight: path("m_loopMidnight"), out_midnight: path("m_outMidnight"),
            in_midnight_slide: path("m_inMidnightSlide"), loop_midnight_slide: path("m_loopMidnightSlide"), out_midnight_slide: path("m_outMidnightSlide"),
            cancel_midnight: path("m_cancelMidnight"), cancel_sound: path("m_cancelSoundEvent"), cancel_sound_event: None,
            windblast: (id == 10).then(|| windblast::Settings::read(fields)).transpose()? })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission { Allowed, Unavailable, InsufficientMana }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Idle, Aiming, Cast }

/// Inputs from the actual family/world and campaign owners. In particular,
/// free-mana windows and health-overcast are not inferred from elapsed time.
pub struct AdmissionContext {
    pub owned: bool,
    pub family_available: bool,
    pub check_recharge_target: bool,
    pub cost_factor: f32,
    pub health_overcast: bool,
    pub health: f32,
}
pub struct Cast {
    pub settings: Settings,
    phase: Phase,
    last_cast: Option<f64>,
    free_until: f64,
}
impl Cast {
    pub fn new(settings: Settings) -> Self { Self { settings, phase: Phase::Idle, last_cast: None, free_until: 0.0 } }
    pub fn phase(&self) -> Phase { self.phase }
    ///The real61bea0/61c320 timestamp, for the existing on-demand report.
    pub fn last_cast_time(&self) -> Option<f64> { self.last_cast }
    /// 61c320 refreshes the same last-use timestamp during traversal.
    pub fn refresh_cooldown(&mut self, now: f64) { self.last_cast = Some(now); }
    /// 14061c350 extends the free-cost deadline, never shortens it.
    pub fn grant_free_window(&mut self, now: f64, duration: f32) { self.free_until = self.free_until.max(now + duration as f64); }
    /// 14061c060: equality at the deadline still uses the normal cost.
    pub fn cost(&self, now: f64, factor: f32) -> f32 {
        if now < self.free_until { 0.0 } else { self.settings.mana_cost * factor }
    }
    fn family_cost(&self, now: f64, factor: f32, attributes: &Attributes) -> f32 {
        let cost = self.cost(now, factor);
        if self.settings.windblast.is_some() { windblast::cost(cost, attributes) } else { cost }
    }
    /// 14061c0e0 compares the last cast's game clock against m_cooldown.
    pub fn cooldown_ready(&self, now: f64, enabled: bool) -> bool {
        !enabled || self.last_cast.is_none_or(|last| now - last >= self.settings.cooldown as f64)
    }
    /// 14061bce0. The lock bits have distinct semantics: no-consumption
    /// bypasses payment; no-regeneration permits a partial bar, but not zero.
    pub fn admission(&self, now: f64, mana: &Mana, attributes: &Attributes, replenish: bool, context: &AdmissionContext) -> Admission {
        if !context.owned || !context.family_available || mana.magic_suppressed(attributes) { return Admission::Unavailable; }
        let cost = self.family_cost(now, context.cost_factor, attributes);
        let available = if context.check_recharge_target && mana.has_potion_target() { mana.regen_target(attributes, replenish) } else { mana.current() };
        if mana.lock_flags() & 1 != 0 { return Admission::Allowed; }
        if mana.lock_flags() & 2 != 0 {
            return if available == 0.0 { Admission::Unavailable } else { Admission::Allowed };
        }
        if available >= cost || (context.health_overcast && context.health >= cost - available + 1.0) { Admission::Allowed }
        else { Admission::InsufficientMana }
    }
    /// Family target selection and state-machine admission must succeed first.
    /// 14061bc20 applies the camera modifier; the host consumes the returned
    /// factor using the shared player attribute modifier owner.
    pub fn begin(&mut self) -> Option<f32> {
        if self.phase != Phase::Idle { return None; }
        self.phase = Phase::Aiming;
        Some(self.settings.camera_speed_factor)
    }
    /// Called only by the real family cast callback after the effect owner
    /// accepts the target. 14061bea0 records the timestamp and charges once.
    /// Native bonecharm random rolls are evaluated by the shared RNG owner;
    /// pass its resulting cost multiplier (double cost or free) explicitly.
    pub fn commit(&mut self, now: f64, mana: &mut Mana, attributes: &Attributes,
        cost_factor: f32, random_cost_factor: f32, replenish: bool) -> Option<f32> {
        if self.phase != Phase::Aiming { return None; }
        let cost = self.family_cost(now, cost_factor, attributes) * random_cost_factor;
        let deficit = if cost > 0.0 { mana.consume_with_replenish(cost, attributes,
            replenish.then_some(self.settings.mana_replenish)) } else { 0.0 };
        self.phase = Phase::Cast; self.last_cast = Some(now);
        Some(deficit)
    }
    /// 14061be50: finish and cancellation both remove the camera modifier.
    /// Cancelling before the cast event has no mana side effects.
    pub fn finish(&mut self) -> bool {
        let active = self.phase != Phase::Idle;
        self.phase = Phase::Idle;
        active
    }
}

/// 14061bea0's two conditional random rolls, in their original order. The
/// caller supplies the shared game's RNG (not a per-power private seed).
/// A successful double-cost roll suppresses the free-cast roll entirely.
/// Constant 141ab3b20 is 2.0 (Ghidra bytes 00 00 00 40).
pub fn random_cost_factor(double_chance: f32, free_chance: f32, mut next_u32: impl FnMut() -> u32) -> f32 {
    let roll = |value: u32| ((value >> 10 & 0x7fff) % 101) as i32;
    if double_chance as i32 != 0 && roll(next_u32()) < double_chance as i32 { return 2.0; }
    if free_chance as i32 != 0 && roll(next_u32()) < free_chance as i32 { return 0.0; }
    1.0
}

/// Attribute lookup uses the constructor fallback for a missing index,
/// which differs from Attributes::get's general zero fallback.
pub fn cost_factor(attributes: &Attributes) -> f32 {
    attributes.0.get("ARK_ATTRIBUTE_POWER_COST_FACTOR").copied().unwrap_or(1.0)
}
pub fn attribute_random_cost_factor(attributes: &Attributes, next_u32: impl FnMut() -> u32) -> f32 {
    random_cost_factor(attributes.get("ARK_ATTRIBUTE_POWER_DOUBLE_COST_CHANCE"),
        attributes.get("ARK_ATTRIBUTE_POWER_FREE_CHANCE"), next_u32)
}
