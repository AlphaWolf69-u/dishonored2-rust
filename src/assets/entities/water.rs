//! Authored water declarations, with4d5bc0 defaults and1417c2300 field layout.
use super::{
    components::{atom, entry_fields, scheduling},
    field, scope, vector, Document, Value,
};
use crate::gameplay::component_state::declaration::DeclarationScheduling;
use std::collections::HashMap;
#[cfg(test)]
#[path = "water/tests.rs"]
mod tests;

pub struct WaterBinding {
    pub entity_id: String,
    pub slot: String,
    pub scheduling: DeclarationScheduling,
    pub water_material: Option<String>,
    pub ocean_material: Option<String>,
    pub density: f32,
    pub current: [f32; 3],
    pub wavelet_amplitude: f32,
    pub is_ocean: bool,
}
impl WaterBinding {
    /// Resolve declaration references through the level's material owner.
    /// The component's retained vector is [current.xyz, density].
    pub fn startup(
        &self,
        mut resolve: impl FnMut(&str) -> Result<usize, String>,
    ) -> Result<crate::gameplay::water_component::startup::WaterDeclaration, String> {
        Ok(
            crate::gameplay::water_component::startup::WaterDeclaration {
                materials: [
                    self.water_material
                        .as_deref()
                        .map(&mut resolve)
                        .transpose()?,
                    self.ocean_material
                        .as_deref()
                        .map(&mut resolve)
                        .transpose()?,
                ],
                is_ocean: self.is_ocean,
                density: self.density,
                current: self.current,
            },
        )
    }
}

pub fn water_bindings(
    document: &Document,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Vec<WaterBinding>, String> {
    let declarations = document
        .definitions
        .iter()
        .filter(|d| d.kind == "component")
        .map(|d| (d.id.as_str(), d))
        .collect::<HashMap<_, _>>();
    let mut cache = HashMap::new();
    let mut result = Vec::new();
    for entity in document.definitions.iter().filter(|d| d.kind == "entity") {
        let Some(edit) = scope(field(&entity.fields, "edit")) else {
            continue;
        };
        let Some(components) = scope(field(edit, "m_componentDecls")) else {
            continue;
        };
        for (slot, entry) in components
            .iter()
            .filter(|(key, _)| key.starts_with("item_add["))
        {
            let Some(entry) = scope(Some(entry)) else {
                continue;
            };
            let Some(fields) =
                entry_fields(entry, "cpntwater", &declarations, &mut cache, &mut read)?
            else {
                continue;
            };
            let number = |name, default| -> Result<f32, String> {
                atom(&fields, name).map_or(Ok(default), |v| {
                    v.trim_end_matches(['f', 'F'])
                        .parse()
                        .map_err(|_| format!("{}: invalid water {name}", entity.id))
                })
            };
            let reference = |name| {
                atom(&fields, name)
                    .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("null"))
                    .map(str::to_owned)
            };
            let current = match field(&fields, "m_waterCurrent") {
                None => [0.; 3],
                Some(Value::Scope(values)) => vector(values, [0.; 3])
                    .ok_or_else(|| format!("{}: invalid water current", entity.id))?,
                Some(_) => return Err(format!("{}: water current must be a vector", entity.id)),
            };
            result.push(WaterBinding {
                entity_id: entity.id.clone(),
                slot: slot.clone(),
                scheduling: scheduling(&fields, DeclarationScheduling::from_base_flags(0))?,
                water_material: reference("m_waterPhysicsMaterial"),
                ocean_material: reference("m_oceanPhysicsMaterial"),
                density: number("m_waterDensity", 1000.)?,
                current,
                wavelet_amplitude: number("m_waveletAmplitude", f32::from_bits(0x3d4c_cccd))?,
                is_ocean: match atom(&fields, "m_isOcean") {
                    None | Some("false") => false,
                    Some("true") => true,
                    Some(_) => return Err(format!("{}: invalid water m_isOcean", entity.id)),
                },
            });
        }
    }
    Ok(result)
}
