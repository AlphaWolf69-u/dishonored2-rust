//! Controls.
use super::player::{PlayerInput, PlayerState};
use super::{
    view_of, view_to_game, AccumulatedMouseMotion, BenchmarkSeconds,
    CoordinateOverlay, CursorGrabMode, CursorOptions, FocusTargets,
};
use crate::gameplay::player::controller;
use bevy::prelude::*;

/// Host benchmark motion only; ordinary player controls never use this state.
pub(super) struct BenchmarkCameraSweep {
    degrees: Option<f32>,
    base: Option<Quat>,
    elapsed: f32,
    /// DH2_BENCHMARK_CAMERA=px,py,pz,fx,fy,fz: game-space eye and forward
    /// (up +Z), applied once, to measure from a captured original view.
    placement: Option<([f32; 3], [f32; 3])>,
    /// DH2_BENCHMARK_PATH=px,py,pz,fx,fy,fz;...: game-space waypoints the
    /// camera travels at DH2_BENCHMARK_PATH_SPEED m/s (default the free
    /// camera's), there and back, so traversal work is measured.
    path: Vec<([f32; 3], [f32; 3])>,
    path_speed: f32,
}

impl Default for BenchmarkCameraSweep {
    fn default() -> Self {
        let degrees = std::env::var("DH2_BENCHMARK_SWEEP_DEGREES")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite() && *value > 0.0 && *value <= 45.0);
        if let Some(degrees) = degrees {
            println!("BENCHMARK camera_sweep_degrees={degrees}");
        }
        let placement = std::env::var("DH2_BENCHMARK_CAMERA").ok().and_then(|value| {
            let v: Vec<f32> = value.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (v.len() == 6).then(|| ([v[0], v[1], v[2]], [v[3], v[4], v[5]]))
        });
        let path: Vec<([f32; 3], [f32; 3])> = std::env::var("DH2_BENCHMARK_PATH").ok().map_or_else(Vec::new, |value| {
            value.split(';').filter_map(|point| {
                let v: Vec<f32> = point.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                (v.len() == 6).then(|| ([v[0], v[1], v[2]], [v[3], v[4], v[5]]))
            }).collect()
        });
        let path_speed = std::env::var("DH2_BENCHMARK_PATH_SPEED").ok().and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite() && *value > 0.0).unwrap_or(DEBUG_FREE_FLY_SPEED);
        if path.len() >= 2 {
            println!("BENCHMARK camera_path points={} speed={path_speed}", path.len());
        }
        Self {
            degrees,
            base: None,
            elapsed: 0.0,
            placement,
            path,
            path_speed,
        }
    }
}

/// Host free-camera feet probe, shared with water contact admission.
pub(super) const DEBUG_CAMERA_EYE_HEIGHT:f32=1.7;

/// Debug free-camera speeds (m/s); the player's own movement comes from its
/// state decls.
const DEBUG_FREE_FLY_SPEED: f32 = 6.0;
const DEBUG_FREE_FLY_FAST_SPEED: f32 = 30.0;
/// Debug free-camera mouse look, radians per count.
const DEBUG_FREE_FLY_LOOK: f32 = 0.0025;

/// Host camera handling: mouse capture, the benchmark sweep, and the debug
/// views (Home/End/PageUp/PageDown focus targets, G free camera). The free
/// camera moves with the bound move actions (MOVE_* and MOVE_UP/MOVE_DOWN,
/// SPRINT for speed); the player drives the camera otherwise.
pub(super) fn camera_controls(
    time: Res<Time>,
    benchmark: Option<Res<BenchmarkSeconds>>,
    mut sweep: Local<BenchmarkCameraSweep>,
    keys: Res<ButtonInput<KeyCode>>,
    mut focuses: ResMut<FocusTargets>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut cursor: Single<&mut CursorOptions>,
    mut player: ResMut<PlayerState>,
    input: Res<PlayerInput>,
) {
    if benchmark.is_some() {
        // DH2_DEBUG_BENCHMARK_AUDIO=1: the placed camera is the listener, as
        // the G free-fly camera is, so captures hear the measured view.
        if std::env::var("DH2_DEBUG_BENCHMARK_AUDIO").as_deref() == Ok("1") {
            player.debug_free_fly = true;
        }
        if let Some((eye, forward)) = sweep.placement.take() {
            if let Ok(mut camera) = cameras.single_mut() {
                let eye = super::game_to_view(eye);
                *camera = Transform::from_translation(eye)
                    .looking_to(super::game_to_view(forward), Vec3::Y);
                println!("BENCHMARK camera placed at {eye:?}");
            }
        }
        if sweep.path.len() >= 2 && !super::player::debug_benchmark_walk_enabled() {
            if let Ok(mut camera) = cameras.single_mut() {
                sweep.elapsed += time.delta_secs();
                let segments: Vec<f32> = sweep.path.windows(2)
                    .map(|pair| Vec3::from(pair[0].0).distance(Vec3::from(pair[1].0))).collect();
                let length: f32 = segments.iter().sum();
                if length > 0.0 {
                    // There and back: distance folds into [0, length].
                    let travelled = (sweep.elapsed * sweep.path_speed) % (2.0 * length);
                    let mut distance = if travelled > length { 2.0 * length - travelled } else { travelled };
                    let mut segment = 0;
                    while segment + 1 < segments.len() && distance > segments[segment] {
                        distance -= segments[segment];
                        segment += 1;
                    }
                    let fraction = if segments[segment] > 0.0 { (distance / segments[segment]).min(1.0) } else { 0.0 };
                    let (from, to) = (sweep.path[segment], sweep.path[segment + 1]);
                    let eye = Vec3::from(from.0).lerp(Vec3::from(to.0), fraction);
                    let forward = Vec3::from(from.1).normalize_or_zero().lerp(Vec3::from(to.1).normalize_or_zero(), fraction);
                    let forward = if forward.length_squared() > 1e-6 { forward } else { Vec3::from(to.1) };
                    *camera = Transform::from_translation(super::game_to_view(eye.to_array()))
                        .looking_to(super::game_to_view(forward.to_array()), Vec3::Y);
                }
            }
        }
        if let Some(degrees) = sweep.degrees {
            if let Ok(mut camera) = cameras.single_mut() {
                let base = *sweep.base.get_or_insert(camera.rotation);
                sweep.elapsed += time.delta_secs();
                let angle = degrees.to_radians() * (sweep.elapsed * 0.8).sin();
                camera.rotation = Quat::from_rotation_y(angle) * base;
            }
        }
        return;
    }
    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    if mouse.just_pressed(MouseButton::Left) {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
    if keys.just_pressed(KeyCode::Home) {
        focuses.active = 0;
    }
    if keys.just_pressed(KeyCode::End) {
        focuses.active = if focuses.targets[0].spawn.is_some() {
            1
        } else {
            0
        };
    }
    if keys.just_pressed(KeyCode::PageDown) {
        focuses.active = (focuses.active + 1) % focuses.targets.len();
    }
    if keys.just_pressed(KeyCode::PageUp) {
        focuses.active = (focuses.active + focuses.targets.len() - 1) % focuses.targets.len();
    }
    if keys.any_just_pressed([
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageDown,
        KeyCode::PageUp,
    ]) {
        let target = focuses.targets[focuses.active].clone();
        *camera = view_of(&target);
        match (target.spawn, player.player.as_mut()) {
            (Some(start), Some(walker)) => {
                walker.teleport(Vec3::from(start.position), start.forward_xy[1].atan2(start.forward_xy[0]));
                player.debug_free_fly = false;
            }
            _ => player.debug_free_fly = true,
        }
        println!("Viewing {} at {:?}", target.label, target.center);
    }
    if keys.just_pressed(KeyCode::KeyG) && player.player.is_some() {
        player.debug_free_fly = !player.debug_free_fly;
        if !player.debug_free_fly {
            // Place the feet so the rest-pose eye is at the free camera.
            let walker = player.player.as_mut().unwrap();
            let eye = walker.pose.positions[walker.pose.camera];
            let (yaw, _, _) = camera.rotation.to_euler(EulerRot::YXZ);
            let feet = Vec3::from(view_to_game(camera.translation)) - Vec3::Z * eye.z;
            // Bevy yaw 0 looks along -Z (game +Y); game yaw 0 looks along +X.
            walker.teleport(feet, yaw + std::f32::consts::FRAC_PI_2);
        }
        println!(
            "Camera mode: {}",
            if player.debug_free_fly { "debug free camera" } else { "player" }
        );
    }
    if !player.debug_free_fly {
        return;
    }
    let dt = time.delta_secs();
    let held = |action: i32| input.held.contains(&action);
    let speed = if held(controller::ACTION_SPRINT) { DEBUG_FREE_FLY_FAST_SPEED } else { DEBUG_FREE_FLY_SPEED };
    let forward = camera.forward().as_vec3();
    let right = camera.right().as_vec3();
    let mut movement = Vec3::ZERO;
    if held(controller::ACTION_MOVE_FORWARD) {
        movement += forward;
    }
    if held(controller::ACTION_MOVE_BACKWARD) {
        movement -= forward;
    }
    if held(controller::ACTION_MOVE_RIGHT) {
        movement += right;
    }
    if held(controller::ACTION_MOVE_LEFT) {
        movement -= right;
    }
    if held(ACTION_MOVE_UP) {
        movement += Vec3::Y;
    }
    if held(ACTION_MOVE_DOWN) {
        movement -= Vec3::Y;
    }
    camera.translation += movement.normalize_or_zero() * speed * dt;
    if cursor.grab_mode == CursorGrabMode::Locked && motion.delta != Vec2::ZERO {
        let look = motion.delta * DEBUG_FREE_FLY_LOOK;
        let (yaw, pitch, _) = camera.rotation.to_euler(EulerRot::YXZ);
        camera.rotation = Quat::from_euler(
            EulerRot::YXZ,
            yaw - look.x,
            (pitch - look.y).clamp(-1.54, 1.54),
            0.0,
        );
    }
}

/// eActions_t MOVE_UP / MOVE_DOWN / INTERACT (binding_tables.rs).
const ACTION_MOVE_UP: i32 = 17;
const ACTION_MOVE_DOWN: i32 = 18;

pub(super) fn update_coordinate_overlay(
    cameras: Query<&Transform, With<Camera3d>>,
    player: Res<PlayerState>,
    mut overlays: Query<&mut Text, With<CoordinateOverlay>>,
) {
    let (Ok(camera), Ok(mut overlay)) = (cameras.single(), overlays.single_mut()) else {
        return;
    };
    let [x, y, z] = view_to_game(camera.translation);
    **overlay = match player.player.as_ref().filter(|_| !player.debug_free_fly) {
        Some(walker) => {
            let feet = walker.feet();
            let states: Vec<i32> = walker.machine.active_states().iter().map(|s| s.0).collect();
            format!(
                "Game XYZ eye: {x:.2}, {y:.2}, {z:.2} | feet: {:.2}, {:.2}, {:.2} | {:?} states {states:?} | F8 report",
                feet.x, feet.y, feet.z, walker.physics.ground.state
            )
        }
        None => format!("Game XYZ camera: {x:.2}, {y:.2}, {z:.2} | debug free camera | F8 report"),
    };
}
