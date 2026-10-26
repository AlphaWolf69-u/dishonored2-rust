//! Controls.
use super::player::{PlayerInput, PlayerState};
use super::{
    navigation, runtime, view_of, view_to_game, AccumulatedMouseMotion, BenchmarkSeconds,
    CoordinateOverlay, CursorGrabMode, CursorOptions, FocusTargets, NavigationWorld,
    PickupState, PickupVisual, SceneRenderQueue,
};
use crate::gameplay::player::controller;
use bevy::prelude::*;

/// Host benchmark motion only; ordinary player controls never use this state.
pub(super) struct BenchmarkCameraSweep {
    degrees: Option<f32>,
    base: Option<Quat>,
    elapsed: f32,
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
        Self {
            degrees,
            base: None,
            elapsed: 0.0,
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
const ACTION_INTERACT: i32 = 22;

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

pub(super) fn pickup_in_view(
    camera: &Transform,
    state: &PickupState,
    collision: Option<&navigation::CollisionWorld>,
) -> Option<usize> {
    let forward = camera.forward().as_vec3();
    state
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| state.world.pickup_available(&item.entity_id))
        .filter_map(|(index, item)| {
            let direction = item.position - camera.translation;
            let distance = direction.length();
            if distance > 2.5 || distance < 0.05 || forward.dot(direction / distance) < 0.8 {
                return None;
            }
            // Leave a short margin for the pickup's own physics surface.
            if collision
                .and_then(|world| world.first_hit(camera.translation, item.position))
                .is_some_and(|hit| hit < distance - 0.25)
            {
                return None;
            }
            Some((index, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

pub(super) fn interact_pickup(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    cameras: Query<&Transform, With<Camera3d>>,
    mut state: ResMut<PickupState>,
    mut scene: ResMut<SceneRenderQueue>,
    mut collision: Option<ResMut<NavigationWorld>>,
    visuals: Query<(Entity, &PickupVisual)>,
    mut windows: Query<&mut Window>,
) {
    let interact = {
        let console = crate::platform::console::shared().read();
        let bindings = &console.bindings.sets[crate::platform::bindings::IN_GAME];
        keys.get_just_pressed().any(|&key| {
            super::engine_keys::engine_key(key)
                .and_then(|code| bindings.get(code as usize))
                .is_some_and(|binding| binding.action == ACTION_INTERACT)
        })
    };
    if !interact {
        return;
    }
    let Ok(camera) = cameras.single() else {
        return;
    };
    let Some(index) = pickup_in_view(camera, &state, collision.as_ref().map(|world| &world.0))
    else {
        return;
    };
    let id = state.items[index].entity_id.clone();
    let Ok(runtime::WorldEvent::PickupCollected {
        item_path: path,
        quantity,
        inventory_total,
        ..
    }) = state.world.collect_pickup(&id)
    else {
        return;
    };
    if let Some(world) = collision.as_mut() {
        world.0.set_owner_active(&id, false);
    }
    for (entity, visual) in visuals.iter() {
        if visual.0 == id {
            commands.entity(entity).despawn();
        }
    }
    let mut removed = 0;
    for instance in &mut scene.instances {
        if instance.pickup_id.as_deref() == Some(id.as_str()) && instance.entity.take().is_some() {
            if instance.visible {
                removed += 1;
            }
            instance.visible = false;
        }
    }
    scene.active = scene.active.saturating_sub(removed);
    if let Ok(mut window) = windows.single_mut() {
        window.title = format!("Dishonored 2 Rust | picked up {quantity} Г— {path}");
    }
    println!("Picked up {id}: {quantity} Г— {path}; inventory total {inventory_total}");
}
