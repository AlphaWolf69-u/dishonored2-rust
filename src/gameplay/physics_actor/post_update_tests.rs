use super::*;
use crate::gameplay::{
    motion_requests::{request_snapshot, MotionRequest, MotionRequestInput, MotionRequestList},
    physics_actor::{ActorBody, ActorMotionBody, PhysicsActor, ResolvedMotion},
};
use std::cell::RefCell;

struct State {
    delta: f32,
    flags: u8,
    response: u8,
    accepts: bool,
    accepts_pose: bool,
    handles: bool,
    has_render: bool,
    has_binding: bool,
    body: PublishedFrame,
    bound: BoundFrame,
    joint: ReferenceFrame,
}
struct World {
    publication: i32,
    pre: i32,
    post: i32,
    mutation: u32,
    states: Vec<State>,
    entities: Vec<PublishedFrame>,
    renders: Vec<(ReferenceFrame, [f32; 3])>,
    events: RefCell<Vec<u32>>,
}
impl World {
    fn event(&self, words: impl IntoIterator<Item = u32>) {
        self.events.borrow_mut().extend(words);
    }
}
impl ActorMotionBody for World {
    fn accepts(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<bool, String> {
        self.event([
            0xb0,
            actor as u32,
            actors.actors[actor].current_body as u32,
            u32::from(actors.actors[actor].flags),
        ]);
        Ok(self.states[actor].accepts)
    }
    fn apply(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        motion: ResolvedMotion,
    ) -> Result<u8, String> {
        self.event([0xb1, actor as u32]);
        self.event(motion.velocities.into_iter().flatten().map(f32::to_bits));
        self.event(motion.valid.map(u32::from));
        if self.mutation & 1 != 0 {
            actors.actors[actor].flags ^= 0x40;
        }
        if self.mutation & 2 != 0 {
            actors.actors[actor].motion.valid[0] = !actors.actors[actor].motion.valid[0];
        }
        Ok(self.states[actor].response)
    }
}
impl PrePhysicsWorld for World {
    fn entity_delta(
        &mut self,
        actors: &mut PhysicsActorLinks,
        entity: usize,
    ) -> Result<f32, String> {
        self.event([0xc0, entity as u32, u32::from(actors.actors[entity].flags)]);
        Ok(self.states[entity].delta)
    }
    fn entity_flags(&self, entity: usize) -> Result<u8, String> {
        Ok(self.states[entity].flags)
    }
    fn bound_velocities(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        delta: f32,
    ) -> Result<[[f32; 3]; 2], String> {
        self.event([
            0xb2,
            actor as u32,
            u32::from(actors.actors[actor].binding.phase),
            delta.to_bits(),
        ]);
        Ok(self.states[actor].bound.velocities)
    }
    fn pre_queue_frame(&self) -> i32 {
        self.pre
    }
    fn enqueue_pre(&mut self, _: &mut PhysicsActorLinks, actor: usize) -> Result<(), String> {
        self.event([0xc2, actor as u32]);
        Ok(())
    }
    fn child_count(&self, entity: usize) -> Result<usize, String> {
        Ok(if entity == 0 { 2 } else { 0 })
    }
    fn child_entity(&self, _: usize, index: usize) -> Result<usize, String> {
        Ok(index + 1)
    }
    fn actor_for_entity(
        &mut self,
        _: &mut PhysicsActorLinks,
        entity: usize,
    ) -> Result<usize, String> {
        self.event([0xc1, entity as u32]);
        Ok(entity)
    }
}
impl PostPhysicsWorld for World {
    fn publication_frame(&self) -> i32 {
        self.event([0xc3]);
        self.publication
    }
    fn post_queue_frame(&self) -> i32 {
        self.post
    }
    fn entity_frame(&self, entity: usize) -> Result<PublishedFrame, String> {
        Ok(self.entities[entity])
    }
    fn publish_entity_frame(&mut self, entity: usize, frame: PublishedFrame) -> Result<(), String> {
        self.entities[entity] = frame;
        Ok(())
    }
    fn bound_frame(
        &mut self,
        _: &mut PhysicsActorLinks,
        actor: usize,
        delta: f32,
    ) -> Result<BoundFrame, String> {
        self.event([0xb2, actor as u32, 3, delta.to_bits()]);
        Ok(BoundFrame {
            pose: self.states[actor].bound.pose,
            velocities: self.states[actor].bound.velocities,
        })
    }
    fn accepts_pose(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
    ) -> Result<bool, String> {
        self.event([0xb4, actor as u32, u32::from(actors.actors[actor].flags)]);
        Ok(self.states[actor].accepts_pose)
    }
    fn update_body_pose(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
    ) -> Result<(), String> {
        self.event([0xb5, actor as u32]);
        let frame = self.states[actor].body;
        actors.actors[actor]
            .frame
            .publish(frame.pose, frame.velocities, self.publication);
        actors.actors[actor].frame.scale = frame.scale;
        Ok(())
    }
    fn body_handles_render_entity(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
    ) -> Result<bool, String> {
        self.event([0xb6, actor as u32, u32::from(actors.actors[actor].flags)]);
        if self.mutation & 4 != 0 {
            actors.actors[actor].flags ^= 0x20;
        }
        if self.mutation & 8 != 0 {
            actors.actors[actor].binding.entity = if actor == 0 { Some(3) } else { None };
            actors.actors[actor].parent = None;
        }
        Ok(self.states[actor].handles)
    }
    fn render_entity(&self, entity: usize) -> Result<Option<usize>, String> {
        Ok(self.states[entity].has_render.then_some(entity))
    }
    fn render_binding(&self, _: &PhysicsActorLinks, actor: usize) -> Result<Option<usize>, String> {
        Ok(self.states[actor].has_binding.then_some(actor))
    }
    fn refresh_render_position(
        &mut self,
        _: &mut PhysicsActorLinks,
        binding: usize,
    ) -> Result<(), String> {
        self.event([0xd2, binding as u32]);
        self.renders[binding].0.position = self.states[binding].bound.pose.position;
        Ok(())
    }
    fn refresh_render_basis(
        &mut self,
        _: &mut PhysicsActorLinks,
        binding: usize,
    ) -> Result<(), String> {
        self.event([0xd3, binding as u32]);
        self.renders[binding].0.basis = self.states[binding].bound.pose.basis;
        Ok(())
    }
    fn set_render_pose(&mut self, render: usize, pose: ReferenceFrame) -> Result<(), String> {
        self.renders[render].0 = pose;
        Ok(())
    }
    fn set_render_scale(&mut self, render: usize, scale: [f32; 3]) -> Result<(), String> {
        self.renders[render].1 = scale;
        Ok(())
    }
    fn notify_render_pose(
        &mut self,
        actors: &mut PhysicsActorLinks,
        render: usize,
    ) -> Result<(), String> {
        self.event([0xd0, render as u32]);
        if self.mutation & 16 != 0 {
            actors.actors[render].binding.joint = 0;
        }
        Ok(())
    }
    fn defer_render_pose(
        &mut self,
        _: &mut PhysicsActorLinks,
        render: usize,
    ) -> Result<(), String> {
        self.event([0xd1, render as u32]);
        Ok(())
    }
    fn post_joint_frame(
        &mut self,
        _: &mut PhysicsActorLinks,
        actor: usize,
        joint: i16,
    ) -> Result<ReferenceFrame, String> {
        self.event([0xd4, actor as u32, 3, joint as i32 as u32]);
        Ok(self.states[actor].joint)
    }
    fn enqueue_post(&mut self, _: &mut PhysicsActorLinks, actor: usize) -> Result<(), String> {
        self.event([0xc4, actor as u32]);
        Ok(())
    }
}

fn read_vec(words: &mut impl Iterator<Item = u32>) -> [f32; 3] {
    std::array::from_fn(|_| f32::from_bits(words.next().unwrap()))
}
fn read_pose(words: &mut impl Iterator<Item = u32>) -> ReferenceFrame {
    ReferenceFrame {
        position: read_vec(words),
        basis: std::array::from_fn(|_| read_vec(words)),
    }
}
fn read_frame(words: &mut impl Iterator<Item = u32>) -> PublishedFrame {
    PublishedFrame {
        pose: read_pose(words),
        scale: read_vec(words),
        velocities: std::array::from_fn(|_| read_vec(words)),
    }
}
fn append_pose(out: &mut Vec<u32>, pose: ReferenceFrame) {
    out.extend(pose.position.map(f32::to_bits));
    out.extend(pose.basis.into_iter().flatten().map(f32::to_bits));
}
fn append_frame(out: &mut Vec<u32>, frame: PublishedFrame) {
    append_pose(out, frame.pose);
    out.extend(frame.scale.map(f32::to_bits));
    out.extend(frame.velocities.into_iter().flatten().map(f32::to_bits));
}
fn snapshot(actors: &PhysicsActorLinks, world: &World) -> Vec<u32> {
    let mut out = Vec::new();
    for (i, actor) in actors.actors.iter().enumerate() {
        out.extend([
            u32::from(actor.flags),
            actor.current_body as u32,
            actor.last_queued_frame as u32,
            actor.last_post_frame as u32,
            actor.frame.marker as u32,
            actor.binding.entity.map_or(u32::MAX, |e| e as u32),
            actor.binding.joint as i32 as u32,
            u32::from(actor.binding.phase),
        ]);
        append_frame(&mut out, actor.frame.published());
        append_pose(&mut out, actor.frame.predicted);
        out.extend(
            actor
                .motion
                .velocities
                .into_iter()
                .flatten()
                .map(f32::to_bits),
        );
        out.extend(actor.motion.valid.map(u32::from));
        out.extend(actor.motion.dirty.map(u32::from));
        out.extend(actor.binding.offset.map(f32::to_bits));
        out.extend(actor.binding.orientation.map(f32::to_bits));
        for list in &actor.requests {
            let data = request_snapshot(list);
            out.extend_from_slice(&data[..3]);
            out.push(u32::from(list.enabled));
            out.extend_from_slice(&data[3..]);
        }
        append_frame(&mut out, world.entities[i]);
        append_pose(&mut out, world.renders[i].0);
        out.extend(world.renders[i].1.map(f32::to_bits));
    }
    let events = world.events.borrow();
    out.push(events.len() as u32);
    out.extend_from_slice(&events);
    out
}

#[test]
fn complete_post_physics_publication_matches_executed_mutable_world_sequence() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/actor_post_update.txt").lines() {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let publication = words.next().unwrap() as i32;
        let pre = words.next().unwrap() as i32;
        let post = words.next().unwrap() as i32;
        let mutation = words.next().unwrap();
        let extra = read_frame(&mut words);
        let mut world = World {
            publication,
            pre,
            post,
            mutation,
            states: Vec::new(),
            entities: Vec::new(),
            renders: Vec::new(),
            events: RefCell::new(Vec::new()),
        };
        let mut actors = PhysicsActorLinks {
            actors: crate::gameplay::physics_actor::test_actor_pool(Vec::new()),
            bodies: (0..6)
                .map(|i| ActorBody {
                    actor: Some(i % 3),
                    body_id: i as u32,
                })
                .collect(),
        };
        for i in 0..3 {
            let flags = words.next().unwrap() as u8;
            let phase = words.next().unwrap() as u8;
            let bound = words.next().unwrap();
            let bound = (bound != u32::MAX).then_some(bound as usize);
            let joint = words.next().unwrap() as i16;
            let delta = f32::from_bits(words.next().unwrap());
            let entity_flags = words.next().unwrap() as u8;
            let response = words.next().unwrap() as u8;
            let accepts = words.next().unwrap() != 0;
            let accepts_pose = words.next().unwrap() != 0;
            let handles = words.next().unwrap() != 0;
            let has_render = words.next().unwrap() != 0;
            let has_binding = words.next().unwrap() != 0;
            let current_body = words.next().unwrap() as usize;
            let marker = words.next().unwrap() as i32;
            let last_pre = words.next().unwrap() as i32;
            let last_post = words.next().unwrap() as i32;
            let entity_frame = read_frame(&mut words);
            let actor_frame = read_frame(&mut words);
            let predicted = read_pose(&mut words);
            let body = read_frame(&mut words);
            let bound_frame = BoundFrame {
                pose: read_pose(&mut words),
                velocities: std::array::from_fn(|_| read_vec(&mut words)),
            };
            let joint_frame = read_pose(&mut words);
            let render = (read_pose(&mut words), read_vec(&mut words));
            let motion = ResolvedMotion {
                velocities: std::array::from_fn(|_| read_vec(&mut words)),
                valid: std::array::from_fn(|_| words.next().unwrap() != 0),
                dirty: std::array::from_fn(|_| words.next().unwrap() != 0),
            };
            let offset = read_vec(&mut words);
            let orientation = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
            let requests = std::array::from_fn(|axis| {
                let count = words.next().unwrap();
                let enabled = words.next().unwrap() != 0;
                let priority = words.next().unwrap() as i32;
                let remaining = f32::from_bits(words.next().unwrap());
                let velocity = read_vec(&mut words);
                let entries = (0..count)
                    .map(|_| MotionRequest {
                        id: 100 + axis as u32,
                        input: MotionRequestInput {
                            velocity,
                            priority,
                            remaining,
                            lifetime: 0,
                            flags: 0,
                        },
                    })
                    .collect();
                MotionRequestList::from_parts(entries, 0, 0, u32::MAX, enabled).unwrap()
            });
            let mut actor = PhysicsActor::new(i);
            actor.owner.entity = Some(i);
            actor.current_body = current_body;
            actor.flags = flags;
            actor.last_queued_frame = last_pre;
            actor.last_post_frame = last_post;
            actor.binding = super::super::ActorBinding {
                entity: bound,
                joint,
                offset,
                orientation,
                phase,
            };
            actor.parent = bound.filter(|&p| p < 3);
            actor.children = if i == 0 { vec![1, 2] } else { Vec::new() };
            actor
                .frame
                .publish(actor_frame.pose, actor_frame.velocities, marker);
            actor.frame.scale = actor_frame.scale;
            actor.frame.predicted = predicted;
            actor.motion = motion;
            actor.requests = requests;
            actors.actors.allocate(|_| Ok(actor)).unwrap();
            world.entities.push(entity_frame);
            world.renders.push(render);
            world.states.push(State {
                delta,
                flags: entity_flags,
                response,
                accepts,
                accepts_pose,
                handles,
                has_render,
                has_binding,
                body,
                bound: bound_frame,
                joint: joint_frame,
            });
        }
        world.entities.push(extra);
        actors.post_update(0, &mut world).unwrap();
        let actual = snapshot(&actors, &world);
        let expected = words.collect::<Vec<_>>();
        assert_eq!(actual.len(), expected.len(), "case {cases}: output length");
        if let Some((index, (a, b))) = actual
            .iter()
            .zip(&expected)
            .enumerate()
            .find(|(_, (a, b))| a != b)
        {
            panic!("case {cases}, output word {index}: got {a:08x}, expected {b:08x}");
        }
        cases += 1;
    }
    assert_eq!(cases, 512);
}
