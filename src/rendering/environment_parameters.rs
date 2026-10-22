//! Immutable typed environment inputs resolved before gameplay. No archive reads or
//! declaration parsing are needed when camera-area tracks evaluate a frame.
use crate::assets::{
    archive::Catalog,
    entities::EnvironmentRenderParameters,
    environment::{ParameterReader, ParameterSource},
};
use bevy_math::Vec4;
use std::{cell::RefCell, collections::HashMap};

#[derive(Clone, Debug, PartialEq)]
pub enum EnvironmentValue {
    Float(Vec4),
    Boolean(bool),
    Image(String),
}

#[derive(Clone, Debug, Default)]
pub struct EnvironmentParameters {
    pub values: HashMap<String, EnvironmentValue>,
}

pub struct NumericOverride<'a> {
    pub name: &'a str,
    /// Explicit producers supply float values, including for boolean parameters.
    pub value: [f32; 4],
    pub weight: f32,
}
pub struct WeightedEnvironment<'a> {
    pub parameters: &'a EnvironmentParameters,
    pub weight: f32,
    pub influence: f32,
}

#[cfg(test)]
#[path = "environment_parameters/override_tests.rs"]
mod override_tests;

impl EnvironmentParameters {
    /// Apply numeric entries after the ordinary track publication. Image
    /// grouping remains separate; parameters outside this captured schema do
    /// not participate in this subsystem's prepared frame.
    fn apply_numeric_overrides(
        &mut self,
        explicit: &[NumericOverride<'_>],
        environments: &[WeightedEnvironment<'_>],
    ) -> Result<(), String> {
        use super::environment_parameter_overrides::{ParameterKind, ParameterOverrides};
        let convert = |name: &str, value: &EnvironmentValue| -> Result<_, String> {
            let Some(base) = self.values.get(name) else {
                return Ok(None);
            };
            match (base, value) {
                (EnvironmentValue::Float(base), EnvironmentValue::Float(value)) => Ok(Some((
                    ParameterKind::Float,
                    base.to_array().map(f32::to_bits),
                    value.to_array().map(f32::to_bits),
                ))),
                (EnvironmentValue::Boolean(base), EnvironmentValue::Boolean(value)) => Ok(Some((
                    ParameterKind::Boolean,
                    [u32::from(*base), 0, 0, 0],
                    [u32::from(*value), 0, 0, 0],
                ))),
                (EnvironmentValue::Image(_), EnvironmentValue::Image(_)) => Ok(None),
                _ => Err(format!(
                    "environment override type differs from prepared schema at {name}"
                )),
            }
        };
        let mut overlays = ParameterOverrides::default();
        for entry in explicit {
            let Some(base) = self.values.get(entry.name) else {
                continue;
            };
            let (kind, base) = match base {
                EnvironmentValue::Float(base) => {
                    (ParameterKind::Float, base.to_array().map(f32::to_bits))
                }
                EnvironmentValue::Boolean(base) => {
                    (ParameterKind::Boolean, [u32::from(*base), 0, 0, 0])
                }
                EnvironmentValue::Image(_) => continue,
            };
            overlays.push_override(entry.name, kind, base, entry.value, entry.weight);
        }
        for entry in environments {
            for (name, value) in &entry.parameters.values {
                if let Some((kind, base, value)) = convert(name, value)? {
                    overlays.add_environment(
                        name.as_str(),
                        kind,
                        base,
                        value,
                        entry.weight,
                        entry.influence,
                    );
                }
            }
        }
        for (name, value) in overlays.finish() {
            match self.values.get_mut(name).expect("prepared override schema") {
                EnvironmentValue::Float(target) => {
                    *target = Vec4::from_array(value.map(f32::from_bits))
                }
                EnvironmentValue::Boolean(target) => *target = value[0] != 0,
                EnvironmentValue::Image(_) => unreachable!("image overrides use separate grouping"),
            }
        }
        Ok(())
    }
    pub fn defaults(catalog: &Catalog, schema: &Self) -> Result<Self, String> {
        let empty = EnvironmentRenderParameters { layers: Vec::new() };
        let reader = ParameterReader {
            catalog,
            parameters: &empty,
        };
        let mut values = HashMap::new();
        for (name, kind) in &schema.values {
            let value = match kind {
                EnvironmentValue::Float(_) => {
                    EnvironmentValue::Float(Vec4::from_array(reader.vector(name)?))
                }
                EnvironmentValue::Boolean(_) => EnvironmentValue::Boolean(reader.boolean(name)?),
                EnvironmentValue::Image(_) => EnvironmentValue::Image(reader.image_source(name)?),
            };
            values.insert(name.clone(), value);
        }
        Ok(Self { values })
    }
    /// Record precisely the scalar, vector, boolean and image inputs consumed
    /// by a subsystem's production resolver, including its authored branches.
    pub fn capture<T>(
        catalog: &Catalog,
        parameters: &EnvironmentRenderParameters,
        prepare: impl FnOnce(&ParameterCapture<'_>) -> Result<T, String>,
    ) -> Result<Self, String> {
        let capture = ParameterCapture {
            reader: ParameterReader {
                catalog,
                parameters,
            },
            values: RefCell::new(Self::default()),
        };
        prepare(&capture)?;
        Ok(capture.values.into_inner())
    }

    /// An area or effect profile overrides only supplied parameters. Filling
    /// an absent field with its registered default would erase the world.
    pub fn supplied(
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
                EnvironmentValue::Float(_) => {
                    EnvironmentValue::Float(Vec4::from_array(reader.vector(name)?))
                }
                EnvironmentValue::Boolean(_) => EnvironmentValue::Boolean(reader.boolean(name)?),
                EnvironmentValue::Image(_) => EnvironmentValue::Image(reader.image_source(name)?),
            };
            values.insert(name.clone(), value);
        }
        Ok(Self { values })
    }

    fn value(&self, name: &str) -> Result<&EnvironmentValue, String> {
        self.values
            .get(name)
            .ok_or_else(|| format!("unresolved prepared environment parameter {name}"))
    }
}

/// 88e220's per-parameter tracks, using only prepared values. Area entries
/// arrive in the same descending-volume order as the physical area query.
pub struct EnvironmentTracks {
    tracks: HashMap<String, crate::rendering::environment::EnvironmentTrack<EnvironmentValue>>,
    frame: EnvironmentParameters,
}
impl EnvironmentTracks {
    /// External overrides affect this frame, not the underlying ordinary
    /// tracks. The next update always starts from their unmodified publication.
    pub fn update_with_overrides(
        &mut self,
        world: &EnvironmentParameters,
        world_duration: f32,
        areas: &[(usize, &EnvironmentParameters, f32)],
        delta: f32,
        explicit: &[NumericOverride<'_>],
        environments: &[WeightedEnvironment<'_>],
    ) -> Result<&EnvironmentParameters, String> {
        self.update(world, world_duration, areas, delta)?;
        self.frame.apply_numeric_overrides(explicit, environments)?;
        Ok(&self.frame)
    }
    pub fn current(&self) -> &EnvironmentParameters {
        &self.frame
    }
    pub fn new(defaults: &EnvironmentParameters) -> Self {
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
        world: &EnvironmentParameters,
        world_duration: f32,
        areas: &[(usize, &EnvironmentParameters, f32)],
        delta: f32,
    ) -> Result<&EnvironmentParameters, String> {
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
                .ok_or_else(|| format!("environment parameter schema changed at {name}"))?;
            let value = track.advance_with(
                source,
                target.clone(),
                duration,
                delta,
                |start, target, fraction| {
                    match (start, target) {
                        (EnvironmentValue::Float(start), EnvironmentValue::Float(target)) => {
                            EnvironmentValue::Float((*target - *start) * fraction + *start)
                        }
                        (EnvironmentValue::Boolean(start), EnvironmentValue::Boolean(target)) => {
                            // Native bools turn on near the start and turn off at
                            // the end; they are not a generic halfway switch.
                            let threshold = if !start && *target {
                                f32::from_bits(0x3586_37bd)
                            } else {
                                f32::from_bits(0x3f7f_be77)
                            };
                            EnvironmentValue::Boolean(if fraction >= threshold {
                                *target
                            } else {
                                *start
                            })
                        }
                        (EnvironmentValue::Image(start), EnvironmentValue::Image(target)) => {
                            EnvironmentValue::Image(if fraction >= 0.5 { target } else { start }.clone())
                        }
                        _ => unreachable!(
                            "prepared environment parameter types must match their registered schema"
                        ),
                    }
                },
            );
            *self
                .frame
                .values
                .get_mut(name)
                .expect("prepared environment frame schema") = value;
        }
        Ok(&self.frame)
    }
}
impl ParameterSource for EnvironmentParameters {
    fn scalar(&self, name: &str) -> Result<f32, String> {
        self.vector(name).map(|v| v[0])
    }
    fn vector(&self, name: &str) -> Result<[f32; 4], String> {
        match self.value(name)? {
            EnvironmentValue::Float(value) => Ok(value.to_array()),
            _ => Err(format!(
                "{name}: prepared environment parameter is not float"
            )),
        }
    }
    fn boolean(&self, name: &str) -> Result<bool, String> {
        match self.value(name)? {
            EnvironmentValue::Boolean(value) => Ok(*value),
            _ => Err(format!(
                "{name}: prepared environment parameter is not boolean"
            )),
        }
    }
    fn image_source(&self, name: &str) -> Result<String, String> {
        match self.value(name)? {
            EnvironmentValue::Image(value) => Ok(value.clone()),
            _ => Err(format!(
                "{name}: prepared environment parameter is not an image"
            )),
        }
    }
}

pub struct ParameterCapture<'a> {
    reader: ParameterReader<'a>,
    values: RefCell<EnvironmentParameters>,
}
impl ParameterSource for ParameterCapture<'_> {
    fn scalar(&self, name: &str) -> Result<f32, String> {
        let value = self.reader.scalar(name)?;
        self.values
            .borrow_mut()
            .values
            .insert(name.into(), EnvironmentValue::Float(Vec4::splat(value)));
        Ok(value)
    }
    fn vector(&self, name: &str) -> Result<[f32; 4], String> {
        let value = self.reader.vector(name)?;
        self.values.borrow_mut().values.insert(
            name.into(),
            EnvironmentValue::Float(Vec4::from_array(value)),
        );
        Ok(value)
    }
    fn boolean(&self, name: &str) -> Result<bool, String> {
        let value = self.reader.boolean(name)?;
        self.values
            .borrow_mut()
            .values
            .insert(name.into(), EnvironmentValue::Boolean(value));
        Ok(value)
    }
    fn image_source(&self, name: &str) -> Result<String, String> {
        let value = self.reader.image_source(name)?;
        self.values
            .borrow_mut()
            .values
            .insert(name.into(), EnvironmentValue::Image(value.clone()));
        Ok(value)
    }
}
