use super::*;

fn parameters(words: &[u32]) -> WaterReflectionParameters {
    let f = |offset: usize| std::array::from_fn(|i| f32::from_bits(words[offset + i]));
    WaterReflectionParameters {
        view_from_world: std::array::from_fn(|i| f32::from_bits(words[i])),
        camera_to_clip: std::array::from_fn(|i| f32::from_bits(words[16 + i])),
        unproject: f(32),
        thickness: f(36),
        trace: f(40),
        screen_hdr: f(44),
        options: words[48..52].try_into().unwrap(),
    }
}

#[test]
#[ignore = "requires GPU; complete installed water reflection with depth, HDR, fog and cubes"]
fn water_reflection_matches_installed_shader() {
    super::super::test_water::compare(
        SHADER_SOURCE,
        include_str!("tests.wgsl"),
        include_str!("../../../../tests/fixtures/water_reflection.txt"),
        false,
        |words| {
            let mut bytes = parameters(words).uniform_bytes().to_vec();
            bytes.extend(words[52..].iter().flat_map(|v| v.to_le_bytes()));
            bytes
        },
    );
}
