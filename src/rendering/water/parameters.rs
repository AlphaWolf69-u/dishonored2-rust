//! Prepared water view values and the original per-parameter area transitions.
use super::{FLOAT_PARAMETER_NAMES, IMAGE_PARAMETER_NAMES};
use crate::rendering::{
    environment::EnvironmentSource,
    environment_tracks::{FloatParameter, ImageTrack, ParameterTrack},
};
use crate::{
    assets::{
        archive::Catalog,
        environment::{ParameterReader, ParameterSource},
    },
    entities::EnvironmentRenderParameters,
};
use bevy_math::Vec4;

#[derive(Clone)]
struct PreparedImage {
    source: String,
    identity: String,
}
impl PartialEq for PreparedImage {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}
pub struct PreparedWaterValues {
    floats: [Option<Vec4>; 28],
    images: [Option<PreparedImage>; 2],
}
impl PreparedWaterValues {
    pub fn load(
        catalog: &Catalog,
        parameters: &EnvironmentRenderParameters,
        registered: bool,
    ) -> Result<Self, String> {
        let reader = ParameterReader {
            catalog,
            parameters,
        };
        let mut floats = [None; 28];
        let mut images = std::array::from_fn(|_| None);
        for (i, name) in FLOAT_PARAMETER_NAMES.iter().enumerate() {
            if registered || parameters.parameter(name).is_some() {
                floats[i] = Some(Vec4::from_array(reader.vector(name)?));
            }
        }
        for (i, name) in IMAGE_PARAMETER_NAMES.iter().enumerate() {
            if registered || parameters.parameter(name).is_some() {
                let source = reader.image_source(name)?;
                let resource = if catalog.contains(&source) {
                    source.clone()
                } else {
                    crate::material::compiled_image_stem(catalog, &source)
                        .ok_or_else(|| format!("unresolved water image identity {source}"))?
                };
                images[i] = Some(PreparedImage {
                    source,
                    identity: resource.to_ascii_lowercase(),
                });
            }
        }
        Ok(Self { floats, images })
    }
    pub fn image_sources(&self) -> impl Iterator<Item = (usize, &str)> {
        self.images
            .iter()
            .enumerate()
            .filter_map(|(i, name)| name.as_ref().map(|name| (i, name.source.as_str())))
    }
}
#[derive(Clone)]
pub struct WaterViewParameters {
    floats: [[f32; 4]; 28],
    pub images: [String; 2],
    pub rays: crate::rendering::environment_parameters::EnvironmentParameters,
}
impl ParameterSource for WaterViewParameters {
    fn vector(&self, name: &str) -> Result<[f32; 4], String> {
        FLOAT_PARAMETER_NAMES
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
            .map(|i| self.floats[i])
            .map(Ok)
            .unwrap_or_else(|| self.rays.vector(name))
    }
    fn scalar(&self, name: &str) -> Result<f32, String> {
        Ok(self.vector(name)?[0])
    }
    fn image_source(&self, name: &str) -> Result<String, String> {
        IMAGE_PARAMETER_NAMES
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
            .map(|i| self.images[i].clone())
            .ok_or_else(|| format!("unprepared water image {name}"))
    }
    fn boolean(&self, name: &str) -> Result<bool, String> {
        self.rays.boolean(name)
    }
}
pub struct WaterTracks {
    floats: [ParameterTrack<FloatParameter>; 28],
    images: [ImageTrack<PreparedImage>; 2],
    initialized: bool,
}
impl WaterTracks {
    pub fn new(defaults: &PreparedWaterValues) -> Result<Self, String> {
        if defaults.floats.iter().any(Option::is_none)
            || defaults.images.iter().any(Option::is_none)
        {
            return Err("water tracks require complete registered defaults".into());
        }
        Ok(Self {
            floats: std::array::from_fn(|i| {
                ParameterTrack::new(FloatParameter(defaults.floats[i].unwrap()))
            }),
            images: std::array::from_fn(|i| ImageTrack::new(defaults.images[i].clone().unwrap())),
            initialized: false,
        })
    }
    pub fn update(
        &mut self,
        world: &PreparedWaterValues,
        world_duration: f32,
        areas: &[(usize, &PreparedWaterValues, f32)],
        delta: f32,
    ) -> WaterViewParameters {
        let floats = std::array::from_fn(|i| {
            let mut selection = world.floats[i].map(|v| (EnvironmentSource::World, v));
            let mut duration = world_duration;
            for (id, area, time) in areas {
                if let Some(value) = area.floats[i] {
                    selection = Some((EnvironmentSource::Area(*id), value));
                    duration = *time;
                }
            }
            self.floats[i]
                .update(
                    selection.map(|(id, v)| (id, FloatParameter(v))),
                    duration,
                    delta,
                    self.initialized,
                    |a, b, t| FloatParameter((b.0 - a.0) * t + a.0),
                )
                .0
                .to_array()
        });
        let images = std::array::from_fn(|i| {
            let mut selection = world.images[i]
                .clone()
                .map(|v| (EnvironmentSource::World, v));
            let mut duration = world_duration;
            for (id, area, time) in areas {
                if let Some(value) = &area.images[i] {
                    selection = Some((EnvironmentSource::Area(*id), value.clone()));
                    duration = *time;
                }
            }
            self.images[i]
                .update(selection, duration, delta, self.initialized)
                .source
                .clone()
        });
        self.initialized = true;
        WaterViewParameters { floats, images, rays: Default::default() }
    }
}
