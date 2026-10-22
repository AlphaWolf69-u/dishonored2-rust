use super::*;

#[test]
fn update_gates_match_executed_graph_callbacks() {
    let mut cases = 0;
    for line in include_str!("../../../../tests/fixtures/ocean_update_gates.txt").lines() {
        let row = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(row.len(), 17);
        let mut state = OceanUpdateState::default();
        state.view.enabled = row[0] != 0;
        state.refresh_surfaces = row[2] != 0;
        state.refresh_spectrum = row[3] != 0;
        let mut calls = [0, 0, 0];
        for repeat in 0..2 {
            if state.needs_spectrum(row[1]) {
                calls[0] += 1;
                state.spectrum_recorded();
            }
            if state.needs_surfaces(row[1]) {
                calls[1] += 1;
                state.surfaces_recorded();
            }
            if state.needs_height_queries(row[4] as u8) {
                calls[2] += 1;
            }
            let offset = 5 + repeat * 6;
            assert_eq!(calls, row[offset..offset + 3], "case{cases} repeat{repeat}");
            assert_eq!(u32::from(state.refresh_surfaces), row[offset + 3]);
            assert_eq!(u32::from(state.refresh_spectrum), row[offset + 4]);
            assert_eq!(u32::from(!state.view.enabled), row[offset + 5]);
        }
        cases += 1;
    }
    assert_eq!(cases, 96);
}
