use super::*;

fn snapshot(queue: &mut CommandBuffer) -> Vec<u32> {
    let current = queue.current.load(Ordering::SeqCst);
    let storage = queue.storage.get_mut();
    let (count, capacity) = storage.order.packed_header();
    let index = if current.is_null() {
        u32::MAX
    } else {
        storage
            .chunks
            .iter()
            .position(|c| std::ptr::from_ref(&**c).cast_mut() == current)
            .unwrap() as u32
    };
    let mut out = vec![count, capacity, index, storage.order.count()];
    for &index in storage.order.values() {
        let c = &storage.chunks[index as usize];
        out.extend([
            c.reserved.load(Ordering::SeqCst) as u32,
            c.records.load(Ordering::SeqCst) as u32,
        ]);
    }
    assert_eq!(queue.lock.load(Ordering::SeqCst), 0);
    out
}

#[test]
fn command_queue_matches_complete_executed_growth_reuse_and_dispatch() {
    let mut cases = 0;
    for row in include_str!("../../../../tests/fixtures/command_buffer.txt").lines() {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let case = words.next().unwrap();
        let capacity = words.next().unwrap() as i32;
        let tag = words.next().unwrap();
        let grow = words.next().unwrap() != 0;
        let steps = words.next().unwrap();
        let mut queue = CommandBuffer::new(capacity, tag).unwrap();
        assert_eq!((queue.chunk_size(), queue.memory_tag()), (capacity, tag));
        let actual = snapshot(&mut queue);
        assert_eq!(
            actual,
            words.by_ref().take(actual.len()).collect::<Vec<_>>()
        );
        for step in 0..steps {
            match words.next().unwrap() {
                0 => {
                    let n = words.next().unwrap() as usize;
                    let packet: Vec<_> =
                        words.by_ref().take(n).flat_map(u32::to_le_bytes).collect();
                    let location = queue.append_packet(&packet, grow).unwrap();
                    assert_eq!(
                        [location.chunk as u32, location.offset as u32],
                        [words.next().unwrap(), words.next().unwrap()],
                        "location case{case} step{step}"
                    );
                }
                1 => queue.reset(false).unwrap(),
                2 => queue.reset(true).unwrap(),
                3 => {
                    let mut trace = Vec::new();
                    queue
                        .dispatch(|group, packet| {
                            let size = u16::from_le_bytes(packet[..2].try_into().unwrap()) as usize;
                            trace.extend([u32::from(group), size as u32]);
                            trace.extend(
                                packet[..size]
                                    .chunks_exact(4)
                                    .map(|v| u32::from_le_bytes(v.try_into().unwrap())),
                            );
                            if case % 4 == 0 && packet[4] & 1 != 0 {
                                packet[..2].copy_from_slice(&0u16.to_le_bytes());
                            }
                            Ok(())
                        })
                        .unwrap();
                    let n = words.next().unwrap() as usize;
                    assert_eq!(
                        trace,
                        words.by_ref().take(n).collect::<Vec<_>>(),
                        "dispatch case{case} step{step}"
                    );
                }
                _ => panic!("unknown queue operation"),
            }
            let actual = snapshot(&mut queue);
            assert_eq!(
                actual,
                words.by_ref().take(actual.len()).collect::<Vec<_>>(),
                "state case{case} step{step}"
            );
        }
        assert_eq!(words.next(), None);
        cases += 1;
    }
    assert_eq!(cases, 128);
}

#[test]
fn concurrent_command_producers_keep_every_reserved_payload() {
    let mut queue = CommandBuffer::new(128, 5).unwrap();
    std::thread::scope(|scope| {
        for producer in 0..8 {
            let queue = &queue;
            scope.spawn(move || {
                for i in 0..512 {
                    let id = (producer * 512 + i) as u32;
                    let packet: Vec<_> = [0x02000010, id, id ^ 0xa5a5a5a5, !id]
                        .into_iter()
                        .flat_map(u32::to_le_bytes)
                        .collect();
                    queue.append_packet(&packet, false).unwrap();
                }
            });
        }
    });
    let mut seen = vec![false; 4096];
    let mut count = 0;
    queue
        .dispatch(|group, packet| {
            assert_eq!(group, 2);
            assert_eq!(u16::from_le_bytes(packet[..2].try_into().unwrap()), 16);
            let word = |i| u32::from_le_bytes(packet[i..i + 4].try_into().unwrap());
            let id = word(4);
            assert_eq!(word(8), id ^ 0xa5a5a5a5);
            assert_eq!(word(12), !id);
            assert!(!seen[id as usize]);
            seen[id as usize] = true;
            count += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(count, 4096);
    assert!(seen.into_iter().all(|v| v));
    let storage = queue.storage.get_mut();
    assert!(storage
        .chunks
        .iter()
        .all(|c| c.reserved.load(Ordering::SeqCst) == 0 && c.records.load(Ordering::SeqCst) == 0));
}
