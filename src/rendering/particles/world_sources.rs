//! Level-authored world FX declarations. Preparation never activates an effect:
//! its graph must reach Start, and playback must admit the group at its origin.
use super::definition::ParticleDefinition;
use crate::{
    archive::Catalog,
    assets::{
        entities::effects::FxGraph,
        fx::EffectGroupAsset,
    },
    shader_cache::ProgramLibrary,
};
use hashbrown::HashMap;
use std::sync::Arc;

pub struct WorldEffectGroup {
    pub asset: Arc<EffectGroupAsset>,
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
    pub declaration: Arc<crate::assets::wwise::SoundEventDeclaration>,
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
    pub(crate) _source: Arc<crate::assets::fx::definition::Definition>,
    pub groups: Vec<WorldEffectGroup>,
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
        result.load_named_into(names, catalog, library, resources, None);
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
        result.load_named_into(names, catalog, library, resources, None);
        result
    }

    pub(crate) fn load_named_cancellable(
        names: Vec<String>,
        catalog: &Catalog,
        library: &mut ProgramLibrary,
        resources: &mut super::definition::ParticleResources,
        cancelled: impl Fn() -> bool,
    ) -> Result<Self, String> {
        let mut result = Self::default();
        if !result.load_named_into(names, catalog, library, resources, Some(&cancelled)) {
            return Err("effect acquisition cancelled".into());
        }
        Ok(result)
    }

    fn load_named_into(
        &mut self,
        mut names: Vec<String>,
        catalog: &Catalog,
        library: &mut ProgramLibrary,
        resources: &mut super::definition::ParticleResources,
        cancellation: Option<&dyn Fn() -> bool>,
    ) -> bool {
        let cancelled = || cancellation.is_some_and(|stop| stop());
        let result = self;
        let trace = std::env::var_os("DH2_TRACE_WORLD_FX").is_some();
        let mut sound_events = HashMap::new();
        // Sub-FX declarations (m_subFXs) are loaded as further effects, in
        // reference order after the declarations that name them.
        let mut next = 0;
        while next < names.len() {
            if cancelled() { return false; }
            let name = names[next].clone();
            let name = &name;
            next += 1;
            if result.effects.contains_key(name) {
                continue;
            }
            let mut children = Vec::new();
            let mut load = || -> Result<Arc<WorldEffectDefinition>, String> {
                let source = crate::assets::fx::declaration::acquire(catalog, name,
                    crate::assets::fx::definition::Consumer::World, cancellation)?;
                let mut groups = Vec::new();
                for group in &source.groups {
                    if cancelled() { return Err("effect acquisition cancelled".into()); }
                    let asset = group.asset.clone();
                    if !asset.enabled { continue; }
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
                    let particle_assets = group.particles.get().expect("world FX families acquired")
                        .as_ref().map_err(Clone::clone)?;
                    let mut particles = Vec::with_capacity(particle_assets.len());
                    for asset in particle_assets {
                        if cancelled() { return Err("effect acquisition cancelled".into()); }
                        particles.push(asset.as_ref().map_err(Clone::clone).and_then(|asset|
                            ParticleDefinition::from_asset_cancellable(asset.clone(), catalog, library, resources, cancellation)));
                    }
                    if cancelled() { return Err("effect acquisition cancelled".into()); }
                    let decals = match group.decals.get().expect("world FX families acquired") {
                        Ok(descriptors) => {
                            let mut decals = Vec::with_capacity(descriptors.len());
                            for asset in descriptors {
                                if cancelled() { return Err("effect acquisition cancelled".into()); }
                                decals.push(asset.as_ref().map_err(Clone::clone).and_then(|asset|
                                    crate::assets::fx::decal::DecalDefinition::from_asset(asset.clone(), catalog,
                                        &mut resources.decal_resources).map(Arc::new)));
                            }
                            decals
                        }
                        Err(error) => vec![Err(error.clone())],
                    };
                    let admission = group.world_other_families.get().expect("world FX families acquired");
                    result.deferred_syntax_bytes += admission.syntax_bytes;
                    result.deferred_admission_bytes += admission.payload_bytes;
                    let other_families = admission.families.clone();
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
                Ok(Arc::new(WorldEffectDefinition { _source: source, groups }))
            };
            let loaded = load().map_err(|error| format!("{name}: {error}"));
            if cancelled() { return false; }
            if trace {
                match &loaded {
                    Ok(definition) => eprintln!("World FX definition acquired: {name} groups={}", definition.groups.len()),
                    Err(error) => eprintln!("World FX definition rejected: {error}"),
                }
            }
            result.effects.insert(name.clone(), loaded);
            names.extend(children);
        }
        !cancelled()
    }
}
