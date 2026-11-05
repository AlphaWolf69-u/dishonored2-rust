//! The player's health value and evolved maximum (1407177e0/1407185b0,
//! 1407197f0). Damage dispatch and regeneration feed this same owner.
use super::states::Attributes;

pub struct Health {
    current: f32,
    base_max: f32,
    suppression_base: Option<f32>,
    accepts_water: bool,
    regen_amount: f32,
    regen_delay: f32,
    regen_rate: f32,
    regen_countdown: f32,
    dead: bool,
}
#[derive(Clone, Copy)]
pub struct Settings { pub maximum: f32, pub regen_amount: f32, pub regen_delay: f32, pub regen_rate: f32 }
impl Health {
    pub fn new(settings: Settings, accepts_water: bool, attributes: &Attributes) -> Self {
        let mut health = Self {
            current: 0.0,
            base_max: settings.maximum,
            suppression_base: attributes.0.get("ARK_ATTRIBUTE_HEALTH_MAX_SUPPRESSED").copied(),
            accepts_water,
            regen_amount: settings.regen_amount,
            regen_delay: settings.regen_delay,
            regen_rate: settings.regen_rate,
            regen_countdown: 0.0,
            dead: false,
        };
        health.current = health.maximum(attributes);
        health
    }
    pub fn current(&self) -> f32 { self.current }
    pub fn is_dead(&self) -> bool { self.dead }
    pub fn accepts_water(&self) -> bool { self.accepts_water }
    /// 140719990 for the ordinary player while health and mana are separate.
    pub fn projected(&self, attributes: &Attributes, difficulty_factor: f32) -> f32 {
        if self.dead || self.regen_amount == 0.0 || self.regen_countdown > 0.0 {
            return self.current;
        }
        let suppressed = self.suppression_base.is_some_and(|base| {
            attributes.0.get("ARK_ATTRIBUTE_HEALTH_MAX_SUPPRESSED").is_some_and(|current| *current != base)
        });
        let bonus = if suppressed { 0.0 } else { attributes.get("ARK_ATTRIBUTE_HEALTH_REGEN_AMOUNT") };
        self.current.max((self.regen_amount + bonus) * difficulty_factor).min(self.maximum(attributes))
    }
    pub fn maximum(&self, attributes: &Attributes) -> f32 {
        let suppressed = self.suppression_base.is_some_and(|base| {
            attributes.0.get("ARK_ATTRIBUTE_HEALTH_MAX_SUPPRESSED").is_some_and(|current| *current != base)
        });
        self.base_max + if suppressed { 0.0 } else { attributes.get("ARK_ATTRIBUTE_HEALTH_MAX") }
    }
    pub fn set(&mut self, value: f32, attributes: &Attributes) {
        let maximum = self.maximum(attributes);
        self.current = if value < 0.0 || value.is_nan() { 0.0 } else { value.min(maximum) };
    }
    pub fn damage(&mut self, amount: f32, attributes: &Attributes) {
        self.set(self.current - amount, attributes);
        // 140717950 uses the half-health-unit threshold before dispatching
        // the evolved-health death callback (14071a9e0).
        if self.current < 0.5 {
            self.current = 0.0;
            self.dead = true;
        }
        self.regen_countdown = self.regen_delay + attributes.get("ARK_ATTRIBUTE_HEALTH_REGEN_DELAY");
    }
    /// 14071b7c0: normal passive regeneration, after its authored delay.
    pub fn update(&mut self, dt: f32, attributes: &Attributes, difficulty_factor: f32) {
        if self.current <= 0.0 || self.regen_amount == 0.0 { return; }
        let suppressed = self.suppression_base.is_some_and(|base| {
            attributes.0.get("ARK_ATTRIBUTE_HEALTH_MAX_SUPPRESSED").is_some_and(|current| *current != base)
        });
        let bonus = if suppressed { 0.0 } else { attributes.get("ARK_ATTRIBUTE_HEALTH_REGEN_AMOUNT") };
        let target = ((self.regen_amount + bonus) * difficulty_factor).min(self.maximum(attributes));
        if self.current >= target { return; }
        self.regen_countdown -= dt;
        if self.regen_countdown <= 0.0 {
            let rate = attributes.0.get("ARK_ATTRIBUTE_HEALTH_REGEN_RATE_FACTOR").copied().unwrap_or(1.0);
            self.set((self.current + dt * self.regen_rate * rate).min(target), attributes);
        }
    }
}
