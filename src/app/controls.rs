//! Controls.
use super::{
    navigation, runtime, view_of, view_to_game, AccumulatedMouseMotion, BenchmarkSeconds,
    CoordinateOverlay, CursorGrabMode, CursorOptions, FocusTargets, NavigationState,
    NavigationWorld, PickupState, PickupVisual, SceneRenderQueue,
};
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
    mut navigation: ResMut<NavigationState>,
    collision: Option<Res<NavigationWorld>>,
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
        let target = &focuses.targets[focuses.active];
        *camera = view_of(target);
        navigation.walker = collision
            .as_ref()
            .map(|world| navigation::Walker::new(camera.translation - Vec3::Y * 1.7, &world.0))
            .unwrap_or(navigation::Walker {
                feet: camera.translation - Vec3::Y * 1.7,
                vertical_speed: 0.0,
                supported: false,
            });
        if target.spawn.is_none() {
            navigation.walking = false;
        }
        println!("Viewing {} at {:?}", target.label, target.center);
    }
    if keys.just_pressed(KeyCode::KeyG) && collision.is_some() {
        navigation.walking = !navigation.walking;
        navigation.walker = navigation::Walker::new(
            camera.translation - Vec3::Y * 1.7,
            &collision.as_ref().unwrap().0,
        );
        println!(
            "Navigation mode: {}",
            if navigation.walking {
                "grounded walk"
            } else {
                "free-fly"
            }
        );
    }
    let dt = time.delta_secs();
    let speed = if keys.pressed(KeyCode::ShiftLeft) {
        if navigation.walking {
            10.0
        } else {
            30.0
        }
    } else {
        6.0
    };
    let forward = camera.forward().as_vec3();
    let right = camera.right().as_vec3();
    let mut movement = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        movement += forward;
    }
    if keys.pressed(KeyCode::KeyS) {
        movement -= forward;
    }
    if keys.pressed(KeyCode::KeyD) {
        movement += right;
    }
    if keys.pressed(KeyCode::KeyA) {
        movement -= right;
    }
    if keys.pressed(KeyCode::KeyE) {
        movement += Vec3::Y;
    }
    if keys.pressed(KeyCode::KeyQ) {
        movement -= Vec3::Y;
    }
    if navigation.walking {
        if let Some(world) = collision.as_ref() {
            navigation.walker.step(
                &world.0,
                movement,
                speed,
                dt,
                keys.just_pressed(KeyCode::Space),
            );
            camera.translation = navigation.walker.feet + Vec3::Y * 1.7;
        }
    } else {
        camera.translation += movement.normalize_or_zero() * speed * dt;
    }
    let mut look = Vec2::ZERO;
    if cursor.grab_mode == CursorGrabMode::Locked {
        look += motion.delta * 0.0025;
    }
    if keys.pressed(KeyCode::ArrowLeft) {
        look.x -= 0.9 * dt;
    }
    if keys.pressed(KeyCode::ArrowRight) {
        look.x += 0.9 * dt;
    }
    if keys.pressed(KeyCode::ArrowUp) {
        look.y -= 0.9 * dt;
    }
    if keys.pressed(KeyCode::ArrowDown) {
        look.y += 0.9 * dt;
    }
    if look != Vec2::ZERO {
        let (yaw, pitch, _) = camera.rotation.to_euler(EulerRot::YXZ);
        camera.rotation = Quat::from_euler(
            EulerRot::YXZ,
            yaw - look.x,
            (pitch - look.y).clamp(-1.54, 1.54),
            0.0,
        );
    }
}

pub(super) fn update_coordinate_overlay(
    cameras: Query<&Transform, With<Camera3d>>,
    navigation: Res<NavigationState>,
    mut overlays: Query<&mut Text, With<CoordinateOverlay>>,
) {
    let (Ok(camera), Ok(mut overlay)) = (cameras.single(), overlays.single_mut()) else {
        return;
    };
    let [x, y, z] = view_to_game(camera.translation);
    let [fx, fy, fz] = view_to_game(navigation.walker.feet);
    **overlay = if navigation.walking {
        format!(
            "Game XYZ eye: {x:.2}, {y:.2}, {z:.2} | feet: {fx:.2}, {fy:.2}, {fz:.2} | walking | F8 report"
        )
    } else {
        format!("Game XYZ camera: {x:.2}, {y:.2}, {z:.2} | free-fly | F8 report")
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
    if !keys.just_pressed(KeyCode::KeyF) {
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
