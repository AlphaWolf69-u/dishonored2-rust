//! One88e220 view publication shared by every canonical parameter consumer.
//! Timing, order, slot allocation and image residency belong to their owners.
use super::{
    environment_images::{ImageOverrideBlend, ImageOverrides},
    environment_parameter_overrides::{blend_environments, ParameterKind},
    environment_parameters::{EnvironmentParameters, EnvironmentValue},
    environment_resources::{PreparedEnvironment, PreparedValue},
};
use std::sync::Arc;

pub struct Row {
    pub environment: Arc<PreparedEnvironment>,
    pub weight: f32,
    pub influence: f32,
}
pub struct Frame {
    rows: Vec<Row>,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            rows: Vec::with_capacity(16),
        }
    }
}
impl Frame {
    pub fn clear(&mut self) {
        self.rows.clear();
    }
    pub fn push(
        &mut self,
        environment: Arc<PreparedEnvironment>,
        weight: f32,
        influence: f32,
    ) -> Result<(), String> {
        if self.rows.len() == 16 {
            return Err("environment frame exceeds actual16 view slots".into());
        }
        self.rows.push(Row {
            environment,
            weight,
            influence,
        });
        Ok(())
    }
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
    pub fn update_slot(
        &mut self,
        index: usize,
        environment: &Arc<PreparedEnvironment>,
        weight: f32,
        influence: f32,
    ) -> Result<bool, String> {
        if index >= 16 {
            return Err("environment frame exceeds actual16 view slots".into());
        }
        if let Some(row) = self.rows.get_mut(index) {
            let changed = !Arc::ptr_eq(&row.environment, environment)
                || row.weight.to_bits() != weight.to_bits()
                || row.influence.to_bits() != influence.to_bits();
            if !Arc::ptr_eq(&row.environment, environment) {
                row.environment = environment.clone();
            }
            row.weight = weight;
            row.influence = influence;
            Ok(changed)
        } else {
            self.rows.push(Row {
                environment: environment.clone(),
                weight,
                influence,
            });
            Ok(true)
        }
    }
    pub fn finish_slots(&mut self, count: usize) -> bool {
        let changed = self.rows.len() != count;
        self.rows.truncate(count);
        changed
    }
    fn kind(&self, name: &str) -> Option<ParameterKind> {
        self.rows
            .iter()
            .find_map(|row| match row.environment.values.get(name)? {
                PreparedValue::Float(_) => Some(ParameterKind::Float),
                PreparedValue::Unsigned(_) => Some(ParameterKind::Unsigned),
                PreparedValue::Signed(_) => Some(ParameterKind::Signed),
                PreparedValue::Boolean(_) => Some(ParameterKind::Boolean),
                _ => None,
            })
    }
    fn words(&self, name: &str, kind: ParameterKind, base: [u32; 4]) -> [u32; 4] {
        blend_environments(
            kind,
            base,
            self.rows.iter().filter_map(|row| {
                let words = match row.environment.values.get(name)? {
                    PreparedValue::Float(value) => value.map(f32::to_bits),
                    PreparedValue::Unsigned(value) => *value,
                    PreparedValue::Signed(value) => value.map(|value| value as u32),
                    PreparedValue::Boolean(value) => [u32::from(*value), 0, 0, 0],
                    _ => return None,
                };
                Some((words, row.weight, row.influence))
            }),
        )
    }
    /// Typed owners that expose floats retain the original registered numeric
    /// subtype while88e220 runs; conversion back happens only at their boundary.
    pub fn vector(&self, name: &str, base: [f32; 4]) -> [f32; 4] {
        let Some(kind) = self.kind(name) else {
            return base;
        };
        let words = match kind {
            ParameterKind::Float => base.map(f32::to_bits),
            ParameterKind::Unsigned => base.map(|v| v as u32),
            ParameterKind::Signed => base.map(|v| crate::entities::integer_word(v.to_bits(), 0x30)),
            ParameterKind::Boolean => [u32::from(base[0] != 0.), 0, 0, 0],
        };
        let words = self.words(name, kind, words);
        match kind {
            ParameterKind::Float => words.map(f32::from_bits),
            ParameterKind::Unsigned => words.map(|v| v as f32),
            ParameterKind::Signed => words.map(|v| v as i32 as f32),
            ParameterKind::Boolean => words.map(|v| v as f32),
        }
    }
    pub fn unsigned(&self, name: &str, base: u32) -> u32 {
        self.words(name, ParameterKind::Unsigned, [base, 0, 0, 0])[0]
    }
    pub fn apply_parameters(&self, parameters: &mut EnvironmentParameters) {
        if self.rows.is_empty() {
            return;
        }
        for (name, value) in &mut parameters.values {
            match value {
                EnvironmentValue::Float(value) => {
                    *value = bevy_math::Vec4::from_array(self.vector(name, value.to_array()))
                }
                EnvironmentValue::Boolean(value) => {
                    *value = self.vector(name, [f32::from(u8::from(*value)), 0., 0., 0.])[0] != 0.
                }
                EnvironmentValue::Image(_) => (),
            }
        }
    }
    pub fn image_overrides<T: PartialEq>(
        &self,
        name: &str,
        mut image: impl FnMut(&PreparedEnvironment, &str) -> Result<T, String>,
    ) -> Result<ImageOverrideBlend<T>, String> {
        let mut result = ImageOverrides::default();
        for row in &self.rows {
            if let Some(PreparedValue::Image(source)) = row.environment.values.get(name) {
                result.add_environment(
                    Some(image(&row.environment, source)?),
                    row.weight,
                    row.influence,
                );
            }
        }
        Ok(result.finish())
    }
}
