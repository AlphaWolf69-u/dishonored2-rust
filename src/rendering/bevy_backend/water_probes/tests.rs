use super::*;

#[test]
#[ignore = "requires graphics adapter; compares source tetrahedron probe selection"]
fn water_probe_selection_matches_installed_interpolation() {
    let source = format!("{SHADER_SOURCE}\n{}", include_str!("select_tests.wgsl"));
    let frames = super::super::test_water_inputs::compare_exact_inputs(
        &source,
        include_str!("../../../../tests/fixtures/water_probe_selection.txt"),
        40,
        4,
        |header| header.iter().flat_map(|v| v.to_le_bytes()).collect(),
    );
    assert_eq!(frames, 1);
}

#[test]
#[ignore = "requires graphics adapter; compares installed PC water probe transport"]
fn water_probes_match_installed_transport() {
    let source = format!("{SHADER_SOURCE}\n{}", include_str!("tests.wgsl"));
    let frames = super::super::test_water_inputs::compare_exact_inputs(
        &source,
        include_str!("../../../../tests/fixtures/water_probes.txt"),
        8,
        12,
        |header| header.iter().flat_map(|v| v.to_le_bytes()).collect(),
    );
    assert_eq!(frames, 4);
}
