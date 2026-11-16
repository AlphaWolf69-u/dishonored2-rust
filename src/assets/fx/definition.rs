//! Immutable FX descriptor data, before map-specific material/image resolution.
//! idDeclFX registration1413630d0, inherited acquisition140022fd0 and release
//! 1401c5990. Child parser errors stay deferred until their consumer admits them.
use super::{EffectGroupAsset, decal::{decal, DecalAsset}, particle_system::{particle_system, ParticleSystemAsset}, screen_quad::{screen_quad, ScreenQuadAsset}};
use crate::assets::entities::Value;
use crate::gameplay::options::decl::{atom, list_items, scope};
use std::{borrow::Cow, sync::{Arc, OnceLock}};

type Fields = [(String, Value)];
pub(crate) type Family<T> = Result<Vec<Result<Arc<T>, String>>, String>;

/// Consumers decode only families they can actually use. Typed child data is
/// shared; source syntax and all material/image bindings remain temporary or
/// consumer-owned. This host deferral replaces eager idDeclFX typed payloads.
#[derive(Clone, Copy)]
pub(crate) enum Consumer { Common, World, Screen }
impl Consumer {
    fn particles(self) -> bool { matches!(self, Self::World | Self::Screen) }
}
pub(crate) struct Definition {
    pub groups_present: bool,
    pub source_paths: Vec<String>,
    pub groups: Vec<Group>,
}
pub(crate) struct Group {
    pub asset: Arc<EffectGroupAsset>,
    pub particles: OnceLock<Family<ParticleSystemAsset>>,
    pub decals: OnceLock<Family<DecalAsset>>,
    pub quads: OnceLock<Family<ScreenQuadAsset>>,
    /// Preserve the world consumer's independent family-admission boundary.
    /// This contains no generic syntax, image data, GPU handle or playback state.
    pub world_other_families: OnceLock<WorldAdmission>,
}

fn descriptors(fields: &Fields) -> Result<Vec<Cow<'_, Fields>>, String> {
    let mut result = Vec::new();
    super::descriptor_list(&mut result, fields, |fields| Ok(Cow::Borrowed(fields)))?;
    Ok(result)
}
fn family<T>(fields: &Fields, name: &str, parse: fn(&Fields) -> Result<T, String>) -> Family<T> {
    Ok(descriptors(scope(fields, name).unwrap_or(&[]))?.iter()
        .filter(|fields| name == "m_fullScreenQuads" || atom(fields, "m_enabled") != Some("false"))
        .map(|fields| parse(fields).map(Arc::new)).collect())
}
pub(crate) struct WorldAdmission {
    pub families: Vec<(String, Result<bool, String>)>,
    pub syntax_bytes: usize,
    pub payload_bytes: usize,
}
fn syntax_bytes(value: &Value) -> usize {
    match value {
        Value::Atom(value) => value.len(),
        Value::Scope(fields) => fields.len() * std::mem::size_of::<(String, Value)>()
            + fields.iter().map(|(key, value)| key.len() + syntax_bytes(value)).sum::<usize>(),
    }
}
fn other_families(fields: &Fields) -> WorldAdmission {
    let mut result = WorldAdmission { families: Vec::new(), syntax_bytes: 0, payload_bytes: 0 };
    for (name, value) in fields.iter().filter(|(name, value)| {
        !matches!(name.as_str(), "m_particleSystems" | "m_sounds" | "m_decals" | "m_subFXs" | "m_entityDefs" | "m_explosions")
            && matches!(value, Value::Scope(_))
            && scope(fields, name).is_some_and(|entries| !list_items(entries).is_empty()
                || atom(entries, "num").is_some_and(|count| count != "0"))
    }) {
        let Value::Scope(entries) = value else { unreachable!() };
        let admission = descriptors(entries).map(|items|
            items.iter().any(|fields| atom(fields, "m_enabled") != Some("false")));
        result.syntax_bytes += std::mem::size_of::<(String, Value)>() + name.len() + syntax_bytes(value);
        result.payload_bytes += std::mem::size_of::<(String, Result<bool, String>)>() + name.len()
            + admission.as_ref().err().map_or(0, |error| error.capacity());
        result.families.push((name.clone(), admission));
    }
    result
}
struct PendingGroup {
    particles: Option<Family<ParticleSystemAsset>>,
    decals: Option<Family<DecalAsset>>,
    quads: Option<Family<ScreenQuadAsset>>,
    other: Option<WorldAdmission>,
}
impl Definition {
    pub fn from_fields(fields: &Fields, consumer: Consumer,
        cancelled: Option<&dyn Fn() -> bool>) -> Result<Self, String> {
        let sources = descriptors(scope(fields, "m_groups").unwrap_or(&[]))?;
        let groups = sources.iter().map(|fields| Ok(Group {
                asset: Arc::new(super::effect_group(fields)?),
                particles: OnceLock::new(), decals: OnceLock::new(),
                quads: OnceLock::new(), world_other_families: OnceLock::new(),
            })).collect::<Result<Vec<_>, String>>()?;
        let result = Self { groups_present: scope(fields, "m_groups").is_some(), source_paths: Vec::new(), groups };
        result.prepare_sources(&sources, consumer, cancelled)?;
        Ok(result)
    }
    pub fn needs(&self, consumer: Consumer) -> bool {
        self.groups.iter().filter(|group| group.asset.enabled).any(|group|
            (consumer.particles() && group.particles.get().is_none())
                || (matches!(consumer, Consumer::World)
                    && (group.decals.get().is_none() || group.world_other_families.get().is_none()))
                || (matches!(consumer, Consumer::Screen) && group.quads.get().is_none()))
    }
    /// Stage every newly requested family before publication. A cancelled
    /// acquisition must not change the existing immutable resource generation.
    pub fn prepare(&self, fields: &Fields, consumer: Consumer,
        cancelled: Option<&dyn Fn() -> bool>) -> Result<(), String> {
        if !self.needs(consumer) { return Ok(()); }
        let sources = descriptors(scope(fields, "m_groups").unwrap_or(&[]))?;
        self.prepare_sources(&sources, consumer, cancelled)
    }
    fn prepare_sources(&self, sources: &[Cow<'_, Fields>], consumer: Consumer,
        cancelled: Option<&dyn Fn() -> bool>) -> Result<(), String> {
        if !self.needs(consumer) { return Ok(()); }
        if sources.len() != self.groups.len() { return Err("FX generation changed during acquisition".into()); }
        let mut pending = Vec::with_capacity(self.groups.len());
        for (group, fields) in self.groups.iter().zip(sources) {
            if cancelled.is_some_and(|stop| stop()) { return Err("effect acquisition cancelled".into()); }
            let active = group.asset.enabled;
            pending.push(PendingGroup {
                particles: (active && consumer.particles() && group.particles.get().is_none())
                    .then(|| family(fields, "m_particleSystems", particle_system)),
                decals: (active && matches!(consumer, Consumer::World) && group.decals.get().is_none())
                    .then(|| family(fields, "m_decals", decal)),
                quads: (active && matches!(consumer, Consumer::Screen) && group.quads.get().is_none())
                    .then(|| family(fields, "m_fullScreenQuads", screen_quad)),
                other: (active && matches!(consumer, Consumer::World) && group.world_other_families.get().is_none())
                    .then(|| other_families(fields)),
            });
        }
        if cancelled.is_some_and(|stop| stop()) { return Err("effect acquisition cancelled".into()); }
        for (group, pending) in self.groups.iter().zip(pending) {
            if let Some(value) = pending.particles { let _ = group.particles.set(value); }
            if let Some(value) = pending.decals { let _ = group.decals.set(value); }
            if let Some(value) = pending.quads { let _ = group.quads.set(value); }
            if let Some(value) = pending.other { let _ = group.world_other_families.set(value); }
        }
        Ok(())
    }
}
