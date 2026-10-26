use super::*;

pub(crate) fn snapshot(list: &MotionRequestList) -> Vec<u32> {
    let mut out = vec![
        list.timed_count as u32,
        list.next_id,
        list.cached_id,
        list.entries.len() as u32,
    ];
    for entry in &list.entries {
        out.extend(entry.input.velocity.map(f32::to_bits));
        out.extend([
            entry.id,
            entry.input.priority as u32,
            entry.input.remaining.to_bits(),
            entry.input.lifetime as u32,
            u32::from(entry.input.flags),
        ]);
    }
    out
}

#[test]
fn ordered_timed_requests_match_complete_executed_operations() {
    let mut list = MotionRequestList::from_parts(Vec::new(), 0, 0, 0, false).unwrap();
    let mut count = 0;
    for row in include_str!("../../../tests/fixtures/motion_requests.txt").lines() {
        let mut parts = row.split_whitespace();
        let kind = parts.next().unwrap();
        let mut words = parts.map(|v| u32::from_str_radix(v, 16).unwrap());
        if kind == "initial" {
            list = MotionRequestList::from_parts(
                Vec::new(),
                0,
                words.next().unwrap(),
                words.next().unwrap(),
                false,
            )
            .unwrap();
            assert!(words.next().is_none());
            continue;
        }
        let result = match words.next().unwrap() {
            0 => list.push(MotionRequestInput {
                velocity: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
                priority: words.next().unwrap() as i32,
                remaining: f32::from_bits(words.next().unwrap()),
                lifetime: words.next().unwrap() as i32,
                flags: words.next().unwrap() as u8,
            }),
            1 => u32::from(list.advance(f32::from_bits(words.next().unwrap()))),
            2 => u32::from(list.remove_at(words.next().unwrap() as i32)),
            _ => unreachable!(),
        };
        let mut actual = vec![result];
        actual.extend(snapshot(&list));
        assert_eq!(actual, words.collect::<Vec<_>>(), "case {count}");
        count += 1;
    }
    assert_eq!(count, 1536);
}
