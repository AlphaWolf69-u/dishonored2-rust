use super::*;
use crate::gameplay::motion_requests::{request_snapshot, MotionRequest, MotionRequestInput};

fn read_list(words: &mut impl Iterator<Item = u32>) -> MotionRequestList {
    let timed = words.next().unwrap() as i32;
    let next = words.next().unwrap();
    let cached = words.next().unwrap();
    let enabled = words.next().unwrap() != 0;
    let count = words.next().unwrap();
    let entries = (0..count)
        .map(|_| {
            let velocity = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
            let id = words.next().unwrap();
            let priority = words.next().unwrap() as i32;
            let remaining = f32::from_bits(words.next().unwrap());
            let lifetime = words.next().unwrap() as i32;
            let flags = words.next().unwrap() as u8;
            MotionRequest {
                id,
                input: MotionRequestInput {
                    velocity,
                    priority,
                    remaining,
                    lifetime,
                    flags,
                },
            }
        })
        .collect();
    MotionRequestList::from_parts(entries, timed, next, cached, enabled).unwrap()
}

fn snapshot(actor: &PhysicsActor) -> Vec<u32> {
    let mut out = vec![u32::from(actor.flags), actor.current_body as u32];
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
        let values = request_snapshot(list);
        out.extend(&values[..3]);
        out.push(u32::from(list.enabled));
        out.extend(&values[3..]);
    }
    out
}

struct Body {
    mode: u32,
    response: u8,
    events: Vec<u32>,
}
impl ActorMotionBody for Body {
    fn accepts(&mut self, links: &mut PhysicsActorLinks, actor: usize) -> Result<bool, String> {
        let actor = &mut links.actors[actor];
        self.events.extend([0xb0, actor.current_body as u32]);
        if self.mode & 1 != 0 {
            actor.flags ^= 0x20;
        }
        if self.mode & 8 != 0 {
            actor.current_body = 1 - actor.current_body;
        }
        Ok(self.mode & 16 == 0)
    }
    fn apply(
        &mut self,
        links: &mut PhysicsActorLinks,
        actor: usize,
        motion: ResolvedMotion,
    ) -> Result<u8, String> {
        let actor = &mut links.actors[actor];
        self.events.extend([0xb1, actor.current_body as u32]);
        self.events
            .extend(motion.velocities.iter().flatten().map(|v| v.to_bits()));
        self.events.extend(motion.valid.map(u32::from));
        if self.mode & 2 != 0 {
            actor.motion.valid[0] = !actor.motion.valid[0];
        }
        if self.mode & 4 != 0 {
            actor.flags ^= 0x40;
        }
        Ok(self.response)
    }
}

#[test]
fn request_resolution_matches_executed_actor_and_body_virtual_sequence() {
    let mut count = 0;
    for row in include_str!("../../../tests/fixtures/actor_motion_resolution.txt").lines() {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let mut body = Body {
            mode: words.next().unwrap(),
            response: words.next().unwrap() as u8,
            events: Vec::new(),
        };
        let delta = f32::from_bits(words.next().unwrap());
        let flags = words.next().unwrap() as u8;
        let current_body = words.next().unwrap() as usize;
        let motion = ResolvedMotion {
            velocities: std::array::from_fn(|_| {
                std::array::from_fn(|_| f32::from_bits(words.next().unwrap()))
            }),
            valid: std::array::from_fn(|_| words.next().unwrap() != 0),
            dirty: std::array::from_fn(|_| words.next().unwrap() != 0),
        };
        let requests = std::array::from_fn(|_| read_list(&mut words));
        let mut links = PhysicsActorLinks {
            actors: crate::gameplay::physics_actor::test_actor_pool(vec![PhysicsActor {
                binding: Default::default(),
                frame: Default::default(),
                owner: crate::gameplay::component_state::ComponentOwner {
                    entity: Some(0),
                    generation: 0,
                    ..Default::default()
                },
                default_body: 0,
                current_body,
                parent: None,
                children: Vec::new(),
                flags,
                last_queued_frame: 0,
                last_post_frame: 0,
                requests,
                motion,
            }]),
            bodies: Vec::new(),
        };
        let result = links.resolve_motion(0, delta, &mut body).unwrap();
        let mut actual = vec![u32::from(result)];
        actual.extend(snapshot(&links.actors[0]));
        actual.push(body.events.len() as u32);
        actual.extend(body.events);
        assert_eq!(actual, words.collect::<Vec<_>>(), "case {count}");
        count += 1;
    }
    assert_eq!(count, 512);
}
