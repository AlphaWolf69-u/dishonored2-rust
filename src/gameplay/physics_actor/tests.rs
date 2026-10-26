use super::*;

fn read_initial(values: &mut impl Iterator<Item = u32>) -> PhysicsActorLinks {
    let parents = [
        None,
        Some(0),
        Some(0),
        Some(1),
        Some(1),
        Some(2),
        Some(2),
        Some(6),
    ];
    let actors: Vec<_> = (0..8)
        .map(|index| {
            let current_body = values.next().unwrap() as usize;
            let flags = values.next().unwrap() as u8;
            let last_queued_frame = values.next().unwrap() as i32;
            let last_post_frame = values.next().unwrap() as i32;
            let requests = std::array::from_fn(|_| {
                let enabled = values.next().unwrap() != 0;
                let count = values.next().unwrap();
                let entries = (0..count)
                    .map(|_| crate::gameplay::motion_requests::MotionRequest {
                        id: 0,
                        input: crate::gameplay::motion_requests::MotionRequestInput {
                            velocity: [0.; 3],
                            priority: 0,
                            remaining: f32::from_bits(values.next().unwrap()),
                            lifetime: 0,
                            flags: 0,
                        },
                    })
                    .collect();
                MotionRequestList::from_parts(entries, 0, 0, 0, enabled).unwrap()
            });
            PhysicsActor {
                binding: super::ActorBinding {
                    entity: parents[index].map(|p| 1000 + p),
                    ..Default::default()
                },
                frame: Default::default(),
                owner: crate::gameplay::component_state::ComponentOwner {
                    entity: Some(1000 + index),
                    generation: 0,
                    ..Default::default()
                },
                default_body: index,
                current_body,
                parent: parents[index],
                children: (0..8).filter(|&i| parents[i] == Some(index)).collect(),
                flags,
                last_queued_frame,
                last_post_frame,
                requests,
                motion: ResolvedMotion::default(),
            }
        })
        .collect();
    PhysicsActorLinks {
        actors: super::test_actor_pool(actors),
        bodies: (0..12)
            .map(|i| ActorBody {
                actor: None,
                body_id: 0x1000000 + i,
            })
            .collect(),
    }
}

fn snapshot(links: &PhysicsActorLinks) -> Vec<u32> {
    let mut out = Vec::new();
    for actor in links.actors.iter() {
        out.extend([
            actor.current_body as u32,
            actor.flags as u32,
            actor.last_queued_frame as u32,
            actor.last_post_frame as u32,
        ]);
    }
    out.extend(
        links
            .bodies
            .iter()
            .map(|b| b.actor.map_or(u32::MAX, |a| a as u32)),
    );
    out
}

#[test]
fn attachment_and_transform_gates_match_executed_game_routines() {
    let mut links = None;
    let mut cases = 0;
    for row in include_str!("../../../tests/fixtures/physics_actor.txt").lines() {
        let mut parts = row.split_whitespace();
        let kind = parts.next().unwrap();
        let mut words = parts.map(|p| u32::from_str_radix(p, 16).unwrap());
        if kind == "initial" {
            links = Some(read_initial(&mut words));
            assert!(words.next().is_none());
            continue;
        }
        let links = links.as_mut().unwrap();
        let op = words.next().unwrap();
        let actor = words.next().unwrap() as usize;
        let arg = words.next().unwrap() as usize;
        let frame = words.next().unwrap() as i32;
        let mut events = Vec::new();
        let mut queue = |links: &mut PhysicsActorLinks, a: usize| {
            // The source stamps before handing the actor to the queue.
            let marker = if op == 5 {
                links.actors[a].last_post_frame
            } else {
                links.actors[a].last_queued_frame
            };
            assert_eq!(marker, frame);
            events.push(a as u32);
            Ok(())
        };
        let result = match op {
            0 => {
                links.attach_body(actor, arg, frame, &mut queue).unwrap();
                0
            }
            1 => {
                links.propagate_body_flag(actor, arg != 0).unwrap();
                0
            }
            2 => {
                links.request_update(actor, frame, &mut queue).unwrap();
                0
            }
            3 => u32::from(links.actors[actor].has_enabled_motion()),
            4 => u32::from(links.ancestor_has_update(actor, frame).unwrap()),
            5 => {
                links
                    .request_phase_update(actor, frame, ActorUpdatePhase::PostPhysics, &mut queue)
                    .unwrap();
                0
            }
            6 => {
                links.detach_body(actor, arg).unwrap();
                0
            }
            _ => unreachable!(),
        };
        let mut actual = vec![result];
        actual.extend(snapshot(links));
        actual.push(events.len() as u32);
        actual.extend(events);
        assert_eq!(
            actual,
            words.collect::<Vec<_>>(),
            "case {cases}, operation {op}"
        );
        cases += 1;
    }
    assert_eq!(cases, 896);
}
