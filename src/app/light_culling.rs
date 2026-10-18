//! Light culling.
use super::{
    entities, original_light_volume_intersects_camera, render_light, tome, view_to_game, Instant,
    LocalLightSphereBatch, OriginalLocalLightCull, OriginalLocalLightCullEnabled,
    OriginalLocalLightCullStats, SceneRenderQueue, TomeWorld, ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS,
};
use bevy::prelude::*;

pub(super) fn local_light_kind_index(kind: entities::LightType) -> Option<usize> {
    match kind {
        entities::LightType::Point => Some(0),
        entities::LightType::HalfPoint => Some(1),
        entities::LightType::Spot => Some(2),
        entities::LightType::Area => Some(3),
        entities::LightType::Parallel => None,
    }
}

// FUN_1400f0aa0 expands the light position by its cutoff to build the AABB
// projected by FUN_14010aea0. The non-Umbra path also gates this area test by
// its light-volume radius and clip-origin tests. The supported map Umbra
// path instead uses the cached-AABB query followed by the visible-cell sphere.
pub(super) fn projected_local_light_area_pixels(
    clip_from_world: Mat4,
    center: Vec3,
    range: f32,
    viewport: UVec2,
) -> Option<f32> {
    if viewport.x == 0 || viewport.y == 0 || !range.is_finite() || range <= 0.0 {
        return None;
    }

    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for corner in 0..8 {
        let offset = Vec3::new(
            if corner & 1 == 0 { -range } else { range },
            if corner & 2 == 0 { -range } else { range },
            if corner & 4 == 0 { -range } else { range },
        );
        let clip = clip_from_world * (center + offset).extend(1.0);
        if !clip.x.is_finite()
            || !clip.y.is_finite()
            || !clip.w.is_finite()
            || clip.w < f32::MIN_POSITIVE
        {
            return None;
        }
        let ndc = Vec2::new(clip.x / clip.w, clip.y / clip.w);
        min = min.min(ndc);
        max = max.max(ndc);
    }

    let ndc_area = (max.x - min.x) * (max.y - min.y);
    let pixel_area = ndc_area * viewport.x as f32 * viewport.y as f32 * 0.25;
    pixel_area.is_finite().then_some(pixel_area.max(0.0))
}

// FUN_140232bc0 retains the transformed canonical volume at +0x170.
// This is not the position +/- range box used by the non-Umbra area gate.
pub(super) fn original_light_projection_bounds(clip_to_world: Mat4) -> Option<(Vec3, Vec3)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for index in 0..8 {
        let clip = Vec3::new(
            (index & 1) as f32,
            ((index >> 1) & 1) as f32,
            ((index >> 2) & 1) as f32,
        );
        let world = clip_to_world.project_point3(clip);
        if !world.is_finite() {
            return None;
        }
        min = min.min(world);
        max = max.max(world);
    }
    Some((min, max))
}

pub(super) fn original_light_proximity_from_cached_bounds(
    max_visible_range: f32,
    bounds: Option<(Vec3, Vec3)>,
) -> Option<(Vec3, f32)> {
    let bounds = if max_visible_range > 0.0 {
        let (min, max) = bounds?;
        [min.to_array(), max.to_array()]
    } else {
        [[0.0; 3]; 2]
    };
    let (center, radius) = render_light::proximity_from_bounds(max_visible_range, bounds);
    let center = Vec3::from_array(center);
    (center.is_finite() && radius.is_finite() && radius >= 0.0).then_some((center, radius))
}

#[cfg(test)]
pub(super) fn original_light_proximity_bounds(
    max_visible_range: f32,
    clip_to_world: Option<Mat4>,
) -> Option<(Vec3, f32)> {
    original_light_proximity_from_cached_bounds(
        max_visible_range,
        clip_to_world.and_then(original_light_projection_bounds),
    )
}

pub(super) fn original_light_proximity_pass(view_origin: Vec3, center: Vec3, radius: f32) -> bool {
    // FUN_14031bcc0 compares the maximum absolute component, not Euclidean distance.
    if !view_origin.is_finite() || !center.is_finite() || !radius.is_finite() || radius < 0.0 {
        return true;
    }
    (view_origin - center).abs().max_element() <= radius
}

// FUN_1408812b0 hashes the assigned declaration's name into actor +0x4b0;
// FUN_140881640 transfers it to +0x58 for static, nonparallel idLights.
// Blueprint/runtime-generated identities are not supplied by this map parser.
pub(super) fn original_local_light_id_pass(
    kind: entities::LightType,
    culling: entities::LightCulling,
    definition_name_hash: u32,
    visible_ids: Option<&[u32]>,
) -> bool {
    kind == entities::LightType::Parallel
        || culling == entities::LightCulling::DynamicPosition
        || visible_ids
            .is_none_or(|ids| tome::sorted_authored_id_index(ids, definition_name_hash).is_some())
}

pub(super) fn original_light_rgb_nonzero(color: [f32; 3]) -> bool {
    color
        .into_iter()
        .fold(0, |bits, value| bits | value.to_bits())
        & 0x7fff_ffff
        != 0
}

// FUN_140232bc0's flag update; the owning-world update lifecycle is separate.
pub(super) fn original_render_light_update_flags(
    primary: u8,
    previous: u8,
    updates: u32,
    color: [f32; 3],
) -> u8 {
    let active = updates != 0;
    let eligible = active && primary & 1 != 0 && original_light_rgb_nonzero(color);
    (previous & 0xe7) | (u8::from(active) << 4) | (u8::from(eligible) << 3)
}

pub(super) fn cull_original_small_local_lights(
    enabled: Res<OriginalLocalLightCullEnabled>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut lights: Query<(
        Entity,
        &OriginalLocalLightCull,
        &GlobalTransform,
        &mut Visibility,
    )>,
    scene: Option<Res<SceneRenderQueue>>,
    tome_world: Option<Res<TomeWorld>>,
    mut batch: Local<LocalLightSphereBatch>,
    mut stats: ResMut<OriginalLocalLightCullStats>,
) {
    stats.total = 0;
    stats.zero_preset_rgb = 0;
    stats.total_by_kind = [0; 4];
    stats.proximity_culled = 0;
    stats.proximity_unverified = 0;
    stats.projection_bounds_unverified = 0;
    stats.source_class_counts = [0; 2];
    stats.static_name_hash_matches = 0;
    stats.static_id_culled = 0;
    stats.static_id_binding_active = false;
    stats.clip_culled = 0;
    stats.area_culled = 0;
    stats.sphere_candidates = 0;
    stats.sphere_culled = 0;
    stats.sphere_unverified = 0;
    stats.sphere_query_ns = 0;
    stats.sphere_error = None;
    batch.spheres.clear();
    batch.point_cells.clear();
    batch.entities.clear();
    let camera = cameras.single().ok().and_then(|(camera, transform)| {
        let viewport = camera.physical_viewport_size()?;
        (enabled.0 && viewport.x > 0 && viewport.y > 0).then(|| {
            (
                camera.clip_from_view() * transform.to_matrix().inverse(),
                viewport,
                transform.translation(),
            )
        })
    });
    // A mask publishes fresh ordinary query outputs, including failed-worker
    // cleared lists. Unsupported camera/mapping frames publish no mask.
    let visible_ids = scene.as_ref().and_then(|scene| {
        (camera.is_some() && !scene.visible_cell_mask.is_empty())
            .then_some(scene.visible_authored_ids.as_slice())
    });
    stats.static_id_binding_active = visible_ids.is_some();
    let world_query = visible_ids
        .and_then(|_| scene.as_ref()?.light_query_state.as_ref())
        .map(tome::RenderAabbQuery::from_state)
        .transpose();
    stats.world_query_bound = matches!(&world_query, Ok(Some(_)));
    stats.world_query_depth_grid_bound = stats.world_query_bound
        && scene
            .as_ref()
            .and_then(|scene| scene.light_query_state.as_ref())
            .is_some_and(|state| state.depth_grid.is_some());
    stats.world_query_unverified = 0;
    stats.world_query_error = world_query.as_ref().err().cloned();
    let minimum_area = camera.and_then(|(_, viewport, _)| {
        tome::minimum_light_projected_area_metric(
            i32::try_from(viewport.x).ok()?,
            i32::try_from(viewport.y).ok()?,
        )
        .ok()
    });
    for (entity, light, light_transform, mut visibility) in &mut lights {
        let Some(kind_index) = local_light_kind_index(light.kind) else {
            *visibility = Visibility::Visible;
            continue;
        };
        stats.total += 1;
        stats.total_by_kind[kind_index] += 1;
        // Preset RGB does not establish the runtime record after actor +0x408 updates.
        stats.zero_preset_rgb += usize::from(!original_light_rgb_nonzero(light.linear_color));
        stats.projection_bounds_unverified += usize::from(light.projection_bounds.is_none());
        let dynamic = light.culling == entities::LightCulling::DynamicPosition;
        stats.source_class_counts[usize::from(dynamic)] += 1;
        // The native source is the assigned compiled declaration, not the
        // optional runtime display name set by FUN_140878510.
        if !dynamic
            && scene.as_ref().is_some_and(|scene| {
                tome::sorted_authored_id_index(
                    &scene.visible_authored_ids,
                    light.definition_name_hash,
                )
                .is_some()
            })
        {
            stats.static_name_hash_matches += 1;
        }
        let mut culled = false;
        if !original_local_light_id_pass(
            light.kind,
            light.culling,
            light.definition_name_hash,
            visible_ids,
        ) {
            stats.static_id_culled += 1;
            culled = true;
        } else if visible_ids.is_some() {
            // World-query candidates use cached AABB area, then sphere cells.
            // Proximity/clip-volume checks still follow in BOTH native branches.
            match (&world_query, light.projection_bounds, minimum_area) {
                (Ok(Some(query)), Some((low, high)), Some(minimum_area)) => {
                    let low = Vec3::from_array(view_to_game(low));
                    let high = Vec3::from_array(view_to_game(high));
                    let bounds = [low.min(high).to_array(), low.max(high).to_array()];
                    match query.visibility_and_area(bounds) {
                        Ok((false, _)) => {
                            stats.clip_culled += 1;
                            culled = true;
                        }
                        Ok((true, area)) if area < minimum_area => {
                            stats.area_culled += 1;
                            culled = true;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            stats.world_query_unverified += 1;
                            stats.world_query_error.get_or_insert(error);
                        }
                    }
                }
                _ => stats.world_query_unverified += 1,
            }
        } else if let Some((clip, viewport, view_origin)) = camera {
            let proximity_pass = match light.proximity_bounds {
                Some((center, radius)) => {
                    original_light_proximity_pass(view_origin, center, radius)
                }
                None => {
                    stats.proximity_unverified += 1;
                    true
                }
            };
            if !proximity_pass {
                stats.proximity_culled += 1;
                culled = true;
            } else if light
                .clip_to_world
                .and_then(|clip_to_world| {
                    original_light_volume_intersects_camera(clip, clip_to_world)
                })
                .is_some_and(|intersects| !intersects)
            {
                stats.clip_culled += 1;
                culled = true;
            } else if projected_local_light_area_pixels(
                clip,
                light_transform.translation(),
                light.range,
                viewport,
            )
            .is_some_and(|area| area < ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS)
            {
                stats.area_culled += 1;
                culled = true;
            }
        }
        if !culled && visible_ids.is_some() {
            if let Some(sphere) = light.initial_sphere {
                if sphere.iter().all(|v| v.is_finite()) {
                    batch.spheres.push(sphere);
                    batch.entities.push(entity);
                } else {
                    stats.sphere_unverified += 1;
                }
            } else {
                stats.sphere_unverified += 1;
            }
        }
        *visibility = if culled {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
    stats.sphere_candidates = batch.spheres.len();
    if !batch.spheres.is_empty() {
        let started = Instant::now();
        let result = (|| {
            let world = tome_world
                .as_ref()
                .ok_or("light sphere query has no Tome world")?;
            let graph = world.sphere_graph.as_ref().map_err(String::as_str)?;
            let scene = scene
                .as_ref()
                .ok_or("light sphere query has no world result")?;
            let LocalLightSphereBatch {
                worker,
                spheres,
                point_cells,
                ..
            } = &mut *batch;
            for sphere in spheres.iter() {
                point_cells.push(
                    world
                        .query_data
                        .point_cell_id(&world.bytes, &world.tree, [sphere[0], sphere[1], sphere[2]])
                        .map_err(|error| {
                            stats.sphere_error.get_or_insert(error);
                            "light sphere point lookup failed"
                        })?,
                );
            }
            graph
                .query_spheres(
                    &world.bytes,
                    worker,
                    spheres,
                    point_cells,
                    &scene.visible_cell_mask,
                    Some(&world.gate_states),
                    None,
                    spheres.len(),
                )
                .map_err(|error| {
                    stats.sphere_error.get_or_insert(error);
                    "light sphere batch failed"
                })
        })();
        stats.sphere_query_ns = started.elapsed().as_nanos();
        match result {
            Ok(result) if result.error_code == 0 => {
                for &entity in &batch.entities {
                    if let Ok((_, _, _, mut visibility)) = lights.get_mut(entity) {
                        *visibility = Visibility::Hidden;
                    }
                }
                for index in result.indices {
                    if let Ok((_, _, _, mut visibility)) =
                        lights.get_mut(batch.entities[index as usize])
                    {
                        *visibility = Visibility::Visible;
                    }
                }
                stats.sphere_culled = batch.entities.len() - result.count;
            }
            Ok(result) => {
                // The original gather treats errors other than 0/5 as fatal.
                // Keep this unsupported allocator/budget path explicitly visible.
                stats.sphere_unverified += batch.entities.len();
                stats.sphere_error = Some(format!(
                    "native sphere error {}: allocator/budget handling unsupported",
                    result.error_code
                ));
            }
            Err(error) => {
                stats.sphere_unverified += batch.entities.len();
                stats.sphere_error.get_or_insert(error.to_owned());
            }
        }
    }
    // FUN_14031bcc0: sphere survivors still pass the +0x188 proximity
    // quad and +0x130 projected-volume intersection before submission.
    if visible_ids.is_some() {
        if let Some((clip, _, view_origin)) = camera {
            for (_, light, _, mut visibility) in &mut lights {
                if *visibility == Visibility::Hidden || local_light_kind_index(light.kind).is_none()
                {
                    continue;
                }
                let proximity_pass = match light.proximity_bounds {
                    Some((center, radius)) => {
                        original_light_proximity_pass(view_origin, center, radius)
                    }
                    None => {
                        stats.proximity_unverified += 1;
                        true
                    }
                };
                if !proximity_pass {
                    stats.proximity_culled += 1;
                    *visibility = Visibility::Hidden;
                } else if light
                    .clip_to_world
                    .and_then(|matrix| original_light_volume_intersects_camera(clip, matrix))
                    .is_some_and(|hit| !hit)
                {
                    stats.clip_culled += 1;
                    *visibility = Visibility::Hidden;
                }
            }
        }
    }
}
