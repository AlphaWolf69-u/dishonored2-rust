//! The player's health value and evolved maximum (1407177e0/1407185b0,
//! 1407197f0). Damage dispatch and regeneration feed this same owner.
use super::states::Attributes;

pub struct Health {
    current: f32,
    base_max: f32,
    suppression_base: Option<f32>,
    extensions: i32,
    suppressed_health: f32,
    heal_target: f32,
    heal_rate: f32,
    accepts_water: bool,
    regen_amount: f32,
    regen_delay: f32,
    regen_rate: f32,
    regen_countdown: f32,
    dead: bool,
}
#[derive(Clone, Copy)]
pub struct Settings { pub maximum: f32, pub regen_amount: f32, pub regen_delay: f32, pub regen_rate: f32, pub heal_rate: f32 }
impl Health {
    pub fn new(settings: Settings, accepts_water: bool, attributes: &Attributes) -> Self {
        let mut health = Self {
            current: 0.0,
            base_max: settings.maximum,
            suppression_base: attributes.0.get("ARK_ATTRIBUTE_HEALTH_MAX_SUPPRESSED").copied(),
            extensions: 0,
            suppressed_health: 0.0,
            heal_target: 0.0,
            heal_rate: settings.heal_rate,
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
    /// 719fe0 adds positive consumables/potions to the player's heal target.
    pub fn gain(&mut self, amount: f32, attributes: &Attributes) {
        if !self.dead && amount > 0.0 {
            self.heal_target = (self.heal_target.max(self.current) + amount).min(self.maximum(attributes));
        }
    }
    /// 718ce0 with separate health/mana and the ordinary conscious player.
    pub fn can_use_potion(&self, attributes: &Attributes, difficulty_factor: f32) -> bool {
        !self.dead && self.current >= 0.5 && self.projected(attributes, difficulty_factor) < self.maximum(attributes)
    }
    /// 14071a490 suppresses extensions while max health is suppressed.
    pub fn extensions(&self, attributes: &Attributes) -> i32 {
        let suppressed = self.suppression_base.is_some_and(|base| attributes.0.get("ARK_ATTRIBUTE_HEALTH_MAX_SUPPRESSED").is_some_and(|current| *current != base));
        if suppressed { 0 } else { self.extensions }
    }
    /// 140719e50 handles max-health attribute modifier notifications.
    pub fn attribute_changed(&mut self, tag: &str, added: bool, attributes: &Attributes, difficulty_factor: f32) {
        match tag {
            "ARK_ATTRIBUTE_HEALTH_MAX" => {
                if added {
                    self.extensions += 1;
                    if self.suppression_base.is_some_and(|base| attributes.0.get("ARK_ATTRIBUTE_HEALTH_MAX_SUPPRESSED").is_some_and(|current| *current != base)) {
                        self.suppressed_health = attributes.get("ARK_ATTRIBUTE_HEALTH_MAX");
                    }
                    if self.current >= self.base_max { self.set(self.maximum(attributes), attributes); }
                } else {
                    self.extensions -= 1;
                    let maximum = self.maximum(attributes);
                    if maximum < self.current {
                        self.heal_target -= self.current - maximum;
                        self.set(maximum, attributes);
                    }
                }
            }
            "ARK_ATTRIBUTE_HEALTH_MAX_SUPPRESSED" => {
                if added {
                    let maximum = self.maximum(attributes);
                    if maximum < self.current {
                        self.suppressed_health = self.current - maximum;
                        self.set(maximum, attributes);
                        self.heal_target -= self.suppressed_health;
                    }
                } else if self.suppressed_health > 0.0 { self.suppressed_health = 0.0; }
            }
            _ => return,
        }
        let suppressed = self.suppression_base.is_some_and(|base| attributes.0.get("ARK_ATTRIBUTE_HEALTH_MAX_SUPPRESSED").is_some_and(|current| *current != base));
        let bonus = if suppressed { 0.0 } else { attributes.get("ARK_ATTRIBUTE_HEALTH_REGEN_AMOUNT") };
        if self.regen_amount != 0.0 && self.current < (self.regen_amount + bonus) * difficulty_factor {
            self.regen_countdown = self.regen_delay + attributes.get("ARK_ATTRIBUTE_HEALTH_REGEN_DELAY");
        }
    }
    /// 140719990 for the ordinary player while health and mana are separate.
    pub fn projected(&self, attributes: &Attributes, difficulty_factor: f32) -> f32 {
        let mut projected = self.current.max(self.heal_target);
        let suppressed = self.suppression_base.is_some_and(|base| {
            attributes.0.get("ARK_ATTRIBUTE_HEALTH_MAX_SUPPRESSED").is_some_and(|current| *current != base)
        });
        let bonus = if suppressed { 0.0 } else { attributes.get("ARK_ATTRIBUTE_HEALTH_REGEN_AMOUNT") };
        if !self.dead && self.regen_amount != 0.0 && self.regen_countdown <= 0.0 && self.current < (self.regen_amount + bonus) * difficulty_factor {
            projected = projected.max((self.regen_amount + bonus) * difficulty_factor);
        }
        projected.min(self.maximum(attributes))
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
        // The evolved-health damage virtual (718c80) subtracts the same
        // damage from the pending healing target so spent food cannot heal a
        // later hit again. Passive regeneration has its own target below.
        self.heal_target -= amount;
        // 140717950 uses the half-health-unit threshold before dispatching
        // the evolved-health death callback (14071a9e0).
        if self.current < 0.5 {
            self.current = 0.0;
            self.dead = true;
            self.heal_target = 0.0;
        }
        self.regen_countdown = self.regen_delay + attributes.get("ARK_ATTRIBUTE_HEALTH_REGEN_DELAY");
    }
    /// Typed 717800/718fe0 calculation followed by the same 717950 health
    /// mutation/death threshold as drowning. The player consumes mana and
    /// dispatches entity incapacitation after this call, through its owners.
    pub fn receive_damage(&mut self, receiver: &crate::gameplay::damage::Receiver,
        params: &crate::gameplay::damage::Parameters, context: crate::gameplay::damage::Context,
        attributes: &Attributes) -> crate::gameplay::damage::DamageResult {
        if self.dead { return crate::gameplay::damage::DamageResult::default(); }
        let result = receiver.evaluate(self.current, params, context);
        if result.accepted && !receiver.infinite_health {
            self.damage(result.health_damage, attributes);
        }
        result
    }
    /// 14071b7c0: normal passive regeneration, after its authored delay.
    pub fn update(&mut self, dt: f32, attributes: &Attributes, difficulty_factor: f32) {
        self.update_with_healing(dt, attributes, difficulty_factor, 1.0);
    }
    /// 71b7c0 uses the potion/food target's heal rate while that target exceeds
    /// health. Otherwise it uses delayed regeneration's rate and target.
    pub fn update_with_healing(&mut self, dt: f32, attributes: &Attributes, difficulty_factor: f32, heal_factor: f32) {
        if self.dead || self.current <= 0.0 { return; }
        let suppressed = self.suppression_base.is_some_and(|base| {
            attributes.0.get("ARK_ATTRIBUTE_HEALTH_MAX_SUPPRESSED").is_some_and(|current| *current != base)
        });
        let bonus = if suppressed { 0.0 } else { attributes.get("ARK_ATTRIBUTE_HEALTH_REGEN_AMOUNT") };
        let mut target = 0.0;
        let mut speed = 0.0;
        if self.regen_amount != 0.0 {
            let regen_target = ((self.regen_amount + bonus) * difficulty_factor).min(self.maximum(attributes));
            if self.current < regen_target {
                self.regen_countdown -= dt;
                if self.regen_countdown <= 0.0 { target = regen_target; speed = self.regen_rate; }
            }
        }
        if self.current < self.heal_target { target = target.max(self.heal_target); speed = self.heal_rate * heal_factor; }
        if self.current < target {
            let factor = attributes.0.get("ARK_ATTRIBUTE_HEALTH_REGEN_RATE_FACTOR").copied().unwrap_or(1.0);
            self.set((self.current + dt * speed * factor).min(target), attributes);
        }
    }
}
