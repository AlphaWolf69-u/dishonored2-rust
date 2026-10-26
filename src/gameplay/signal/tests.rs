use super::*;
#[test]
fn signal_slots_match_executed_subscription_and_emission() {
    let mut signal = Signal::<u32, u32>::default();
    let mut count = 0;
    for (case, line) in include_str!("../../../tests/fixtures/signal_slots.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap());
        if words.next().unwrap() != 0 {
            signal = Signal::default();
        }
        let kind = words.next().unwrap();
        let sub = words.next().unwrap();
        let key = words.next().unwrap();
        let n = words.next().unwrap() as usize;
        let mut actions: Vec<([u32; 5], bool)> = (0..n)
            .map(|_| (std::array::from_fn(|_| words.next().unwrap()), false))
            .collect();
        let mut removed = u32::MAX;
        let mut events = Vec::new();
        let mut action_results = Vec::new();
        match kind {
            0 => signal.subscribe(sub, key),
            1 => removed = u32::from(signal.unsubscribe(sub, key)),
            2 => signal.emit(&(), |signal, subscriber, callback, _| {
                events.extend([subscriber, callback]);
                for (action, used) in &mut actions {
                    if !*used && action[0] == subscriber && action[1] == callback {
                        *used = true;
                        if action[2] == 0 {
                            signal.subscribe(action[3], action[4]);
                            action_results.push(u32::MAX);
                        } else if action[2] == 1 {
                            action_results
                                .push(u32::from(signal.unsubscribe(action[3], action[4])));
                        } else {
                            signal.destroy();
                            action_results.push(0xffff_fffe);
                        }
                    }
                }
            }),
            3 => signal.head.flags = sub as u8,
            4 => signal.destroy(),
            _ => unreachable!(),
        }
        let mut slots = Vec::new();
        let mut at = signal.head.target;
        while let Some(index) = at {
            let slot = signal.slots[index].as_ref().unwrap();
            slots.extend([slot.subscriber, slot.callback, u32::from(slot.next.flags)]);
            at = slot.next.target;
        }
        let mut actual = vec![removed, (events.len() / 2) as u32];
        actual.extend(events);
        actual.push(action_results.len() as u32);
        actual.extend(action_results);
        actual.extend([u32::from(signal.head.flags), (slots.len() / 3) as u32]);
        actual.extend(slots);
        let expected: Vec<_> = words.collect();
        assert_eq!(actual, expected, "case {case}");
        count += 1;
    }
    assert_eq!(count, 512);
}
