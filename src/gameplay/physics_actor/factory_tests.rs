use super::*;
use crate::{
    gameplay::component_state::{pool::ComponentPool, ComponentRegistry},
    storage::PointerList,
};

#[test]
fn typed_actor_factory_matches_composed_game_calls() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/actor_factory.txt").lines() {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let initial = words.next().unwrap() as i32;
        let generation = words.next().unwrap();
        let steps = words.next().unwrap();
        let mut pool = ComponentPool::new(initial, 0x420).unwrap();
        let mut registry = ComponentRegistry {
            next_generation: generation,
            maximum_registered: 0,
            registered: PointerList::with_capacity(4096, 0x40).unwrap(),
            pending_start: Default::default(),
            pending_stop: Default::default(),
            processed_stop_count: 0,
            processed_initialization_count: 0,
            processed_start_count: 0,
            queue_reset_count: 0,
            pending_release: crate::storage::PointerList::owned_with_capacity(64).unwrap(),
        };
        for index in 0..steps {
            let slot = registry
                .allocate(&mut pool, |slot, _| Ok(PhysicsActor::new(slot as usize)))
                .unwrap();
            let actor = pool.get_mut(slot).unwrap();
            registry
                .register(
                    &mut actor.owner,
                    slot as usize,
                    0x1000,
                    0x200 + index as usize,
                    false,
                )
                .unwrap();
            assert_eq!(
                [
                    actor.owner.declaration.unwrap() as u32,
                    actor.owner.entity.unwrap() as u32,
                    actor.owner.generation,
                    actor.owner.flags,
                    registry.next_generation,
                    registry.registered.count(),
                    registry.maximum_registered as u32,
                    u32::from(actor.flags),
                    actor.last_queued_frame as u32,
                    actor.last_post_frame as u32
                ],
                std::array::from_fn(|_| words.next().unwrap())
            );
            assert_eq!(
                registry.registered.values()[index as usize],
                u64::from(slot)
            );
            assert_eq!(actor.current_body, actor.default_body);
            assert_eq!(actor.default_body, slot as usize);
        }
        assert!(words.next().is_none());
        cases += 1;
    }
    assert_eq!(cases, 15);
}
