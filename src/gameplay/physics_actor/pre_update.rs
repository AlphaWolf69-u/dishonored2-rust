//! Full 7545a0 pre-physics traversal. The entity/clock/binding/queue owner is
//! required; it must supply actual live entities and the selected body virtuals.
use super::{ActorMotionBody, PhysicsActorLinks};

pub trait PrePhysicsWorld: ActorMotionBody {
    fn entity_delta(
        &mut self,
        actors: &mut PhysicsActorLinks,
        entity: usize,
    ) -> Result<f32, String>;
    /// Common entity flags at +d0; bit7 admits stopped-time binding updates.
    fn entity_flags(&self, entity: usize) -> Result<u8, String>;
    /// 7506d0's bound-pose and velocity calculation, using live phase/joint data.
    fn bound_velocities(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        delta: f32,
    ) -> Result<[[f32; 3]; 2], String>;
    fn pre_queue_frame(&self) -> i32;
    fn enqueue_pre(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<(), String>;
    fn child_count(&self, entity: usize) -> Result<usize, String>;
    fn child_entity(&self, entity: usize, index: usize) -> Result<usize, String>;
    /// 8783d0 includes lazy actor creation; a missing cached actor is not a skip.
    fn actor_for_entity(
        &mut self,
        actors: &mut PhysicsActorLinks,
        entity: usize,
    ) -> Result<usize, String>;
}

impl PhysicsActorLinks {
    /// 7551c0's existing task admission dispatches its admitted actors through
    /// the full pre-physics traversal, with the same mutable entity/world owner.
    pub fn consume_pre_updates(
        &mut self,
        task: &crate::gameplay::physics_world::update_queue::ObjectUpdateTask<usize>,
        world: &mut impl PrePhysicsWorld,
    ) -> Result<(), String> {
        self.consume_updates(
            task,
            super::ActorUpdatePhase::PrePhysics,
            |actors, actor, _| actors.pre_update(actor, world),
        )
    }

    pub fn pre_update(
        &mut self,
        actor: usize,
        world: &mut impl PrePhysicsWorld,
    ) -> Result<(), String> {
        self.pre_update_inner(actor, world, &mut Vec::new())
    }

    fn pre_update_inner(
        &mut self,
        actor: usize,
        world: &mut impl PrePhysicsWorld,
        path: &mut Vec<usize>,
    ) -> Result<(), String> {
        if path.contains(&actor) {
            return Err("cyclic entity pre-physics traversal".into());
        }
        path.push(actor);
        let result = (|| {
            let value = self
                .actors
                .get_at_mut(actor)
                .ok_or("pre-physics actor is missing")?;
            if value.flags & 2 == 0 {
                value.flags &= !4;
            }
            value.flags &= !3;
            let entity = value
                .owner
                .entity
                .ok_or("pre-physics actor has no owning entity")?;
            let bound = self.actors[actor].binding.entity;
            let delta = world.entity_delta(self, entity)?;
            // COMISS/JBE takes the stopped-time branch for NaN as well as <=0.
            if delta > 0. {
                if self.actors[actor].flags & 0x20 != 0 {
                    if !self.resolve_motion(actor, delta, world)?
                        && bound.is_some()
                        && world.accepts(self, actor)?
                    {
                        let velocities = world.bound_velocities(self, actor, delta)?;
                        self.actors[actor].motion.velocities = velocities;
                        self.actors[actor].motion.valid = [true; 2];
                        self.actors[actor].motion.dirty = [true; 2];
                        let motion = self.actors[actor].motion;
                        let response = world.apply(self, actor, motion)?;
                        let flags = &mut self.actors[actor].flags;
                        *flags = (*flags & !1) | (response & 1) | 8;
                    }
                    self.enqueue_enabled_pre_motion(actor, world)?;
                }
            } else {
                if world.entity_flags(entity)? & 0x80 != 0 {
                    self.actors[actor].flags |= 0x10;
                }
                self.enqueue_enabled_pre_motion(actor, world)?;
            }
            let count = world.child_count(entity)?;
            for index in 0..count {
                // The source caches the count but reloads the live child pointer.
                let child = world.child_entity(entity, index)?;
                let child = world.actor_for_entity(self, child)?;
                self.pre_update_inner(child, world, path)?;
            }
            Ok(())
        })();
        path.pop();
        result
    }

    fn enqueue_enabled_pre_motion(
        &mut self,
        actor: usize,
        world: &mut impl PrePhysicsWorld,
    ) -> Result<(), String> {
        if self.actors[actor].flags & 0x20 != 0 && self.actors[actor].has_enabled_motion() {
            self.request_update(actor, world.pre_queue_frame(), |actors, actor| {
                world.enqueue_pre(actors, actor)
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "pre_update_tests.rs"]
mod tests;
