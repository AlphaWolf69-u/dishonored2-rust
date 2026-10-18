//! Diagnostics.
use super::{
    fs, reference_fov_from_projection, save_to_disk, screen_shadow_pass, shadow_pass, tome,
    transformed, view_to_game, world_mesh, NavigationState, NavigationWorld, Projection, SceneData,
    SceneRenderQueue, Screenshot, SnapshotPath, StreamingDiagnostics, SystemTime, TomeWorld,
    ORIGINAL_DEFAULT_FAR, ORIGINAL_DEFAULT_NEAR, UNIX_EPOCH,
};
use bevy::prelude::*;

pub(super) fn snapshot_once(
    mut commands: Commands,
    request: Option<Res<SnapshotPath>>,
    time: Res<Time>,
    native_screen: Option<Res<screen_shadow_pass::NativeScreenShadowFrame>>,
    pipelines: Option<Res<shadow_pass::NativeShadowCaptureReadiness>>,
    mut captured: Local<bool>,
    mut ready_since: Local<Option<f32>>,
) {
    let Some(request) = request else {
        return;
    };
    let shadows_ready = native_screen
        .as_ref()
        .is_none_or(|screen| screen.produced.load(std::sync::atomic::Ordering::Acquire) >= 30);
    let materials_ready = pipelines
        .as_ref()
        .is_none_or(|ready| ready.0.load(std::sync::atomic::Ordering::Acquire) >= 30);
    if !shadows_ready || !materials_ready {
        *ready_since = None;
        return;
    }
    // Let exposure and texture residency settle after drawable materials exist,
    // rather than counting shader-compilation time as rendered warmup.
    let since = ready_since.get_or_insert(time.elapsed_secs());
    if !*captured && time.elapsed_secs() - *since > 6.0 {
        *captured = true;
        let destination = request.0.clone();
        println!("Capturing screenshot to {}", destination.display());
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(destination));
    }
}

pub(super) fn ray_triangle(origin: Vec3, direction: Vec3, triangle: [Vec3; 3]) -> Option<f32> {
    let [a, b, c] = triangle;
    let edge1 = b - a;
    let edge2 = c - a;
    let cross = direction.cross(edge2);
    let determinant = edge1.dot(cross);
    if determinant.abs() < 1e-7 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let from_a = origin - a;
    let u = from_a.dot(cross) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let v = direction.dot(from_a.cross(edge1)) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = edge2.dot(from_a.cross(edge1)) * inverse;
    (distance > 0.001).then_some(distance)
}

pub(super) fn sightline_world_surface(
    world: &world_mesh::WorldMesh,
    origin: Vec3,
    direction: Vec3,
    limit: f32,
) -> Option<(f32, usize, usize, String)> {
    let mut nearest: Option<(f32, usize, usize, String)> = None;
    for object in world.objects.iter().filter(|object| object.lod == 0) {
        let mesh = &world.meshes[object.mesh_index];
        for matrix_index in object.matrix_start..object.matrix_end {
            let matrix = &world.matrices[matrix_index];
            for face in mesh.indices.chunks_exact(3) {
                let triangle = [face[0], face[1], face[2]]
                    .map(|index| transformed(matrix, mesh.positions[index as usize]));
                let Some(distance) = ray_triangle(origin, direction, triangle) else {
                    continue;
                };
                if distance > limit || nearest.as_ref().is_some_and(|hit| distance >= hit.0) {
                    continue;
                }
                nearest = Some((
                    distance,
                    object.mesh_index,
                    matrix_index,
                    object.material_path.clone(),
                ));
            }
        }
    }
    nearest
}

pub(super) fn game_pose_from_view_transform(transform: &Transform) -> [[f32; 4]; 4] {
    let right = view_to_game(transform.rotation * Vec3::X);
    let up = view_to_game(transform.rotation * Vec3::Y);
    let backward = view_to_game(transform.rotation * Vec3::Z);
    let translation = view_to_game(transform.translation);
    [
        [right[0], up[0], backward[0], translation[0]],
        [right[1], up[1], backward[1], translation[1]],
        [right[2], up[2], backward[2], translation[2]],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub(super) fn tome_camera_matrix_text(
    tome_world: &TomeWorld,
    camera: &Camera,
    transform: &Transform,
    projection: &Projection,
) -> String {
    let Some(viewport) = camera.physical_viewport_size() else {
        return "Tome camera matrix unavailable: camera has no physical viewport".to_owned();
    };
    // Supported full-view, unit-scale path captured in the original game.
    // Dynamic resolution and subrectangle ownership remain unsupported.
    let dimensions = (|| {
        let width = i32::try_from(viewport.x).map_err(|_| "viewport width overflow")?;
        let height = i32::try_from(viewport.y).map_err(|_| "viewport height overflow")?;
        let rect = tome::scaled_render_viewport([0, 0, width - 1, height - 1], [1.0; 2])?;
        tome::render_viewport_dimensions(rect)
    })();
    let [width, height] = match dimensions {
        Ok(dimensions) => dimensions,
        Err(error) => return format!("Tome camera viewport unavailable: {error}"),
    };
    let reference_fov = match reference_fov_from_projection(projection) {
        Ok(fov) => fov,
        Err(error) => return format!("Tome camera matrix unavailable: {error}"),
    };
    let camera_pose = game_pose_from_view_transform(transform);
    let matrix = match tome::original_world_visibility_query_matrix(
        reference_fov,
        width as u32,
        height as u32,
        1.0,
        camera_pose,
        ORIGINAL_DEFAULT_NEAR,
        ORIGINAL_DEFAULT_FAR,
    ) {
        Ok(matrix) => matrix,
        Err(error) => return format!("Tome camera matrix unavailable: {error}"),
    };
    let planes = tome::query_planes_from_rows(matrix, matrix);
    let camera_position = view_to_game(transform.translation);
    match tome_world.tree.record_nodes_for_camera_query_prefilter(
        camera_position,
        &planes,
        [0.0; 3],
    ) {
        Ok(nodes) => format!(
            "Tome matrix from current camera projection: viewport={}x{}, pixel aspect=1, camera_game_xyz={camera_position:?}, rows={matrix:?}\nGlobal spatial-record candidates from geometric prefilter: {nodes:?}. This geometric prefilter is not the final visibility result; the live query's traversal, accepted-object output, mapping and upload diagnostics are in the matching streaming history.",
            viewport.x, viewport.y
        ),
        Err(error) => format!(
            "Tome matrix from current camera projection: viewport={}x{}, camera_game_xyz={camera_position:?}, rows={matrix:?}\nGlobal spatial-record prefilter failed: {error}",
            viewport.x, viewport.y
        ),
    }
}

pub(super) fn tome_point_text(
    tome_world: &TomeWorld,
    world: &world_mesh::WorldMesh,
    render_queue: &SceneRenderQueue,
    label: &str,
    point: [f32; 3],
) -> String {
    let point_volume_records = tome_world
        .tree
        .record_leaves_containing_point(point)
        .into_iter()
        .map(|index| (index, tome_world.tree.record_offset(index)))
        .collect::<Vec<_>>();
    if point_volume_records.is_empty() {
        return format!("Tome {label} {point:?}: outside Tome bounds");
    }
    let mut lines = vec![format!(
        "Tome {label} {point:?}: point-containment prefilter touches {} authored spatial records (not the full visibility result); {} gate-state words; cell-slot rows {} Г— {} bits",
        point_volume_records.len(),
        tome_world.gate_states.len(),
        tome_world.query_data.slot_rows.spatial_record_count(),
        tome_world.query_data.slot_rows.bit_count()
    )];
    let point_cell =
        tome_world
            .query_data
            .point_cell_id(&tome_world.bytes, &tome_world.tree, point);
    let sphere_result = point_cell.and_then(|cell| {
        let graph = tome_world.sphere_graph.as_ref().map_err(Clone::clone)?;
        graph.query_spheres(
            &tome_world.bytes,
            &mut tome::SphereCellWorker::default(),
            &[[point[0], point[1], point[2], 0.0]],
            &[cell],
            &render_queue.visible_cell_mask,
            Some(&tome_world.gate_states),
            None,
            1,
        )
    });
    lines.push(format!(
        "  zero-radius sphere diagnostic (not light rejection): {sphere_result:?}"
    ));
    for (leaf, offset) in point_volume_records {
        let Some(offset) = offset.filter(|&offset| offset != 0) else {
            lines.push(format!("  spatial record {leaf}: no local Tome record"));
            continue;
        };
        let result = tome::CellTree::parse(&tome_world.bytes, offset).and_then(|cells| {
            let Some(cell) = cells.cell_for_point(&tome_world.bytes, point)? else {
                return Ok(None);
            };
            let candidates = cells.object_candidates(&tome_world.bytes, cell.cell_index)?;
            let authored_ids = candidates
                .iter()
                .map(|&index| tome::object_ids_for_index(&tome_world.bytes, index))
                .collect::<Result<Vec<_>, _>>()?;
            let bounds = candidates
                .iter()
                .map(|&index| tome::object_bounds(&tome_world.bytes, index))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some((cell, candidates, authored_ids, bounds)))
        });
        match result {
            Ok(Some((cell, candidates, authored_ids, bounds))) => {
                lines.push(format!(
                    "  point-containing spatial record {leaf}, cell ID {}, bounds {:?} .. {:?}, {} traversal links, {} point-query object candidates",
                    cell.cell_id, cell.bounds[0], cell.bounds[1], cell.link_count, candidates.len()));
                for ((index, authored_ids), bounds) in
                    candidates.into_iter().zip(authored_ids).zip(bounds)
                {
                    let instances = world.tome_object_instances.get(index as usize);
                    let materials = instances
                        .into_iter()
                        .flatten()
                        .filter_map(|&instance| {
                            world
                                .objects
                                .iter()
                                .find(|object| {
                                    object.matrix_start <= instance && instance < object.matrix_end
                                })
                                .map(|object| {
                                    format!("{} (LOD {})", object.material_path, object.lod)
                                })
                        })
                        .collect::<Vec<_>>();
                    let render_indices = render_queue.tome_render_instances.get(index as usize);
                    let active = render_indices
                        .iter()
                        .filter(|&&render_index| {
                            render_queue
                                .instances
                                .get(render_index)
                                .is_some_and(|instance| instance.visible)
                        })
                        .count();
                    lines.push(format!("    Tome index {index}, bounds {:?} .. {:?}, authored IDs {authored_ids:?}: BWM instances {}, rewrite render instances {} (currently active {}), materials {:?}",
                    bounds[0], bounds[1], instances.map_or(0, Vec::len),
                    render_indices.len(), active, materials));
                }
            }
            Ok(None) => lines.push(format!(
                "  point-candidate spatial record {leaf}: point is outside its local cells"
            )),
            Err(error) => lines.push(format!(
                "  point-candidate spatial record {leaf}: local lookup failed: {error}"
            )),
        }
    }
    lines.join("\n")
}

pub(super) fn collision_report(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    scene: Res<SceneData>,
    navigation: Res<NavigationState>,
    collision: Option<Res<NavigationWorld>>,
    tome_world: Option<Res<TomeWorld>>,
    render_queue: Res<SceneRenderQueue>,
    cameras: Query<(&Camera, &Transform, &Projection), With<Camera3d>>,
    diagnostics: Option<Res<StreamingDiagnostics>>,
) {
    if !keys.just_pressed(KeyCode::F8) {
        return;
    }
    let Ok((camera_component, camera, projection)) = cameras.single() else {
        return;
    };
    let direction = camera.forward().as_vec3();
    let distance_limit = match projection {
        Projection::Perspective(p) => p.far,
        _ => 40.0,
    };
    let visual =
        sightline_world_surface(&scene.world, camera.translation, direction, distance_limit);
    let visual_text = match &visual {
        Some((distance, mesh, matrix, material)) => format!(
            "BWM surface: distance={distance:.3} m, game_xyz={:?}, mesh={mesh}, matrix={matrix}, material={material}",
            view_to_game(camera.translation + direction * *distance)
        ),
        None => format!("BWM surface: none along center ray within {distance_limit} m"),
    };
    let probe_feet = if navigation.walking {
        navigation.walker.feet
    } else {
        camera.translation - Vec3::Y * 1.7
    };
    let collision_text = if let Some(world) = collision.as_ref() {
        let details = world.0.first_hit_details(
            camera.translation,
            camera.translation + direction * distance_limit,
        );
        let hit = details.map(|hit| hit.0);
        let provenance = details.map(|(_, owner, vertices)| {
            (
                world.0.owner_name(owner).unwrap_or("level bphysworld"),
                vertices.map(view_to_game),
            )
        });
        let target = visual
            .as_ref()
            .map(|(distance, _, _, _)| camera.translation + direction * *distance)
            .unwrap_or(camera.translation + direction * 5.0);
        let hit_position =
            hit.map(|distance| view_to_game(camera.translation + direction * distance));
        let visual_gap = visual.as_ref().and_then(|(distance, _, _, _)| {
            hit.map(|collision_distance| collision_distance - *distance)
        });
        format!(
            "Collision ray hit: {hit:?} m; game_xyz={hit_position:?}; distance from visual surface={visual_gap:?} m\nCollision source and triangle game_xyz: {provenance:?}\nCollision triangles within 2 m of visual target: {}\nCollision triangles within 2 m of feet probe: {}\nFloor at feet probe: {:?}",
            world.0.nearby_triangle_count(target, 2.0),
            world.0.nearby_triangle_count(probe_feet, 2.0),
            world.0.floor_at(probe_feet, 0.5, 2.0)
        )
    } else {
        "Collision world: unavailable".to_owned()
    };
    let tome_text = if let Some(tome_world) = tome_world.as_ref() {
        format!(
            "{}\n{}\n{}",
            tome_point_text(
                tome_world,
                &scene.world,
                &render_queue,
                "eye",
                view_to_game(camera.translation)
            ),
            tome_point_text(
                tome_world,
                &scene.world,
                &render_queue,
                "feet",
                view_to_game(probe_feet)
            ),
            tome_camera_matrix_text(tome_world, camera_component, camera, projection),
        )
    } else {
        "Tome: no matching installed level resource loaded".to_owned()
    };
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_millis())
        .unwrap_or(0);
    let directory = super::state::diagnostic_directory("collision-reports");
    if let Err(error) = fs::create_dir_all(&directory) {
        eprintln!("Collision report directory: {error}");
        return;
    }
    let stem = format!("collision-{stamp}");
    let report_path = directory.join(format!("{stem}.txt"));
    let screenshot_path = directory.join(format!("{stem}.png"));
    let report = format!(
        "Level: {}\nCamera game_xyz: {:?}\nCamera direction game_xyz: {:?}\nCamera up game_xyz: {:?}\nReference horizontal FOV degrees: {}\nFeet probe game_xyz: {:?}\nMode: {}\nWalker supported: {}\nWalker vertical speed: {:.3} m/s\n{}\n{}\n{}\nScreenshot: {}\nNote: In free-fly, feet probe is 1.7 m below the camera; walker state is inactive. BWM ray tests static world geometry; placed visual models are identified in the screenshot. Collision ray tests the combined loaded collision index. Tome object candidates precede the game's final visibility tests.\n",
        scene.source_path.display(),
        view_to_game(camera.translation),
        view_to_game(direction),
        view_to_game(camera.up().as_vec3()),
        reference_fov_from_projection(projection)
            .expect("F8 perspective camera must have a valid FOV"),
        view_to_game(probe_feet),
        if navigation.walking {
            "walk"
        } else {
            "free-fly"
        },
        navigation.walking && navigation.walker.supported,
        if navigation.walking {
            navigation.walker.vertical_speed
        } else {
            0.0
        },
        visual_text,
        collision_text,
        tome_text,
        screenshot_path.display()
    );
    let history_path = directory.join(format!("{stem}-streaming.log"));
    if let Some(diagnostics) = diagnostics.as_ref() {
        let texture_event_history = if diagnostics.trace_texture_streaming {
            diagnostics
                .texture_event_history
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            "disabled (set DH2_TRACE_TEXTURE_STREAMING=1 before launch)".to_owned()
        };
        let history = format!(
            "Level: {}\nSession log: {}\n{}\nRecent texture-stream events (up to 256 retained):\n{}",
            scene.source_path.display(),
            diagnostics.log_path.display(),
            diagnostics
                .history
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
            texture_event_history,
        );
        if let Err(error) = fs::write(&history_path, history) {
            eprintln!(
                "Streaming history capture {}: {error}",
                history_path.display()
            );
        } else {
            println!("Streaming history saved: {}", history_path.display());
        }
    }
    match fs::write(&report_path, report) {
        Ok(()) => {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(screenshot_path));
            println!("Collision report saved: {}", report_path.display());
        }
        Err(error) => eprintln!("Collision report write failed: {error}"),
    }
}
