//! Typed trigger state constructed by7792c0. Movement ownership stays in the
//! same body-interface table used by reference-frame actors.
use crate::gameplay::{
    collision_user_data::velocity::CollisionVelocities,
    component_state::{entity::ComponentHandle, ComponentInstance, ComponentOwner},
    physics_actor::ActorBody,
};

pub struct TriggerComponent {
    pub owner: ComponentOwner,
    /// Index of the trigger's +20 movement interface in the level's shared table.
    /// Its body_id is the sole equivalent of trigger+30, including live changes.
    pub movement_body: usize,
    pub velocities: CollisionVelocities,
    pub water: Option<ComponentHandle>,
    pub wind: Option<ComponentHandle>,
    /// Source+d0: initialized false, copied from owner bit30 by779530.
    pub spawn_owner_bit30: bool,
}

impl TriggerComponent {
    pub fn new(movement_bodies: &mut Vec<ActorBody>) -> Result<Self, String> {
        movement_bodies
            .try_reserve(1)
            .map_err(|e| format!("trigger movement interface allocation failed: {e}"))?;
        let movement_body = movement_bodies.len();
        movement_bodies.push(ActorBody {
            actor: None,
            body_id: 0x00ff_ffff,
        });
        Ok(Self {
            owner: ComponentOwner::default(),
            movement_body,
            velocities: CollisionVelocities::default(),
            water: None,
            wind: None,
            spawn_owner_bit30: false,
        })
    }
}

impl ComponentInstance for TriggerComponent {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_constructor_matches_executed_state_and_uses_shared_body_identity() {
        let mut bodies = vec![ActorBody {
            actor: Some(7),
            body_id: 0x12000009,
        }];
        let trigger = TriggerComponent::new(&mut bodies).unwrap();
        assert_eq!(trigger.movement_body, 1);
        assert_eq!(bodies[0].actor, Some(7));
        assert_eq!(bodies[0].body_id, 0x12000009);
        let body = &bodies[trigger.movement_body];
        let mut actual = vec![
            u32::from(trigger.owner.declaration.is_some()),
            u32::from(trigger.owner.entity.is_some()),
            trigger.owner.generation,
            trigger.owner.flags,
            u32::from(body.actor.is_some()),
            body.body_id,
        ];
        for v in [
            trigger.velocities.scaled_linear,
            trigger.velocities.scaled_angular,
            trigger.velocities.requested_linear,
            trigger.velocities.requested_angular,
        ] {
            actual.extend(v.map(f32::to_bits));
        }
        actual.extend([
            u32::from(trigger.water.is_some()),
            u32::from(trigger.wind.is_some()),
            u32::from(trigger.spawn_owner_bit30),
        ]);
        let mut count = 0;
        for row in include_str!("../../../tests/fixtures/trigger_component.txt").lines() {
            let expected: Vec<_> = row
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            assert_eq!(actual, expected);
            count += 1;
        }
        assert_eq!(count, 16);
    }
}
