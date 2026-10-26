use super::*;
#[test]
fn body_change_queues_match_executed_add_paths() {
    check_changes(
        include_str!("../../../tests/fixtures/body_changes.txt"),
        false,
    );
}

#[test]
fn body_change_queue_reset_matches_executed_manager() {
    check_changes(
        include_str!("../../../tests/fixtures/body_changes_reset.txt"),
        true,
    );
}

fn check_changes(fixture: &str, with_reset: bool) {
    let mut storage = BodyStorage::new(32).unwrap();
    let mut count = 0;
    for (case, line) in fixture.lines().enumerate() {
        let mut w = line
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap());
        let reset_queues = with_reset && w.next().unwrap() != 0;
        if w.next().unwrap() != 0 {
            storage = BodyStorage::new(32).unwrap();
        }
        let kind = w.next().unwrap();
        let enabled = w.next().unwrap() != 0;
        let keep = w.next().unwrap() != 0;
        let n = w.next().unwrap() as usize;
        let ids: Vec<_> = w.by_ref().take(n).collect();
        for &id in &ids {
            storage.previous_bounds[(id & 0xffffff) as usize].words =
                std::array::from_fn(|_| w.next().unwrap());
        }
        if kind == 1 {
            for &id in &ids {
                storage.mark_bounds_changed(id).unwrap();
                storage.reset_previous_bounds(id).unwrap();
            }
        } else {
            let activation = if enabled && keep {
                BodyActivation::KeepDeactivated
            } else {
                BodyActivation::Activate
            };
            storage.queue_additions(&ids, activation).unwrap();
        }
        if reset_queues {
            storage.reset_queued_additions();
        }
        let mut actual = vec![storage.changes().len() as u32];
        for change in storage.changes() {
            actual.extend([
                change.body_index,
                change.queue_index,
                u32::from(change.flags),
            ]);
        }
        for activation in [BodyActivation::Activate, BodyActivation::KeepDeactivated] {
            let queue = storage.queued_additions(activation);
            actual.push(queue.len() as u32);
            actual.extend_from_slice(queue);
        }
        actual.extend_from_slice(storage.change_indices());
        for &id in &ids {
            actual.extend(storage.previous_bounds[(id & 0xffffff) as usize].words);
        }
        assert_eq!(actual, w.collect::<Vec<_>>(), "case {case}");
        count += 1;
    }
    assert_eq!(count, 512);
}

#[test]
fn queued_addition_flush_matches_executed_world_sequence() {
    use std::cell::RefCell;
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/body_add_flush.txt").lines() {
        let mut words = line
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap());
        let mut batches = Vec::new();
        for _ in 0..3 {
            let n = words.next().unwrap() as usize;
            batches.push(words.by_ref().take(n).collect::<Vec<_>>());
        }
        let n = words.next().unwrap() as usize;
        let expected_events: Vec<_> = words.by_ref().take(n).collect();
        let expected_state: Vec<_> = words.collect();
        let mut storage = BodyStorage::new(32).unwrap();
        storage
            .queue_additions(&batches[0], BodyActivation::Activate)
            .unwrap();
        storage
            .queue_additions(&batches[1], BodyActivation::KeepDeactivated)
            .unwrap();
        let events = RefCell::new(Vec::new());
        storage
            .flush_additions(
                |storage, ids, activation| {
                    let tag = u32::from(activation == BodyActivation::KeepDeactivated);
                    events.borrow_mut().extend([tag, ids.len() as u32]);
                    events.borrow_mut().extend_from_slice(ids);
                    if activation == BodyActivation::Activate {
                        storage.queue_additions(&batches[2], BodyActivation::KeepDeactivated)?;
                    }
                    Ok(())
                },
                || {
                    events.borrow_mut().push(2);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(*events.borrow(), expected_events, "case {cases}");
        let mut state = vec![storage.changes().len() as u32];
        for change in storage.changes() {
            state.extend([
                change.body_index,
                change.queue_index,
                u32::from(change.flags),
            ]);
        }
        for activation in [BodyActivation::Activate, BodyActivation::KeepDeactivated] {
            state.push(storage.queued_additions(activation).len() as u32);
            state.extend_from_slice(storage.queued_additions(activation));
        }
        state.extend_from_slice(storage.change_indices());
        assert_eq!(state, expected_state, "case {cases}");
        cases += 1;
    }
    assert_eq!(cases, 512);
}
