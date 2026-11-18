//! Level-authored world FX declarations. Preparation never activates an effect:
//! its graph must reach Start, and playback must admit the group at its origin.
use super::definition::ParticleDefinition;
use crate::{
    archive::Catalog,
    assets::{entities::effects::FxGraph, fx::EffectGroupAsset},
    shader_cache::ProgramLibrary,
};
use hashbrown::HashMap;
use std::sync::Arc;

pub struct WorldEffectGroup {
    pub asset: Arc<EffectGroupAsset>,
    pub particles: Vec<Result<ParticleDefinition, String>>,
    pub sounds: Vec<WorldSoundDefinition>,
    pub lights: Vec<Result<Arc<crate::assets::fx::light::LightAsset>, String>>,
    pub decals: Vec<Result<Arc<crate::assets::fx::decal::DecalDefinition>, String>>,
    pub(crate) quads: Vec<Result<Arc<crate::assets::fx::screen_quad::ScreenQuadAsset>, String>>,
    pub(crate) quad_materials: Vec<(crate::assets::fx::screen_quad::ScreenQuadStage,Arc<crate::rendering::screen_quad::ScreenQuadMaterial>)>,
    pub(crate) fovs: Vec<Result<Arc<crate::assets::fx::fov::Asset>, String>>,
    pub(crate) rumbles: Vec<Result<Arc<crate::assets::fx::rumble::Asset>, String>>,
    pub(crate) screen_distortions: Vec<Result<Arc<crate::assets::fx::screen_distortion::Prepared>, String>>,
    pub(crate) entity_defs: Vec<Arc<WorldEntityDef>>,
    pub(crate) explosions: Vec<Arc<WorldExplosion>>,
    /// Index in the shared source generation, including disabled source groups.
    /// Deferred admission borrows that generation rather than copying names.
    source_group: usize,
}
pub(crate) struct WorldEntityDef {
    pub lifetime: Arc<()>,
    pub asset: Arc<EffectGroupAsset>,
    pub index: usize,
    pub declarations: Vec<Option<Arc<crate::assets::runtime_entity::Definition>>>,
    pub prepared: std::sync::atomic::AtomicBool,
}
pub(crate) struct WorldExplosion {
    pub lifetime: Arc<()>,
    pub asset: Arc<EffectGroupAsset>,
    pub index: usize,
    pub declaration: Arc<crate::assets::explosions::Declaration>,
    pub prepared: std::sync::atomic::AtomicBool,
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
    pub(crate) source: Arc<crate::assets::fx::definition::Definition>,
    pub groups: Vec<WorldEffectGroup>,
}

impl WorldEffectDefinition {
    pub(crate) fn gameplay_ready(&self) -> bool {
        self.groups.iter().all(|group| {
            group
                .entity_defs
                .iter()
                .all(|entry| entry.prepared.load(std::sync::atomic::Ordering::Acquire))
                && group
                    .explosions
                    .iter()
                    .all(|entry| entry.prepared.load(std::sync::atomic::Ordering::Acquire))
        })
    }
    pub(crate) fn other_families(
        &self,
        group: &WorldEffectGroup,
    ) -> &[(String, Result<bool, String>)] {
        &self.source.groups[group.source_group]
            .world_other_families
            .get()
            .expect("world FX families acquired")
            .families
    }
}

#[derive(Default)]
pub struct WorldFxSources {
    pub effects: HashMap<String, Result<Arc<WorldEffectDefinition>, String>>,
    pub declarations: Arc<crate::assets::entities::effects::declarations::FxDeclarations>,
    /// Heap payload previously retained for unsupported-family syntax, and its
    /// compact replacement. Excludes allocator bookkeeping in both figures.
    pub deferred_syntax_bytes: usize,
    pub deferred_admission_bytes: usize,
}

/// The surface source naming a shared sprite grid level.
pub fn sprite_grid_source(level: u32) -> String {
    format!("sprite-grid:{level}")
}

/// The FX of the enabled, valid m_fxEvents entries of loaded particle
/// definitions.
fn fx_event_effects(
    particles: &[Result<super::definition::ParticleDefinition, String>],
) -> Vec<String> {
    particles
        .iter()
        .filter_map(|particle| particle.as_ref().ok())
        .flat_map(|particle| particle.declarations.iter().flatten())
        .flat_map(|declaration| declaration.fx_events.iter())
        .filter(|event| event.enabled && event.valid())
        .filter_map(|event| event.fx.clone())
        .collect()
}

impl WorldFxSources {
    pub(crate) fn gameplay_ready(&self) -> bool {
        self.effects
            .values()
            .filter_map(|effect| effect.as_ref().ok())
            .all(|effect| effect.gameplay_ready())
    }
    /// Reference one already prepared effect and its declared sub-effects
    /// without retaining every unrelated effect in the level.
    pub fn subset(&self, name: &str) -> Result<Self, String> {
        let mut result = Self::default();
        let mut pending = vec![name.to_owned()];
        while let Some(name) = pending.pop() {
            if result.effects.contains_key(&name) {
                continue;
            }
            let definition = self
                .effects
                .get(&name)
                .ok_or_else(|| format!("effect {name} was not prepared"))?;
            let loaded = definition.as_ref().map_err(Clone::clone)?;
            for child in loaded
                .groups
                .iter()
                .flat_map(|group| &group.asset.sub_effects)
                .flat_map(|sub| sub.effects.iter().flatten())
            {
                pending.push(child.clone());
            }
            for group in &loaded.groups {
                pending.extend(fx_event_effects(&group.particles));
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
                effects
                    .entry(name.clone())
                    .or_insert_with(|| definition.clone());
            }
        }
        Self {
            effects,
            declarations: base.declarations.clone(),
            deferred_syntax_bytes: base.deferred_syntax_bytes,
            deferred_admission_bytes: base.deferred_admission_bytes,
        }
    }

    /// Every particle material of the level's world FX.
    /// Names of the materials the level's FX draw as trail strips.
    pub fn strip_materials(&self) -> std::collections::HashSet<String> {
        self.effects
            .values()
            .filter_map(|e| e.as_ref().ok())
            .flat_map(|e| &e.groups)
            .flat_map(|g| &g.particles)
            .filter_map(|p| p.as_ref().ok())
            .flat_map(|p| p.strip_materials.iter().cloned())
            .collect()
    }

    pub(crate) fn quad_materials(&self) -> impl Iterator<Item=&Arc<crate::rendering::screen_quad::ScreenQuadMaterial>> {
        self.effects.values().filter_map(|effect|effect.as_ref().ok())
            .flat_map(|effect|&effect.groups).flat_map(|group|&group.quad_materials).map(|(_,material)|material)
    }
    pub(crate) fn quad_material_stages(&self) -> impl Iterator<Item=(crate::assets::fx::screen_quad::ScreenQuadStage,&Arc<crate::rendering::screen_quad::ScreenQuadMaterial>)> {
        self.effects.values().filter_map(|effect|effect.as_ref().ok()).flat_map(|effect|&effect.groups)
            .flat_map(|group|&group.quad_materials).map(|(stage,material)|(*stage,material))
    }
    pub(crate) fn material_images(&self) -> impl Iterator<Item=&Arc<super::image_resources::ParticleImage>> {
        self.particle_materials().flat_map(|material|material.images.values())
            .chain(self.quad_materials().flat_map(|material|material.images.values()))
    }
    pub fn particle_materials(
        &self,
    ) -> impl Iterator<Item = &Arc<super::material::ParticleMaterial>> {
        self.effects
            .values()
            .filter_map(|e| e.as_ref().ok())
            .flat_map(|e| &e.groups)
            .flat_map(|g| &g.particles)
            .filter_map(|p| p.as_ref().ok())
            .flat_map(|p| p.materials.values())
    }

    /// Every (PMODEL resource, world material) pair the level's world FX can
    /// draw as a material surface, with its model, in a stable order.
    pub fn world_meshes(
        &self,
    ) -> Vec<(
        String,
        String,
        Arc<crate::assets::particle_model::ParticleModel>,
    )> {
        let mut pairs = std::collections::BTreeMap::new();
        for definition in self
            .effects
            .values()
            .filter_map(|e| e.as_ref().ok())
            .flat_map(|e| &e.groups)
            .flat_map(|g| &g.particles)
            .filter_map(|p| p.as_ref().ok())
        {
            for declaration in definition.declarations.iter().flatten() {
                let havok_stages = declaration.havok_stages.iter().map(|stage| &stage.stage);
                for stage in declaration
                    .stages
                    .iter()
                    .chain(havok_stages)
                    .filter(|stage| stage.enabled)
                {
                    for mesh in stage.meshes.iter().filter(|mesh| mesh.enabled) {
                        let (Some(material), Some(source), Some(model)) =
                            (&mesh.material, &mesh.source, &mesh.model)
                        else {
                            continue;
                        };
                        if definition.world_mesh_materials.contains(material) {
                            pairs
                                .entry((source.clone(), material.clone()))
                                .or_insert_with(|| model.clone());
                        }
                    }
                    // A sprite renderer's surface is the shared grid its
                    // tessellation distance selects (1eee40): level 0 unless
                    // the distance can select finer ones.
                    for sprite in stage.sprites.iter().filter(|sprite| sprite.enabled) {
                        let Some(material) = sprite
                            .material
                            .as_ref()
                            .filter(|material| definition.world_mesh_materials.contains(*material))
                        else {
                            continue;
                        };
                        let levels = if 0. < sprite.tessellation_distance {
                            0..=5
                        } else {
                            0..=0
                        };
                        for level in levels {
                            pairs
                                .entry((sprite_grid_source(level), material.clone()))
                                .or_insert_with(|| {
                                    Arc::new(crate::assets::particle_model::ParticleModel {
                                        vertices: Vec::new(),
                                        bytes_after_vertices: 0,
                                    })
                                });
                        }
                    }
                }
            }
        }
        pairs
            .into_iter()
            .map(|((source, material), model)| (source, material, model))
            .collect()
    }

    /// Material variants required by enabled projected decals, deduplicated
    /// in the same order-independent form used by scene preparation.
    pub fn decal_materials(&self) -> Vec<(String, bool)> {
        let mut names = std::collections::BTreeSet::new();
        for decal in self
            .effects
            .values()
            .filter_map(|effect| effect.as_ref().ok())
            .flat_map(|effect| &effect.groups)
            .flat_map(|group| &group.decals)
            .filter_map(|decal| decal.as_ref().ok())
            .filter(|decal| decal.asset.enabled)
        {
            for references in decal.declarations.iter().flatten() {
                for (name, world) in [
                    (&references.opaque, false),
                    (&references.alpha, false),
                    (&references.opaque_instanced, true),
                    (&references.alpha_instanced, true),
                ] {
                    if let Some(name) = name {
                        names.insert((name.clone(), world));
                    }
                }
            }
        }
        names.into_iter().collect()
    }

    /// Every sound event the level's world FX post.
    pub fn sound_events(&self) -> impl Iterator<Item = u32> + '_ {
        self.effects
            .values()
            .filter_map(|e| e.as_ref().ok())
            .flat_map(|e| &e.groups)
            .flat_map(|g| &g.sounds)
            .flat_map(|s| [&s.start, &s.stop])
            .filter_map(|e| e.as_ref())
            .map(|e| e.declaration.id)
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
        let declarations=Arc::new(crate::assets::entities::effects::declarations::FxDeclarations::load(graphs,catalog));
        Self::load_prepared_shared(graphs,actors,catalog,library,resources,declarations)
    }
    /// Gameplay preparation already resolves named actor demands through this
    /// same canonical declaration index. Retain it, rather than decoding or
    /// constructing another copy at render-source publication.
    pub fn load_prepared_shared(
        graphs:&[Arc<FxGraph>],
        actors:&[crate::assets::entities::effects::actors::ActorDefinition],
        catalog:&Catalog,library:&mut ProgramLibrary,
        resources:&mut super::definition::ParticleResources,
        declarations:Arc<crate::assets::entities::effects::declarations::FxDeclarations>,
    )->Self{
        let mut result = Self {
            declarations,
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
            if cancelled() {
                return false;
            }
            let name = names[next].clone();
            let name = &name;
            next += 1;
            if result.effects.contains_key(name) {
                continue;
            }
            let mut children = Vec::new();
            let mut load = || -> Result<Arc<WorldEffectDefinition>, String> {
                let source = crate::assets::fx::declaration::acquire(
                    catalog,
                    name,
                    crate::assets::fx::definition::Consumer::World,
                    cancellation,
                )?;
                //0041a0's ContactSystemDB descriptor keeps the SAME canonical
                //typed declaration. Only metadata is retained by that owner;
                //this prepared world bundle and all GPU dependencies still
                //follow their existing instance/tail weak-lease lifetimes.
                let contacts = catalog.contact_database.lock().unwrap_or_else(|error| error.into_inner()).clone();
                if let Some(contacts) = contacts { contacts.retain_definition(name, source.clone()); }
                let mut groups = Vec::new();
                for (source_group, group) in source.groups.iter().enumerate() {
                    if cancelled() {
                        return Err("effect acquisition cancelled".into());
                    }
                    let asset = group.asset.clone();
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
                    let particle_assets = group
                        .particles
                        .get()
                        .expect("world FX families acquired")
                        .as_ref()
                        .map_err(Clone::clone)?;
                    let mut particles = Vec::with_capacity(particle_assets.len());
                    for asset in particle_assets {
                        if cancelled() {
                            return Err("effect acquisition cancelled".into());
                        }
                        particles.push(asset.as_ref().map_err(Clone::clone).and_then(|asset| {
                            ParticleDefinition::from_asset_cancellable(
                                asset.clone(),
                                catalog,
                                library,
                                resources,
                                cancellation,
                            )
                        }));
                    }
                    if cancelled() {
                        return Err("effect acquisition cancelled".into());
                    }
                    let decals = match group.decals.get().expect("world FX families acquired") {
                        Ok(descriptors) => {
                            let mut decals = Vec::with_capacity(descriptors.len());
                            for asset in descriptors {
                                if cancelled() {
                                    return Err("effect acquisition cancelled".into());
                                }
                                decals.push(asset.as_ref().map_err(Clone::clone).and_then(
                                    |asset| {
                                        crate::assets::fx::decal::DecalDefinition::from_asset(
                                            asset.clone(),
                                            catalog,
                                            &mut resources.decal_resources,
                                        )
                                        .map(Arc::new)
                                    },
                                ));
                            }
                            decals
                        }
                        Err(error) => vec![Err(error.clone())],
                    };
                    let lights = match group.lights.get().expect("world FX families acquired") {
                        Ok(lights) => lights.clone(),
                        Err(error) => vec![Err(error.clone())],
                    };
                    let admission = group
                        .world_other_families
                        .get()
                        .expect("world FX families acquired");
                    result.deferred_syntax_bytes += admission.syntax_bytes;
                    result.deferred_admission_bytes += admission.payload_bytes;
                    children.extend(
                        asset
                            .sub_effects
                            .iter()
                            .flat_map(|sub| sub.effects.iter().flatten().cloned()),
                    );
                    // The FX the particle declarations' m_fxEvents start
                    // (their idDeclFX references load with the declaration).
                    children.extend(fx_event_effects(&particles));
                    let mut entity_defs = Vec::new();
                    for (index, descriptor) in
                        asset
                            .entity_defs
                            .iter()
                            .enumerate()
                            .filter(|(_, descriptor)| {
                                descriptor.enabled
                                    && descriptor.declarations.iter().any(Option::is_some)
                            })
                    {
                        let declarations = descriptor
                            .declarations
                            .iter()
                            .map(|name| {
                                name.as_deref()
                                    .map(|name| {
                                        crate::assets::runtime_entity::Definition::acquire(
                                            catalog,
                                            name,
                                            cancellation,
                                        )
                                    })
                                    .transpose()
                            })
                            .collect::<Result<Vec<_>, String>>()?;
                        entity_defs.push(Arc::new(WorldEntityDef {
                            lifetime: Arc::new(()),
                            asset: asset.clone(),
                            index,
                            declarations,
                            prepared: false.into(),
                        }));
                    }
                    let mut explosions = Vec::new();
                    for (index, descriptor) in asset
                        .explosions
                        .iter()
                        .enumerate()
                        .filter(|(_, descriptor)| descriptor.enabled)
                    {
                        let Some(name) = descriptor.explosion.as_deref() else {
                            continue;
                        };
                        explosions.push(Arc::new(WorldExplosion {
                            lifetime: Arc::new(()),
                            asset: asset.clone(),
                            index,
                            declaration: crate::assets::explosions::Declaration::acquire(
                                catalog, name,
                            )?,
                            prepared: false.into(),
                        }));
                    }
                    let quads: Vec<_> = group.quads.get().ok_or("world quad family not acquired")?.as_ref().map_err(Clone::clone)?.iter().cloned().collect();
                    let mut quad_materials=Vec::new();
                    for quad in &quads {
                        let quad=quad.as_ref().map_err(Clone::clone)?;
                        if !quad.enabled || quad.stage==crate::assets::fx::screen_quad::ScreenQuadStage::Never {continue;}
                        if quad.material.is_some() && quad.stage==crate::assets::fx::screen_quad::ScreenQuadStage::ShadowWalkEnd {
                            return Err("screen quad ShadowWalkEnd requires the actual31dce0 ShadowWalk geometry consumer".into());
                        }
                        if let Some(name)=&quad.material {quad_materials.push((quad.stage,resources.quad_material(catalog,library,name)?));}
                    }
                    let fovs = group.fovs.get().ok_or("world FOV family not acquired")?.as_ref().map_err(Clone::clone)?.iter().cloned().collect();
                    let rumbles = group.rumbles.get().ok_or("world rumble family not acquired")?.as_ref().map_err(Clone::clone)?.iter().cloned().collect();
                    let screen_distortions = group.screen_distortions.get()
                        .ok_or("world FSDisto family not acquired")?.as_ref().map_err(Clone::clone)?
                        .iter().map(|asset| asset.as_ref().map_err(Clone::clone)
                            .and_then(|asset| crate::assets::fx::screen_distortion::Prepared::new(asset.clone(), catalog).map(Arc::new)))
                        .collect();
                    groups.push(WorldEffectGroup {
                        asset,
                        particles,
                        sounds,
                        lights,
                        decals,
                        entity_defs,
                        screen_distortions,
                        rumbles,
                        quads,
                        quad_materials,
                        fovs,
                        explosions,
                        source_group,
                    });
                }
                Ok(Arc::new(WorldEffectDefinition { source, groups }))
            };
            let loaded = load().map_err(|error| format!("{name}: {error}"));
            if cancelled() {
                return false;
            }
            if trace {
                match &loaded {
                    Ok(definition) => eprintln!(
                        "World FX definition acquired: {name} groups={}",
                        definition.groups.len()
                    ),
                    Err(error) => eprintln!("World FX definition rejected: {error}"),
                }
            }
            result.effects.insert(name.clone(), loaded);
            names.extend(children);
        }
        !cancelled()
    }
}
