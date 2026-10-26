//! Level-authored world FX declarations. Preparation never activates an effect:
//! its graph must reach Start, and playback must admit the group at its origin.
use super::definition::ParticleDefinition;
use crate::{
    archive::Catalog,
    assets::{
        entities::{effects::FxGraph, Value},
        fx::EffectGroupAsset,
    },
    gameplay::options::decl::{atom, list_items, read_edit, scope},
    shader_cache::ProgramLibrary,
};
use std::{collections::HashMap, sync::Arc};

pub struct WorldEffectGroup {
    pub asset: EffectGroupAsset,
    pub particles: Vec<Result<ParticleDefinition, String>>,
    /// Keep every other authored family for the common FX owner. A particle
    /// consumer cannot silently call such a group completely implemented.
    pub other_families: Vec<(String, Value)>,
}

pub struct WorldEffectDefinition {
    pub groups: Vec<WorldEffectGroup>,
}

#[derive(Default)]
pub struct WorldFxSources {
    pub effects: HashMap<String, Result<Arc<WorldEffectDefinition>, String>>,
}

impl WorldFxSources {
    pub fn load(graphs: &[FxGraph], catalog: &Catalog, library: &mut ProgramLibrary) -> Self {
        let mut result = Self::default();
        let mut materials = HashMap::new();
        // Preserve graph/node order for preparation and share declaration and
        // material storage across all node references to the same FX resource.
        for graph in graphs {
            for node in &graph.nodes {
                let Some(name) = graph.effects.get(&node.id).and_then(|p| p.effect.as_ref()) else {
                    continue;
                };
                if result.effects.contains_key(name) {
                    continue;
                }
                let mut load = || -> Result<Arc<WorldEffectDefinition>, String> {
                    let mut read_error = None;
                    let mut read = |path: &str| match catalog.read(path) {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            read_error = Some(format!("{path}: {error}"));
                            None
                        }
                    };
                    let fields = read_edit(&mut read, "fx", name);
                    if let Some(error) = read_error {
                        return Err(error);
                    }
                    let fields = fields?;
                    let mut groups = Vec::new();
                    for group in scope(&fields, "m_groups").into_iter().flat_map(list_items) {
                        let Value::Scope(fields) = group else {
                            return Err(format!("{name}: FX group requires a scope"));
                        };
                        let asset = crate::assets::fx::effect_group(fields)?;
                        if !asset.enabled {
                            continue;
                        }
                        let particles = scope(fields, "m_particleSystems")
                            .into_iter()
                            .flat_map(list_items)
                            .filter(|entry| match entry {
                                Value::Scope(fields) => atom(fields, "m_enabled") != Some("false"),
                                _ => true,
                            })
                            .map(|entry| match entry {
                                Value::Scope(fields) => ParticleDefinition::load(
                                    fields,
                                    catalog,
                                    library,
                                    &mut materials,
                                ),
                                _ => Err("particle descriptor requires a scope".into()),
                            })
                            .collect();
                        let other_families = fields
                            .iter()
                            .filter(|(field, value)| {
                                field != "m_particleSystems"
                                    && matches!(value, Value::Scope(_))
                                    && scope(fields, field)
                                        .is_some_and(|entries| !list_items(entries).is_empty())
                            })
                            .cloned()
                            .collect();
                        groups.push(WorldEffectGroup {
                            asset,
                            particles,
                            other_families,
                        });
                    }
                    Ok(Arc::new(WorldEffectDefinition { groups }))
                };
                result.effects.insert(
                    name.clone(),
                    load().map_err(|error| format!("{name}: {error}")),
                );
            }
        }
        result
    }
}
