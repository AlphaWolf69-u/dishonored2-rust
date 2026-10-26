use super::*;
use crate::gameplay::{
    component_state::ComponentOwner,
    motion_requests::{request_snapshot, MotionRequest, MotionRequestInput, MotionRequestList},
    physics_actor::{ActorBody, PhysicsActor, ResolvedMotion},
};

struct EntityState {
    delta: f32,
    stopped: bool,
    response: u8,
    accepts: bool,
}
struct World {
    entities: Vec<EntityState>,
    frame: i32,
    mutation: u32,
    events: Vec<u32>,
}
impl ActorMotionBody for World {
    fn accepts(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<bool, String> {
        let value = &actors.actors[actor];
        self.events.extend([
            0xb0,
            actor as u32,
            value.current_body as u32,
            u32::from(value.flags),
        ]);
        Ok(self.entities[actor].accepts)
    }
    fn apply(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        motion: ResolvedMotion,
    ) -> Result<u8, String> {
        self.events.extend([0xb1, actor as u32]);
        self.events
            .extend(motion.velocities.iter().flatten().map(|v| v.to_bits()));
        self.events.extend(motion.valid.map(u32::from));
        if self.mutation & 1 != 0 {
            actors.actors[actor].flags ^= 0x40;
        }
        if self.mutation & 2 != 0 {
            actors.actors[actor].motion.valid[0] = !actors.actors[actor].motion.valid[0];
        }
        Ok(self.entities[actor].response)
    }
}
impl PrePhysicsWorld for World {
    fn entity_delta(
        &mut self,
        actors: &mut PhysicsActorLinks,
        entity: usize,
    ) -> Result<f32, String> {
        self.events
            .extend([0xc0, entity as u32, u32::from(actors.actors[entity].flags)]);
        Ok(self.entities[entity].delta)
    }
    fn entity_flags(&self, entity: usize) -> Result<u8, String> {
        Ok(if self.entities[entity].stopped {
            0x80
        } else {
            0
        })
    }
    fn bound_velocities(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        delta: f32,
    ) -> Result<[[f32; 3]; 2], String> {
        self.events.extend([
            0xb2,
            actor as u32,
            u32::from(actors.actors[actor].binding.phase),
            delta.to_bits(),
        ]);
        let a = actor as f32;
        Ok([[10. + a, 20. + a, 30. + a], [-1. - a, -2. - a, -3. - a]])
    }
    fn pre_queue_frame(&self) -> i32 {
        self.frame
    }
    fn enqueue_pre(&mut self, _: &mut PhysicsActorLinks, actor: usize) -> Result<(), String> {
        self.events.extend([0xc2, actor as u32]);
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
        self.events.extend([0xc1, entity as u32]);
        Ok(entity)
    }
}

fn snapshot(actors: &PhysicsActorLinks) -> Vec<u32> {
    let mut out = Vec::new();
    for actor in actors.actors.iter() {
        out.extend([
            u32::from(actor.flags),
            actor.current_body as u32,
            actor.last_queued_frame as u32,
        ]);
        out.extend(
            actor
                .motion
                .velocities
                .iter()
                .flatten()
                .map(|v| v.to_bits()),
        );
        out.extend(actor.motion.valid.map(u32::from));
        out.extend(actor.motion.dirty.map(u32::from));
        for list in &actor.requests {
            let words = request_snapshot(list);
            out.extend_from_slice(&words[..3]);
            out.push(u32::from(list.enabled));
            out.extend_from_slice(&words[3..]);
        }
    }
    out
}

#[test]
fn complete_pre_physics_traversal_matches_executed_motion_and_entity_sequence() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/actor_pre_update.txt").lines() {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let frame = words.next().unwrap() as i32;
        let mutation = words.next().unwrap();
        let mut actors = PhysicsActorLinks {
            actors: crate::gameplay::physics_actor::test_actor_pool(Vec::new()),
            bodies: (0..6)
                .map(|i| ActorBody {
                    actor: Some(i % 3),
                    body_id: i as u32,
                })
                .collect(),
        };
        let mut world = World {
            entities: Vec::new(),
            frame,
            mutation,
            events: Vec::new(),
        };
        for index in 0..3 {
            let flags = words.next().unwrap() as u8;
            let kind = words.next().unwrap() as u8;
            let bound = words.next().unwrap();
            let bound = (bound != u32::MAX).then_some(bound as usize);
            let delta = f32::from_bits(words.next().unwrap());
            let stopped = words.next().unwrap() != 0;
            let response = words.next().unwrap() as u8;
            let accepts = words.next().unwrap() != 0;
            let current_body = words.next().unwrap() as usize;
            let marker = words.next().unwrap() as i32;
            let velocities = std::array::from_fn(|_| {
                std::array::from_fn(|_| f32::from_bits(words.next().unwrap()))
            });
            let valid = std::array::from_fn(|_| words.next().unwrap() != 0);
            let dirty = std::array::from_fn(|_| words.next().unwrap() != 0);
            let requests = std::array::from_fn(|axis| {
                let count = words.next().unwrap();
                let enabled = words.next().unwrap() != 0;
                let priority = words.next().unwrap() as i32;
                let remaining = f32::from_bits(words.next().unwrap());
                let velocity = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
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
            let mut actor = PhysicsActor::new(index);
            actor.owner = ComponentOwner {
                entity: Some(index),
                ..Default::default()
            };
            actor.current_body = current_body;
            actor.flags = flags;
            actor.last_queued_frame = marker;
            actor.parent = bound.filter(|&b| b < 3);
            actor.binding.entity = bound;
            actor.binding.phase = kind;
            actor.children = if index == 0 { vec![1, 2] } else { Vec::new() };
            actor.motion = ResolvedMotion {
                velocities,
                valid,
                dirty,
            };
            actor.requests = requests;
            actors.actors.allocate(|_| Ok(actor)).unwrap();
            world.entities.push(EntityState {
                delta,
                stopped,
                response,
                accepts,
            });
        }
        actors.pre_update(0, &mut world).unwrap();
        let mut actual = snapshot(&actors);
        actual.push(world.events.len() as u32);
        actual.extend(world.events);
        assert_eq!(actual, words.collect::<Vec<_>>(), "case {cases}");
        cases += 1;
    }
    assert_eq!(cases, 512);
}
