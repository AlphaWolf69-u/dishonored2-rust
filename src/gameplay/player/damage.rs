//! Borrow the actual player's damage recipient during state callbacks.
//! The same Health/Mana/event owners serve external and landing damage.
use super::{
    adrenaline::Adrenaline, controller::Controller, health::Health, mana::Mana, states::Attributes,
    EntityEvent,
};
use crate::gameplay::damage::{Context, DamageResult, Parameters, Receiver};
use bevy_math::Vec3;

pub struct Recipient<'a> {
    pub health: &'a mut Health,
    pub receiver: &'a Receiver,
    pub adrenaline: &'a mut Adrenaline,
    pub events: &'a mut Vec<EntityEvent>,
    pub game_over_fallback: &'a mut Option<f64>,
    pub event_time: f64,
    pub incoming_factor: f32,
    pub incoming_back_factor: f32,
}

impl Recipient<'_> {
    pub fn receive(
        &mut self,
        parameters: &Parameters,
        attributes: &Attributes,
        mana: &mut Mana,
        mana_replenishes: bool,
        controller: &mut Controller,
        forward: Vec3,
    ) -> DamageResult {
        let was_dead = self.health.is_dead();
        if was_dead {
            let result = Default::default();
            self.events.push(EntityEvent::Damage {
                params: *parameters,
                result,
                was_dead,
            });
            return result;
        }
        let attribute = |name: &str| attributes.0.get(name).copied().unwrap_or(1.0);
        let difficulty_factor = if forward.dot(parameters.hit_direction) <= 0.0 {
            self.incoming_factor
        } else {
            self.incoming_back_factor
        };
        let context = Context {
            forward,
            difficulty_factor,
            incoming_scale: attribute("ARK_ATTRIBUTE_INCOMING_DAMAGE_SCALE"),
            explosion_factor: attribute("ARK_ATTRIBUTE_EXPLOSION_DAMAGE_FACTOR"),
            health_to_mana: attributes.get("ARK_ATTRIBUTE_MALUS_HEALTH_TO_MANA_DAMAGES"),
            mana: mana.current(),
            evolved: true,
            ..Default::default()
        };
        let result = self
            .health
            .receive_damage(self.receiver, parameters, context, attributes);
        if result.mana_damage > 0.0 {
            mana.consume(result.mana_damage, attributes, mana_replenishes);
        }
        if result.incapacitated {
            self.adrenaline.stop();
            controller.input_move = Vec3::ZERO;
            controller.input_look = Vec3::ZERO;
            *self.game_over_fallback = Some(self.event_time + 7.0);
        }
        self.events.push(EntityEvent::Damage {
            params: *parameters,
            result,
            was_dead,
        });
        result
    }
}
