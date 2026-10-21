use super::*;

fn uniform(words: &[u32]) -> Vec<u8> {
    use super::super::{
        water_reflection::WaterReflectionParameters,
        water_refraction::{WaterRefractionParameters, WaterShadowParameters},
    };
    let f = |i: usize| std::array::from_fn(|j| f32::from_bits(words[i + j]));
    let reflection = WaterReflectionParameters {
        view_from_world: std::array::from_fn(|i| f32::from_bits(words[i])),
        camera_to_clip: std::array::from_fn(|i| f32::from_bits(words[16 + i])),
        unproject: f(32),
        thickness: f(36),
        trace: f(40),
        screen_hdr: f(44),
        options: words[48..52].try_into().unwrap(),
    };
    let refraction = WaterRefractionParameters {
        clock_caustics: f(52),
        screen: f(56),
        unproject: f(60),
        light_level: f(64),
        extinction: f(68),
        hdr: f(72),
    };
    let shadow = WaterShadowParameters {
        world_to_shadow: std::array::from_fn(|i| {
            std::array::from_fn(|j| f32::from_bits(words[76 + i * 16 + j]))
        }),
        view_row: f(156),
        slice_fars: f(160),
        sun_direction: f(164),
        shadow_size: f(168),
        bias: f(172),
        options: words[176..180].try_into().unwrap(),
    };
    let surface = WaterSurfaceParameters {
        underwater_visibility_boost: f32::from_bits(words[180]),
        shore_foam_luminance: f32::from_bits(words[181]),
        use_sun: words[184] != 0,
        disable_ssr_on_rough_water: words[185] != 0,
    };
    let mut bytes = reflection.uniform_bytes().to_vec();
    bytes.extend(super::super::water_refraction::uniform_bytes(
        &refraction,
        &shadow,
    ));
    bytes.extend(surface.uniform_bytes());
    bytes.extend(words[188..].iter().flat_map(|v| v.to_le_bytes()));
    bytes
}

#[test]
#[ignore = "requires GPU; complete installed above/below-water shading with actual textures"]
fn water_surface_matches_installed_shader() {
    super::super::test_water::compare(
        SHADER_SOURCE,
        include_str!("tests.wgsl"),
        include_str!("../../../../tests/fixtures/water_surface.txt"),
        true,
        uniform,
    );
}
