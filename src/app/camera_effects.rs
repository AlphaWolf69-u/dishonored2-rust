//! Prepared camera-area parameters. All archive reads finish at level loading;
//! publication uses the same presence-aware parameter transitions as lenses.
use crate::{
    assets::{archive::Catalog, camera_blur::BlurResources, environment::ParameterReader},
    entities::EnvironmentRenderParameters,
    platform::game_settings::{read_camera_render_settings, CameraRenderSettings},
    rendering::{
        camera_effects::{CameraEffectsFrame, CameraSettings, SettingsInputs, PARAMETER_NAMES},
        environment::{AreaEnvironment, EnvironmentSource},
        environment_tracks::{FloatParameter, ParameterTrack},
    },
};
use bevy::math::Vec4;
struct Inputs {
    values: [Option<Vec4>; 33],
}
pub(super) struct CameraScene {
    blur_resources: std::sync::Arc<BlurResources>,
    world: Inputs,
    areas: Vec<(String, Inputs)>,
    /// The pause screen FX's override environment.
    screen_fx: Option<Inputs>,
    tracks: [ParameterTrack<FloatParameter>; 33],
    controls: CameraRenderSettings,
    state: CameraSettings,
    initialized: bool,
}
impl CameraScene {
    pub(super) fn load(
        catalog: &Catalog,
        world: &EnvironmentRenderParameters,
        areas: &[AreaEnvironment],
        screen_fx: Option<&EnvironmentRenderParameters>,
    ) -> Result<Self, String> {
        let empty = EnvironmentRenderParameters { layers: Vec::new() };
        let defaults = ParameterReader {
            catalog,
            parameters: &empty,
        };
        let values = PARAMETER_NAMES
            .iter()
            .map(|name| defaults.vector(name).map(Vec4::from_array))
            .collect::<Result<Vec<_>, _>>()?;
        let prepare = |parameters: &EnvironmentRenderParameters| -> Result<Inputs, String> {
            let reader = ParameterReader {
                catalog,
                parameters,
            };
            let mut values = [None; 33];
            for (i, name) in PARAMETER_NAMES.iter().enumerate() {
                if parameters.parameter(name).is_some() {
                    values[i] = Some(Vec4::from_array(reader.vector(name)?));
                }
            }
            Ok(Inputs { values })
        };
        let mut state = CameraSettings::from_words([0; 41]);
        state.reset();
        Ok(Self {
            blur_resources: std::sync::Arc::new(BlurResources::load(catalog)?),
            world: prepare(world)?,
            areas: areas
                .iter()
                .map(|area| Ok((area.id.clone(), prepare(&area.parameters)?)))
                .collect::<Result<_, String>>()?,
            screen_fx: screen_fx.map(prepare).transpose()?,
            tracks: std::array::from_fn(|i| ParameterTrack::new(FloatParameter(values[i]))),
            controls: read_camera_render_settings()?,
            state,
            initialized: false,
        })
    }
    pub(super) fn blur_resources(&self) -> &std::sync::Arc<BlurResources> {
        &self.blur_resources
    }
    pub(super) fn update(
        &mut self,
        areas: &[AreaEnvironment],
        screen_fx: f32,
        position: [f32; 3],
        world_duration: f32,
        delta: f32,
    ) -> Result<CameraEffectsFrame, String> {
        if areas.len() != self.areas.len()
            || areas
                .iter()
                .zip(&self.areas)
                .any(|(a, (id, _))| &a.id != id)
        {
            return Err("camera areas changed without preparing their inputs".into());
        }
        let active: Vec<_> = areas
            .iter()
            .enumerate()
            .filter(|(_, area)| area.contains(position))
            .collect();
        let mut values = [0.; 33];
        for i in 0..33 {
            let mut selection = self.world.values[i].map(|v| (EnvironmentSource::World, v));
            let mut duration = world_duration;
            for (index, area) in &active {
                if let Some(value) = self.areas[*index].1.values[i] {
                    selection = Some((EnvironmentSource::Area(*index), value));
                    duration = area.blend_duration;
                }
            }
            values[i] = self.tracks[i]
                .update(
                    selection.map(|(source, value)| (source, FloatParameter(value))),
                    duration,
                    delta,
                    self.initialized,
                    |start, target, t| FloatParameter((target.0 - start.0) * t + start.0),
                )
                .0
                .x;
            // 88e220's override stage: base + (v - base)·w.
            if let Some(v) = self.screen_fx.as_ref().and_then(|s| s.values[i]).filter(|_| screen_fx > 0.) {
                values[i] += (v.x - values[i]) * screen_fx.clamp(0., 1.);
            }
        }
        self.initialized = true;
        // This free-camera gameplay view has no optics-suppression mode. Future
        // menu/cutscene owners must publish their actual 1439a529d state separately.
        self.state.update(SettingsInputs {
            environment: &values,
            console: self.controls.console.as_ref(),
            autofocus: self.controls.autofocus,
            near_clip: self.controls.near_clip,
            far_clip: self.controls.far_clip,
            suppress_optics: false,
        });
        Ok(CameraEffectsFrame {
            settings: self.state.clone(),
            near_clip: self.controls.near_clip,
            far_clip: self.controls.far_clip,
            opaque_enabled: self.controls.opaque_enabled,
            console_override: self.controls.console.is_some(),
            near_mask_gain: self.controls.near_mask_gain,
            stencil_enabled: self.controls.stencil_enabled,
        })
    }
}
