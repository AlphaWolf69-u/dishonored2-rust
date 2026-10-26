//! Shared movement interface for embedded actors and trigger components.
//! Their actual vtables use the same five methods. This is not a fallback for
//! rigid bodies or unrecognized component types.
use super::{ActorBody, ActorMotionBody, PhysicsActorLinks, ResolvedMotion};

pub enum BasicBody {
    Embedded,
    Trigger { body: usize },
}
impl BasicBody {
    fn selected<'a>(
        &self,
        actors: &'a PhysicsActorLinks,
        actor: usize,
    ) -> Result<&'a ActorBody, String> {
        let actor = actors
            .actors
            .get_at(actor)
            .ok_or("basic movement body actor is missing")?;
        let expected = match self {
            Self::Embedded => actor.default_body,
            Self::Trigger { body } => *body,
        };
        if actor.current_body != expected {
            return Err("selected movement body does not match its admitted interface".into());
        }
        actors
            .bodies
            .get(actor.current_body)
            .ok_or_else(|| "basic movement body is missing".into())
    }

    /// Vtable +10: both admitted body types accept pose updates (11d080).
    pub fn accepts_pose(&self, actors: &PhysicsActorLinks, actor: usize) -> Result<bool, String> {
        self.selected(actors, actor)?;
        Ok(true)
    }

    /// Vtable +20: the shared 399620 callback returns zero without side effects.
    pub fn update_pose(&self, actors: &PhysicsActorLinks, actor: usize) -> Result<bool, String> {
        self.selected(actors, actor)?;
        Ok(false)
    }

    /// Vtable +28: the same callback declines owning render-entity publication.
    pub fn handles_render_entity(
        &self,
        actors: &PhysicsActorLinks,
        actor: usize,
    ) -> Result<bool, String> {
        self.selected(actors, actor)?;
        Ok(false)
    }
}
impl ActorMotionBody for BasicBody {
    fn accepts(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<bool, String> {
        self.selected(actors, actor)?;
        Ok(true)
    }
    fn apply(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        _motion: ResolvedMotion,
    ) -> Result<u8, String> {
        // +18 is 493270, not a physics velocity setter. In particular, the
        // constructor leaves this embedded body's reciprocal owner null.
        Ok(u8::from(self.selected(actors, actor)?.actor.is_some()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::physics_actor::PhysicsActor;
    #[test]
    fn interface_matches_both_executed_actor_and_trigger_vtables() {
        let mut cases = 0;
        for line in include_str!("../../../tests/fixtures/basic_actor_body.txt").lines() {
            let words: Vec<_> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let index = words[0] as usize;
            let mut actor = PhysicsActor::new(0);
            actor.current_body = index;
            let mut actors = PhysicsActorLinks {
                actors: crate::gameplay::physics_actor::test_actor_pool(vec![actor]),
                bodies: (0..2)
                    .map(|_| ActorBody {
                        actor: (words[1] != 0).then_some(0),
                        body_id: u32::MAX,
                    })
                    .collect(),
            };
            let mut body = if index == 0 {
                BasicBody::Embedded
            } else {
                BasicBody::Trigger { body: index }
            };
            assert_eq!(
                [
                    u32::from(body.accepts(&mut actors, 0).unwrap()),
                    u32::from(body.accepts_pose(&actors, 0).unwrap()),
                    u32::from(
                        body.apply(&mut actors, 0, ResolvedMotion::default())
                            .unwrap()
                    ),
                    u32::from(body.update_pose(&actors, 0).unwrap()),
                    u32::from(body.handles_render_entity(&actors, 0).unwrap())
                ],
                words[2..]
            );
            cases += 1;
        }
        assert_eq!(cases, 512);
    }
}
