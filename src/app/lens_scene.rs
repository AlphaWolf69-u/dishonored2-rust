//! Resident lens images and camera-area parameter tracks. Archive reads finish
//! at level preparation; frame publication only selects and blends typed data.
use crate::{
    assets::{archive::Catalog, environment::ParameterReader},
    entities::EnvironmentRenderParameters,
    rendering::{
        bevy_backend::lens_scene::LensFrame,
        environment::{AreaEnvironment, EnvironmentSource},
        environment_tracks::{blend_unsigned, FloatParameter, LensProfileTrack, ParameterTrack},
        lens_dirt::{LensDirtTrack, PreparedLensDirt},
    },
};
use bevy::prelude::*;
use std::collections::HashMap;

const FLOATS: [&str; 13] = [
    "env/bloom/veil_size",
    "env/bloom/veil_strength",
    "postfx/bloom/lensdirt_strength",
    "postfx/lensflares/thresholdbias",
    "postfx/lensflares/thresholdscale",
    "postfx/lensflares/dispersal",
    "postfx/lensflares/distortion",
    "postfx/lensflares/halowidth",
    "postfx/lensflares/haloweight",
    "postfx/lensflares/streakradius",
    "postfx/lensflares/streakoffset",
    "postfx/lensflares/streakwidth",
    "postfx/lensflares/streakopacity",
];
const INTEGERS: [&str; 2] = [
    "postfx/lensflares/blurradius",
    "postfx/lensflares/nbsamples",
];
const PROFILE: &str = "postfx/lensflares/lenscolor_from";
struct Inputs {
    floats: [Option<Vec4>; 13],
    integers: [Option<u32>; 2],
    profile: Option<Handle<Image>>,
}
pub(super) struct LensScene {
    black: Handle<Image>,
    settings: crate::platform::game_settings::PostProcessSettings,
    settings_generation: u64,
    world: Inputs,
    areas: Vec<(String, Inputs)>,
    float_tracks: [ParameterTrack<FloatParameter>; 13],
    integer_tracks: [ParameterTrack<u32>; 2],
    dirt: PreparedLensDirt<Handle<Image>>,
    dirt_track: LensDirtTrack<Handle<Image>>,
    profile: LensProfileTrack<Handle<Image>>,
    initialized: bool,
}
impl LensScene {
    pub(super) fn refresh_settings(&mut self) -> Result<(), String> {
        let generation = crate::platform::console::shared().generation();
        if generation != self.settings_generation {
            self.settings = crate::platform::game_settings::read_post_process_settings()?;
            self.settings_generation = generation;
        }
        Ok(())
    }
    pub(super) fn settings(&self) -> crate::platform::game_settings::PostProcessSettings {
        self.settings
    }
    pub(super) fn load(
        catalog: &Catalog,
        world: &EnvironmentRenderParameters,
        areas: &[AreaEnvironment],
        images: &mut Assets<Image>,
    ) -> Result<Self, String> {
        let mut resident = HashMap::new();
        let mut image = |name: &str| {
            super::texture_streaming::load_resident_2d(catalog, name, images, &mut resident)
        };
        let empty = EnvironmentRenderParameters { layers: Vec::new() };
        let defaults = ParameterReader {
            catalog,
            parameters: &empty,
        };
        let floats: Vec<_> = FLOATS
            .iter()
            .map(|name| defaults.vector(name).map(Vec4::from_array))
            .collect::<Result<_, _>>()?;
        let integers = [
            defaults.unsigned(INTEGERS[0])?,
            defaults.unsigned(INTEGERS[1])?,
        ];
        let default_profile = image(&defaults.image_source(PROFILE)?)?;
        let white = image("_white")?;
        let black = image("_black")?;
        let dirt = PreparedLensDirt::load(catalog, world, areas, &mut image)?;
        let dirt_track = LensDirtTrack::new(dirt.registered_default.clone());
        let mut prepare = |parameters: &EnvironmentRenderParameters| -> Result<Inputs, String> {
            let reader = ParameterReader {
                catalog,
                parameters,
            };
            let mut floats = [None; 13];
            let mut integers = [None; 2];
            for (i, name) in FLOATS.iter().enumerate() {
                if parameters.parameter(name).is_some() {
                    floats[i] = Some(Vec4::from_array(reader.vector(name)?));
                }
            }
            for (i, name) in INTEGERS.iter().enumerate() {
                if parameters.parameter(name).is_some() {
                    integers[i] = Some(reader.unsigned(name)?);
                }
            }
            let profile = if parameters.parameter(PROFILE).is_some() {
                Some(image(&reader.image_source(PROFILE)?)?)
            } else {
                None
            };
            Ok(Inputs {
                floats,
                integers,
                profile,
            })
        };
        let world = prepare(world)?;
        let areas = areas
            .iter()
            .map(|area| Ok((area.id.clone(), prepare(&area.parameters)?)))
            .collect::<Result<_, String>>()?;
        Ok(Self {
            black,
            settings: crate::platform::game_settings::read_post_process_settings()?,
            settings_generation: crate::platform::console::shared().generation(),
            world,
            areas,
            float_tracks: std::array::from_fn(|i| ParameterTrack::new(FloatParameter(floats[i]))),
            integer_tracks: std::array::from_fn(|i| ParameterTrack::new(integers[i])),
            dirt,
            dirt_track,
            profile: LensProfileTrack::new(default_profile, white),
            initialized: false,
        })
    }
    pub(super) fn update(
        &mut self,
        areas: &[AreaEnvironment],
        position: [f32; 3],
        world_duration: f32,
        delta: f32,
        camera_axis: f32,
        white_level: f32,
    ) -> Result<LensFrame, String> {
        if areas.len() != self.areas.len()
            || areas
                .iter()
                .zip(&self.areas)
                .any(|(a, (id, _))| &a.id != id)
        {
            return Err("lens areas changed without preparing their inputs".into());
        }
        let active: Vec<_> = areas
            .iter()
            .enumerate()
            .filter(|(_, area)| area.contains(position))
            .collect();
        let select_float = |i: usize| {
            let mut selection = self.world.floats[i].map(|v| (EnvironmentSource::World, v));
            let mut duration = world_duration;
            for (index, area) in &active {
                if let Some(value) = self.areas[*index].1.floats[i] {
                    selection = Some((EnvironmentSource::Area(*index), value));
                    duration = area.blend_duration;
                }
            }
            (selection, duration)
        };
        let mut values = [Vec4::ZERO; 13];
        for i in 0..13 {
            let (selection, duration) = select_float(i);
            values[i] = self.float_tracks[i]
                .update(
                    selection.map(|(source, value)| (source, FloatParameter(value))),
                    duration,
                    delta,
                    self.initialized,
                    |start, target, t| FloatParameter((target.0 - start.0) * t + start.0),
                )
                .0;
        }
        let mut integers = [0; 2];
        for i in 0..2 {
            let mut selection = self.world.integers[i].map(|v| (EnvironmentSource::World, v));
            let mut duration = world_duration;
            for (index, area) in &active {
                if let Some(value) = self.areas[*index].1.integers[i] {
                    selection = Some((EnvironmentSource::Area(*index), value));
                    duration = area.blend_duration;
                }
            }
            integers[i] = *self.integer_tracks[i].update(
                selection,
                duration,
                delta,
                self.initialized,
                |start, target, t| blend_unsigned(*start, *target, t),
            );
        }
        let mut selected_profile = self
            .world
            .profile
            .clone()
            .map(|v| (EnvironmentSource::World, v));
        let mut duration = world_duration;
        for (index, area) in &active {
            if let Some(value) = &self.areas[*index].1.profile {
                selected_profile = Some((EnvironmentSource::Area(*index), value.clone()));
                duration = area.blend_duration;
            }
        }
        self.profile
            .update(selected_profile, duration, delta, self.initialized);
        self.dirt.update(
            &mut self.dirt_track,
            areas,
            position,
            world_duration,
            delta,
            self.initialized,
            None,
            &[],
        )?;
        self.initialized = true;
        Ok(LensFrame {
            black: self.black.clone(),
            camera_axis,
            white_level,
            settings: self.settings,
            veil_size: values[0].x,
            veil_strength: values[1].x,
            dirt_strength: values[2].x,
            // 3586a0 overrides this view uniform from resolved image identity.
            use_default_dirt: self.dirt_track.value().from.id() == self.dirt_track.value().to.id(),
            threshold: [values[3].x, values[4].x],
            ghost: [values[5].x, values[6].x, values[7].x, values[8].x],
            streak: [values[9].x, values[10].x, values[11].x, values[12].x],
            blur_radius: integers[0],
            ghost_samples: integers[1],
            dirt: self.dirt_track.value().clone(),
            profile: self.profile.value(),
        })
    }
}
