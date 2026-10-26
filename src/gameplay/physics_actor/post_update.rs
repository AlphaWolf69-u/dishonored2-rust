//! 753c10 publication/traversal, sharing the real request and prediction owners.
use super::{pre_update::PrePhysicsWorld, ActorUpdatePhase, PhysicsActorLinks};
use crate::gameplay::reference_frame::{relative_binding, PublishedFrame, ReferenceFrame};

pub struct BoundFrame {
    pub pose: ReferenceFrame,
    pub velocities: [[f32; 3]; 2],
}

#[cfg(test)]
#[path = "post_update_tests.rs"]
mod tests;

/// All external owners are required. Concrete body, entity, render and joint
/// implementations must preserve live callback mutations; none has a no-op default.
pub trait PostPhysicsWorld: PrePhysicsWorld {
    fn publication_frame(&self) -> i32;
    fn post_queue_frame(&self) -> i32;
    fn entity_frame(&self, entity: usize) -> Result<PublishedFrame, String>;
    fn publish_entity_frame(&mut self, entity: usize, frame: PublishedFrame) -> Result<(), String>;
    /// 7506d0 with phase3 and the supplied delta.
    fn bound_frame(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        delta: f32,
    ) -> Result<BoundFrame, String>;
    fn accepts_pose(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
    ) -> Result<bool, String>;
    fn update_body_pose(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
    ) -> Result<(), String>;
    fn body_handles_render_entity(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
    ) -> Result<bool, String>;
    fn render_entity(&self, entity: usize) -> Result<Option<usize>, String>;
    fn render_binding(
        &self,
        actors: &PhysicsActorLinks,
        actor: usize,
    ) -> Result<Option<usize>, String>;
    /// 541420 and541390, in that order, on the retained render binding owner.
    fn refresh_render_position(
        &mut self,
        actors: &mut PhysicsActorLinks,
        binding: usize,
    ) -> Result<(), String>;
    fn refresh_render_basis(
        &mut self,
        actors: &mut PhysicsActorLinks,
        binding: usize,
    ) -> Result<(), String>;
    fn set_render_pose(&mut self, render: usize, pose: ReferenceFrame) -> Result<(), String>;
    fn set_render_scale(&mut self, render: usize, scale: [f32; 3]) -> Result<(), String>;
    /// Render entity virtual28(mode1) and virtual38 are distinct notifications.
    fn notify_render_pose(
        &mut self,
        actors: &mut PhysicsActorLinks,
        render: usize,
    ) -> Result<(), String>;
    fn defer_render_pose(
        &mut self,
        actors: &mut PhysicsActorLinks,
        render: usize,
    ) -> Result<(), String>;
    /// 53d6a0's live phase3 local joint frame, through the retained model owner.
    fn post_joint_frame(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        joint: i16,
    ) -> Result<ReferenceFrame, String>;
    fn enqueue_post(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<(), String>;
}

impl PhysicsActorLinks {
    pub fn consume_post_updates(
        &mut self,
        task: &crate::gameplay::physics_world::update_queue::ObjectUpdateTask<usize>,
        world: &mut impl PostPhysicsWorld,
    ) -> Result<(), String> {
        self.consume_updates(task, ActorUpdatePhase::PostPhysics, |actors, actor, _| {
            actors.post_update(actor, world)
        })
    }

    pub fn post_update(
        &mut self,
        actor: usize,
        world: &mut impl PostPhysicsWorld,
    ) -> Result<(), String> {
        self.post_update_inner(actor, world, &mut Vec::new())
    }

    fn post_update_inner(
        &mut self,
        actor: usize,
        world: &mut impl PostPhysicsWorld,
        path: &mut Vec<usize>,
    ) -> Result<(), String> {
        if path.contains(&actor) {
            return Err("cyclic entity post-physics traversal".into());
        }
        path.push(actor);
        let result = (|| {
            let marker = world.publication_frame();
            let entity = self
                .actors
                .get_at(actor)
                .ok_or("post-physics actor is missing")?
                .owner
                .entity
                .ok_or("post-physics actor has no owning entity")?;
            let delta = world.entity_delta(self, entity)?;
            let mut changed_binding = None;
            if delta > 0. {
                let resolved = self.resolve_motion(actor, delta, world)?;
                let bound = self.actors[actor].binding.entity;
                if let Some(bound) = bound {
                    if resolved {
                        if self.actors[actor].flags & 8 == 0 {
                            changed_binding = Some(bound);
                        }
                    } else if self.actors[actor].flags & 0x20 == 0
                        && self.actors[actor].frame.marker != marker
                        && world.accepts(self, actor)?
                    {
                        self.actors[actor].motion.velocities =
                            world.bound_frame(self, actor, delta)?.velocities;
                        self.actors[actor].motion.valid = [true; 2];
                        self.actors[actor].motion.dirty = [true; 2];
                    }
                }
                if self.actors[actor].current_body == self.actors[actor].default_body {
                    if self.actors[actor].frame.marker != marker {
                        let position = self.post_predicted_position(actor, entity, world)?;
                        let basis = self.post_predicted_basis(actor, entity, world)?;
                        let value = &mut self.actors[actor];
                        let velocities = std::array::from_fn(|i| {
                            if value.motion.valid[i] {
                                value.motion.velocities[i]
                            } else {
                                [0.; 3]
                            }
                        });
                        value
                            .frame
                            .publish(ReferenceFrame { position, basis }, velocities, marker);
                        self.actors[actor].frame.scale = world.entity_frame(entity)?.scale;
                    }
                } else if self.actors[actor].frame.marker != marker
                    && world.accepts_pose(self, actor)?
                {
                    world.update_body_pose(self, actor)?;
                }
            } else if world.entity_flags(entity)? & 0x80 != 0
                && self.actors[actor].flags & 0x10 != 0
                && self.actors[actor].frame.marker != marker
            {
                let bound = world.bound_frame(self, actor, 1.)?;
                self.actors[actor]
                    .frame
                    .publish(bound.pose, [[0.; 3]; 2], marker);
            }
            self.actors[actor].flags &= !0x14;
            let previous = world.entity_frame(entity)?;
            let published = self.actors[actor].frame.published();
            // 09b460 is the standard ordered float equality of all nine words.
            let pose_changed = previous.pose.position != published.pose.position
                || previous.pose.basis != published.pose.basis;
            let scale_changed = previous.scale != published.scale;
            world.publish_entity_frame(entity, published)?;
            self.actors[actor].motion.valid = [false; 2];
            self.actors[actor].motion.dirty = [true; 2];
            let render = world.render_entity(entity)?;
            if !world.body_handles_render_entity(self, actor)? {
                if let Some(render) = render {
                    // 878c60/878d10 read bits0/1 of this same entity flag byte.
                    if world.entity_flags(entity)? & 3 != 0 {
                        world.defer_render_pose(self, render)?;
                    } else {
                        let binding = world.render_binding(self, actor)?;
                        if let Some(binding) = binding {
                            world.refresh_render_position(self, binding)?;
                            world.refresh_render_basis(self, binding)?;
                        } else if pose_changed || scale_changed {
                            world.set_render_pose(render, world.entity_frame(entity)?.pose)?;
                        }
                        if binding.is_some() || pose_changed || scale_changed {
                            world.set_render_scale(render, world.entity_frame(entity)?.scale)?;
                            world.notify_render_pose(self, render)?;
                        }
                    }
                }
            }
            if let Some(parent) = changed_binding {
                // Keep the parent identity captured after motion resolution,
                // while reading the joint and parent pose after render callbacks.
                let joint = self.actors[actor].binding.joint;
                let parent = if joint < 0 {
                    world.entity_frame(parent)?.pose
                } else {
                    let local = world.post_joint_frame(self, actor, joint)?;
                    crate::gameplay::reference_frame::compose(
                        world.entity_frame(parent)?.pose,
                        local,
                    )
                };
                let (offset, orientation) =
                    relative_binding(parent, world.entity_frame(entity)?.pose);
                self.actors[actor].binding.offset = offset;
                self.actors[actor].binding.orientation = orientation;
            }
            self.actors[actor].flags |= 2;
            if self.actors[actor].has_enabled_motion() {
                if self.actors[actor].flags & 0x20 == 0 {
                    self.request_phase_update(
                        actor,
                        world.post_queue_frame(),
                        ActorUpdatePhase::PostPhysics,
                        |actors, actor| world.enqueue_post(actors, actor),
                    )?;
                } else {
                    self.request_update(actor, world.pre_queue_frame(), |actors, actor| {
                        world.enqueue_pre(actors, actor)
                    })?;
                }
            }
            let count = world.child_count(entity)?;
            for index in 0..count {
                let child = world.child_entity(entity, index)?;
                let child = world.actor_for_entity(self, child)?;
                self.post_update_inner(child, world, path)?;
            }
            Ok(())
        })();
        path.pop();
        result
    }

    fn post_predicted_position(
        &mut self,
        actor: usize,
        entity: usize,
        world: &mut impl PostPhysicsWorld,
    ) -> Result<[f32; 3], String> {
        if self.actors[actor].frame.marker == world.publication_frame() {
            return Ok(self.actors[actor].frame.current.position);
        }
        if self.actors[actor].motion.dirty[0] {
            let delta = world.entity_delta(self, entity)?;
            let source = world.entity_frame(entity)?.pose;
            let value = &mut self.actors[actor];
            value
                .frame
                .predict_position(source, &mut value.motion, delta);
        }
        Ok(self.actors[actor].frame.predicted.position)
    }

    fn post_predicted_basis(
        &mut self,
        actor: usize,
        entity: usize,
        world: &mut impl PostPhysicsWorld,
    ) -> Result<[[f32; 3]; 3], String> {
        if self.actors[actor].frame.marker == world.publication_frame() {
            return Ok(self.actors[actor].frame.current.basis);
        }
        if self.actors[actor].motion.dirty[1] {
            let delta = world.entity_delta(self, entity)?;
            let source = world.entity_frame(entity)?.pose;
            let value = &mut self.actors[actor];
            value.frame.predict_basis(source, &mut value.motion, delta);
        }
        Ok(self.actors[actor].frame.predicted.basis)
    }
}
