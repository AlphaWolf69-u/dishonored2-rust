use super::*;

#[test]
fn dispatch_and_release_match_executed_chains_and_deferred_destruction() {
    use std::cell::RefCell;
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/body_property_dispatch.txt").lines() {
        let mut words = line
            .split_whitespace()
            .map(|v| u64::from_str_radix(v, 16).unwrap());
        let key = words.next().unwrap() as u16;
        let bodies = std::array::from_fn(|_| words.next().unwrap() as u32);
        let release_target = words.next().unwrap() as u32;
        let after = words.next().unwrap() as u32;
        let mut table = BodyProperties::default();
        for body in 0..6 {
            for (j, key) in [0, 3, 10, 17, 9, 18].into_iter().enumerate() {
                *table.get_or_create(6, body, key).unwrap() =
                    0x1000 + u64::from(body) * 6 + j as u64;
            }
        }
        let events = RefCell::new(Vec::new());
        let destroyed = RefCell::new(Vec::new());
        let value_addresses: Vec<_> = table
            .properties
            .iter()
            .map(|p| &p.value as *const u64 as usize)
            .collect();
        let mut destroy = |value: &mut u64| {
            let address = value as *mut u64 as usize;
            destroyed
                .borrow_mut()
                .push(value_addresses.iter().position(|&v| v == address).unwrap());
            *value ^= 0xf000;
            Ok(())
        };
        let mut did_release = false;
        table
            .dispatch(
                key,
                bodies,
                |table, property, index| {
                    events
                        .borrow_mut()
                        .extend([u64::from(property), index as u64]);
                    *table.value_mut(property).unwrap() ^= 1;
                    if !did_release {
                        did_release = true;
                        // Destruction must be deferred; this callback cannot run yet.
                        table.release_body(release_target, |_| {
                            panic!("destroyed during dispatch")
                        })?;
                    }
                    Ok(())
                },
                &mut destroy,
            )
            .unwrap();
        table.release_body(after, &mut destroy).unwrap();
        let events = events.into_inner();
        let mut actual = vec![events.len() as u64 / 2];
        actual.extend_from_slice(&events);
        let destroyed = destroyed.into_inner();
        actual.push(destroyed.len() as u64);
        actual.extend(destroyed.iter().map(|&v| v as u64));
        actual.extend(table.heads.iter().map(|&v| u64::from(v)));
        actual.extend([
            u64::from(table.free.unwrap_or(u16::MAX)),
            u64::from(table.dispatching),
            table.deferred_release.len() as u64,
        ]);
        for p in &table.properties {
            actual.extend([u64::from(p.next), u64::from(p.key), p.value]);
        }
        assert_eq!(actual, words.collect::<Vec<_>>(), "case {cases}");
        cases += 1;
    }
    assert_eq!(cases, 256);
}

#[test]
fn keyed_body_values_match_executed_lookup_and_allocation() {
    let mut table = BodyProperties::default();
    let mut count = 0;
    for line in include_str!("../../../tests/fixtures/body_property_values.txt").lines() {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let reset = words.next().unwrap() != 0;
        let seeds = words.next().unwrap() as usize;
        if reset {
            table = BodyProperties::default();
        }
        for i in 0..seeds {
            let value = u64::from(words.next().unwrap()) | (u64::from(words.next().unwrap()) << 32);
            table.properties.push(Property {
                next: if i + 1 < seeds {
                    (i + 1) as u16
                } else {
                    u16::MAX
                },
                key: 0xaaaa + i as u16,
                value,
            });
        }
        if seeds > 0 {
            table.free = Some(0);
        }
        let capacity = words.next().unwrap() as usize;
        let body = words.next().unwrap();
        let key = words.next().unwrap() as u16;
        let value = u64::from(words.next().unwrap()) | (u64::from(words.next().unwrap()) << 32);
        let old = u64::from(words.next().unwrap()) | (u64::from(words.next().unwrap()) << 32);
        let slot = table.get_or_create(capacity, body, key).unwrap();
        assert_eq!(*slot, old, "case {count}: new versus reused value");
        *slot = value;
        let mut actual = vec![table.heads.len() as u32];
        actual.extend(table.heads.iter().map(|&v| u32::from(v)));
        actual.push(u32::from(table.free.unwrap_or(u16::MAX)));
        actual.push(table.properties.len() as u32);
        for property in &table.properties {
            actual.extend([
                u32::from(property.next),
                u32::from(property.key),
                property.value as u32,
                (property.value >> 32) as u32,
            ]);
        }
        assert_eq!(actual, words.collect::<Vec<_>>(), "case {count}");
        count += 1;
    }
    assert_eq!(count, 1024);
}
