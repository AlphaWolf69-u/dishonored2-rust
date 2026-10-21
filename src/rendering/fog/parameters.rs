//! Immutable typed fog inputs resolved before gameplay. No archive reads or
//! declaration parsing are needed when camera-area tracks evaluate a frame.
use super::settings::FogSettings;
use crate::assets::{
    archive::Catalog, entities::EnvironmentRenderParameters, environment::{ParameterReader, ParameterSource},
};
use bevy_math::{Vec3, Vec4};
use std::{cell::RefCell, collections::HashMap};

#[derive(Clone, Debug, PartialEq)]
pub enum FogValue {
    Float(Vec4),
    Boolean(bool),
    Image(String),
}

#[derive(Clone, Debug, Default)]
pub struct FogParameterValues {
    pub values: HashMap<String, FogValue>,
}

impl FogParameterValues {
    pub fn defaults(catalog: &Catalog, schema: &Self) -> Result<Self, String> {
        let empty = EnvironmentRenderParameters { layers: Vec::new() };
        let reader = ParameterReader {
            catalog,
            parameters: &empty,
        };
        let mut values = HashMap::new();
        for (name, kind) in &schema.values {
            let value = match kind {
                FogValue::Float(_) => FogValue::Float(Vec4::from_array(reader.vector(name)?)),
                FogValue::Boolean(_) => FogValue::Boolean(reader.boolean(name)?),
                FogValue::Image(_) => FogValue::Image(reader.image_source(name)?),
            };
            values.insert(name.clone(), value);
        }
        Ok(Self { values })
    }
    pub fn world(
        catalog: &Catalog,
        parameters: &EnvironmentRenderParameters,
        sun: Option<(Vec3, Vec3)>,
        far_clip: f32,
    ) -> Result<Self, String> {
        let capture = Capture {
            reader: ParameterReader {
                catalog,
                parameters,
            },
            values: RefCell::new(Self::default()),
        };
        // Capture exactly the inputs consumed by the production resolver,
        // including both authored/actual sun branches read by 306840.
        FogSettings::resolve(&capture, sun, far_clip)?;
        Ok(capture.values.into_inner())
    }

    /// The area dictionary overrides only supplied parameters. Filling an
    /// absent area field with its registered default would erase the world.
    pub fn area(
        catalog: &Catalog,
        parameters: &EnvironmentRenderParameters,
        schema: &Self,
    ) -> Result<Self, String> {
        let reader = ParameterReader {
            catalog,
            parameters,
        };
        let mut values = HashMap::new();
        for (name, kind) in &schema.values {
            if parameters.parameter(name).is_none() {
                continue;
            }
            let value = match kind {
                FogValue::Float(_) => FogValue::Float(Vec4::from_array(reader.vector(name)?)),
                FogValue::Boolean(_) => FogValue::Boolean(reader.boolean(name)?),
                FogValue::Image(_) => FogValue::Image(reader.image_source(name)?),
            };
            values.insert(name.clone(), value);
        }
        Ok(Self { values })
    }

    fn value(&self, name: &str) -> Result<&FogValue, String> {
        self.values
            .get(name)
            .ok_or_else(|| format!("unresolved prepared fog parameter {name}"))
    }
}

/// 88e220's per-parameter tracks, using only prepared values. Area entries
/// arrive in the same descending-volume order as the physical area query.
pub struct FogTracks {
    tracks: HashMap<String, crate::rendering::environment::EnvironmentTrack<FogValue>>,
    frame: FogParameterValues,
}
impl FogTracks {
    pub fn current(&self) -> &FogParameterValues {
        &self.frame
    }
    pub fn new(defaults: &FogParameterValues) -> Self {
        Self {
            tracks: defaults
                .values
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        crate::rendering::environment::EnvironmentTrack::new(value.clone()),
                    )
                })
                .collect(),
            frame: defaults.clone(),
        }
    }

    pub fn update(
        &mut self,
        world: &FogParameterValues,
        world_duration: f32,
        areas: &[(usize, &FogParameterValues, f32)],
        delta: f32,
    ) -> Result<&FogParameterValues, String> {
        for (name, global) in &world.values {
            let mut source = None;
            let mut target = global;
            let mut duration = world_duration;
            for (index, area, blend_duration) in areas {
                if let Some(value) = area.values.get(name) {
                    source = Some(*index);
                    target = value;
                    duration = *blend_duration;
                }
            }
            let track = self
                .tracks
                .get_mut(name)
                .ok_or_else(|| format!("fog parameter schema changed at {name}"))?;
            let value = track.advance_with(
                source,
                target.clone(),
                duration,
                delta,
                |start, target, fraction| {
                    match (start, target) {
                        (FogValue::Float(start), FogValue::Float(target)) => {
                            FogValue::Float((*target - *start) * fraction + *start)
                        }
                        (FogValue::Boolean(start), FogValue::Boolean(target)) => {
                            // Native bools turn on near the start and turn off at
                            // the end; they are not a generic halfway switch.
                            let threshold = if !start && *target {
                                f32::from_bits(0x3586_37bd)
                            } else {
                                f32::from_bits(0x3f7f_be77)
                            };
                            FogValue::Boolean(if fraction >= threshold {
                                *target
                            } else {
                                *start
                            })
                        }
                        (FogValue::Image(start), FogValue::Image(target)) => {
                            FogValue::Image(if fraction >= 0.5 { target } else { start }.clone())
                        }
                        _ => unreachable!(
                            "prepared fog parameter types must match their registered schema"
                        ),
                    }
                },
            );
            *self
                .frame
                .values
                .get_mut(name)
                .expect("prepared fog frame schema") = value;
        }
        Ok(&self.frame)
    }
}
impl ParameterSource for FogParameterValues {
    fn scalar(&self, name: &str) -> Result<f32, String> {
        self.vector(name).map(|v| v[0])
    }
    fn vector(&self, name: &str) -> Result<[f32; 4], String> {
        match self.value(name)? {
            FogValue::Float(value) => Ok(value.to_array()),
            _ => Err(format!("{name}: prepared fog parameter is not float")),
        }
    }
    fn boolean(&self, name: &str) -> Result<bool, String> {
        match self.value(name)? {
            FogValue::Boolean(value) => Ok(*value),
            _ => Err(format!("{name}: prepared fog parameter is not boolean")),
        }
    }
    fn image_source(&self, name: &str) -> Result<String, String> {
        match self.value(name)? {
            FogValue::Image(value) => Ok(value.clone()),
            _ => Err(format!("{name}: prepared fog parameter is not an image")),
        }
    }
}

struct Capture<'a> {
    reader: ParameterReader<'a>,
    values: RefCell<FogParameterValues>,
}
impl ParameterSource for Capture<'_> {
    fn scalar(&self, name: &str) -> Result<f32, String> {
        let value = self.reader.scalar(name)?;
        self.values
            .borrow_mut()
            .values
            .insert(name.into(), FogValue::Float(Vec4::splat(value)));
        Ok(value)
    }
    fn vector(&self, name: &str) -> Result<[f32; 4], String> {
        let value = self.reader.vector(name)?;
        self.values
            .borrow_mut()
            .values
            .insert(name.into(), FogValue::Float(Vec4::from_array(value)));
        Ok(value)
    }
    fn boolean(&self, name: &str) -> Result<bool, String> {
        let value = self.reader.boolean(name)?;
        self.values
            .borrow_mut()
            .values
            .insert(name.into(), FogValue::Boolean(value));
        Ok(value)
    }
    fn image_source(&self, name: &str) -> Result<String, String> {
        let value = self.reader.image_source(name)?;
        self.values
            .borrow_mut()
            .values
            .insert(name.into(), FogValue::Image(value.clone()));
        Ok(value)
    }
}
