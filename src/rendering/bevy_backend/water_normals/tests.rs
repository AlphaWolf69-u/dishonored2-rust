use super::*;

#[test]
#[ignore = "requires graphics adapter; compares installed placed-water normal functions"]
fn water_normals_match_installed_functions() {
    let source = format!("{SHADER_SOURCE}\n{}", include_str!("tests.wgsl"));
    let frames = super::super::test_water_inputs::compare_noise_inputs(
        &source,
        include_str!("../../../../tests/fixtures/water_normals.txt"),
        12,
        8,
        |header| {
            let f = |i| f32::from_bits(header[i]);
            let settings = crate::rendering::water::WaterWaveSettings {
                maximum_octaves: f(1),
                anti_alias: f(2),
                frequency: f(3),
                speed: f(4),
                persistence: f(5),
                amplitude: f(6),
            };
            WaterNormalParameters::from_settings(&settings, f(0))
                .uniform_bytes()
                .to_vec()
        },
    );
    assert_eq!(frames, 3);
}
