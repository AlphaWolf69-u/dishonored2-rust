use super::*;

fn equal_float(actual: f32, expected: u32) {
    if f32::from_bits(expected).is_nan() {
        assert!(actual.is_nan());
    } else {
        assert_eq!(actual.to_bits(), expected);
    }
}

#[test]
fn ocean_view_and_wave_controls_match_executed_preparation() {
    let mut cases = 0;
    for line in include_str!("../../../../tests/fixtures/ocean_view_controls.txt").lines() {
        let row = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(row.len(), 54);
        let float = |i: usize| f32::from_bits(row[i]);
        let input = OceanViewInput {
            enabled: row[0] != 0,
            enable_ssr: row[1] != 0,
            disable_ssr_on_rough_water: row[2] != 0,
            use_sky_cube_map: row[3] != 0,
            water_volume: row[4] != 0,
            surface_water_level: float(11),
            ocean_water_level: float(12),
            max_wave_amplitude: float(13),
            wind: [float(14), float(15)],
            foam_type: float(16),
            wind_speed: float(17),
            wind_dependency: float(18),
            small_waves: float(19),
            choppiness: float(20),
        };
        let (state, controls) = prepare_ocean_view(&input, float(10), float(9), row[5] != 0);
        let flags = u32::from(state.enabled)
            | (u32::from(state.enable_ssr) << 8)
            | (u32::from(state.disable_ssr_on_rough_water) << 16)
            | (u32::from(state.use_sky_cube_map) << 24);
        assert_eq!(flags, row[30]);
        assert_eq!(u32::from(state.water_volume), row[31]);
        equal_float(state.tile_size, row[32]);
        equal_float(state.water_level, row[33]);
        equal_float(state.max_wave_amplitude, row[34]);
        for (actual, &expected) in controls.parameter_slots().iter().zip(&row[35..44]) {
            equal_float(*actual, expected);
        }
        let mut cached = std::array::from_fn(|j| float(21 + j));
        let mut dirty = row[8] != 0;
        controls.update_simulation_cache(&mut cached, &mut dirty, row[6] != 0, row[7] != 0);
        for (actual, &expected) in cached.iter().zip(&row[44..53]) {
            equal_float(*actual, expected);
        }
        assert_eq!(dirty, row[53] != 0);
        let spectrum = controls.spectrum_parameters();
        equal_float(spectrum.wind[0], row[35]);
        equal_float(spectrum.wind[1], row[36]);
        equal_float(spectrum.amplitude, row[38]);
        equal_float(spectrum.wind_speed, row[39]);
        equal_float(spectrum.opposite_wind_attenuation, row[40]);
        equal_float(spectrum.domain_length, row[41]);
        equal_float(spectrum.short_wave_divisor, row[42]);
        cases += 1;
    }
    assert_eq!(cases, 528);
}
