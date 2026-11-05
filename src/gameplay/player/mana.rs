//! Player mana value and its authored recharge target (140746310/746cf0).
use super::states::Attributes;

#[derive(Clone, Copy)]
pub struct Settings { pub maximum: f32, pub regen_amount: f32, pub regen_delay: f32, pub regen_rate: f32 }

pub struct Mana { settings: Settings, current: f32, target: f32, countdown: f32 }
impl Mana {
    pub fn new(settings: Settings, attributes: &Attributes) -> Self {
        let maximum = settings.maximum + attributes.get("ARK_ATTRIBUTE_MANA_MAX");
        Self { settings, current: maximum, target: maximum, countdown: 0.0 }
    }
    pub fn current(&self) -> f32 { self.current }
    pub fn maximum(&self, attributes: &Attributes) -> f32 { self.settings.maximum + attributes.get("ARK_ATTRIBUTE_MANA_MAX") }
    pub fn consume(&mut self, amount: f32, attributes: &Attributes, replenish: bool) {
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
        let factor = attributes.0.get("ARK_ATTRIBUTE_MANA_REGEN_AMOUNT_FACTOR").copied().unwrap_or(1.0);
        let floor = if replenish { self.settings.regen_amount * factor } else { 0.0 };
        let target = self.target.max(floor).min(self.settings.maximum + attributes.get("ARK_ATTRIBUTE_MANA_MAX"));
        if self.current >= target { return; }
        self.countdown -= dt;
        if self.countdown <= 0.0 {
            let rate = attributes.get("ARK_ATTRIBUTE_MANA_REGEN_RATE");
            self.current = (self.current + dt * self.settings.regen_rate * rate).min(target);
        }
    }
}
