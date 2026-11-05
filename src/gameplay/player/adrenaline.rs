//! The player's adrenaline component (14049afd0/14049ba60).
use super::states::Attributes;

#[derive(Clone, Copy)]
pub struct Settings { pub start_on_spawn: bool, pub maximum: f32, pub reduce_delay: f32, pub reduce_rate: f32, pub burst_rate: f32 }

pub struct Adrenaline { settings: Settings, started: bool, value: f32, countdown: f32, suppression_base: Option<f32>, burst_base: Option<f32> }
impl Adrenaline {
    pub fn new(settings: Settings, attributes: &Attributes) -> Self {
        Self { started: settings.start_on_spawn, settings, value: 0.0, countdown: 0.0,
            suppression_base: attributes.0.get("ARK_ATTRIBUTE_BLOODTHIRSTY_SUPPRESSED").copied(),
            burst_base: attributes.0.get("ARK_ATTRIBUTE_ADRENALINE_BURST_ENABLED").copied() }
    }
    pub fn value(&self) -> f32 { self.value }
    pub fn is_started(&self) -> bool { self.started }
    pub fn start(&mut self) { self.started = true; }
    pub fn stop(&mut self) { self.started = false; }
    fn suppressed(&self, attributes: &Attributes) -> bool {
        attributes.0.get("ARK_ATTRIBUTE_BLOODTHIRSTY_SUPPRESSED").copied() != self.suppression_base
    }
    pub fn gain(&mut self, amount: f32, attributes: &Attributes, bloodthirsty_active: bool) {
        if !self.started || amount == 0.0 || bloodthirsty_active || self.suppressed(attributes) { return; }
        let factor = attributes.0.get("ARK_ATTRIBUTE_ADRENALINE_GAIN_FACTOR").copied().unwrap_or(1.0);
        self.value = (self.value + amount * factor).clamp(0.0, self.settings.maximum);
        let cooldown = attributes.0.get("ARK_ATTRIBUTE_ADRENALINE_COOLDOWN_FACTOR").copied().unwrap_or(1.0);
        self.countdown = cooldown * self.settings.reduce_delay;
    }
    pub fn update(&mut self, dt: f32, attributes: &Attributes, in_combat: bool) {
        if !self.started { return; }
        if !self.suppressed(attributes) { self.countdown -= dt; }
        let burst = attributes.0.get("ARK_ATTRIBUTE_ADRENALINE_BURST_ENABLED").copied() != self.burst_base;
        if burst && self.value / self.settings.maximum <= 0.5 && !in_combat {
            let factor = attributes.0.get("ARK_ATTRIBUTE_ADRENALINE_GAIN_FACTOR").copied().unwrap_or(1.0);
            self.value = (self.value + dt * self.settings.burst_rate * factor).clamp(0.0, self.settings.maximum);
            return;
        }
        if self.countdown <= 0.0 && self.value > 0.0 {
            self.value = (self.value - dt * self.settings.reduce_rate).max(0.0);
        }
    }
}
