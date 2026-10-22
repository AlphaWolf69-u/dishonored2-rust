use super::*;

#[test]
fn ordered_contacts_match_executed_view_selection() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/environment_contacts.txt").lines() {
        let row = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        let count = row[2] as usize;
        let contacts = row[3..3 + count * 10]
            .chunks_exact(10)
            .map(|record| EnvironmentContact {
                water_component_flags: (record[1] != 0).then_some(record[2]),
                environment: (record[3] != 0).then(|| EnvironmentVolumeContact {
                    index: record[0] as usize,
                    is_water_volume: record[3] == 2,
                    bounds: std::array::from_fn(|i| {
                        std::array::from_fn(|j| f32::from_bits(record[4 + i * 3 + j]))
                    }),
                }),
            })
            .collect::<Vec<_>>();
        let mut selection = EnvironmentContactSelection::new(f32::from_bits(row[1]));
        // Exercise clearing stale selections independently of the fresh query.
        selection.water = Some(999);
        selection.touches_water_trigger = true;
        selection.areas.push(999);
        selection.sizes.push(0.);
        selection.update((row[0] != 0).then_some(contacts.as_slice()));
        let result = &row[3 + count * 10..];
        let n = result[0] as usize;
        assert_eq!(
            selection.areas,
            result[1..1 + n]
                .iter()
                .map(|&v| v as usize)
                .collect::<Vec<_>>(),
            "case{cases}"
        );
        assert_eq!(
            selection.water.map_or(u32::MAX, |v| v as u32),
            result[1 + n],
            "case{cases}"
        );
        assert_eq!(
            selection.water_level.to_bits(),
            result[2 + n],
            "case{cases}"
        );
        assert_eq!(
            u32::from(selection.touches_water_trigger),
            result[3 + n],
            "case{cases}"
        );
        assert_eq!(selection.update_ocean_height(), result[3 + n] == 0);
        cases += 1;
    }
    assert_eq!(cases, 128);
}
