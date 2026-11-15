//! Immediate potion actions (7194a0/745b20), backed by the existing player's
//! Health/Mana and shared random seed. These actions do not start a midnight:
//! their +d8/+80 resources are consumeSoundEvent declarations.
use super::Player;
use crate::assets::{archive::Catalog, entities::Value, inventory_item::PickableItem, wwise::SoundEventDeclaration};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PotionKind { Health, Mana }

pub trait PotionInventory {
    fn potion_item(&self, kind: PotionKind) -> Result<Option<Arc<PickableItem>>, String>;
    fn consume_potion(&mut self, path: &str) -> bool;
}

/// Prepared from the already resolved player declarations and actual
/// difficulty manager. Sound declarations remain owned by Wwise resources.
pub struct PotionSettings {
    pub health_sound: Option<Arc<SoundEventDeclaration>>,
    pub mana_sound: Option<Arc<SoundEventDeclaration>>,
    pub health_effect: f32,
    pub mana_effect: f32,
    pub mana_speed: f32,
    pub suppress_sound: bool,
}
impl PotionSettings {
    pub fn sounds(catalog: &Catalog, health: &[(String, Value)], mana: &[(String, Value)])
        -> Result<[Option<Arc<SoundEventDeclaration>>; 2], String> {
        let sound = |fields: &[(String, Value)]| -> Result<_, String> {
            let Some(Value::Atom(name)) = fields.iter().find(|(key, _)| key == "m_consumeSoundEvent").map(|(_, value)| value) else { return Ok(None) };
            let name = name.trim_matches('"');
            if name.is_empty() || name.eq_ignore_ascii_case("NULL") { return Ok(None); }
            crate::assets::wwise::event_resources::acquire_with(catalog, name, |path| {
                catalog.read(path).map(|bytes| bytes.map(crate::assets::entities::owned_declaration_text)).map_err(|error| error.to_string())
            }).map(Some)
        };
        Ok([sound(health)?, sound(mana)?])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PotionEffect { Health, Mana, ManaEmpty, ManaFull }

pub struct PotionResult {
    pub effect: PotionEffect,
    /// Native 7194a0 queues player message 15c only for health potions.
    pub health_used_message: bool,
    /// Statistics 16/17 are skipped by the two mana random early returns.
    pub statistic: Option<u32>,
    /// MsgA9 at the entity (handle -1), unless original system mode is 8.
    pub sound: Option<Arc<SoundEventDeclaration>>,
}

impl Player {
    pub fn inventory_random(&mut self) -> f32 { crate::random::lcg_unit(&mut self.random) }
    /// Positive 71a140 ->719fe0 pickup gain; signed negative food is sent
    /// through the actual damage declaration/event owner by the app adapter.
    pub fn inventory_health_gain(&mut self, amount: f32) { self.health.gain(amount, &self.attributes); }
    pub fn inventory_mana_gain(&mut self, amount: f32) { self.mana.gain(amount, &self.attributes); }

    pub fn can_use_potion(&self, kind: PotionKind) -> bool {
        match kind {
            PotionKind::Health => self.health.can_use_potion(&self.attributes, self.health_regen_factor),
            PotionKind::Mana => self.mana.can_use_potion(&self.attributes),
        }
    }

    /// The native operation first rechecks eligibility, then removes one
    /// actual item, then applies the gain. Quantity is never charged on a
    /// timer or animation completion, so an interrupted sound cannot refund it.
    pub fn use_potion(&mut self, kind: PotionKind, inventory: &mut impl PotionInventory,
        settings: &PotionSettings) -> Result<Option<PotionResult>, String> {
        // 399620 is an executable constant-false getter in this build.
        // Although custom difficulty declares m_mergeHealthMana, the native
        // potion cross-component branches are unreachable here.
        if !self.can_use_potion(kind) { return Ok(None); }
        let Some(item) = inventory.potion_item(kind)? else { return Ok(None) };
        if !inventory.consume_potion(&item.path) { return Ok(None); }
        let attribute = |player: &Self, name: &str, default: f32| player.attributes.0.get(name).copied().unwrap_or(default);
        match kind {
            PotionKind::Health => {
                let amount = item.consumable.health as f32 * settings.health_effect
                    * attribute(self, "ARK_ATTRIBUTE_POTION_HEALTH_FACTOR", 1.0)
                    + attribute(self, "ARK_ATTRIBUTE_POTION_HEALTH_GAIN", 0.0);
                self.health.gain(amount, &self.attributes);
                Ok(Some(PotionResult { effect: PotionEffect::Health, health_used_message: true,
                    statistic: Some(0x16), sound: (!settings.suppress_sound).then(|| settings.health_sound.clone()).flatten() }))
            }
            PotionKind::Mana => {
                // 745b20 draws integer percentages in [0,100], and only
                // draws at all when the corresponding truncated attribute
                // is nonzero. This consumes the same seed as player gameplay.
                let empty = attribute(self, "ARK_ATTRIBUTE_POTION_EMPTY_MANA_CHANCE", 0.0) as i32;
                if empty != 0 && ((((crate::random::lcg_step(&mut self.random) >> 10) & 0x7fff) % 101) as i32) < empty {
                    self.mana.consume_with_replenish(self.mana.current(), &self.attributes,
                        self.mana_replenishes.then_some(0.0));
                    return Ok(Some(PotionResult { effect: PotionEffect::ManaEmpty, health_used_message: false, statistic: None, sound: None }));
                }
                let full = attribute(self, "ARK_ATTRIBUTE_POTION_FULL_MANA_CHANCE", 0.0) as i32;
                if full != 0 && ((((crate::random::lcg_step(&mut self.random) >> 10) & 0x7fff) % 101) as i32) < full {
                    let amount = self.mana.maximum(&self.attributes);
                    self.mana.begin_potion(amount, settings.mana_speed, &self.attributes, false);
                    return Ok(Some(PotionResult { effect: PotionEffect::ManaFull, health_used_message: false, statistic: None, sound: None }));
                }
                let amount = item.consumable.mana as f32 * settings.mana_effect
                    * attribute(self, "ARK_ATTRIBUTE_POTION_MANA_FACTOR", 1.0)
                    + attribute(self, "ARK_ATTRIBUTE_POTION_MANA_GAIN", 0.0);
                self.mana.begin_potion(amount, settings.mana_speed, &self.attributes, true);
                Ok(Some(PotionResult { effect: PotionEffect::Mana, health_used_message: false,
                    statistic: Some(0x17), sound: (!settings.suppress_sound).then(|| settings.mana_sound.clone()).flatten() }))
            }
        }
    }
}
