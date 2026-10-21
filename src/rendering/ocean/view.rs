//! 3014b0's per-view water state and FFT controls. Environment/owner inputs are explicit.
use super::{update_ocean_wave_parameters, WaveSpectrumParameters};

#[derive(Clone, Copy, Debug)]
pub struct OceanViewInput {
    pub enabled: bool,
    pub enable_ssr: bool,
    pub disable_ssr_on_rough_water: bool,
    pub use_sky_cube_map: bool,
    pub water_volume: bool,
    pub surface_water_level: f32,
    pub ocean_water_level: f32,
    pub max_wave_amplitude: f32,
    pub wind: [f32; 2],
    pub foam_type: f32,
    pub wind_speed: f32,
    pub wind_dependency: f32,
    pub small_waves: f32,
    pub choppiness: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct OceanViewState {
    pub enabled: bool,
    pub enable_ssr: bool,
    pub disable_ssr_on_rough_water: bool,
    pub use_sky_cube_map: bool,
    pub water_volume: bool,
    pub tile_size: f32,
    pub water_level: f32,
    pub max_wave_amplitude: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct OceanWaveControls {
    pub wind: [f32; 2],
    pub foam_type: i32,
    pub amplitude: f32,
    pub wind_speed: f32,
    pub wind_dependency: f32,
    pub domain_length: f32,
    pub small_waves: f32,
    pub choppiness: f32,
}
impl OceanWaveControls {
    /// Source nine-word layout; slot2 contains an INTEGER bit pattern, not
    /// its floating numerical value. The verified cache copier preserves it.
    pub fn parameter_slots(self) -> [f32; 9] {
        [
            self.wind[0],
            self.wind[1],
            f32::from_bits(self.foam_type as u32),
            self.amplitude,
            self.wind_speed,
            self.wind_dependency,
            self.domain_length,
            self.small_waves,
            self.choppiness,
        ]
    }
    pub fn spectrum_parameters(self) -> WaveSpectrumParameters {
        WaveSpectrumParameters {
            wind: self.wind,
            amplitude: self.amplitude,
            wind_speed: self.wind_speed,
            opposite_wind_attenuation: self.wind_dependency,
            domain_length: self.domain_length,
            short_wave_divisor: self.small_waves,
        }
    }
    /// 3014b0 refreshes the simulation cache only if owner+d09 or+d0a is set.
    pub fn update_simulation_cache(
        self,
        cached: &mut [f32; 9],
        dirty: &mut bool,
        refresh_displacement: bool,
        refresh_spectrum: bool,
    ) {
        if refresh_displacement || refresh_spectrum {
            update_ocean_wave_parameters(cached, dirty, self.parameter_slots());
        }
    }
}

pub fn prepare_ocean_view(
    input: &OceanViewInput,
    previous_water_level: f32,
    tile_size: f32,
    update_ocean_height: bool,
) -> (OceanViewState, OceanWaveControls) {
    let [mut x, mut y] = input.wind;
    let threshold = f32::from_bits(0x3c23d70a);
    if x.abs() < threshold && y.abs() < threshold {
        x = f32::from_bits(0x3f19999a);
        y = f32::from_bits(0x3f4ccccd);
    }
    // MAXSS's second operand wins for NaN, as well as clamping tiny lengths.
    let squared = (y * y + x * x).max(f32::MIN_POSITIVE);
    let inverse = 1.0 / squared.sqrt();
    let foam_type =
        crate::rendering::render_state::cvtt_f32((input.foam_type + 0.5).floor()) as i32;
    let state = OceanViewState {
        enabled: input.enabled,
        enable_ssr: input.enable_ssr,
        disable_ssr_on_rough_water: input.disable_ssr_on_rough_water,
        use_sky_cube_map: input.use_sky_cube_map,
        water_volume: input.water_volume,
        tile_size,
        water_level: if input.water_volume {
            input.surface_water_level
        } else if update_ocean_height {
            input.ocean_water_level
        } else {
            previous_water_level
        },
        max_wave_amplitude: input.max_wave_amplitude,
    };
    let controls = OceanWaveControls {
        wind: [inverse * x, inverse * y],
        foam_type,
        amplitude: f32::from_bits(0x3e4ccccd),
        wind_speed: input.wind_speed,
        wind_dependency: input.wind_dependency,
        domain_length: 50.0,
        small_waves: input.small_waves,
        choppiness: input.choppiness,
    };
    (state, controls)
}

#[cfg(test)]
mod tests;
