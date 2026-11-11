//! Immutable FX descriptor data, before map-specific material/image resolution.
//! idDeclFX registration1413630d0, inherited acquisition140022fd0 and release
//! 1401c5990. Child parser errors stay deferred until their consumer admits them.
use super::{EffectGroupAsset, decal::{decal, DecalAsset}, particle_system::{particle_system, ParticleSystemAsset}, screen_quad::{screen_quad, ScreenQuadAsset}};
use crate::assets::entities::Value;
use crate::gameplay::options::decl::{atom, list_items, scope};
use std::{borrow::Cow, sync::Arc};

type Fields = [(String, Value)];
pub(crate) type Family<T> = Result<Vec<Result<Arc<T>, String>>, String>;

pub(crate) struct Definition {
    pub groups: Vec<Group>,
}
pub(crate) struct Group {
    pub asset: Arc<EffectGroupAsset>,
    pub particles: Family<ParticleSystemAsset>,
    pub decals: Family<DecalAsset>,
    pub quads: Family<ScreenQuadAsset>,
    /// Preserve the world consumer's independent family-admission boundary.
    /// This contains no generic syntax, image data, GPU handle or playback state.
    pub world_other_families: Vec<(String, Result<bool, String>)>,
}

fn descriptors(fields: &Fields) -> Result<Vec<Cow<'_, Fields>>, String> {
    let mut result = Vec::new();
    super::descriptor_list(&mut result, fields, |fields| Ok(Cow::Borrowed(fields)))?;
    Ok(result)
}
fn family<T>(fields: &Fields, name: &str, parse: fn(&Fields) -> Result<T, String>) -> Family<T> {
    Ok(descriptors(scope(fields, name).unwrap_or(&[]))?.iter()
        .filter(|fields| atom(fields, "m_enabled") != Some("false"))
        .map(|fields| parse(fields).map(Arc::new)).collect())
}
impl Definition {
    /// Constructor/default/array interpretation stays in the existing asset
    /// parsers. Resolved inheritance syntax is borrowed only during this call.
    pub fn from_fields(fields: &Fields) -> Result<Self, String> {
        let mut groups = Vec::new();
        for fields in descriptors(scope(fields, "m_groups").unwrap_or(&[]))? {
            let asset = Arc::new(super::effect_group(&fields)?);
            let world_other_families = fields.iter().filter(|(name, value)| {
                !matches!(name.as_str(), "m_particleSystems" | "m_sounds" | "m_decals" | "m_subFXs")
                    && matches!(value, Value::Scope(_))
                    && scope(&fields, name).is_some_and(|entries| !list_items(entries).is_empty()
                        || atom(entries, "num").is_some_and(|count| count != "0"))
            }).map(|(name, value)| {
                let Value::Scope(entries) = value else { unreachable!() };
                (name.clone(), descriptors(entries).map(|items|
                    items.iter().any(|fields| atom(fields, "m_enabled") != Some("false"))))
            }).collect();
            groups.push(Group {
                particles: family(&fields, "m_particleSystems", particle_system),
                decals: family(&fields, "m_decals", decal),
                quads: family(&fields, "m_fullScreenQuads", screen_quad),
                asset,
                world_other_families,
            });
        }
        Ok(Self { groups })
    }
}
