//! Forward arkvegetation wind inputs: arkWindManager::PostLoadMap (7b7be0)
//! over the shared wind centers (295e40), the per-frame windInfo samples
//! (339160 -> 77fbc0) and one AnimateVegetation record per render instance.
use super::{
    archive,
    fog_scene::WorldWind,
    shadow_preparation::{VegetationMeshInputs, WorldShadowMeshSources},
    MaterialHandle, RenderModelSurface, RenderSourcePlacements, SceneData, SceneRenderQueue,
};
use crate::{
    game_clock,
    gameplay::{
        game_clock_runtime::{ClockRuntime, GameplayViewClock},
        wind::{trigger_wind_sources, TriggerWindSource, WindManager},
    },
    render_model,
    rendering::bevy_backend::vegetation_wind::{instance_words, INSTANCE_WORDS, NO_SLOT},
};
use bevy::{prelude::*, render::storage::ShaderBuffer};
use std::collections::HashMap;

/// The shared buffers every probe material binds, plus each arkVegetation
/// material's vegetAnim vectors (literal overrides over registered defaults,
/// the same resolution as its shadow parameters, with no draw fallback).
#[derive(Resource, Clone, Default)]
pub(super) struct VegetationMaterials {
    pub(super) slots: Handle<ShaderBuffer>,
    pub(super) instances: Handle<ShaderBuffer>,
    pub(super) parameters: HashMap<MaterialHandle, Result<[f32; 7], String>>,
}

impl VegetationMaterials {
    pub(super) fn new(buffers: &mut Assets<ShaderBuffer>) -> Self {
        Self {
            slots: buffers.add(ShaderBuffer::from(vec![0u32])),
            instances: buffers.add(ShaderBuffer::from(vec![0u32; INSTANCE_WORDS])),
            parameters: HashMap::new(),
        }
    }
}

/// The level's trigger winds with their installed shapes.
#[derive(Resource)]
pub(super) struct LevelTriggerWinds(Result<Vec<TriggerWindSource>, String>);

impl LevelTriggerWinds {
    pub(super) fn load(
        catalog: &archive::Catalog,
        document: Option<&crate::entities::Document>,
    ) -> Self {
        Self(document.map_or_else(|| Ok(Vec::new()), |d| trigger_wind_sources(catalog, d)))
    }
}

/// World `$localTime` (3586a0, gameplay view definition byte4 left at its
/// constructor value by 88b9f0) or a model's initial group-0 clock, as the
/// shadow programs of the same instances use.
#[derive(Clone, Copy)]
enum InstanceClock {
    View,
    InitialModel,
}

struct InstanceSource {
    rows: Option<[[f32; 4]; 3]>,
    vectors: [f32; 7],
    clock: InstanceClock,
    /// Wind center of a multi-instanced needsWind world instance.
    center: Option<usize>,
}

#[derive(Resource)]
pub(super) struct VegetationWind {
    manager: WindManager,
    centers: Vec<[f32; 3]>,
    sources: Vec<InstanceSource>,
    instances: Handle<ShaderBuffer>,
    uploaded: bool,
}

/// Builds the slot table and instance sources after setup has resolved every
/// render instance, its mesh channels and the material +0x48 flags.
pub(super) fn initialize_vegetation_wind(
    mut commands: Commands,
    scene: Res<SceneData>,
    queue: Res<SceneRenderQueue>,
    placements: Res<RenderSourcePlacements>,
    world_sources: Res<WorldShadowMeshSources>,
    meshes: Res<VegetationMeshInputs>,
    materials: Res<VegetationMaterials>,
    world_wind: Res<WorldWind>,
    triggers: Option<ResMut<LevelTriggerWinds>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let flags48: Vec<u8> = world_sources
        .objects
        .iter()
        .map(|source| match &source.gather_flags48 {
            Some(Ok(flags)) => *flags,
            _ => 0,
        })
        .collect();
    let unresolved_flags = world_sources
        .objects
        .iter()
        .filter(|source| !matches!(source.gather_flags48, Some(Ok(_))))
        .count();
    let mut centers = Vec::new();
    let mut center_indices = vec![-1i16; scene.world.visibility.len()];
    if let Err(error) = scene
        .world
        .prepare_wind_centers(&flags48, &mut centers, &mut center_indices)
    {
        warn!("Vegetation wind centers unavailable: {error}");
        center_indices.fill(-1);
        centers.clear();
    }
    let triggers = match triggers.map(|mut t| std::mem::replace(&mut t.0, Ok(Vec::new()))) {
        Some(Ok(triggers)) => triggers,
        Some(Err(error)) => {
            warn!("Trigger winds unavailable, centers keep the world wind: {error}");
            Vec::new()
        }
        None => Vec::new(),
    };
    let trigger_count = triggers.len();
    let manager = WindManager::post_load_map(world_wind.0.is_some(), triggers, &centers);
    let world_count = scene.world.matrices.len();
    let probe_count = queue
        .instances
        .iter()
        .filter(|instance| instance.probe_index & 0x8000_0000 == 0)
        .map(|instance| instance.probe_index as usize + 1)
        .max()
        .unwrap_or(0);
    let mut slots = vec![NO_SLOT; 1 + world_count + probe_count];
    slots[0] = world_count as u32;
    let mut sources = Vec::new();
    let mut errors = HashMap::<String, usize>::new();
    for instance in &queue.instances {
        let Some(parameters) = materials.parameters.get(&instance.material) else {
            continue;
        };
        let result = (|| -> Result<InstanceSource, String> {
            let vectors = parameters.clone()?;
            meshes
                .0
                .get(&instance.mesh.id())
                .ok_or("mesh has no original vegetation channels")?
                .clone()?;
            let source = instance.source.as_ref().ok_or("instance has no source")?;
            Ok(match source.surface {
                RenderModelSurface::World { object, matrix } => {
                    let m = scene.world.matrices[matrix];
                    let wind = flags48[object] & 0x40 != 0;
                    InstanceSource {
                        rows: Some([
                            [m[0], m[1], m[2], m[3]],
                            [m[4], m[5], m[6], m[7]],
                            [m[8], m[9], m[10], m[11]],
                        ]),
                        vectors,
                        clock: InstanceClock::View,
                        center: wind
                            .then(|| usize::try_from(center_indices[matrix]).ok())
                            .flatten(),
                    }
                }
                RenderModelSurface::Bmodel { placement, .. }
                | RenderModelSurface::Bmd6 { placement, .. } => {
                    let placement = &placements.0[placement];
                    let rows = render_model::affine_rows(
                        placement.position,
                        placement.rotation,
                        placement.scale,
                    );
                    InstanceSource {
                        rows: source.world_baked.then(|| [rows[0], rows[1], rows[2]]),
                        vectors,
                        clock: InstanceClock::InitialModel,
                        center: None,
                    }
                }
            })
        })();
        match result {
            Ok(source) => {
                let tag = instance.probe_index;
                let index = if tag & 0x8000_0000 != 0 {
                    1 + (tag & 0x7fff_ffff) as usize
                } else {
                    1 + world_count + tag as usize
                };
                slots[index] = sources.len() as u32;
                sources.push(source);
            }
            Err(error) => *errors.entry(error).or_default() += 1,
        }
    }
    for (error, count) in &errors {
        warn!("{count} vegetation instances keep their rest pose: {error}");
    }
    let wind_instances = sources.iter().filter(|s| s.center.is_some()).count();
    info!(
        "Vegetation wind: {} instances ({wind_instances} on {} wind centers), {trigger_count} \
         trigger winds, {} ambiguous centers, {unresolved_flags} BWM objects without +0x48 flags",
        sources.len(),
        centers.len(),
        manager.ambiguous_centers
    );
    if let Some(mut buffer) = buffers.get_mut(&materials.slots) {
        buffer.set_data(slots);
    }
    if let Some(mut buffer) = buffers.get_mut(&materials.instances) {
        buffer.set_data(vec![0u32; sources.len().max(1) * INSTANCE_WORDS]);
    }
    commands.insert_resource(VegetationWind {
        manager,
        centers,
        sources,
        instances: materials.instances.clone(),
        uploaded: false,
    });
}

/// Advances the trigger winds with the world wind's actor clock group and
/// rewrites every instance record. The record count never changes, so the
/// GPU buffer and material bindings stay in place.
pub(super) fn update_vegetation_wind(
    clock: Res<ClockRuntime>,
    view: Res<GameplayViewClock>,
    world_wind: Res<WorldWind>,
    wind: Option<ResMut<VegetationWind>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut last_error: Local<Option<String>>,
) {
    let Some(mut wind) = wind else {
        return;
    };
    let advanced = clock.advanced();
    if advanced {
        if let Ok(clock) = clock.clock() {
            // Default entity timespace 0 -> clock group3 (877eb0), as WorldWind.
            wind.manager.advance(clock.delta(3).expect("five clock groups"));
        }
    }
    if wind.uploaded && !advanced {
        return;
    }
    let result = (|| -> Result<Vec<u32>, String> {
        let view_time = view
            .view_shader_times(game_clock::RESET_VIEW_LOCAL_TIME_FLAG)?
            .local_time[0];
        let model_time = view.model_local_time(0.0, 4)?[0];
        let mut words = Vec::with_capacity(wind.sources.len().max(1) * INSTANCE_WORDS);
        for source in &wind.sources {
            let record = source.center.and_then(|center| {
                wind.manager
                    .center_wind(world_wind.0.as_ref(), center, wind.centers[center])
            });
            let time = match source.clock {
                InstanceClock::View => view_time,
                InstanceClock::InitialModel => model_time,
            };
            words.extend(instance_words(
                source.rows,
                source.vectors,
                time,
                record.map(|[_, current]| current),
            ));
        }
        if words.is_empty() {
            words.resize(INSTANCE_WORDS, 0);
        }
        Ok(words)
    })();
    match result {
        Ok(words) => {
            if let Some(mut buffer) = buffers.get_mut(&wind.instances) {
                buffer.set_data(words);
            }
            wind.uploaded = true;
            *last_error = None;
        }
        Err(error) => {
            if last_error.as_ref() != Some(&error) {
                warn!("Vegetation wind records pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}
