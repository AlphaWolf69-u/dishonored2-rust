use super::*;
use crate::gameplay::{motion_info::MotionInfo, space_cells::SpaceCells};

fn records<const N: usize>(words: &mut impl Iterator<Item = u32>) -> Vec<[u32; N]> {
    let n = words.next().unwrap();
    (0..n)
        .map(|_| std::array::from_fn(|_| words.next().unwrap()))
        .collect()
}

pub(super) fn read_world(words: &mut impl Iterator<Item = u32>) -> DeactivationWorld {
    let states = (0..3)
        .map(|i| {
            let mut state = MotionDeactivation::new(i, super::super::MODE_DEACTIVATE);
            state.set_island(words.next().unwrap() as i32);
            state
        })
        .collect();
    let mut world = DeactivationWorld::new(states);
    world.free_islands = records::<1>(words).into_iter().map(|v| v[0]).collect();
    world.activation_queue = records::<1>(words).into_iter().map(|v| v[0]).collect();
    world.cache_links = records::<2>(words)
        .into_iter()
        .map(|v| IslandCacheLink {
            island: v[0],
            index: v[1],
        })
        .collect();
    let n = words.next().unwrap();
    world.islands = (0..n)
        .map(|_| {
            if words.next().unwrap() == 0 {
                return None;
            }
            let id = words.next().unwrap();
            let wake = words.next().unwrap() as u8;
            let queued = words.next().unwrap() as u8;
            let cache_priority = words.next().unwrap();
            let pair_step = words.next().unwrap();
            let bodies = records::<1>(words).into_iter().map(|v| v[0]).collect();
            let cache_links = records::<1>(words)
                .into_iter()
                .map(|v| v[0] as usize)
                .collect();
            let cache_records = records::<4>(words);
            let pairs = records::<2>(words);
            Some(DeactivatedIsland {
                id,
                bodies,
                cache_links,
                cache_records,
                cache_priority,
                pairs,
                pair_step,
                wake,
                queued,
            })
        })
        .collect();
    world
}

pub(super) fn snapshot(world: &DeactivationWorld) -> Vec<u32> {
    let mut result: Vec<_> = world
        .states
        .iter()
        .flatten()
        .map(|s| s.island() as u32)
        .collect();
    for values in [&world.free_islands, &world.activation_queue] {
        result.push(values.len() as u32);
        result.extend_from_slice(values);
    }
    result.push(world.cache_links.len() as u32);
    for link in &world.cache_links {
        result.extend([link.island, link.index]);
    }
    result.push(world.islands.len() as u32);
    for island in &world.islands {
        result.push(u32::from(island.is_some()));
        let Some(island) = island else {
            continue;
        };
        result.extend([
            island.id,
            u32::from(island.wake),
            u32::from(island.queued),
            island.cache_priority,
            island.pair_step,
        ]);
        result.push(island.bodies.len() as u32);
        result.extend_from_slice(&island.bodies);
        result.push(island.cache_links.len() as u32);
        result.extend(island.cache_links.iter().map(|&v| v as u32));
        result.push(island.cache_records.len() as u32);
        for record in &island.cache_records {
            result.extend(record);
        }
        result.push(island.pairs.len() as u32);
        for pair in &island.pairs {
            result.extend(pair);
        }
    }
    result
}

#[test]
fn island_link_and_merge_match_complete_executed_routines() {
    let mut cases = 0;
    for row in include_str!("../../../tests/fixtures/deactivation_islands.txt").lines() {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let operation = words.next().unwrap();
        let step = words.next().unwrap();
        let solver: [u32; 3] = std::array::from_fn(|_| words.next().unwrap());
        let mut bodies = BodyStorage::new(5).unwrap();
        let mut ids = [0; 5];
        for (i, id) in ids.iter_mut().enumerate() {
            let reserved = if i == 0 {
                0
            } else {
                bodies.reserve(&mut []).unwrap()
            };
            let body = bodies.get_mut(reserved).unwrap();
            body.id = words.next().unwrap();
            *id = body.id;
            body.motion_id = words.next().unwrap();
            body.flags = words.next().unwrap();
            body.next_attached_body_id = words.next().unwrap();
            body.broadphase_id = words.next().unwrap();
        }
        let mut motions = MotionStorage::new(3).unwrap();
        for i in 1..3 {
            let id = motions.reserve().unwrap();
            motions
                .initialize(id, &MotionInfo::default(), &SpaceCells::Single)
                .unwrap();
            motions.get_mut(id).unwrap().solver_id = solver[i];
        }
        let mut world = read_world(&mut words);
        if operation == 0 {
            world
                .link_bodies(&bodies, &motions, ids[1], ids[2], step)
                .unwrap();
        } else {
            world.merge(&bodies, 0, 1).unwrap();
        }
        let expected: Vec<_> = words.collect();
        assert_eq!(snapshot(&world), expected, "case {cases}");
        cases += 1;
    }
    assert_eq!(cases, 1296);
}
