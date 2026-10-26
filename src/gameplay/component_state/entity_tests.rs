use super::*;
use crate::gameplay::physics_actor::PhysicsActor;
use std::cell::RefCell;

#[test]
fn release_matches_complete_executed_component_and_entity_unlink() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/component_release.txt").lines() {
        let mut words = line
            .split_whitespace()
            .map(|v| u64::from_str_radix(v, 16).unwrap());
        let target = words.next().unwrap() as u16;
        let flags = words.next().unwrap() as u32;
        let generation = words.next().unwrap() as u32;
        let count = words.next().unwrap() as usize;
        let indices: Vec<_> = words.by_ref().take(count).collect();
        let count = words.next().unwrap() as usize;
        let classes = words.by_ref().take(count).map(|v| v as u16).collect();
        let mut entity = EntityComponents::new(42);
        entity.cached_classes = Some(classes);
        for &index in &indices {
            entity
                .entries
                .append(
                    ComponentHandle {
                        class_id: 7,
                        slot: index as u16,
                    }
                    .packed(),
                    false,
                )
                .unwrap();
        }
        let mut component = super::super::ComponentOwner {
            declaration: Some(0x1234),
            entity: Some(42),
            generation,
            flags,
        };
        entity
            .release(
                ComponentHandle {
                    class_id: 7,
                    slot: target,
                },
                &mut component,
            )
            .unwrap();
        let mut actual = vec![u64::from(entity.entries.count())];
        actual.extend(
            entity
                .entries
                .values()
                .iter()
                .map(|&v| u64::from(ComponentHandle::from_packed(v).unwrap().slot)),
        );
        let classes = entity.cached_classes.as_ref().unwrap();
        actual.push(classes.len() as u64);
        actual.extend(classes.iter().map(|&v| u64::from(v)));
        actual.extend([
            component.declaration.unwrap_or(0) as u64,
            component.entity.unwrap_or(0) as u64,
            u64::from(component.generation),
            u64::from(component.flags),
        ]);
        assert_eq!(actual, words.collect::<Vec<_>>(), "case {cases}");
        cases += 1;
    }
    assert_eq!(cases, 512);
}

fn append_list(out: &mut Vec<u64>, list: &PointerList) {
    out.push(u64::from(list.count()));
    out.extend_from_slice(list.values());
}
fn append_cache(out: &mut Vec<u64>, entity: &EntityComponents) {
    if let Some(cache) = &entity.cached_classes {
        out.push(cache.len() as u64);
        out.extend(cache.iter().map(|&v| u64::from(v)));
    } else {
        out.push(u64::from(u32::MAX));
    }
}

#[test]
fn creation_matches_complete_executed_entity_factory() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/entity_component_factory.txt").lines() {
        let mut words = line
            .split_whitespace()
            .map(|v| u64::from_str_radix(v, 16).unwrap());
        let initial = words.next().unwrap() as i32;
        let generation = words.next().unwrap() as u32;
        let maximum = words.next().unwrap() as i32;
        let cached = words.next().unwrap() != 0;
        let disabled = words.next().unwrap() != 0;
        let deferred = words.next().unwrap() != 0;
        let phase = words.next().unwrap() as u32;
        let mask = words.next().unwrap() as u32;
        let immediate = words.next().unwrap() != 0;
        let mutation = words.next().unwrap() as u32;
        let key = words.next().unwrap() as i32;
        let count = words.next().unwrap();
        let classes: Vec<_> = (0..count).map(|_| words.next().unwrap() as u16).collect();
        let mut entity = EntityComponents {
            identity: 0x200,
            creation_disabled: disabled,
            start_deferred: deferred,
            phase,
            entries: PointerList::with_capacity(64, 0x40).unwrap(),
            cached_classes: cached.then_some(classes.clone()),
            reference_actor: None,
        };
        for (index, &class_id) in classes.iter().enumerate() {
            let handle = ComponentHandle {
                class_id: i32::from(class_id),
                slot: 0x8000 + index as u16,
            };
            entity.entries.append(handle.packed(), false).unwrap();
        }
        let mut registry = ComponentRegistry {
            next_generation: generation,
            maximum_registered: maximum,
            registered: PointerList::with_capacity(4096, 0x40).unwrap(),
            pending_start: PointerList::with_capacity(64, 0x40).unwrap(),
            pending_stop: Default::default(),
            processed_stop_count: 0,
            processed_initialization_count: 0,
            processed_start_count: 0,
            queue_reset_count: 0,
            pending_release: crate::storage::PointerList::owned_with_capacity(64).unwrap(),
        };
        let declaration = ComponentDeclaration { identity: 0x1000, class_id: key,
            scheduling: crate::gameplay::component_state::declaration::DeclarationScheduling {
                start_phases: mask,
                ..crate::gameplay::component_state::declaration::DeclarationScheduling::from_base_flags(0)
            } };
        let mut pool = ComponentPool::new(initial, 0x420).unwrap();
        let observed = RefCell::new(Vec::new());
        let result = ComponentFactory {
            declaration: &declaration,
            pool: &mut pool,
            registry: &mut registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut entity,
            immediate,
            |slot, entity, registry| {
                let mut events = observed.borrow_mut();
                events.extend([0xf0, u64::from(registry.next_generation)]);
                append_list(&mut events, &entity.entries);
                append_cache(&mut events, entity);
                Ok(PhysicsActor::new(slot as usize))
            },
            |registry, actor, entity| {
                assert_eq!(actor.owner.declaration, Some(0x1000));
                assert_eq!(actor.owner.entity, Some(0x200));
                let mut events = observed.borrow_mut();
                events.extend([
                    0xb0,
                    u64::from(actor.owner.flags),
                    u64::from(actor.owner.generation),
                ]);
                append_list(&mut events, &entity.entries);
                events.extend([
                    u64::from(registry.registered.count()),
                    u64::from(registry.pending_start.count()),
                ]);
                actor.owner.flags ^= mutation;
                Ok(())
            },
        )
        .unwrap();
        let mut actual = vec![u64::from(result.is_some())];
        if let Some(handle) = result {
            let actor = pool.get(handle.slot).unwrap();
            assert_eq!(actor.owner.declaration, Some(0x1000));
            assert_eq!(actor.owner.entity, Some(0x200));
            actual.extend([
                handle.packed(),
                u64::from(actor.owner.generation),
                u64::from(actor.owner.flags),
            ]);
        }
        actual.extend([
            u64::from(registry.next_generation),
            u64::from(registry.maximum_registered as u32),
        ]);
        append_list(&mut actual, &entity.entries);
        append_cache(&mut actual, &entity);
        append_list(&mut actual, &registry.registered);
        append_list(&mut actual, &registry.pending_start);
        let observed = observed.into_inner();
        actual.push(observed.len() as u64);
        actual.extend(observed);
        assert_eq!(actual, words.collect::<Vec<_>>(), "case {cases}");
        cases += 1;
    }
    assert_eq!(cases, 512);
}
