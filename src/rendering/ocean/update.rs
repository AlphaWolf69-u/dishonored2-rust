//! Retained state for3014b0 and the two305270/305300 wave graph tasks.
//! Current water-contact overrides and authored view inputs belong to the caller.
use super::{prepare_ocean_view, OceanViewInput, OceanViewState, OceanWaveControls};

#[derive(Clone, Debug)]
pub struct OceanUpdateState {
    pub view: OceanViewState,
    parameters: [f32; 9],
    spectrum_dirty: bool,
    refresh_surfaces: bool,
    refresh_spectrum: bool,
}

impl Default for OceanUpdateState {
    fn default() -> Self {
        Self {
            // 300e20 stores a QWORD at+80: tile size50, water height ZERO.
            // It is not a 50-unit initial water height.
            view: OceanViewState {
                enabled: false,
                enable_ssr: false,
                disable_ssr_on_rough_water: false,
                use_sky_cube_map: false,
                water_volume: false,
                tile_size: 50.,
                water_level: 0.,
                max_wave_amplitude: 5.,
            },
            // 2f9a80's nine-word cache; successful2fa110 marks spectrum dirty.
            parameters: [1., 0., 0., 0., 0., 0., 50., 0., 0.],
            spectrum_dirty: true,
            // 302960 initializes owner+d09/d0a independently.
            refresh_surfaces: true,
            refresh_spectrum: true,
        }
    }
}

impl OceanUpdateState {
    /// 35f5b0 resets both graph-task flags on its admitted world-frame branch,
    /// before its views. Displacement clears, query reset and admission are separate.
    pub fn begin_frame(&mut self) {
        self.refresh_surfaces = true;
        self.refresh_spectrum = true;
    }

    /// `update_ocean_height` is the real water-contact owner's+d08 flag.
    /// Do not derive it from ordinary camera-area containment.
    pub fn prepare(
        &mut self,
        input: &OceanViewInput,
        tile_size: f32,
        update_ocean_height: bool,
    ) -> OceanWaveControls {
        let (view, controls) =
            prepare_ocean_view(input, self.view.water_level, tile_size, update_ocean_height);
        controls.update_simulation_cache(
            &mut self.parameters,
            &mut self.spectrum_dirty,
            self.refresh_surfaces,
            self.refresh_spectrum,
        );
        self.view = view;
        controls
    }

    /// The cached controls may differ from this frame's prepared controls when
    /// both refresh flags are clear. Preserve that distinction for FFT inputs.
    pub fn cached_controls(&self) -> OceanWaveControls {
        let p = self.parameters;
        OceanWaveControls {
            wind: [p[0], p[1]],
            foam_type: p[2].to_bits() as i32,
            amplitude: p[3],
            wind_speed: p[4],
            wind_dependency: p[5],
            domain_length: p[6],
            small_waves: p[7],
            choppiness: p[8],
        }
    }

    pub fn spectrum_dirty(&self) -> bool {
        self.spectrum_dirty
    }

    /// Graph callbacks305370/3053b0 skip a disabled ocean without consuming
    /// either refresh flag. The packed patch-array count occupies24bits.
    pub fn needs_spectrum(&self, packed_patch_count: u32) -> bool {
        self.view.enabled && (packed_patch_count & 0x00ff_ffff != 0 || self.refresh_spectrum)
    }

    pub fn needs_surfaces(&self, packed_patch_count: u32) -> bool {
        self.view.enabled && (packed_patch_count & 0x00ff_ffff != 0 || self.refresh_surfaces)
    }

    /// 305270 submits queries even when its surface refresh was skipped.
    pub fn needs_height_queries(&self, view_flags: u8) -> bool {
        self.view.enabled && view_flags & 1 == 0
    }

    /// Call only after the admitted spectrum initialization/FFT was recorded.
    pub fn spectrum_recorded(&mut self) {
        self.spectrum_dirty = false;
        self.refresh_spectrum = false;
    }

    /// Collision-height and foam production are part of2fb5e0's surface task.
    pub fn surfaces_recorded(&mut self) {
        self.refresh_surfaces = false;
    }
}

#[cfg(test)]
mod tests;
