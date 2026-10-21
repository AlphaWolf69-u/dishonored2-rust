use super::*;

#[test]
#[ignore = "requires graphics adapter and installed registered water defaults"]
fn water_colour_inputs_match_installed_fragment() {
    let catalog =
        crate::assets::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
            .unwrap();
    let defaults = crate::assets::entities::EnvironmentRenderParameters { layers: Vec::new() };
    let reader = crate::assets::environment::ParameterReader {
        catalog: &catalog,
        parameters: &defaults,
    };
    let mut settings = crate::rendering::water::WaterSettings::resolve(&reader).unwrap();
    let source = format!("{}\n{}", shader_source(), include_str!("tests.wgsl"));
    let frames = super::super::test_water_inputs::compare_noise_inputs(
        &source,
        include_str!("../../../../tests/fixtures/water_colour.txt"),
        16,
        36,
        |header| {
            let f = |i| f32::from_bits(header[i]);
            settings.waves = crate::rendering::water::WaterWaveSettings {
                maximum_octaves: f(1),
                anti_alias: f(2),
                frequency: f(3),
                speed: f(4),
                persistence: f(5),
                amplitude: f(6),
            };
            settings.refraction = f(27);
            settings.colour = [f(28), f(29), f(30)];
            settings.extinction = f(31);
            settings.inscattering = f(35);
            settings.specular_power = f(39);
            WaterColourParameters::from_settings(
                &settings,
                f(0),
                [f(24), f(25), f(26)],
                std::array::from_fn(|i| f(i + 8)),
                ([f(32), f(33), f(34)], [f(36), f(37), f(38)]),
                header[40] != 0,
            )
            .uniform_bytes()
            .to_vec()
        },
    );
    assert_eq!(frames, 6);
}
