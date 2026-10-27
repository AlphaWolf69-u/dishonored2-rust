//! Visibility queries.
use super::{
    bwm_lod_visibility_counts, game_pose_from_view_transform,
    game_settings, view_distance_scale, reference_fov_from_projection, tome, view_to_game,
    Instant, MaterialHandle, MeshTag, ViewSettings, PickupState, PickupVisual,
    PlacedModelQueries, Projection, RenderSuppressionGroup, SceneRenderQueue, StreamingDiagnostics,
    TomeWorld, VisibilityElapsedTiming, DEFAULT_FAR, DEFAULT_NEAR,
};
use bevy::prelude::*;

// FUN_14037d8d0 dispatches the query jobs and waits before consuming their
// outputs. Reuse Bevy's persistent worker pool, not per-frame OS threads.
pub(super) fn run_tome_render_jobs(
    world: &TomeWorld,
    input: &tome::OrdinaryDirectQueryInput<'_>,
) -> Vec<tome::RenderQueryJob> {
    let dispatch = |count| {
        let run = |job| {
            tome::ordinary_direct_tome_render_job(
                &world.tree,
                &world.bytes,
                &world.query_data,
                input,
                job,
                count,
            )
        };
        if count == 1 {
            return vec![run(0)];
        }
        let pool: &bevy::tasks::TaskPool = match world.query_pool.as_ref() {
            Some(pool) => pool,
            None => bevy::tasks::ComputeTaskPool::get(),
        };
        pool.scope(|scope| {
            for job in 0..count {
                let run = &run;
                scope.spawn(async move { run(job) });
            }
        })
    };
    let actual = dispatch(world.query_job_count);
    if world.query_job_count != tome::DEFAULT_QUERY_JOB_COUNT
        && std::env::var("DH2_VERIFY_TOME_QUERY_JOBS").as_deref() == Ok("1")
    {
        let expected = dispatch(tome::DEFAULT_QUERY_JOB_COUNT);
        verify_tome_job_outputs(&actual, &expected);
    }
    actual
}

// Worker lists intentionally retain duplicates and their dispatch order.
// Compare downstream object metrics/cell membership and the merged descriptor,
// rather than requiring different native tile partitions to emit equal lists.
fn verify_tome_job_outputs(actual: &[tome::RenderQueryJob], expected: &[tome::RenderQueryJob]) {
    assert!(
        actual.iter().chain(expected).all(|job| job.result.is_ok()),
        "query-job comparison requires successful workers"
    );
    let actual = tome::merge_render_query_outputs(actual).unwrap();
    let expected = tome::merge_render_query_outputs(expected).unwrap();
    assert_eq!(
        actual.is_some(),
        expected.is_some(),
        "query-job root membership changed"
    );
    let (Some(actual), Some(expected)) = (actual, expected) else {
        return;
    };
    let metrics = |output: &tome::RenderQueryObjectOutput| {
        let mut objects = std::collections::BTreeMap::<u32, Option<f32>>::new();
        for (&index, &area) in output.indices().iter().zip(output.area_metrics()) {
            let entry = objects.entry(index).or_insert(area);
            if let Some(area) = area {
                if entry.is_none_or(|previous| area > previous) {
                    *entry = Some(area);
                }
            }
        }
        objects
    };
    let actual_objects = metrics(&actual);
    let expected_objects = metrics(&expected);
    assert!(actual_objects == expected_objects,
        "query-job object/area mismatch: actual={} expected={} first_actual_difference={:?} first_expected_difference={:?}",
        actual_objects.len(), expected_objects.len(),
        actual_objects.iter().find(|(id, area)| expected_objects.get(*id) != Some(*area)),
        expected_objects.iter().find(|(id, area)| actual_objects.get(*id) != Some(*area)));
    let cells = |output: &tome::RenderQueryObjectOutput| {
        output
            .cell_indices()
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert!(
        cells(&actual) == cells(&expected),
        "query-job visible cell membership changed"
    );
    assert!(
        actual.state == expected.state,
        "query-job merged depth/descriptor changed"
    );
}

#[derive(PartialEq, Eq)]
pub(super) struct TomeRenderQueryKey {
    pub(super) camera_point: [u32; 3],
    pub(super) render_rows: [[u32; 4]; 4],
    pub(super) cropped_rows: [[u32; 4]; 4],
    pub(super) initial_tile_rect: [u8; 4],
    pub(super) camera_imprecision: [u32; 3],
    pub(super) object_imprecision: u32,
    pub(super) minimum_area_metric: u32,
    pub(super) global_distance_threshold: u32,
    pub(super) object_distance_scale: u32,
    pub(super) winding_flip: bool,
    pub(super) gate_states: Option<Vec<u32>>,
    pub(super) output_capacity: usize,
    pub(super) job_count: u32,
}

impl TomeRenderQueryKey {
    pub(super) fn new(input: &tome::OrdinaryDirectQueryInput<'_>, job_count: u32) -> Self {
        // Exhaustive destructuring makes added query inputs a compile error.
        let tome::OrdinaryDirectQueryInput {
            camera_point,
            render_rows,
            cropped_rows,
            initial_tile_rect,
            camera_imprecision,
            object_imprecision,
            minimum_area_metric,
            global_distance_threshold,
            object_distance_scale,
            winding_flip,
            gate_states,
            output_capacity,
        } = *input;
        Self {
            camera_point: camera_point.map(f32::to_bits),
            render_rows: render_rows.map(|row| row.map(f32::to_bits)),
            cropped_rows: cropped_rows.map(|row| row.map(f32::to_bits)),
            initial_tile_rect,
            camera_imprecision: camera_imprecision.map(f32::to_bits),
            object_imprecision: object_imprecision.to_bits(),
            minimum_area_metric: minimum_area_metric.to_bits(),
            global_distance_threshold: global_distance_threshold.to_bits(),
            object_distance_scale: object_distance_scale.to_bits(),
            winding_flip,
            gate_states: gate_states.map(<[u32]>::to_vec),
            output_capacity,
            job_count,
        }
    }
}

#[derive(Default)]
pub(super) struct TomeRenderQueryCache {
    pub(super) key: Option<TomeRenderQueryKey>,
    pub(super) output: Option<tome::RenderQueryObjectOutput>,
    pub(super) diagnostics: Vec<tome::RenderQueryJobDiagnostics>,
}

impl TomeRenderQueryCache {
    pub(super) fn matches(&self, key: &TomeRenderQueryKey, world_changed: bool) -> bool {
        !world_changed && self.key.as_ref() == Some(key)
    }

    pub(super) fn store(
        &mut self,
        key: TomeRenderQueryKey,
        result: &Result<Option<tome::RenderQueryObjectOutput>, String>,
        diagnostics: &[tome::RenderQueryJobDiagnostics],
    ) {
        self.key = None;
        if diagnostics.iter().any(|job| job.error.is_some()) {
            return;
        }
        if let Ok(output) = result {
            self.output = output.clone();
            self.diagnostics = diagnostics.to_vec();
            self.key = Some(key);
        }
    }
}

/// Mirror only visibility membership, keeping the per-frame update scan off
/// large mesh/material records. Pickup removal may leave an extra true bit;
/// the next touched instance corrects it. Only this system makes bits true.
#[derive(Default)]
pub(super) struct VisibilityUpdateScratch {
    instance_buffer: usize,
    visible: Vec<bool>,
}

// The ordinary direct-Tome query selects mapped static BWM instances and
// forwards their projected image demand. The installed worker clears only
// its own output on error, then returns; merge the successful siblings, log
// every failed job, and never retain a failed job's old or partial results.
pub(super) fn stream_visible_models(
    mut commands: Commands,
    cameras: Query<(Option<&Camera>, Option<&Transform>, Option<&Projection>), With<Camera3d>>,
    mut scene: ResMut<SceneRenderQueue>,
    tome_world: Option<Res<TomeWorld>>,
    placed_model_queries: Option<Res<PlacedModelQueries>>,
    pickups: Res<PickupState>,
    mut timing: Option<ResMut<VisibilityElapsedTiming>>,
    mut diagnostics: Option<ResMut<StreamingDiagnostics>>,
    mut last_tome_error: Local<Option<String>>,
    mut query_cache: Local<TomeRenderQueryCache>,
    mut updates: Local<VisibilityUpdateScratch>,
    view_settings: Option<Res<ViewSettings>>,
    mut visible_instances: Option<ResMut<super::scene_queue::VisibleSceneInstances>>,
    profile: Option<Res<super::frame_profile::CpuFrameProfile>>,
    arms: Option<Res<super::player_arms::PlayerArms>>,
) {
    let started = Instant::now();
    if tome_world.as_ref().is_none_or(|world| world.is_changed()) {
        *query_cache = TomeRenderQueryCache::default();
    }
    // Every frame replaces these outputs, including frames without a usable
    // camera. Do not expose a previous view's cells or IDs to light selection.
    scene.visible_authored_ids.clear();
    scene.visible_cell_indices.clear();
    scene.visible_cell_mask.clear();
    scene.light_query_state = None;
    scene.shadow_ordinary_query = None;
    // An unavailable view must still replace model visibility below.
    let view = cameras.single().ok();
    let (camera, transform, projection) = view.unwrap_or_default();
    let mut query_ns = 0;
    let mut query_cached = false;
    let mut mapping_ns = 0;
    let mut status = if view.is_none() {
        "camera_unavailable".to_owned()
    } else if tome_world.is_some() {
        "camera_not_ready".to_owned()
    } else {
        "no_tome".to_owned()
    };
    let mut query_diagnostics = Vec::new();
    let mut job_errors = Vec::new();
    let mut query_rows = None;
    let mut query_viewport = None;
    let mut merged_query_tile = None;
    let mut requested = 0;
    let mut mapped_count = 0;
    let mut spatial_accepted = 0;
    let mut suppressed = 0;
    let mut accepted_sample = Vec::new();
    let mut shown = 0;
    let mut spawned = 0;
    let mut hidden = 0;
    let mut tome_visibility = tome_world
        .as_ref()
        .map(|_| vec![false; scene.instances.len()]);
    if let (Some(tome_world), Some(camera), Some(transform), Some(projection)) =
        (tome_world.as_ref(), camera, transform, projection)
    {
        if let Some(viewport) = camera.physical_viewport_size() {
            query_viewport = Some([viewport.x, viewport.y]);
            scene.texture_viewport_pixels = viewport.x.saturating_mul(viewport.y);
            let query_started = Instant::now();
            let query = reference_fov_from_projection(projection)
                .and_then(|reference_fov| {
                    tome::world_visibility_query_matrix(
                        reference_fov,
                        viewport.x,
                        viewport.y,
                        1.0,
                        game_pose_from_view_transform(transform),
                        DEFAULT_NEAR,
                        DEFAULT_FAR,
                    )
                })
                .and_then(|rows| {
                    query_rows = Some(rows);
                    let query_settings = match view_settings.as_deref() {
                        Some(settings) => settings.0.clone()?,
                        None => game_settings::view_settings("")?,
                    };
                    let input = tome::OrdinaryDirectQueryInput {
                        camera_point: view_to_game(transform.translation),
                        render_rows: rows,
                        cropped_rows: rows,
                        initial_tile_rect: [0, 0, 8, 16],
                        camera_imprecision: [query_settings.umbra_distance_imprecision; 3],
                        global_distance_threshold: query_settings.umbra_accurate_threshold,
                        object_distance_scale: view_distance_scale(
                            projection,
                            viewport,
                            view_settings.as_deref(),
                        )?,
                        object_imprecision: query_settings.umbra_distance_imprecision,
                        minimum_area_metric: 25.0 / (viewport.x as f32 * viewport.y as f32),
                        winding_flip: tome::query_matrix_winding_flip(rows),
                        gate_states: Some(&tome_world.gate_states),
                        output_capacity: tome::WORLD_QUERY_OUTPUT_CAPACITY,
                    };
                    let key = TomeRenderQueryKey::new(&input, tome_world.query_job_count);
                    if query_cache.matches(&key, tome_world.is_changed()) {
                        query_cached = true;
                        query_diagnostics = query_cache.diagnostics.clone();
                        for job in &mut query_diagnostics {
                            job.elapsed_ns = 0;
                        }
                        return Ok(query_cache.output.clone());
                    }
                    query_cache.key = None;
                    let jobs = run_tome_render_jobs(tome_world, &input);
                    let result = tome::merge_render_query_outputs(&jobs);
                    job_errors = jobs
                        .iter()
                        .filter_map(|job| {
                            job.result.as_ref().err().map(|error| {
                                format!("{error} (jobId {})", job.diagnostics.job_index)
                            })
                        })
                        .collect::<Vec<_>>();
                    query_diagnostics = jobs
                        .into_iter()
                        .map(|job| job.diagnostics)
                        .collect::<Vec<_>>();
                    // Only the immutable Tome query is reused. Mapping, moving
                    // models/pickups, light gathering and texture demand still run.
                    // Failed/partial jobs are always retried, never cached.
                    query_cache.store(key, &result, &query_diagnostics);
                    result
                });
            query_ns = query_started.elapsed().as_nanos();
            let mapping_started = Instant::now();
            let mut arm_query_state = None;
            match query {
                Ok(Some(output)) => {
                    arm_query_state = output.state.clone();
                    // Native shadow preparation consumes the merged ordinary
                    // descriptor, not color-visible BWM IDs or a prior frame.
                    // Forward mapping errors do not invalidate this query.
                    {
                        let _span = profile.as_deref().map(|p| p.measure(44));
                        scene.publish_shadow_ordinary_query(&output);
                    }
                    merged_query_tile = output.state.as_ref().map(|state| state.viewport);
                    requested = output.indices().len();
                    accepted_sample.extend(output.indices().iter().take(16).copied());
                    let mut visible = vec![false; scene.instances.len()];
                    let mapping = &scene.tome_render_instances;
                    let marked = {
                        let _span = profile.as_deref().map(|p| p.measure(45));
                        mapping.mark_visible_instances(output.indices(), &mut visible)
                    };
                    let mapped = marked
                        .and_then(|()| {
                            let _span = profile.as_deref().map(|p| p.measure(46));
                            spatial_accepted = mapping
                                .apply_spatial_queries(output.state.as_ref(), &mut visible)?;
                            Ok(())
                        })
                        .and_then(|()| {
                            let _span = profile.as_deref().map(|p| p.measure(47));
                            if let Some(prepared) = &mapping.image_demands {
                                if std::env::var("DH2_VERIFY_VISIBILITY_UPDATES").as_deref()
                                    == Ok("1")
                                {
                                    prepared.verify(
                                        mapping,
                                        output.indices(),
                                        output.area_metrics(),
                                        &scene.instances,
                                        &scene.material_images,
                                        scene.diffuse_image_demands.len(),
                                    );
                                }
                                return prepared
                                    .submit(
                                        output.indices(),
                                        output.area_metrics(),
                                        &scene.diffuse_image_demands,
                                    )
                                    .map(|_| ());
                            }
                            mapping
                                .submit_material_texture_demands(
                                    output.indices(),
                                    output.area_metrics(),
                                    &scene.instances,
                                    &scene.diffuse_image_demands,
                                    &scene.material_images,
                                )
                                .map(|_| ())
                        })
                        .and_then(|()| {
                            let _span = profile.as_deref().map(|p| p.measure(48));
                            scene.publish_light_query_outputs(&tome_world, &output)
                        })
                        .and_then(|()| {
                            let _span = profile.as_deref().map(|p| p.measure(49));
                            if let (Some(groups), Some(state)) =
                                (placed_model_queries.as_ref(), output.state.as_ref())
                            {
                                super::view_geometry::query_placed_models(
                                    visible.len(),
                                    &groups.0,
                                    state,
                                    view_to_game(transform.translation),
                                    [viewport.x, viewport.y],
                                    |indices, area| {
                                        visible[indices.clone()].fill(true);
                                        for instance in &scene.instances[indices] {
                                            let slots = scene.material_images.get(&instance.material)
                                                .map(Vec::as_slice)
                                                .unwrap_or_else(|| instance.diffuse_image_slot.as_slice());
                                            for &slot in slots {
                                                scene.diffuse_image_demands.get(slot)
                                                    .ok_or("placed model material references an unknown image")?
                                                    .submit(area);
                                            }
                                        }
                                        Ok(())
                                    },
                                )?;
                            }
                            Ok(())
                        });
                    match mapped {
                        Ok(()) => {
                            mapped_count = visible.iter().filter(|&&visible| visible).count();
                            let bwm_lod_before_suppression = scene.trace_bwm_lod.then(|| {
                                bwm_lod_visibility_counts(
                                    &scene.world_lods_by_render_instance,
                                    &visible,
                                )
                            });
                            RenderSuppressionGroup::apply_all(
                                &scene.visibility_suppression_groups,
                                &mut visible,
                            );
                            suppressed =
                                mapped_count - visible.iter().filter(|&&visible| visible).count();
                            let bwm_lod_report = bwm_lod_before_suppression.map(|before| {
                                let after = bwm_lod_visibility_counts(
                                    &scene.world_lods_by_render_instance,
                                    &visible,
                                );
                                (before, after)
                            });
                            tome_visibility = Some(visible);
                            if job_errors.is_empty() {
                                status = "ok".to_owned();
                                *last_tome_error = None;
                            } else {
                                let error = job_errors.join("; ");
                                status = if job_errors.len() == query_diagnostics.len() {
                                    // Native workers clear their lists on error;
                                    // the main query still publishes the fresh
                                    // (empty) merged result to light gathering.
                                    format!("all_jobs_failed: {}", job_errors.len())
                                } else {
                                    format!("ok_with_job_errors: {}", job_errors.len())
                                };
                                if last_tome_error.as_deref() != Some(&error) {
                                    eprintln!("Tome visibility jobs: {error}");
                                    *last_tome_error = Some(error);
                                }
                            }
                            if let Some(report) = bwm_lod_report {
                                if scene.last_bwm_lod_visibility_report.as_ref() != Some(&report) {
                                    eprintln!(
                                        "BWM LOD visibility changed: before_parent_suppression={:?} after_parent_suppression={:?} requested_tome={} query_cached={} camera={:?}",
                                        report.0,
                                        report.1,
                                        requested,
                                        query_cached,
                                        game_pose_from_view_transform(transform),
                                    );
                                    scene.last_bwm_lod_visibility_report = Some(report);
                                }
                            }
                        }
                        Err(error) => {
                            status = format!("mapping_error: {error}");
                            if last_tome_error.as_deref() != Some(&error) {
                                eprintln!("Tome visibility mapping: {error}");
                                *last_tome_error = Some(error);
                            }
                        }
                    }
                }
                Ok(None) => {
                    status = "outside_tome_root".to_owned();
                    *last_tome_error = None;
                }
                Err(error) => {
                    status = format!("query_error: {error}");
                    if last_tome_error.as_deref() != Some(&error) {
                        eprintln!("Tome visibility query: {error}");
                        *last_tome_error = Some(error);
                    }
                }
            }
            // Player models are addAlways: their material images are gathered
            // even without an ordinary Tome root, independently of BWM results.
            if let Some(arms) = arms.as_deref() {
                if let Err(error) = arms.submit_texture_demands(
                    arm_query_state.as_ref(),
                    &scene.material_images,
                    &scene.diffuse_image_demands,
                ) {
                    status = format!("mapping_error: {error}");
                    if last_tome_error.as_deref() != Some(&error) {
                        eprintln!("Player texture demand: {error}");
                        *last_tome_error = Some(error);
                    }
                }
            }
            mapping_ns = mapping_started.elapsed().as_nanos();
        }
    }
    let update_started = Instant::now();
    let mut active = scene.active;
    let updates = &mut *updates;
    let instance_buffer = scene.instances.as_ptr() as usize;
    if updates.instance_buffer != instance_buffer || updates.visible.len() != scene.instances.len()
    {
        updates.instance_buffer = instance_buffer;
        updates.visible = scene
            .instances
            .iter()
            .map(|instance| instance.visible)
            .collect();
    }
    let verify_updates = std::env::var("DH2_VERIFY_VISIBILITY_UPDATES").as_deref() == Ok("1");
    if verify_updates {
        assert!(
            scene
                .instances
                .iter()
                .zip(&updates.visible)
                .all(|(instance, &recorded)| !instance.visible || recorded),
            "visibility membership missed a previously visible instance"
        );
    }
    if let Some(selection) = visible_instances.as_deref_mut() {
        selection.begin(&scene);
    }
    for index in 0..scene.instances.len() {
        // FUN_14037e1d0 handles BWM indices; FUN_14031ad30 handles the
        // supported initial BML groups independently of serialized Tome IDs.
        let visible = tome_visibility.as_ref().is_some_and(|result| result[index]);
        if !visible && !updates.visible[index] {
            continue;
        }
        let instance = &mut scene.instances[index];
        let available = instance
            .pickup_id
            .as_ref()
            .is_none_or(|id| pickups.world.pickup_available(id));
        if visible && available {
            if let Some(entity) = instance.entity {
                if !instance.visible {
                    commands.entity(entity).insert(Visibility::Visible);
                    instance.visible = true;
                    active += 1;
                    shown += 1;
                }
            } else {
                let mut entity = match &instance.material {
                    MaterialHandle::Water(material) => commands.spawn((
                        Mesh3d(instance.mesh.clone()),
                        crate::rendering::bevy_backend::water_scene::WaterMesh {
                            material: material.clone(), order: index,
                            geometry: instance.source.map(|source| match source.surface {
                                super::RenderModelSurface::World { object, matrix } => crate::rendering::bevy_backend::water_scene::WaterGeometry::World { object, matrix },
                                super::RenderModelSurface::Bmd6 {skin_influences,..} if skin_influences>0 => crate::rendering::bevy_backend::water_scene::WaterGeometry::SkinnedModel,
                                _ => crate::rendering::bevy_backend::water_scene::WaterGeometry::Model,
                            }),
                        },
                        instance.transform,
                        Visibility::Visible,
                    )),
                    MaterialHandle::BoatVolume(material) => commands.spawn((
                        Mesh3d(instance.mesh.clone()),
                        crate::rendering::bevy_backend::boat_scene::BoatMesh {
                            material:material.clone(),order:index,
                            geometry:match instance.source.map(|s|s.surface) {
                                Some(super::RenderModelSurface::World{object,matrix})=>crate::rendering::bevy_backend::boat_scene::BoatGeometry::World{object,matrix},
                                Some(super::RenderModelSurface::Bmodel{..})=>crate::rendering::bevy_backend::boat_scene::BoatGeometry::RigidModel,
                                _=>crate::rendering::bevy_backend::boat_scene::BoatGeometry::Unsupported,
                            },
                        },
                        instance.transform,
                        Visibility::Visible,
                    )),
                    MaterialHandle::Interior(_) => commands.spawn((
                        Mesh3d(instance.mesh.clone()),
                        crate::rendering::bevy_backend::fog::InteriorVolume,
                        instance.transform,
                        Visibility::Visible,
                    )),
                    MaterialHandle::Sky(material) => commands.spawn((
                        Mesh3d(instance.mesh.clone()),
                        crate::rendering::bevy_backend::sky_mask::SkyMesh {material:material.clone(),order:index},
                        instance.transform,
                        Visibility::Visible,
                    )),
                    MaterialHandle::Probe(material) => commands.spawn((
                        Mesh3d(instance.mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        instance.transform,
                        Visibility::Visible,
                    )),
                    MaterialHandle::Decal(material) => commands.spawn((
                        Mesh3d(instance.mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        instance.transform,
                        Visibility::Visible,
                    )),
                };
                // Streaming runs after transform propagation. These are
                // root entities: publish the authored world transform now,
                // rather than culling at the default identity for one frame.
                entity.insert((
                    MeshTag(instance.probe_index),
                    GlobalTransform::from(instance.transform),
                ));
                if let Some(id) = &instance.pickup_id {
                    entity.insert(PickupVisual(id.clone()));
                }
                instance.entity = Some(entity.id());
                instance.visible = true;
                active += 1;
                spawned += 1;
            }
        } else if instance.visible {
            if let Some(entity) = instance.entity {
                commands.entity(entity).insert(Visibility::Hidden);
                instance.visible = false;
                active -= 1;
                hidden += 1;
            }
        }
        updates.visible[index] = instance.visible;
        if instance.visible {
            if let Some(selection) = visible_instances.as_deref_mut() {
                selection.indices.push(index);
            }
        }
    }
    if verify_updates {
        assert!(
            scene
                .instances
                .iter()
                .zip(&updates.visible)
                .all(|(instance, &recorded)| instance.visible == recorded),
            "visibility update mirror differs from actual instance state"
        );
    }
    if verify_updates {
        if let Some(selection) = visible_instances.as_deref() {
            let actual: Vec<_> = scene
                .instances
                .iter()
                .enumerate()
                .filter_map(|(index, instance)| instance.visible.then_some(index))
                .collect();
            assert_eq!(
                selection.indices, actual,
                "forward visible indices differ from actual instance visibility"
            );
        }
    }
    scene.active = active;
    if let Some(diagnostics) = diagnostics.as_deref_mut() {
        diagnostics.frame += 1;
        if diagnostics.frame % 120 == 0
            && !query_cached
            && std::env::var_os("DH2_PROFILE_TOME_QUERY").is_some()
        {
            let mut costs = [0u128; 5];
            for job in &query_diagnostics {
                for (total, cost) in costs.iter_mut().zip(job.progress.cost_ns) {
                    *total += cost;
                }
            }
            if let Some(maximum) = query_diagnostics.iter().max_by_key(|job| job.elapsed_ns) {
                eprintln!("Tome query CPU cost sum_ms={:?} max_job={} max_wall_ms={:.3} max_cost_ms={:?} cells={} object_passes={} objects={}",
                    costs.map(|cost| cost as f64 / 1e6), maximum.job_index,
                    maximum.elapsed_ns as f64 / 1e6, maximum.progress.cost_ns.map(|cost| cost as f64 / 1e6),
                    maximum.progress.cells_processed, maximum.progress.object_passes, maximum.progress.accepted_objects);
            }
        }
        diagnostics.status = status;
        let pose = transform.map(game_pose_from_view_transform);
        let authored_id_sample =
            &scene.visible_authored_ids[..scene.visible_authored_ids.len().min(16)];
        let visible_cell_entries = scene.visible_cell_indices.len();
        let visible_cell_unique: u32 = scene
            .visible_cell_mask
            .iter()
            .map(|word| word.count_ones())
            .sum();
        let visible_cell_mask_words = scene.visible_cell_mask.len();
        let visible_cell_sample =
            &scene.visible_cell_indices[..scene.visible_cell_indices.len().min(16)];
        let distance_rejected_job_candidates: usize = query_diagnostics
            .iter()
            .map(|job| job.progress.distance_rejected_objects)
            .sum();
        let depth_grid_bound = scene
            .light_query_state
            .as_ref()
            .is_some_and(|state| state.depth_grid.is_some());
        diagnostics.pending = format!(
            "frame={} status={:?} jobs={} distance_rejected_job_candidates={distance_rejected_job_candidates} authored_ids={} authored_id_consumer=static_light_id_gate visible_cell_entries={visible_cell_entries} visible_cell_unique={visible_cell_unique} cell_mask_words={visible_cell_mask_words} light_binding=authored_ids_and_cells light_consumer=see_light_stage query_cached={query_cached} viewport={query_viewport:?} merged_query_tile={merged_query_tile:?} depth_grid_bound={depth_grid_bound} requested={requested} mapped={mapped_count} spatial_tested={} spatial_accepted_bwm={spatial_accepted} suppressed={suppressed} active={active}/{} spawned={spawned} shown={shown} hidden={hidden} query_ms={:.3} mapping_ms={:.3} elapsed_ms={:.3}\npose={pose:?}\nrender_rows={query_rows:?} winding_flip={:?}\naccepted_sample={accepted_sample:?}\nauthored_id_sample={authored_id_sample:?}\nvisible_cell_sample={visible_cell_sample:?}\n{query_diagnostics:?}",
            diagnostics.frame,
            diagnostics.status,
            query_diagnostics.len(),
            scene.visible_authored_ids.len(),
            scene.tome_render_instances.spatial_query_instances.len(),
            scene.instances.len(),
            query_ns as f64 / 1_000_000.0,
            mapping_ns as f64 / 1_000_000.0,
            started.elapsed().as_secs_f64() * 1000.0,
            query_rows.map(tome::query_matrix_winding_flip),
        );
    }
    if let Some(timing) = timing.as_deref_mut() {
        let elapsed_ns = started.elapsed().as_nanos();
        timing.samples += 1;
        timing.total_ns += elapsed_ns;
        timing.max_ns = timing.max_ns.max(elapsed_ns);
        timing.query_ns += query_ns;
        timing.mapping_ns += mapping_ns;
        timing.update_ns += update_started.elapsed().as_nanos();
    }
}
