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
use hashbrown::HashMap;
use std::sync::Arc;

pub struct WorldEffectGroup {
    pub asset: EffectGroupAsset,
    pub particles: Vec<Result<ParticleDefinition, String>>,
    pub sounds: Vec<WorldSoundDefinition>,
    pub decals: Vec<Result<Arc<crate::assets::fx::decal::DecalDefinition>, String>>,
    /// Deferred admission of other families, in authored order. Keep parsing
    /// errors until activation, without retaining their generic syntax trees.
    pub other_families: Vec<(String, Result<bool, String>)>,
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
) -> Result<Vec<std::borrow::Cow<'_, [(String, Value)]>>, String> {
    let mut items = Vec::new();
    crate::assets::fx::descriptor_list(&mut items, fields, |fields| Ok(std::borrow::Cow::Borrowed(fields)))?;
    static MODE: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
    let (copy, trace) = *MODE.get_or_init(|| (
        std::env::var("DH2_DEBUG_COPY_FX_DESCRIPTORS").as_deref() == Ok("1"),
        std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1"),
    ));
    let bytes = trace.then(|| items.iter().map(|fields|
        fields.len() * std::mem::size_of::<(String, Value)>()
            + fields.iter().map(|(key, value)| key.len() + cloned_value_bytes(value)).sum::<usize>()).sum::<usize>());
    if copy {
        for fields in &mut items { *fields = std::borrow::Cow::Owned(fields.to_vec()); }
    }
    if let Some(bytes) = bytes {
        eprintln!("MEMORY FX descriptor fields: entries={} selected_tree_bytes={bytes} copied_tree_bytes={} metadata_bytes={}",
            items.len(), if copy {bytes} else {0}, items.capacity() * std::mem::size_of::<std::borrow::Cow<'_, [(String, Value)]>>());
    }
    Ok(items)
}

#[derive(Default)]
pub struct WorldFxSources {
    pub effects: HashMap<String, Result<Arc<WorldEffectDefinition>, String>>,
    pub declarations: crate::assets::entities::effects::declarations::FxDeclarations,
    /// Heap payload previously retained for unsupported-family syntax, and its
    /// compact replacement. Excludes allocator bookkeeping in both figures.
    pub deferred_syntax_bytes: usize,
    pub deferred_admission_bytes: usize,
}

fn cloned_value_bytes(value: &Value) -> usize {
    match value {
        Value::Atom(value) => value.len(),
        Value::Scope(fields) => fields.len() * std::mem::size_of::<(String, Value)>()
            + fields.iter().map(|(key, value)| key.len() + cloned_value_bytes(value)).sum::<usize>(),
    }
}

impl WorldFxSources {
    /// Reference one already prepared effect and its declared sub-effects
    /// without retaining every unrelated effect in the level.
    pub fn subset(&self, name: &str) -> Result<Self, String> {
        let mut result = Self::default();
        let mut pending = vec![name.to_owned()];
        while let Some(name) = pending.pop() {
            if result.effects.contains_key(&name) { continue; }
            let definition = self.effects.get(&name).ok_or_else(|| format!("effect {name} was not prepared"))?;
            let loaded = definition.as_ref().map_err(Clone::clone)?;
            for child in loaded.groups.iter().flat_map(|group| &group.asset.sub_effects)
                .flat_map(|sub| sub.effects.iter().flatten()) {
                pending.push(child.clone());
            }
            result.effects.insert(name, definition.clone());
        }
        Ok(result)
    }

    /// Publish a new level snapshot without cloning any decoded FX assets.
    /// Requests own only their contributions; dropping them from the next
    /// snapshot lets the last instance release its declarations naturally.
    pub fn with_additions<'a>(base: &Self, additions: impl IntoIterator<Item = &'a Self>) -> Self {
        let mut effects = base.effects.clone();
        for addition in additions {
            for (name, definition) in &addition.effects {
                effects.entry(name.clone()).or_insert_with(|| definition.clone());
            }
        }
        Self { effects, declarations: base.declarations.clone(),
            deferred_syntax_bytes: base.deferred_syntax_bytes,
            deferred_admission_bytes: base.deferred_admission_bytes }
    }

    /// Every particle material of the level's world FX.
    pub fn particle_materials(&self) -> impl Iterator<Item = &Arc<super::material::ParticleMaterial>> {
        self.effects.values().filter_map(|e| e.as_ref().ok())
            .flat_map(|e| &e.groups).flat_map(|g| &g.particles).filter_map(|p| p.as_ref().ok())
            .flat_map(|p| p.materials.values())
    }

    /// Every (PMODEL resource, world material) pair the level's world FX can
    /// draw as a material surface, with its model, in a stable order.
    pub fn world_meshes(&self) -> Vec<(String, String, Arc<crate::assets::particle_model::ParticleModel>)> {
        let mut pairs = std::collections::BTreeMap::new();
        for definition in self.effects.values().filter_map(|e| e.as_ref().ok())
            .flat_map(|e| &e.groups).flat_map(|g| &g.particles).filter_map(|p| p.as_ref().ok())
        {
            for declaration in definition.declarations.iter().flatten() {
                for stage in declaration.stages.iter().filter(|stage| stage.enabled) {
                    for mesh in stage.meshes.iter().filter(|mesh| mesh.enabled) {
                        let (Some(material), Some(source), Some(model)) = (&mesh.material, &mesh.source, &mesh.model) else {
                            continue;
                        };
                        if definition.world_mesh_materials.contains(material) {
                            pairs.entry((source.clone(), material.clone())).or_insert_with(|| model.clone());
                        }
                    }
                }
            }
        }
        pairs.into_iter().map(|((source, material), model)| (source, material, model)).collect()
    }

    /// Material variants required by enabled projected decals, deduplicated
    /// in the same order-independent form used by scene preparation.
    pub fn decal_materials(&self) -> Vec<(String, bool)> {
        let mut names = std::collections::BTreeSet::new();
        for decal in self.effects.values().filter_map(|effect| effect.as_ref().ok())
            .flat_map(|effect| &effect.groups).flat_map(|group| &group.decals)
            .filter_map(|decal| decal.as_ref().ok()).filter(|decal| decal.asset.enabled)
        {
            for references in decal.declarations.iter().flatten() {
                for (name, world) in [(&references.opaque, false), (&references.alpha, false),
                    (&references.opaque_instanced, true), (&references.alpha_instanced, true)] {
                    if let Some(name) = name { names.insert((name.clone(), world)); }
                }
            }
        }
        names.into_iter().collect()
    }

    /// Every sound event the level's world FX post.
    pub fn sound_events(&self) -> impl Iterator<Item = u32> + '_ {
        self.effects.values().filter_map(|e| e.as_ref().ok())
            .flat_map(|e| &e.groups).flat_map(|g| &g.sounds)
            .flat_map(|s| [&s.start, &s.stop]).filter_map(|e| e.as_ref()).map(|e| e.declaration.id)
    }

    pub fn load(
        graphs: &[Arc<FxGraph>],
        actors: &[crate::assets::entities::effects::actors::ActorDefinition],
        catalog: &Catalog,
        library: &mut ProgramLibrary,
    ) -> Self {
        Self::load_shared(graphs, actors, catalog, library, &mut Default::default())
    }

    pub fn load_shared(
        graphs: &[Arc<FxGraph>],
        actors: &[crate::assets::entities::effects::actors::ActorDefinition],
        catalog: &Catalog,
        library: &mut ProgramLibrary,
        resources: &mut super::definition::ParticleResources,
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
        // Opt-in installed FX preview uses the same acquisition owner.
        if let Ok(effect) = std::env::var("DH2_DEBUG_PLAY_FX") {
            names.push(effect);
        }
        result.load_named_into(names, catalog, library, resources);
        result
    }

    /// Acquire explicitly requested effects and their sub-effects through the
    /// same parser and resource owner used by authored world FX.
    pub fn load_named_shared(
        names: Vec<String>,
        catalog: &Catalog,
        library: &mut ProgramLibrary,
        resources: &mut super::definition::ParticleResources,
    ) -> Self {
        let mut result = Self::default();
        result.load_named_into(names, catalog, library, resources);
        result
    }

    fn load_named_into(
        &mut self,
        mut names: Vec<String>,
        catalog: &Catalog,
        library: &mut ProgramLibrary,
        resources: &mut super::definition::ParticleResources,
    ) {
        let result = self;
        let trace = std::env::var_os("DH2_TRACE_WORLD_FX").is_some();
        let mut sound_events = HashMap::new();
        let mut decal_resources = crate::assets::decal::DecalResources::default();
        // Sub-FX declarations (m_subFXs) are loaded as further effects, in
        // reference order after the declarations that name them.
        let mut next = 0;
        while next < names.len() {
            let name = names[next].clone();
            let name = &name;
            next += 1;
            if result.effects.contains_key(name) {
                continue;
            }
            let mut children = Vec::new();
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
                            ParticleDefinition::load(fields, catalog, library, resources)
                        })
                        .collect();
                    let decals = match descriptor_fields(scope(fields, "m_decals").unwrap_or(&[])) {
                        Ok(descriptors) => descriptors.iter()
                            .filter(|fields| atom(fields, "m_enabled") != Some("false"))
                            .map(|fields| crate::assets::fx::decal::DecalDefinition::load(fields, catalog, &mut decal_resources).map(Arc::new))
                            .collect(),
                        Err(error) => vec![Err(error)],
                    };
                    let other_families = fields
                        .iter()
                        .filter(|(field, value)| {
                            field != "m_particleSystems"
                                && field != "m_sounds"
                                && field != "m_decals"
                                && field != "m_subFXs"
                                && matches!(value, Value::Scope(_))
                                && scope(fields, field).is_some_and(|entries| {
                                    !list_items(entries).is_empty()
                                        || atom(entries, "num").is_some_and(|count| count != "0")
                                })
                        })
                        .map(|(family, value)| {
                            let Value::Scope(fields) = value else { unreachable!() };
                            let admission = descriptor_fields(fields).map(|descriptors|
                                descriptors.iter().any(|fields| atom(fields, "m_enabled") != Some("false")));
                            result.deferred_syntax_bytes += std::mem::size_of::<(String, Value)>()
                                + family.len() + cloned_value_bytes(value);
                            result.deferred_admission_bytes += std::mem::size_of::<(String, Result<bool, String>)>()
                                + family.len() + admission.as_ref().err().map_or(0, |error| error.capacity());
                            (family.clone(), admission)
                        })
                        .collect();
                    children.extend(
                        asset
                            .sub_effects
                            .iter()
                            .flat_map(|sub| sub.effects.iter().flatten().cloned()),
                    );
                    groups.push(WorldEffectGroup {
                        asset,
                        particles,
                        sounds,
                        decals,
                        other_families,
                    });
                }
                Ok(Arc::new(WorldEffectDefinition { groups }))
            };
            let loaded = load().map_err(|error| format!("{name}: {error}"));
            if trace {
                match &loaded {
                    Ok(definition) => eprintln!("World FX definition acquired: {name} groups={}", definition.groups.len()),
                    Err(error) => eprintln!("World FX definition rejected: {error}"),
                }
            }
            result.effects.insert(name.clone(), loaded);
            names.extend(children);
        }
    }
}
