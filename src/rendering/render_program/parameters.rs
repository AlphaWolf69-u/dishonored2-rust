//! Ordered draw parameter scopes over the evaluated view/registered values.
use super::{SerializedParameter, SerializedParameterBlock, SerializedParameterValue};
use crate::assets::environment::ParameterSource;

/// Blocks are applied in original publication order: program, material, then
/// any instance overrides. Missing assignments retain the evaluated view value.
pub struct ParameterScopes<'a> {
    pub view: &'a dyn ParameterSource,
    pub blocks: &'a [&'a SerializedParameterBlock],
}
impl ParameterScopes<'_> {
    fn parameter(&self, name: &str) -> Option<&SerializedParameter> {
        self.blocks.iter().rev().find_map(|block| {
            block.parameters.iter().rev().find(|p| {
                p.name
                    .split(|&b| b == 0)
                    .next()
                    .unwrap()
                    .eq_ignore_ascii_case(name.as_bytes())
            })
        })
    }
    fn words(&self, name: &str) -> Result<Option<[u32; 4]>, String> {
        match self.parameter(name).map(|p| &p.value) {
            Some(SerializedParameterValue::Words(words)) => Ok(Some(*words)),
            None => Ok(None),
            _ => Err(format!(
                "numeric draw parameter has a nonnumeric value: {name}"
            )),
        }
    }
}
impl ParameterSource for ParameterScopes<'_> {
    fn vector(&self, name: &str) -> Result<[f32; 4], String> {
        match self.words(name)? {
            Some(words) => Ok(words.map(f32::from_bits)),
            None => self.view.vector(name),
        }
    }
    fn scalar(&self, name: &str) -> Result<f32, String> {
        match self.words(name)? {
            Some(words) => Ok(f32::from_bits(words[0])),
            None => self.view.scalar(name),
        }
    }
    fn boolean(&self, name: &str) -> Result<bool, String> {
        match self.words(name)? {
            Some(words) => Ok(words[0] != 0),
            None => self.view.boolean(name),
        }
    }
    fn image_source(&self, name: &str) -> Result<String, String> {
        match self.parameter(name).map(|p| &p.value) {
            Some(SerializedParameterValue::Name(value)) => {
                String::from_utf8(value.split(|&b| b == 0).next().unwrap().to_vec())
                    .map_err(|e| format!("invalid draw image name {name}: {e}"))
            }
            None => self.view.image_source(name),
            _ => Err(format!("image draw parameter has a nonimage value: {name}")),
        }
    }
}

#[cfg(test)]
#[path = "parameters/tests.rs"]
mod tests;
