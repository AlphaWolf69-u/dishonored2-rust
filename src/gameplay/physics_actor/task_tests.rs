use super::*;
use crate::gameplay::physics_world::update_queue::{
    ObjectReference, ObjectUpdateTask, UPDATE_CAPACITY,
};

fn snapshot(
    actors: &PhysicsActorLinks,
    task: &ObjectUpdateTask<usize>,
    events: &[u32],
) -> Vec<u32> {
    let mut result = Vec::new();
    for actor in actors.actors.iter() {
        result.extend([
            actor.owner.generation,
            actor.last_queued_frame as u32,
            actor.last_post_frame as u32,
            actor.parent.map_or(u32::MAX, |p| p as u32),
        ]);
    }
    result.push(task.updates.pool.head());
    let changed: Vec<_> = task
        .updates
        .pool
        .free_links()
        .enumerate()
        .filter(|(i, value)| *value != *i as u32 + 1)
        .flat_map(|(i, v)| [i as u32, v])
        .collect();
    result.push(changed.len() as u32 / 2);
    result.extend(changed);
    let mut entries = Vec::new();
    for i in 0..UPDATE_CAPACITY {
        let reference = task.updates.entry(i).unwrap();
        if reference.generation != u32::MAX || reference.object.is_some() {
            entries.extend([
                i as u32,
                reference.generation,
                reference.object.map_or(u32::MAX, |a| a as u32),
            ]);
        }
    }
    result.push(entries.len() as u32 / 3);
    result.extend(entries);
    let [read, reserved, committed] = task.updates.queue.counters();
    let n = committed.wrapping_sub(read);
    result.extend([read, reserved, committed, n]);
    result.extend((0..n).map(|i| task.updates.queue.slot(read.wrapping_add(i))));
    result.push(events.len() as u32);
    result.extend(events);
    result
}

#[test]
fn pre_and_post_tasks_match_complete_executed_lifecycle_and_dispatch() {
    let mut links = PhysicsActorLinks {
        actors: crate::gameplay::physics_actor::test_actor_pool(Vec::new()),
        bodies: Vec::new(),
    };
    let mut task = None;
    let mut phase = ActorUpdatePhase::PrePhysics;
    let mut mode = 0;
    let mut batches = 0;
    let mut constructors = 0;
    for row in include_str!("../../../tests/fixtures/reference_frame_tasks.txt").lines() {
        let mut parts = row.split_whitespace();
        let kind = parts.next().unwrap();
        let mut words = parts.map(|v| u32::from_str_radix(v, 16).unwrap());
        match kind {
            "initial" => {
                phase = if words.next().unwrap() == 0 {
                    ActorUpdatePhase::PrePhysics
                } else {
                    ActorUpdatePhase::PostPhysics
                };
                mode = words.next().unwrap();
                links.actors = super::test_actor_pool(
                    (0..8)
                        .map(|i| {
                            let reference_generation = words.next().unwrap();
                            let last_queued_frame = words.next().unwrap() as i32;
                            let last_post_frame = words.next().unwrap() as i32;
                            let parent = words.next().unwrap();
                            PhysicsActor {
                                binding: super::ActorBinding {
                                    entity: (parent != u32::MAX).then_some(parent as usize),
                                    ..Default::default()
                                },
                                frame: Default::default(),
                                owner: crate::gameplay::component_state::ComponentOwner {
                                    entity: Some(i),
                                    generation: reference_generation,
                                    ..Default::default()
                                },
                                last_queued_frame,
                                last_post_frame,
                                default_body: i,
                                current_body: i,
                                parent: (parent != u32::MAX).then_some(parent as usize),
                                children: Vec::new(),
                                flags: 0,
                                requests: std::array::from_fn(|_| {
                                    MotionRequestList::from_parts(Vec::new(), 0, 0, 0, false)
                                        .unwrap()
                                }),
                                motion: ResolvedMotion::default(),
                            }
                        })
                        .collect(),
                );
                assert!(words.next().is_none());
                task = Some(ObjectUpdateTask::new());
                constructors += 1;
            }
            "batch" => {
                let task = task.as_mut().unwrap();
                task.frame = words.next().unwrap() as i32;
                task.stamp = words.next().unwrap() as i32;
                let n = words.next().unwrap();
                for _ in 0..n {
                    let object = words.next().unwrap();
                    let generation = words.next().unwrap();
                    assert!(task
                        .updates
                        .enqueue(ObjectReference {
                            generation,
                            object: (object != u32::MAX).then_some(object as usize)
                        })
                        .unwrap());
                }
                let stale = words.next().unwrap() as usize;
                if stale < 8 {
                    links.actors[stale].owner.generation ^= 0x123;
                }
                task.publish_sentinels(words.next().unwrap());
                let trailing = words.next().unwrap() as usize;
                let generation = words.next().unwrap();
                task.updates
                    .enqueue(ObjectReference::capture(Some((generation, trailing))))
                    .unwrap();
                let mut events = Vec::new();
                let frame = task.frame;
                links
                    .consume_updates(task, phase, |links, index, phase| {
                        events.push(index as u32);
                        match mode {
                            1 => {
                                let actor = &mut links.actors[(index + 1) % 8];
                                match phase {
                                    ActorUpdatePhase::PrePhysics => actor.last_queued_frame = frame,
                                    ActorUpdatePhase::PostPhysics => actor.last_post_frame = frame,
                                }
                            }
                            2 => links.actors[index].owner.generation ^= 1,
                            3 => links.actors[index].parent = None,
                            _ => {}
                        }
                        Ok(())
                    })
                    .unwrap();
                assert_eq!(
                    snapshot(&links, task, &events),
                    words.collect::<Vec<_>>(),
                    "batch {batches}"
                );
                batches += 1;
            }
            "released" => {
                let task = task.take().unwrap();
                // The independently tested release kernel publishes the same
                // final free head when returning the permanent sentinel.
                assert_eq!(
                    task.updates.pool.head() & 0xffffc000 | task.sentinel(),
                    words.next().unwrap()
                );
                assert!(words.next().is_none());
                drop(task);
            }
            _ => unreachable!(),
        }
    }
    assert_eq!((constructors, batches), (128, 512));
}
