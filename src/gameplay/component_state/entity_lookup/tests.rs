use super::*;
use crate::gameplay::component_state::ComponentOwner;

#[test]
fn component_lookup_matches_complete_executed_cached_and_linear_paths() {
    let mut count = 0;
    for row in include_str!("../../../../tests/fixtures/entity_component_lookup.txt").lines() {
        let w: Vec<_> = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        let mut entity = EntityComponents::new(17);
        entity.phase = w[1];
        let records: Vec<_> = w[8..8 + w[6] as usize * 6].chunks_exact(6).collect();
        let owners: Vec<_> = records
            .iter()
            .map(|record| {
                let mut owner = ComponentOwner {
                    flags: record[1],
                    ..Default::default()
                };
                if record[4] != 0 {
                    owner.mark_initialized();
                }
                assert_eq!(owner.flags, record[5]);
                owner
            })
            .collect();
        for (index, record) in records.iter().enumerate() {
            entity
                .entries
                .append(
                    ComponentHandle {
                        class_id: record[0] as i32,
                        slot: index as u16,
                    }
                    .packed(),
                    false,
                )
                .unwrap();
        }
        entity.cached_classes = (w[0] != 0).then(|| {
            records
                .iter()
                .take(w[7] as usize)
                .map(|r| r[0] as u16)
                .collect()
        });
        let result = entity
            .find_component([w[2] as i32, w[3] as i32], [w[4], w[5]], |h| {
                let record = records[h.slot as usize];
                Ok(ComponentLookupState {
                    flags: owners[h.slot as usize].flags,
                    start_phases: (record[3] != 0).then_some(record[2]),
                })
            })
            .unwrap();
        assert_eq!(
            result.map_or(u32::MAX, |h| u32::from(h.slot)),
            *w.last().unwrap(),
            "case{count}"
        );
        count += 1;
    }
    assert_eq!(count, 1024);
}
