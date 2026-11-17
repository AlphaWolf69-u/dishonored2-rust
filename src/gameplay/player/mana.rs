//! Player mana value and its authored recharge target (140746310/746cf0).
use super::states::Attributes;

#[derive(Clone, Copy)]
pub struct Settings { pub maximum: f32, pub regen_amount: f32, pub regen_delay: f32, pub regen_rate: f32 }

pub struct Mana { settings: Settings, current: f32, target: f32, countdown: f32, maximum_base: Option<f32>, magic_suppression_base: Option<f32>, lock_flags: u8,
    potion_used: bool, potion_regen: bool, potion_speed: f32 }
impl Mana {
    pub fn new(settings: Settings, attributes: &Attributes) -> Self {
        let maximum = settings.maximum + attributes.get("ARK_ATTRIBUTE_MANA_MAX");
        Self { settings, current: maximum, target: maximum, countdown: 0.0,
            maximum_base: attributes.0.get("ARK_ATTRIBUTE_MANA_MAX").copied(),
            magic_suppression_base: attributes.0.get("ARK_ATTRIBUTE_MAGIC_SUPPRESSED").copied(), lock_flags: 0,
            potion_used: false, potion_regen: false, potion_speed: 1.0 }
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
    /// 7459f0 tests the dynamic target, including pending potion gains.
    pub fn can_use_potion(&self, attributes: &Attributes) -> bool { self.target < self.maximum(attributes) }
    /// 745b20 resets a non-potion dynamic target to current before the first
    /// potion, but accumulates repeated potion gains while bit8 remains set.
    pub fn begin_potion(&mut self, amount: f32, speed: f32, attributes: &Attributes, reset_target: bool) {
        if reset_target && !self.potion_used { self.target = self.current; }
        self.potion_used = true;
        self.potion_regen = true;
        self.potion_speed = speed;
        self.countdown = 0.0;
        self.gain(amount, attributes);
    }
    pub fn potion_used(&self) -> bool { self.potion_used }
    pub fn has_potion_target(&self) -> bool { self.potion_regen }
    pub fn consume(&mut self, amount: f32, attributes: &Attributes, replenish: bool) {
        self.consume_with_replenish(amount, attributes, if replenish { Some(self.settings.regen_amount) } else { None });
    }
    /// 140746cf0 receives the selected power's numeric m_manaReplenish,
    /// rather than the mana component's default recharge amount. Damage and
    /// breath retain consume() and its existing component-default policy.
    /// Returns the unfunded share for the health-overcast owner.
    pub fn consume_with_replenish(&mut self, amount: f32, attributes: &Attributes, replenish: Option<f32>) -> f32 {
        if self.lock_flags & 1 != 0 { return 0.0; }
        let deficit = (amount - self.current).max(0.0);
        let previous = self.current;
        self.current = (self.current - amount).max(0.0);
        self.target = (self.target - (previous - self.current)).max(0.0);
        if let Some(replenish) = replenish {
            let factor = attributes.0.get("ARK_ATTRIBUTE_MANA_REGEN_AMOUNT_FACTOR").copied().unwrap_or(1.0);
            let amount = (replenish + attributes.get("ARK_ATTRIBUTE_MANA_REGEN_AMOUNT")) * factor;
            let maximum = self.settings.maximum + attributes.get("ARK_ATTRIBUTE_MANA_MAX");
            self.target = self.target.max(self.current + amount).min(maximum);
        }
        self.countdown = self.settings.regen_delay + attributes.get("ARK_ATTRIBUTE_MANA_REGEN_DELAY");
        deficit
    }
    ///746b60 ->746730 posts the authored sound only when a positive
    ///recharge delay crosses zero. Already-recharging frames do not repost it.
    pub fn update(&mut self, dt: f32, attributes: &Attributes, replenish: bool) -> bool {
        if self.lock_flags & 2 != 0 { return false; }
        let target = self.regen_target(attributes, replenish).min(self.maximum(attributes));
        if self.current >= target {
            if self.countdown <= 0.0 && self.potion_used {
                self.potion_used = false;
                self.potion_regen = false;
                self.countdown = self.settings.regen_delay + attributes.get("ARK_ATTRIBUTE_MANA_REGEN_DELAY");
            }
            return false;
        }
        let previous_countdown = self.countdown;
        self.countdown -= dt;
        let begin_regen = previous_countdown > 0.0 && self.countdown <= 0.0;
        if self.countdown <= 0.0 {
            let rate = attributes.get("ARK_ATTRIBUTE_MANA_REGEN_RATE");
            let speed = if self.potion_used { self.potion_speed } else { 1.0 };
            self.current = (self.current + dt * self.settings.regen_rate * rate * speed).min(target);
            if self.current >= target {
                self.potion_used = false;
                self.potion_regen = false;
                self.countdown = self.settings.regen_delay + attributes.get("ARK_ATTRIBUTE_MANA_REGEN_DELAY");
            }
        }
        begin_regen
    }
}
