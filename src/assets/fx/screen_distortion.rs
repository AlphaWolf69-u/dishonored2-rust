//! Typed arkFXFSDistoParmsDesc (21b770/21b7f0) and its inline16 parameters.
use super::*;
use crate::gameplay::effects::{screen_distortion::VectorEnvelope, vector_curve::EffectVectorCurvePair};
use std::sync::Arc;

/// Complete installed FSDisto renderParm class (game1 IDs1259..1281).
pub(crate) const NAMES: [&str; 23] = [
    "fsdisto/opt/use_radial_blur", "fsdisto/opt/use_lens", "fsdisto/opt/use_wobble_disto",
    "fsdisto/opt/use_sand_storm", "fsdisto/opt/use_cam_blur", "fsdisto/wobblelowbound",
    "fsdisto/wobblehighbound", "fsdisto/wobblespeed", "fsdisto/wobblestrength", "fsdisto/wobbledeadzone",
    "fsdisto/radialdistostrength", "fsdisto/radialblurstrength", "fsdisto/radialblurdeadzone", "fsdisto/lensstrength",
    "fsdisto/sandstorm/deadzone", "fsdisto/sandstorm/startdepth", "fsdisto/sandstorm/enddepth",
    "fsdisto/sandstorm/startvalue", "fsdisto/sandstorm/endvalue", "fsdisto/sandstorm/worldwind",
    "fsdisto/camerablur/strength", "fsdisto/camerablur/startdepth", "fsdisto/camerablur/enddepth",
];
#[derive(Clone, Copy)]
pub(crate) enum Blend { Average, Minimum, Maximum }
pub(crate) struct Binding {
    pub slot: usize,
    pub default: [f32; 4],
    pub blend: Blend,
    pub option: bool,
    _parameter: Arc<crate::assets::render_parameter::Parameter>,
}
pub(crate) struct Prepared {
    pub asset: Arc<Asset>,
    pub bindings: Vec<Option<Binding>>,
    pub defaults: [[f32; 4]; 23],
}
fn binding(catalog: &crate::archive::Catalog, name: &str) -> Result<Binding, String> {
    let slot = NAMES.iter().position(|candidate| candidate.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("FSDisto renderParm {name} has no installed fullscreen consumer"))?;
    let parameter = crate::assets::render_parameter::acquire(catalog, name)?;
    let metadata = parameter.metadata()?;
    let text = parameter.text();
    let tokens = crate::entities::declaration_lexemes(&text)?;
    let option = metadata.kind == crate::render_program::RenderParameterKind::Option;
    let default = if option {
        let value = crate::entities::declaration_option_default(&text)?;
        [value as f32, 0., 0., 0.]
    } else { parameter.floating_words()?.map(f32::from_bits) };
    let blend = if tokens.iter().any(|token| token.eq_ignore_ascii_case("blendmin")) { Blend::Minimum }
        else if tokens.iter().any(|token| token.eq_ignore_ascii_case("blendmax")) { Blend::Maximum }
        else { Blend::Average };
    Ok(Binding { slot, default, blend, option, _parameter: parameter })
}
impl Prepared {
    /// Borrow the canonical renderParm resource owner, once during shared FX
    /// preparation. There is no private declaration/image/parameter cache.
    pub(crate) fn new(asset: Arc<Asset>, catalog: &crate::archive::Catalog) -> Result<Self, String> {
        let mut defaults = [[0.; 4]; 23];
        for (slot, name) in NAMES.iter().enumerate() { defaults[slot] = binding(catalog, name)?.default; }
        let bindings = asset.parameters.iter().map(|parameter| parameter.name.as_deref()
            .map(|name| binding(catalog, name)).transpose()).collect::<Result<Vec<_>, _>>()?;
        Ok(Self { asset, bindings, defaults })
    }
}

pub struct Parameter {
    pub name: Option<String>,
    pub value: VectorEnvelope,
}
impl Default for Parameter {
    ///2241a0/2241e0: null renderParm, zero range, constant-one maximum
    ///curves in all phases, empty minimum curves, nonuniform sampling.
    fn default() -> Self { Self { name: None, value: VectorEnvelope::default() } }
}
pub struct Asset {
    pub name: String,
    pub enabled: bool,
    pub blend: std::sync::Arc<EnvironmentEffectDefinition>,
    pub parameters: Vec<Parameter>,
}

fn curves(pair: &mut EffectVectorCurvePair<4>, fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_maxCurve" => super::particles::vector_curve(&mut pair.maximum, scope(value)?)?,
            "m_minCurve" => super::particles::vector_curve(&mut pair.minimum, scope(value)?)?,
            _ => return Err(format!("unknown FSDisto curve field {name}")),
        }
    }
    Ok(())
}
fn envelope(value: &mut VectorEnvelope, fields: &Fields) -> Result<(), String> {
    for (name, field) in fields {
        match name.as_str() {
            "m_max" => super::particles::vector(&mut value.range[0], scope(field)?)?,
            "m_min" => super::particles::vector(&mut value.range[1], scope(field)?)?,
            "m_uniform" => value.uniform = boolean(field)?,
            "m_durationCurve" => curves(&mut value.duration, scope(field)?)?,
            "m_fadeinCurve" => curves(&mut value.fade_in, scope(field)?)?,
            "m_fadeoutCurve" => curves(&mut value.fade_out, scope(field)?)?,
            _ => return Err(format!("unknown FSDisto value field {name}")),
        }
    }
    Ok(())
}
pub fn descriptor(fields: &Fields) -> Result<Asset, String> {
    let common = environment_effect_fields(fields.iter().filter(|(name, _)| name != "m_renderParms"))?;
    let mut result = Asset { name: common.name, enabled: common.enabled, blend: common.blend, parameters: Vec::new() };
    if let Some(Value::Scope(entries)) = fields.iter().find(|(name, _)| name == "m_renderParms").map(|(_,value)|value) {
        // The native inline list has16 descriptor slots. Keep source order,
        // including null renderParms, because each consumes random samples.
        fn parameter(fields: &Fields) -> Result<Parameter, String> {
            let mut parameter = Parameter::default();
            for (name, value) in fields {
                match name.as_str() {
                    "m_renderParm" => {
                        let name = atom(value)?;
                        parameter.name = (!name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
                            .then(|| name.to_owned());
                    }
                    "m_value" => envelope(&mut parameter.value, scope(value)?)?,
                    _ => return Err(format!("unknown FSDisto parameter field {name}")),
                }
            }
            Ok(parameter)
        }
        descriptor_list(&mut result.parameters, entries, parameter)?;
        if result.parameters.len() > 16 { return Err("FSDisto descriptor exceeds native16 parameter slots".into()); }
    }
    Ok(result)
}
