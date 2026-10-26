use super::*;

#[test]
fn update_producers_match_complete_executed_queue_and_ring() {
    let mut enqueues = 0;
    let mut pushes = 0;
    for row in include_str!("../../../tests/fixtures/object_update_queue.txt").lines() {
        let mut parts = row.split_whitespace();
        let kind = parts.next().unwrap();
        let mut words = parts.map(|v| u32::from_str_radix(v, 16).unwrap());
        if kind == "enqueue" {
            let version = words.next().unwrap();
            let available = words.next().unwrap() as usize;
            let start = words.next().unwrap();
            let mut next = vec![UPDATE_CAPACITY as u32; UPDATE_CAPACITY];
            for i in 0..available.saturating_sub(1) {
                next[i] = i as u32 + 1;
            }
            let pool = UpdateIndexPool::from_parts(
                version | if available == 0 { 0x2000 } else { 0 },
                next,
            )
            .unwrap();
            let ring = UpdateIndexQueue::from_parts(start, start, start, vec![0; UPDATE_CAPACITY])
                .unwrap();
            let queue = ObjectUpdateQueue::from_parts(
                pool,
                ring,
                vec![ObjectReference::capture(None); UPDATE_CAPACITY],
            )
            .unwrap();
            for i in 0..12 {
                let present = words.next().unwrap() != 0;
                let generation = words.next().unwrap();
                assert_eq!(
                    queue
                        .enqueue(ObjectReference::capture(present.then_some((generation, i))))
                        .unwrap(),
                    i < available
                );
            }
            let mut actual = vec![
                queue.pool.head(),
                queue.queue.counters()[1],
                queue.queue.counters()[2],
            ];
            for i in 0..available {
                let reference = queue.entry(i).unwrap();
                actual.extend([reference.generation, u32::from(reference.object.is_some())]);
            }
            actual.extend((0..available).map(|i| queue.queue.slot(start.wrapping_add(i as u32))));
            assert_eq!(actual, words.collect::<Vec<_>>(), "enqueue {enqueues}");
            enqueues += 1;
        } else {
            let read = words.next().unwrap();
            let write = words.next().unwrap();
            let value = words.next().unwrap();
            let prior = words.next().unwrap();
            let mut slots = vec![0; UPDATE_CAPACITY];
            slots[(write & 0x1fff) as usize] = prior;
            let ring = UpdateIndexQueue::from_parts(read, write, write, slots).unwrap();
            let result = ring.push(value);
            assert_eq!(
                [
                    u32::from(result),
                    ring.counters()[1],
                    ring.counters()[2],
                    ring.slot(write)
                ],
                words.collect::<Vec<_>>().as_slice(),
                "push {pushes}"
            );
            pushes += 1;
        }
    }
    assert_eq!((enqueues, pushes), (128, 512));
}

#[test]
fn concurrent_producers_publish_unique_complete_references() {
    let next = (1..=UPDATE_CAPACITY as u32).collect();
    let pool = UpdateIndexPool::from_parts(0, next).unwrap();
    let ring = UpdateIndexQueue::from_parts(0, 0, 0, vec![0; UPDATE_CAPACITY]).unwrap();
    let queue = ObjectUpdateQueue::from_parts(
        pool,
        ring,
        vec![ObjectReference::capture(None); UPDATE_CAPACITY],
    )
    .unwrap();
    std::thread::scope(|scope| {
        for producer in 0..4u32 {
            let queue = &queue;
            scope.spawn(move || {
                for index in 0..512 {
                    let id = producer * 512 + index;
                    assert!(queue
                        .enqueue(ObjectReference::capture(Some((id ^ 0xabcde, id))))
                        .unwrap());
                }
            });
        }
    });
    assert_eq!(queue.queue.counters(), [0, 2048, 2048]);
    let mut seen = std::collections::HashSet::new();
    for position in 0..2048 {
        let slot = queue.queue.slot(position);
        let reference = queue.entry(slot as usize).unwrap();
        let id = reference.object.unwrap();
        assert_eq!(reference.generation, id ^ 0xabcde);
        assert!(seen.insert(id));
    }
    assert_eq!(seen.len(), 2048);
}
