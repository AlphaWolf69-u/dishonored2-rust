//! Player mana value and its authored recharge target (140746310/746cf0).
use super::states::Attributes;

#[derive(Clone, Copy)]
pub struct Settings { pub maximum: f32, pub regen_amount: f32, pub regen_delay: f32, pub regen_rate: f32 }

pub struct Mana { settings: Settings, current: f32, target: f32, countdown: f32, maximum_base: Option<f32>, magic_suppression_base: Option<f32>, lock_flags: u8 }
impl Mana {
    pub fn new(settings: Settings, attributes: &Attributes) -> Self {
        let maximum = settings.maximum + attributes.get("ARK_ATTRIBUTE_MANA_MAX");
        Self { settings, current: maximum, target: maximum, countdown: 0.0,
            maximum_base: attributes.0.get("ARK_ATTRIBUTE_MANA_MAX").copied(),
            magic_suppression_base: attributes.0.get("ARK_ATTRIBUTE_MAGIC_SUPPRESSED").copied(), lock_flags: 0 }
    }
    pub fn current(&self) -> f32 { self.current }
    pub fn maximum(&self, attributes: &Attributes) -> f32 { self.settings.maximum + attributes.get("ARK_ATTRIBUTE_MANA_MAX") }
    pub fn has_extension(&self, attributes: &Attributes) -> bool {
        attributes.0.get("ARK_ATTRIBUTE_MANA_MAX").copied() != self.maximum_base
    }
    pub fn magic_suppressed(&self, attributes: &Attributes) -> bool {
        attributes.0.get("ARK_ATTRIBUTE_MAGIC_SUPPRESSED").copied() != self.magic_suppression_base
    }
    pub fn lock_flags(&self) -> u8 { self.lock_flags }
    pub fn set_lock_flags(&mut self, flags: u8) { self.lock_flags = flags & 3; }
    /// 140746120 adjusts current and target when maximum mana changes.
    pub fn maximum_changed(&mut self, added: bool, attributes: &Attributes) {
        let maximum = self.maximum(attributes).max(0.0);
        if (added && self.current >= self.settings.maximum) || (!added && self.current > maximum) {
            let previous = self.current;
            self.current = maximum;
            self.target = (self.target - (previous - self.current)).clamp(0.0, maximum);
        }
    }
    pub fn regen_target(&self, attributes: &Attributes, replenish: bool) -> f32 {
        let factor = attributes.0.get("ARK_ATTRIBUTE_MANA_REGEN_AMOUNT_FACTOR").copied().unwrap_or(1.0);
        let floor = if replenish { self.settings.regen_amount * factor } else { 0.0 };
        self.target.max(floor)
    }
    /// 140746920 queues positive mana for the player toward its maximum.
    pub fn gain(&mut self, amount: f32, attributes: &Attributes) {
        self.target = (self.target + amount).min(self.maximum(attributes));
    }
    pub fn consume(&mut self, amount: f32, attributes: &Attributes, replenish: bool) {
        if self.lock_flags & 1 != 0 { return; }
        let previous = self.current;
        self.current = (self.current - amount).max(0.0);
        self.target = (self.target - (previous - self.current)).max(0.0);
        if replenish {
            let factor = attributes.0.get("ARK_ATTRIBUTE_MANA_REGEN_AMOUNT_FACTOR").copied().unwrap_or(1.0);
            let amount = (self.settings.regen_amount + attributes.get("ARK_ATTRIBUTE_MANA_REGEN_AMOUNT")) * factor;
            let maximum = self.settings.maximum + attributes.get("ARK_ATTRIBUTE_MANA_MAX");
            self.target = self.target.max(self.current + amount).min(maximum);
        }
        self.countdown = self.settings.regen_delay + attributes.get("ARK_ATTRIBUTE_MANA_REGEN_DELAY");
    }
    pub fn update(&mut self, dt: f32, attributes: &Attributes, replenish: bool) {
        if self.lock_flags & 2 != 0 { return; }
        let target = self.regen_target(attributes, replenish).min(self.maximum(attributes));
        if self.current >= target { return; }
        self.countdown -= dt;
        if self.countdown <= 0.0 {
            let rate = attributes.get("ARK_ATTRIBUTE_MANA_REGEN_RATE");
            self.current = (self.current + dt * self.settings.regen_rate * rate).min(target);
        }
    }
}
