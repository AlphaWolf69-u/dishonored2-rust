//! Performance.
use super::{
    frame_profile, local_light_kind_index, view_to_game, AppExit, BenchmarkSeconds,
    FrameRateOverlay, MeshTag, OriginalLocalLightCull, OriginalLocalLightCullStats,
    SceneRenderQueue, StreamingDiagnostics, VisibilityElapsedTiming,
    ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS,
};
use bevy::prelude::*;

// Run after Bevy's visibility pass to distinguish Tome decisions from Bevy
// culling and texture upload state. ViewVisibility includes any camera/light
// view; this count is not a GPU draw count.
pub(super) fn finish_streaming_diagnostics(
    mut diagnostics: ResMut<StreamingDiagnostics>,
    render_diagnostics: Option<Res<bevy::diagnostic::DiagnosticsStore>>,
    mut light_culling: ResMut<OriginalLocalLightCullStats>,
    scene: Res<SceneRenderQueue>,
    meshes: Query<&ViewVisibility, With<MeshTag>>,
    local_lights: Query<(&ViewVisibility, &OriginalLocalLightCull)>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let mut render_metrics = render_diagnostics
        .as_deref()
        .into_iter()
        .flat_map(|store| store.iter())
        .filter(|diagnostic| {
            let path = diagnostic.path().as_str();
            path.starts_with("render/") || matches!(path, "fps" | "frame_time")
        })
        .filter_map(|diagnostic| {
            diagnostic
                .value()
                .map(|value| format!("{}={value:.3}{}", diagnostic.path(), diagnostic.suffix))
        })
        .collect::<Vec<_>>();
    render_metrics.sort_unstable();
    let render_metrics = if render_metrics.is_empty() {
        String::new()
    } else {
        format!("\nrender_diagnostics {}", render_metrics.join(" "))
    };
    light_culling.visible_by_kind = [0; 4];
    for (visibility, light) in &local_lights {
        if visibility.get() {
            if let Some(kind_index) = local_light_kind_index(light.kind) {
                light_culling.visible_by_kind[kind_index] += 1;
            }
        }
    }
    let identity_sample = format!(
        "light_source_class[static,dynamic]={:?} static_name_hash_matches={} static_id_culled={} zero_preset_rgb={} rgb_eligibility=unsupported identity_binding={} projection_bounds_unverified={} world_query_bound={} world_query_unverified={} world_query_error={:?} projection_binding={} sphere_path=initial_single_tome sphere_candidates={} sphere_culled={} sphere_unverified={} sphere_query_ms={:.3} sphere_error={:?}\n",
        light_culling.source_class_counts,
        light_culling.static_name_hash_matches,
        light_culling.static_id_culled,
        light_culling.zero_preset_rgb,
        if light_culling.static_id_binding_active {
            "compiled_map_declaration"
        } else {
            "unavailable"
        },
        light_culling.projection_bounds_unverified,
        light_culling.world_query_bound,
        light_culling.world_query_unverified,
        light_culling.world_query_error,
        if light_culling.world_query_depth_grid_bound {
            "query_depth_grid"
        } else {
            "no_depth"
        },
        light_culling.sphere_candidates,
        light_culling.sphere_culled,
        light_culling.sphere_unverified,
        light_culling.sphere_query_ns as f64 / 1_000_000.0,
        light_culling.sphere_error,
    );
    let sample = format!(
        "{}{identity_sample}\nbevy_any_view_visible={} entities={} local_lights_culled[proximity,clip,area]={}/{}/{}/{} proximity_unverified={} light_total_by_kind[point,half_point,spot,area]={:?} light_visible_by_kind[point,half_point,spot,area]={:?} cutoff_px={:.0} diffuse_resident={}/{} diffuse_pending={} diffuse_failed={} mip_upgrades={}{}\n",
        diagnostics.pending,
        meshes.iter().filter(|visible| visible.get()).count(),
        meshes.iter().count(),
        light_culling.proximity_culled,
        light_culling.clip_culled,
        light_culling.area_culled,
        light_culling.total,
        light_culling.proximity_unverified,
        light_culling.total_by_kind,
        light_culling.visible_by_kind,
        ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS,
        scene
            .diffuse_image_handles
            .iter()
            .filter(|handle| handle.is_some())
            .count(),
        scene.diffuse_image_handles.len(),
        scene
            .diffuse_image_pending
            .iter()
            .filter(|pending| pending.is_some())
            .count(),
        scene
            .diffuse_image_attempted
            .iter()
            .enumerate()
            .filter(|&(slot, attempted)| *attempted
                && scene.diffuse_image_handles[slot].is_none()
                && scene.diffuse_image_pending[slot].is_none())
            .count(),
        scene.diffuse_mip_upgrades,
        render_metrics,
    );
    diagnostics.record(sample, scene.active, keys.just_pressed(KeyCode::F8));
}

pub(super) fn benchmark_frames(
    // Performance must include stalls; gameplay Time clamps deltas to 250 ms.
    time: Res<Time<bevy::time::Real>>,
    target: Option<Res<BenchmarkSeconds>>,
    models: Res<SceneRenderQueue>,
    camera: Query<&Transform, With<Camera3d>>,
    visibility_timing: Option<Res<VisibilityElapsedTiming>>,
    light_culling: Res<OriginalLocalLightCullStats>,
    mut elapsed: Local<f32>,
    mut measured: Local<(u64, f64)>,
    mut frame_times: Local<Vec<f64>>,
    render_diagnostics: Option<Res<bevy::diagnostic::DiagnosticsStore>>,
    mut render_metrics: Local<std::collections::BTreeMap<String, (u64, f64, f64)>>,
    cpu_profile: Option<Res<frame_profile::CpuFrameProfile>>,
    mut cpu_metrics: Local<(
        u64,
        [u128; frame_profile::COUNT],
        [u64; frame_profile::COUNT],
    )>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(target) = target else {
        return;
    };
    let previous_elapsed = *elapsed;
    *elapsed += time.delta_secs();
    if previous_elapsed >= target.1 {
        measured.0 += 1;
        measured.1 += time.delta_secs_f64();
        frame_times.push(time.delta_secs_f64() * 1000.0);
        if let Some(profile) = cpu_profile.as_ref() {
            let latest = profile.latest_ns();
            cpu_metrics.0 += 1;
            for (i, ns) in latest.into_iter().enumerate() {
                cpu_metrics.1[i] += ns as u128;
                cpu_metrics.2[i] = cpu_metrics.2[i].max(ns);
            }
        }
        if let Some(store) = render_diagnostics.as_deref() {
            for diagnostic in store.iter() {
                let path = diagnostic.path().as_str();
                if path.starts_with("render/")
                    && (path.ends_with("elapsed_gpu") || path.ends_with("shader_invocations"))
                {
                    if let Some(value) = diagnostic.value().filter(|value| value.is_finite()) {
                        let metric = render_metrics
                            .entry(path.to_owned())
                            .or_insert((0, 0.0, 0.0));
                        metric.0 += 1;
                        metric.1 += value;
                        metric.2 = metric.2.max(value);
                    }
                }
            }
        }
    }
    if *elapsed >= target.0 {
        for (i, label) in frame_profile::LABELS
            .iter()
            .enumerate()
            .filter(|_| cpu_metrics.0 > 0)
        {
            println!(
                "BENCHMARK_CPU phase={} samples={} avg_ms={:.3} max_ms={:.3}",
                label,
                cpu_metrics.0,
                cpu_metrics.1[i] as f64 / cpu_metrics.0 as f64 / 1_000_000.0,
                cpu_metrics.2[i] as f64 / 1_000_000.0
            );
        }
        let (samples, average_ms, max_ms) = visibility_timing
            .as_ref()
            .map(|timing| {
                (
                    timing.samples,
                    timing.total_ns as f64 / timing.samples.max(1) as f64 / 1_000_000.0,
                    timing.max_ns as f64 / 1_000_000.0,
                )
            })
            .unwrap_or_default();
        println!(
            "BENCHMARK frames={} measured_seconds={:.2} fps={:.1} visible={}/{} local_lights_culled[proximity,clip,area]={}/{}/{}/{} proximity_unverified={} light_total_by_kind[point,half_point,spot,area]={:?} light_visible_by_kind[point,half_point,spot,area]={:?} cutoff_px={:.0} diffuse_resident={}/{} mip_upgrades={} camera_game_xyz={:?} visibility_elapsed_samples={} visibility_elapsed_avg_ms={:.3} visibility_elapsed_max_ms={:.3} query_avg_ms={:.3} mapping_avg_ms={:.3} update_avg_ms={:.3} static_id_culled={} identity_binding_active={} sphere_path=initial_single_tome sphere_candidates={} sphere_culled={} sphere_unverified={} sphere_query_ms={:.3} rgb_eligibility=unsupported zero_preset_rgb={}",
            measured.0,
            measured.1,
            measured.0 as f64 / measured.1.max(1e-6),
            models.active,
            models.instances.len(),
            light_culling.proximity_culled,
            light_culling.clip_culled,
            light_culling.area_culled,
            light_culling.total,
            light_culling.proximity_unverified,
            light_culling.total_by_kind,
            light_culling.visible_by_kind,
            ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS,
            models
                .diffuse_image_handles
                .iter()
                .filter(|handle| handle.is_some())
                .count(),
            models.diffuse_image_handles.len(),
            models.diffuse_mip_upgrades,
            camera
                .single()
                .ok()
                .map(|transform| view_to_game(transform.translation)),
            samples,
            average_ms,
            max_ms,
            visibility_timing
                .as_ref()
                .map_or(0.0, |timing| timing.query_ns as f64
                    / timing.samples.max(1) as f64
                    / 1_000_000.0),
            visibility_timing
                .as_ref()
                .map_or(0.0, |timing| timing.mapping_ns as f64
                    / timing.samples.max(1) as f64
                    / 1_000_000.0),
            visibility_timing
                .as_ref()
                .map_or(0.0, |timing| timing.update_ns as f64
                    / timing.samples.max(1) as f64
                    / 1_000_000.0),
            light_culling.static_id_culled,
            light_culling.static_id_binding_active,
            light_culling.sphere_candidates,
            light_culling.sphere_culled,
            light_culling.sphere_unverified,
            light_culling.sphere_query_ns as f64 / 1_000_000.0,
            light_culling.zero_preset_rgb,
        );
        frame_times.sort_unstable_by(f64::total_cmp);
        if !frame_times.is_empty() {
            let quantile =
                |q: f64| frame_times[((frame_times.len() - 1) as f64 * q).round() as usize];
            println!(
                "BENCHMARK frame_ms[p50,p95,p99,max]={:.3},{:.3},{:.3},{:.3} warmup_seconds={:.1}",
                quantile(0.5),
                quantile(0.95),
                quantile(0.99),
                quantile(1.0),
                target.1
            );
        }
        for (path, (samples, total, maximum)) in render_metrics.iter() {
            println!(
                "BENCHMARK_GPU {path} samples={samples} avg={:.3} max={maximum:.3}",
                total / *samples as f64
            );
        }
        exit.write(AppExit::Success);
    }
}

pub(super) fn show_frame_rate(
    time: Res<Time<bevy::time::Real>>,
    scene: Res<SceneRenderQueue>,
    mut window: Single<&mut Window>,
    mut overlays: Query<&mut Text, With<FrameRateOverlay>>,
    mut elapsed: Local<f32>,
    mut frames: Local<u32>,
) {
    *elapsed += time.delta_secs();
    *frames += 1;
    if *elapsed >= 1.0 {
        let fps = *frames as f32 / *elapsed;
        let frame_ms = *elapsed * 1000.0 / *frames as f32;
        window.title = format!(
            "Dishonored 2 Rust | {:.0} FPS | {} loaded",
            fps, scene.active
        );
        for mut overlay in &mut overlays {
            **overlay = format!("{fps:.0} FPS | {frame_ms:.1} ms");
        }
        *elapsed = 0.0;
        *frames = 0;
    }
}
