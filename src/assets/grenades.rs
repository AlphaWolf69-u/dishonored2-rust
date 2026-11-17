//! arkDeclCpntGrenade constructor490470 and reflection1416f4710.
//! Immutable component data; FX acquisition belongs to the shared FX owner.
use crate::{
    entities::Value,
    gameplay::damage::{atom, number},
};

#[derive(Debug)]
pub struct Definition {
    pub detonation_delay: f32,
    pub minimum_after_hit: f32,
    pub damaged_delay: f32,
    pub danger_radius: f32,
    pub minimum_on_pickup: f32,
    pub unbent_duration: f32,
    pub melee_impulse: f32,
    pub armed_fx: Option<String>,
    pub in_hand_fx: Option<String>,
    pub underwater_fx: Option<String>,
    pub explosion_fx: Option<String>,
    pub failed_explosion_fx: Option<String>,
    pub player_noise_attribute: Option<String>,
}
impl Definition {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let text = |name| {
            atom(fields, name)
                .filter(|v| !v.is_empty() && *v != "NULL")
                .map(str::to_owned)
        };
        Ok(Self {
            detonation_delay: number(fields, "m_detonationDelay", 3.)?,
            minimum_after_hit: number(fields, "m_minDetonationDelayAfterHit", 0.5)?,
            damaged_delay: number(
                fields,
                "m_explosionDelayWhenDamaged",
                f32::from_bits(0x3e4ccccd),
            )?,
            danger_radius: number(fields, "m_aiDangerZoneRadius", 4.)?,
            minimum_on_pickup: number(fields, "m_minDetonationDelayOnPickup", 1.5)?,
            unbent_duration: number(
                fields,
                "m_explosionUnbentDuration",
                f32::from_bits(0x3e99999a),
            )?,
            melee_impulse: number(fields, "m_meleeImpulse", 15.)?,
            armed_fx: text("m_armedFX"),
            in_hand_fx: text("m_armedFX_InHand"),
            underwater_fx: text("m_armedFX_Underwater"),
            explosion_fx: text("m_explosionFX"),
            failed_explosion_fx: text("m_failExplosionFX"),
            player_noise_attribute: text("m_playerNoiseFactorAttribute"),
        })
    }
    pub fn fx_paths(&self) -> impl Iterator<Item = &str> {
        [
            self.armed_fx.as_deref(),
            self.in_hand_fx.as_deref(),
            self.underwater_fx.as_deref(),
            self.explosion_fx.as_deref(),
            self.failed_explosion_fx.as_deref(),
        ]
        .into_iter()
        .flatten()
    }
}
