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
    pub sounds: Vec<WorldSoundDefinition>,
    /// Keep every other authored family for the common FX owner. A particle
    /// consumer cannot silently call such a group completely implemented.
    pub other_families: Vec<(String, Value)>,
}

pub struct WorldSoundDefinition {
    pub position: [bevy_math::Vec3; 2],
    pub start_delay: [f32; 2],
    pub stop_delay: [f32; 2],
    pub start: Option<Arc<WorldSoundEvent>>,
    pub stop: Option<Arc<WorldSoundEvent>>,
}

/// A declaration reference is distinct from the middleware event it posts.
/// Several authored declarations may share an engine ID but have different
/// game-side range, looping, and virtualization settings (1400c4460).
pub struct WorldSoundEvent {
    pub name: String,
    pub declaration: crate::assets::wwise::SoundEventDeclaration,
}

impl WorldSoundEvent {
    fn resolve(
        asset: &crate::assets::fx::SoundEventAsset,
        catalog: &Catalog,
        cache: &mut HashMap<String, Result<Arc<Self>, String>>,
    ) -> Result<Option<Arc<Self>>, String> {
        let Some(name) = asset.event.as_ref() else {
            return Ok(None);
        };
        cache
            .entry(name.clone())
            .or_insert_with(|| {
                let declaration = asset
                    .resolve(catalog)?
                    .ok_or("sound event reference lost its declaration")?;
                Ok(Arc::new(Self {
                    name: name.clone(),
                    declaration,
                }))
            })
            .clone()
            .map(Some)
    }
}

pub struct WorldEffectDefinition {
    pub groups: Vec<WorldEffectGroup>,
}

/// FX arrays retain constructor defaults for slots omitted by partial edits.
/// Reuse the shared descriptor-array decoder rather than enumerate only the
/// serialized item records and lose default groups or enabled descriptors.
pub(crate) fn descriptor_fields(
    fields: &[(String, Value)],
) -> Result<Vec<Vec<(String, Value)>>, String> {
    let mut items = Vec::new();
    crate::assets::fx::descriptor_list(&mut items, fields, |fields| Ok(fields.to_vec()))?;
    Ok(items)
}

#[derive(Default)]
pub struct WorldFxSources {
    pub effects: HashMap<String, Result<Arc<WorldEffectDefinition>, String>>,
    pub declarations: crate::assets::entities::effects::declarations::FxDeclarations,
}

impl WorldFxSources {
    /// Every particle material of the level's world FX.
    pub fn particle_materials(&self) -> impl Iterator<Item = &Arc<super::material::ParticleMaterial>> {
        self.effects.values().filter_map(|e| e.as_ref().ok())
            .flat_map(|e| &e.groups).flat_map(|g| &g.particles).filter_map(|p| p.as_ref().ok())
            .flat_map(|p| p.materials.values())
    }

    /// Every sound event the level's world FX post.
    pub fn sound_events(&self) -> impl Iterator<Item = u32> + '_ {
        self.effects.values().filter_map(|e| e.as_ref().ok())
            .flat_map(|e| &e.groups).flat_map(|g| &g.sounds)
            .flat_map(|s| [&s.start, &s.stop]).filter_map(|e| e.as_ref()).map(|e| e.declaration.id)
    }

    pub fn load(
        graphs: &[FxGraph],
        actors: &[crate::assets::entities::effects::actors::ActorDefinition],
        catalog: &Catalog,
        library: &mut ProgramLibrary,
    ) -> Self {
        let mut result = Self {
            declarations: crate::assets::entities::effects::declarations::FxDeclarations::load(
                graphs, catalog,
            ),
            ..Default::default()
        };
        if std::env::var_os("DH2_TRACE_WORLD_FX").is_some() {
            for declaration in &result.declarations.declarations {
                match declaration {
                    Ok(graph) => eprintln!(
                        "World FX graph prepared: {} nodes={} variables={} effects={}",
                        graph.owner,
                        graph.nodes.len(),
                        graph.variables.len(),
                        graph.effects.len()
                    ),
                    Err(error) => eprintln!("World FX graph preparation failed: {error}"),
                }
            }
        }
        let mut materials = HashMap::new();
        let mut sound_events = HashMap::new();
        // Preserve graph/node order for preparation and share declaration and
        // material storage across all node references to the same FX resource.
        let mut names = Vec::new();
        for graph in graphs.iter().chain(
            result
                .declarations
                .declarations
                .iter()
                .filter_map(|declaration| declaration.as_ref().ok()),
        ) {
            names.extend(graph.nodes.iter().filter_map(|node| {
                graph
                    .effects
                    .get(&node.id)
                    .and_then(|parameters| parameters.effect.clone())
            }));
        }
        for actor in actors {
            match &actor.parameters {
                Ok(parameters) => {
                    if let Some(effect) = &parameters.effect {
                        names.push(effect.clone());
                    }
                    if std::env::var_os("DH2_TRACE_WORLD_FX").is_some() {
                        eprintln!(
                            "World FX actor prepared: {} effect={:?} start_off={}",
                            actor.owner, parameters.effect, parameters.start_off
                        );
                    }
                }
                Err(error) => eprintln!("World FX actor preparation failed: {error}"),
            }
        }
        for name in &names {
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
                let group_fields = descriptor_fields(scope(&fields, "m_groups").unwrap_or(&[]))?;
                for fields in &group_fields {
                    let asset = crate::assets::fx::effect_group(fields)?;
                    if !asset.enabled {
                        continue;
                    }
                    let sounds = asset
                        .sounds
                        .iter()
                        .filter(|sound| {
                            sound.enabled
                                && (sound.start.event.is_some() || sound.stop.event.is_some())
                        })
                        .map(|sound| {
                            Ok(WorldSoundDefinition {
                                position: sound.position,
                                start_delay: sound.start.delay,
                                stop_delay: sound.stop.delay,
                                start: WorldSoundEvent::resolve(
                                    &sound.start,
                                    catalog,
                                    &mut sound_events,
                                )?,
                                stop: WorldSoundEvent::resolve(
                                    &sound.stop,
                                    catalog,
                                    &mut sound_events,
                                )?,
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    let particle_fields =
                        descriptor_fields(scope(fields, "m_particleSystems").unwrap_or(&[]))?;
                    let particles = particle_fields
                        .iter()
                        .filter(|fields| atom(fields, "m_enabled") != Some("false"))
                        .map(|fields| {
                            ParticleDefinition::load(fields, catalog, library, &mut materials)
                        })
                        .collect();
                    let other_families = fields
                        .iter()
                        .filter(|(field, value)| {
                            field != "m_particleSystems"
                                && field != "m_sounds"
                                && matches!(value, Value::Scope(_))
                                && scope(fields, field).is_some_and(|entries| {
                                    !list_items(entries).is_empty()
                                        || atom(entries, "num").is_some_and(|count| count != "0")
                                })
                        })
                        .cloned()
                        .collect();
                    groups.push(WorldEffectGroup {
                        asset,
                        particles,
                        sounds,
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
        result
    }
}
