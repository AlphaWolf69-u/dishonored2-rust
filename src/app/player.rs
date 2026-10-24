//! Host integration of the first-person player (`gameplay::player`): loads it
//! from the installed declarations, turns Bevy keyboard/mouse input into the
//! engine's bound action events (the IN_GAME binding set), answers its world
//! queries from the level collision, updates it on simulated game-clock
//! frames and places the Bevy camera at its view.
use super::{game_to_view, view_to_game, BenchmarkSeconds, CursorGrabMode, CursorOptions, NavigationWorld, Projection};
use crate::archive::Catalog;
use crate::gameplay::game_clock_runtime::{ClockRuntime, GamePaused};
use crate::gameplay::navigation::CollisionWorld;
use crate::gameplay::options::DifficultyManager;
use crate::gameplay::pause_state::Character;
use crate::gameplay::player::body::{Body, BodySkeleton};
use crate::gameplay::player::camera::CameraDecl;
use crate::gameplay::player::character_proxy::{PlayerWorld, RayHit, WorldHit};
use crate::gameplay::player::controller::{ActionEvent, LookSettings, MOUSE_MOVE_KEY};
use crate::gameplay::player::midnight::{Midnight, PreparedScene};
use crate::gameplay::player::states::{Kind, Scenes};
use crate::gameplay::player::{decls, Player, PlayerSetup};
use crate::gameplay::shape_cast;
use crate::platform::{bindings, console};
use crate::rendering::bevy_backend::camera_projection::GameProjection;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{AccumulatedMouseMotion, MouseButtonInput};
use bevy::input::ButtonState;
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

fn to_view(v: Vec3) -> Vec3 {
    game_to_view(v.to_array())
}

fn to_game(v: Vec3) -> Vec3 {
    Vec3::from(view_to_game(v))
}

/// The level collision in game coordinates (Z up). Level triangles are one
/// static body; placed colliders are bodies of their owner.
pub(super) struct GameCollision<'a>(pub &'a CollisionWorld);

impl PlayerWorld for GameCollision<'_> {
    // The triangle's arkPhysicsMaterial; colliders without a resolved
    // material (placed models, entities) keep the material ctor's flags 3
    // (FUN_14008b640: mantle and vault allowed).
    fn surface(&self, hit: &WorldHit) -> crate::gameplay::player::character_proxy::SurfaceFlags {
        let material = self.0.physics_material(hit.key);
        crate::gameplay::player::character_proxy::SurfaceFlags {
            allow_mantle: material.is_none_or(|m| m.allow_mantle()),
            allow_vault: material.is_none_or(|m| m.allow_vault()),
        }
    }

    fn cast_shape(
        &self,
        shape: &[Vec3],
        rotation: Mat3,
        from: Vec3,
        displacement: Vec3,
        tolerance: f32,
        cast: &mut Vec<WorldHit>,
        start: Option<&mut Vec<WorldHit>>,
    ) {
        let points: Vec<Vec3> = shape.iter().map(|p| to_view(from + rotation * *p)).collect();
        let motion = to_view(displacement);
        let (mut min, mut max) = (points[0], points[0]);
        for &p in &points {
            min = min.min(p).min(p + motion);
            max = max.max(p).max(p + motion);
        }
        let margin = Vec3::splat(tolerance.max(0.0) + 0.01);
        let mut start = start;
        for (key, triangle, owner) in self.0.triangles_in(min - margin, max + margin) {
            let body = if owner == u32::MAX { 0 } else { owner + 1 };
            if let Some((fraction, contact)) = shape_cast::cast(&points, motion, &triangle, 0.001) {
                cast.push(WorldHit { position: to_game(contact.position), normal: to_game(contact.normal), value: fraction, body, key });
            }
            if let Some(start) = start.as_deref_mut().filter(|_| tolerance > 0.0) {
                let contact = shape_cast::closest(&points, &triangle);
                if contact.distance <= tolerance {
                    start.push(WorldHit { position: to_game(contact.position), normal: to_game(contact.normal), value: contact.distance, body, key });
                }
            }
        }
    }

    fn cast_ray(&self, from: Vec3, direction: Vec3, length: f32) -> Option<RayHit> {
        let start = to_view(from);
        let end = to_view(from + direction * length);
        let (distance, key, owner, [a, b, c]) = self.0.first_hit_details(start, end)?;
        let mut normal = to_game((b - a).cross(c - a).normalize_or_zero());
        if normal.dot(direction) > 0.0 {
            normal = -normal;
        }
        let body = if owner == u32::MAX { 0 } else { owner + 1 };
        Some(RayHit { distance, position: from + direction * distance, normal, body, key })
    }
}

fn character_name(character: Character) -> &'static str {
    match character {
        Character::Corvo => "corvo",
        Character::Emily => "emily",
    }
}

/// The body skeleton, its animsets' aliases and the stance clips
/// (`gameplay::player::body`).
fn load_body(
    catalog: &Catalog,
    body_model: &str,
    camera: &CameraDecl,
    locomotion: &[(String, crate::entities::Value)],
    body_decl: &[(String, crate::entities::Value)],
    additives: &[(String, crate::entities::Value)],
    scene_paths: &[String],
) -> Result<(Body, Scenes), String> {
    let stem = body_model.strip_suffix(".md6").ok_or("player body model is not an md6")?;
    let text = |path: &str| -> Result<Option<String>, String> {
        Ok(catalog.read(path).map_err(|e| e.to_string())?.map(|bytes| crate::entities::declaration_text(&bytes).into_owned()))
    };
    let binding = crate::bmd6::resolve_skeleton_binding(&format!("generated/basemodel/{stem}.bmd6model"), |path| text(path).ok().flatten())?
        .ok_or("player body has no skeleton")?;
    let skeleton_stem = binding.name.strip_suffix(".md6skl").ok_or("unsupported skeleton resource suffix")?;
    let path = format!("generated/skeleton/{skeleton_stem}.edgeskl");
    let bytes = catalog.read(&path).map_err(|e| e.to_string())?.ok_or_else(|| format!("{path} is absent"))?;
    let skeleton = crate::edge_skeleton::parse(&bytes)?;
    let joint = |name: &str| {
        skeleton.joint_names.iter().position(|n| n == name).ok_or_else(|| format!("{path} has no joint {name}"))
    };
    let body_skeleton = BodySkeleton {
        rest: skeleton.rest.clone(),
        hierarchy: skeleton.hierarchy.clone(),
        parents: skeleton.parents.clone(),
        root: crate::edge_skeleton::JointRestPose {
            rotation: [0.0, 0.0, 0.0, 1.0],
            translation: binding.offset.unwrap_or([0.0; 3]),
            scale: [1.0; 3],
        },
        camera: joint(&camera.camera_joint)?,
        controls: [joint(&camera.head_joint)?, joint(&camera.neck_joint)?, joint(&camera.arms_joint)?],
        anchor: skeleton.joint_names.iter().position(|n| n == "anchor_jnt"),
    };
    // The md6 `animsets` block, in order.
    let md6_path = format!("generated/decls/md6def/{stem}.md6.decl");
    let md6 = text(&md6_path)?.ok_or_else(|| format!("{md6_path} is absent"))?;
    let tokens = crate::entities::declaration_lexemes(&md6)?;
    let mut aliases = HashMap::new();
    if let Some(start) = tokens.iter().position(|t| *t == "animsets") {
        for token in tokens[start + 2..].iter().take_while(|t| **t != "}") {
            let name = token.trim_matches('"');
            let path = format!("generated/decls/animset/{name}.decl");
            let source = text(&path)?.ok_or_else(|| format!("{path} is absent"))?;
            crate::edge_animation::animset_aliases(&source, &mut aliases)?;
        }
    }
    let mut animations = Vec::new();
    let mut loaded: HashMap<String, usize> = HashMap::new();
    let mut clip = |alias: &str| -> Result<usize, String> {
        if let Some(&index) = loaded.get(alias) {
            return Ok(index);
        }
        let anim = aliases.get(alias).ok_or_else(|| format!("no animset alias {alias}"))?;
        let resource = crate::edge_animation::resource_path(anim).ok_or_else(|| format!("{anim} is not an md6anim"))?;
        let bytes = catalog.read(&resource).map_err(|e| e.to_string())?.ok_or_else(|| format!("{resource} is absent"))?;
        let animation = crate::edge_animation::EdgeAnimation::parse(&bytes).map_err(|e| format!("{resource}: {e}"))?;
        animation.sample(&body_skeleton.rest, 0.0).map_err(|e| format!("{resource}: {e}"))?;
        animations.push(animation);
        loaded.insert(alias.to_owned(), animations.len() - 1);
        Ok(animations.len() - 1)
    };
    use crate::gameplay::player::animation::{LocoData, SpeedSet, Transition};
    let mut body_data = LocoData::default();
    let mut bob_data = LocoData::default();
    for (stance, speeds) in decls::loco_stances(locomotion)? {
        let (mut body_sets, mut bob_sets): ([Option<SpeedSet>; 4], [Option<SpeedSet>; 4]) = Default::default();
        for entry in speeds {
            if entry.speed >= 4 {
                continue;
            }
            let set = |names: &[String; 4], clip: &mut dyn FnMut(&str) -> Result<usize, String>| -> Result<SpeedSet, String> {
                Ok(SpeedSet { clips: [clip(&names[0])?, clip(&names[1])?, clip(&names[2])?, clip(&names[3])?], speeds: entry.speeds })
            };
            if let Some(names) = &entry.body {
                body_sets[entry.speed] = Some(set(names, &mut clip)?);
            }
            if let Some(names) = &entry.bob {
                bob_sets[entry.speed] = Some(set(names, &mut clip)?);
            }
        }
        body_data.stances.insert(stance.clone(), body_sets);
        bob_data.stances.insert(stance, bob_sets);
    }
    for (from, to, alias, blend_in, blend_out) in decls::loco_transitions(locomotion)? {
        if !alias.is_empty() {
            body_data.transitions.insert((from, to), Transition { clip: clip(&alias)?, blend_in, blend_out });
        }
    }
    let mut lean_clips = [None; 2];
    for (slot, alias) in lean_clips.iter_mut().zip(decls::lean_aliases(body_decl)) {
        if let Some(alias) = alias {
            *slot = Some(clip(&alias)?);
        }
    }
    use crate::gameplay::player::animation::{AnimBasicItem, AnimBasicNode};
    let (list, stop_blend_out) = decls::anim_basic_list(additives)?;
    let mut node = AnimBasicNode::default();
    node.stop_blend_out = stop_blend_out;
    for (alias, blend_in, blend_out) in list {
        node.items.push(AnimBasicItem { clip: clip(&alias)?, alias, blend_in, blend_out });
    }
    let jump_items = decls::camera_jump_aliases(body_decl).map(|alias| alias.and_then(|a| node.index_of(&a)));
    // The states' midnight scenes: the player group's clips of every scene
    // the graph reaches for this body md6.
    let mut scenes = Scenes::new();
    for path in scene_paths {
        let resource = format!("generated/decls/midnightscene/{path}.decl");
        let source = text(&resource)?.ok_or_else(|| format!("{resource} is absent"))?;
        let decl = crate::entities::midnight::parse(&source).map_err(|e| format!("{resource}: {e}"))?;
        let mut prepared = PreparedScene { clips: vec![Vec::new(); decl.scenes.len()], decl };
        let mut next = Some(prepared.decl.first);
        for _ in 0..16 {
            let Some(element) = next.take() else { break };
            let Some(index) = Midnight::resolve(&prepared, element, body_model).map_err(|e| format!("{resource}: {e}"))? else { break };
            if let Some(g) = prepared.player_group(index).map_err(|e| format!("{resource}: {e}"))? {
                let keys = prepared.decl.scenes[index].groups[g].anim_keys.clone();
                prepared.clips[index] = keys.iter().map(|k| clip(&k.anim)).collect::<Result<_, _>>()?;
            }
            next = Some(prepared.decl.scenes[index].next).filter(|n| *n != crate::entities::midnight::Next::Final);
        }
        scenes.insert(path.clone(), std::sync::Arc::new(prepared));
    }
    let body_node = crate::gameplay::player::animation::LocoNode::new(body_data);
    let bob_node = crate::gameplay::player::animation::LocoNode::new(bob_data);
    let mut body = Body::new(body_skeleton, animations, Default::default(), body_node, bob_node);
    body.lean_clips = lean_clips;
    body.additives = node;
    body.jump_items = jump_items;
    Ok((body, scenes))
}

pub(super) fn load_player_setup(catalog: &Catalog, character: Character) -> Result<PlayerSetup, String> {
    let declarations = crate::entities::player::player_declarations(character_name(character), |path| {
        Ok(catalog.read(path).map_err(|e| e.to_string())?.map(|bytes| crate::entities::declaration_text(&bytes).into_owned()))
    })?;
    let camera = decls::camera(&declarations.camera)?;
    let states = decls::states(&declarations)?;
    let mut scene_paths: Vec<String> = Vec::new();
    for state in &states {
        let paths: Vec<&String> = match &state.kind {
            Kind::CatFall(d) => d.low_damage_midnight.iter().chain(d.high_damage_midnight.iter()).collect(),
            Kind::Mantle(d) => d.low_midnights.iter().chain(&d.mid_midnights).chain(&d.high_midnights).flatten().collect(),
            Kind::Vault(d) => d.small_midnights.iter().chain(&d.high_midnights).collect(),
            _ => Vec::new(),
        };
        for path in paths {
            if !scene_paths.contains(path) {
                scene_paths.push(path.clone());
            }
        }
    }
    let (body, scenes) =
        load_body(catalog, &declarations.body_model, &camera, &declarations.locomotion, &declarations.body, &declarations.additives, &scene_paths)?;
    let attributes = decls::attributes(&declarations.attributes)?;
    let health = decls::health(&declarations.health)? + attributes.get("ARK_ATTRIBUTE_HEALTH_MAX");
    Ok(PlayerSetup {
        physics: decls::physics(&declarations.physics)?,
        camera,
        mapping: decls::states_mapping(&declarations.controller)?,
        states,
        rules: decls::rules(&declarations.mode)?,
        default_state: decls::default_state(&declarations.mode),
        attributes,
        body,
        autocrouch: decls::autocrouch_sensor(&declarations.autocrouch_sensor)?,
        lean: decls::lean_sensor(&declarations.lean_sensor)?,
        mantle: decls::mantle_sensor(&declarations.mantle_sensor)?,
        slipping: decls::slipping_thresholds(&declarations.controller)?,
        scenes,
        md6: declarations.body_model.clone(),
        health,
    })
}

/// The player, or why it is unavailable (no collision, no player start or
/// declarations that failed to load).
#[derive(Resource)]
pub(super) struct PlayerState {
    pub(super) player: Option<Player>,
    /// Host diagnostic free camera (G); the player does not simulate.
    pub(super) debug_free_fly: bool,
    /// The difficulty manager the body's lean limits read.
    pub(super) difficulty: Option<DifficultyManager>,
}

/// The installed difficulty manager as the game creates it (FUN_140648590):
/// the current preset from `ark_difficulty_current` and the profile's
/// custom values.
pub(super) fn load_difficulty(catalog: &Catalog) -> Option<DifficultyManager> {
    let mut manager = DifficultyManager::read(&mut |path| catalog.read(path).ok().flatten())
        .map_err(|error| eprintln!("Difficulty manager unavailable, lean unlimited: {error}"))
        .ok()?;
    manager.current = console::shared().read().cvars.int("ark_difficulty_current").unwrap_or(0);
    let profile = crate::gameplay::save_list::Saves::locate()
        .and_then(|s| crate::gameplay::save_list::read_profile(&s.folder))
        .unwrap_or_default();
    manager.read_custom(&profile);
    Some(manager)
}

/// Bound action events since the last simulated frame, the mouse sample
/// ring (FUN_1400d23a0) and the held actions for the debug free camera.
#[derive(Resource, Default)]
pub(super) struct PlayerInput {
    events: Vec<ActionEvent>,
    ring: [[f32; 2]; 16],
    index: usize,
    mouse_moving: bool,
    cleared: bool,
    pub(super) held: HashSet<i32>,
}

impl PlayerInput {
    fn push(&mut self, event: ActionEvent) {
        if event.flags & 2 == 0 {
            if event.flags & 1 != 0 {
                self.held.insert(event.action);
            } else {
                self.held.remove(&event.action);
            }
        }
        self.events.push(event);
    }
}

pub(super) fn look_settings() -> LookSettings {
    let console = console::shared().read();
    let cvars = &console.cvars;
    let d = LookSettings::default();
    LookSettings {
        mouse_clamp: cvars.float("g_mouseClamp").unwrap_or(d.mouse_clamp),
        mouse_sensitivity: cvars.float("g_mouseSensitivity").unwrap_or(d.mouse_sensitivity),
        invert_mouse_y: cvars.int("g_invertMouseYAxis").is_some_and(|v| v != 0),
        invert_look_y: cvars.int("g_invertLookYAxis").is_some_and(|v| v != 0),
        fix_mouse_sensibility: cvars.int("g_fixMouseSensibility").is_none_or(|v| v != 0),
        look_stick_sensitivity: [
            cvars.float("g_lookStickSensitivityX").unwrap_or(1.0),
            cvars.float("g_lookStickSensitivityY").unwrap_or(1.0),
        ],
        sprint_toggle: cvars.int("g_sprintToggle").is_none_or(|v| v != 0),
        lean_toggle: cvars.int("g_leanToggle").is_none_or(|v| v != 0),
    }
}

/// `ark_playerSmoothedLoco` and the head-bob option `ark_playerBobAmount`.
pub(super) fn body_settings(difficulty: Option<&DifficultyManager>) -> crate::gameplay::player::body::BodySettings {
    let console = console::shared().read();
    let current = console.cvars.int("ark_difficulty_current").unwrap_or(0);
    crate::gameplay::player::body::BodySettings {
        smoothed_loco: console.cvars.int("ark_playerSmoothedLoco").is_none_or(|v| v != 0),
        bob_amount: console.cvars.float("ark_playerBobAmount").unwrap_or(1.0),
        small_fall_cooldown: console.cvars.float("ark_playerSmallFallCooldown").unwrap_or(0.5),
        lean_limits: difficulty.map_or([1.0; 2], |manager| manager.lean_limits(current)),
    }
}

fn base_fov() -> f32 {
    console::shared().read().cvars.float("ark_fieldOfView").unwrap_or(80.0)
}

/// DirectInput buttons 0..6 are engine codes 0xe0..0xe6.
fn mouse_button_key(button: MouseButton) -> Option<u32> {
    Some(match button {
        MouseButton::Left => 0xe0,
        MouseButton::Right => 0xe1,
        MouseButton::Middle => 0xe2,
        MouseButton::Back => 0xe3,
        MouseButton::Forward => 0xe4,
        MouseButton::Other(n) if n < 7 => 0xe0 + n as u32,
        _ => return None,
    })
}

/// FUN_14003ab70: a bound key becomes `{action, key, v0, v1, flags}`;
/// command bindings (action 0) are not player input.
fn bound(key: u32) -> i32 {
    console::shared().read().bindings.sets[bindings::IN_GAME]
        .get(key as usize)
        .map_or(0, |binding| binding.action)
}

pub(super) fn sample_player_input(
    mut keys: MessageReader<KeyboardInput>,
    mut buttons: MessageReader<MouseButtonInput>,
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions>,
    paused: Res<GamePaused>,
    mut input: ResMut<PlayerInput>,
) {
    if paused.0 {
        keys.clear();
        buttons.clear();
        return;
    }
    for event in keys.read() {
        if event.repeat {
            continue;
        }
        let Some(key) = super::engine_keys::engine_key(event.key_code) else { continue };
        let action = bound(key);
        if action != 0 {
            input.push(ActionEvent { action, key, value: [0.0; 2], flags: (event.state == ButtonState::Pressed) as u32 });
        }
    }
    for event in buttons.read() {
        let Some(key) = mouse_button_key(event.button) else { continue };
        let action = bound(key);
        if action != 0 {
            input.push(ActionEvent { action, key, value: [0.0; 2], flags: (event.state == ButtonState::Pressed) as u32 });
        }
    }
    // The mouse only drives look while the window owns it.
    let delta = if cursor.grab_mode == CursorGrabMode::Locked { motion.delta } else { Vec2::ZERO };
    let index = input.index;
    input.ring[index] = [delta.x, delta.y];
    input.index = (index + 1) & 15;
    let samples = console::shared().read().cvars.float("g_mouseSmooth").unwrap_or(1.0) as usize;
    let samples = samples.clamp(1, 15);
    let mut average = [0.0f32; 2];
    for k in 0..samples {
        let sample = input.ring[(index + 16 - k) & 15];
        average[0] += sample[0];
        average[1] += sample[1];
    }
    average = average.map(|v| v / samples as f32);
    let action = bound(MOUSE_MOVE_KEY);
    if average[0] * average[0] + average[1] * average[1] > f32::EPSILON {
        input.mouse_moving = true;
        if action != 0 {
            input.push(ActionEvent { action, key: MOUSE_MOVE_KEY, value: average, flags: 3 });
        }
    } else if input.mouse_moving {
        input.mouse_moving = false;
        if action != 0 {
            input.push(ActionEvent { action, key: MOUSE_MOVE_KEY, value: [0.0; 2], flags: 2 });
        }
    }
}

/// The simulated frame (input dispatch happens only on simulated frames;
/// unsimulated frames keep accumulating events), then the view.
pub(super) fn update_player(
    clock: Res<ClockRuntime>,
    paused: Res<GamePaused>,
    benchmark: Option<Res<BenchmarkSeconds>>,
    collision: Option<Res<NavigationWorld>>,
    mut state: ResMut<PlayerState>,
    mut input: ResMut<PlayerInput>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
) {
    if benchmark.is_some() || state.debug_free_fly {
        input.events.clear();
        return;
    }
    let PlayerState { player, difficulty, .. } = &mut *state;
    let (Some(player), Some(collision)) = (player.as_mut(), collision) else { return };
    if paused.0 {
        // In a non-game logic state the input components are cleared.
        input.events.clear();
        if !input.cleared {
            player.controller.clear();
            input.held.clear();
            input.cleared = true;
        }
        return;
    }
    input.cleared = false;
    let world = GameCollision(&collision.0);
    let base = base_fov();
    if let Ok(game_clock) = clock.clock() {
        if clock.advanced() {
            let dt = game_clock.delta(3).unwrap_or(0.0);
            let frame_delta = game_clock.delta(0).unwrap_or(0.0);
            let events = std::mem::take(&mut input.events);
            player.body_settings = body_settings(difficulty.as_ref());

            player.frame(&world, &events, &look_settings(), dt, frame_delta, base);
        }
    }
    let view = player.view();
    let Ok((mut transform, mut projection)) = cameras.single_mut() else { return };
    let origin = to_view(view.origin);
    *transform = Transform::from_translation(origin).looking_to(to_view(view.axis.x_axis), to_view(view.axis.z_axis));
    if let Projection::Custom(custom) = &*projection {
        if let Some(game) = custom.get::<GameProjection>() {
            if (game.reference_horizontal_degrees - player.camera.fov).abs() > 1e-4 {
                let (near, far) = (game.near, game.far);
                *projection = Projection::custom(GameProjection::new(player.camera.fov, near, far));
            }
        }
    }
}

/// `DH2_NAV_PROBE` / `DH2_NAV_POINT`: drive the player headlessly at the
/// game's 1/120 s step and print its motion (settle; with `full`, also run,
/// sneak, sprint and jump on bound actions).
pub(super) fn probe_player(catalog: &Catalog, world: &CollisionWorld, feet: Vec3, yaw: f32, full: bool) -> Result<(), String> {
    use crate::gameplay::player::controller::{ACTION_CROUCH, ACTION_JUMP, ACTION_MOVE_FORWARD, ACTION_SPRINT};
    let setup = load_player_setup(catalog, super::pause_menu::debug_character())?;
    let mut player = Player::new(setup, feet, yaw, base_fov())?;
    let world = GameCollision(world);
    let settings = look_settings();
    let dt = 1.0 / 120.0;
    let key = |action: i32, down: bool| ActionEvent { action, key: 0, value: [0.0; 2], flags: down as u32 };
    let run = |player: &mut Player, frames: usize, events: Vec<ActionEvent>| {
        let mut peak = player.feet().z;
        for frame in 0..frames {
            let events = if frame == 0 { events.clone() } else { Vec::new() };
            player.frame(&world, &events, &settings, dt, dt, 80.0);
            peak = peak.max(player.feet().z);
        }
        peak
    };
    let report = |label: &str, player: &Player, from: Vec3, seconds: f32| {
        let feet = player.feet();
        let horizontal = (feet - from).truncate().length();
        println!(
            "NAV PROBE {label}: feet={feet:?} moved={:.3}m ({:.3} m/s) velocity={:?} support={:?} states={:?} eye={:?}",
            horizontal,
            horizontal / seconds,
            player.physics.velocity,
            player.physics.ground.state,
            player.machine.active_states(),
            player.camera.pitch,
        );
    };
    run(&mut player, 360, Vec::new());
    let settled = player.feet();
    report("settle 3s", &player, feet, 3.0);
    println!("NAV PROBE spawn drop {:.3}m, eye {:?}", feet.z - settled.z, player.view().origin);
    if !full {
        // DH2_NAV_YAW (degrees): hold forward and trace the proxy.
        if let Some(degrees) = std::env::var("DH2_NAV_YAW").ok().and_then(|v| v.parse::<f32>().ok()) {
            player.teleport(player.feet(), degrees.to_radians());
            run(&mut player, 1, vec![key(ACTION_MOVE_FORWARD, true)]);
            for step in 0..12 {
                run(&mut player, 10, Vec::new());
                println!(
                    "NAV PROBE push +{}: feet={:?} velocity={:?} support={:?} states={:?} penetrating={} ground={:.3}",
                    (step + 1) * 10,
                    player.feet(),
                    player.physics.velocity,
                    player.physics.ground.state,
                    player.machine.active_states(),
                    player.physics.penetrating,
                    player.physics.ground_distance,
                );
                for (position, normal, distance, key) in player.physics.proxy.contacts() {
                    println!("    contact tri {key} at {position:?} n={normal:?} d={distance:.4}");
                }
            }
            let feet = player.feet();
            for (key, triangle, owner) in world.0.triangles_in(to_view(feet) - Vec3::splat(0.7), to_view(feet) + Vec3::splat(0.7)) {
                println!("    nearby tri {key} owner {owner}: {:?}", triangle.map(to_game));
            }
        }
        return Ok(());
    }
    // Lean right (toggle mode by default): hold, release, then walk away.
    {
        use crate::gameplay::player::controller::ACTION_LEAN_RIGHT;
        let lean_step = |player: &mut Player, label: &str, frames: usize, events: Vec<ActionEvent>| {
            for frame in 0..frames {
                let events = if frame == 0 { events.clone() } else { Vec::new() };
                player.frame(&world, &events, &settings, dt, dt, 80.0);
                player.view();
            }
            let axis = player.axis();
            let eye = player.view().origin - player.feet();
            let lean = player.body.lean;
            println!(
                "NAV PROBE lean {label}: states={:?} left={:.3} up={:.3} blend={:.3} limits=({:.3},{:.3}) offset={:?} eye fwd={:.4} right={:.4} up={:.4}",
                player.machine.active_states(),
                lean.left,
                lean.up,
                lean.blend,
                player.lean_sensor.left_limit,
                player.lean_sensor.right_limit,
                player.physics.lean_offset,
                eye.dot(axis.x_axis),
                -eye.dot(axis.y_axis),
                eye.dot(axis.z_axis),
            );
        };
        lean_step(&mut player, "neutral", 1, Vec::new());
        lean_step(&mut player, "E held 0.1s", 12, vec![key(ACTION_LEAN_RIGHT, true)]);
        lean_step(&mut player, "E held 0.3s", 24, Vec::new());
        lean_step(&mut player, "E released +0.25s", 30, vec![key(ACTION_LEAN_RIGHT, false)]);
        // Turn 70° while holding E, release, turn back, then press E again.
        lean_step(&mut player, "E held again", 1, vec![key(ACTION_LEAN_RIGHT, true)]);
        for _ in 0..30 {
            player.physics.add_yaw(70f32.to_radians() / 30.0);
            player.frame(&world, &[], &settings, dt, dt, 80.0);
            player.view();
        }
        lean_step(&mut player, "turning 70°", 0, Vec::new());
        lean_step(&mut player, "E released after turn", 1, vec![key(ACTION_LEAN_RIGHT, false)]);
        for _ in 0..30 {
            player.physics.add_yaw(-70f32.to_radians() / 30.0);
            player.frame(&world, &[], &settings, dt, dt, 80.0);
            player.view();
        }
        lean_step(&mut player, "turning back 70°", 0, Vec::new());
        lean_step(&mut player, "E pressed after turn +0.3s", 36, vec![key(ACTION_LEAN_RIGHT, true)]);
        lean_step(&mut player, "E released", 1, vec![key(ACTION_LEAN_RIGHT, false)]);
        lean_step(&mut player, "walk +0.5s", 60, vec![key(ACTION_MOVE_FORWARD, true)]);
        lean_step(&mut player, "stop +0.5s", 60, vec![key(ACTION_MOVE_FORWARD, false)]);
    }
    run(&mut player, 60, vec![key(ACTION_MOVE_FORWARD, true)]);
    let from = player.feet();
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for _ in 0..120 {
        run(&mut player, 1, Vec::new());
        let eye = player.view().origin.z - player.feet().z;
        low = low.min(eye);
        high = high.max(eye);
    }
    report("run 1s after 0.5s", &player, from, 1.0);
    println!("NAV PROBE run eye height {low:.4}..{high:.4} m (body speed {:.3})", player.body.speed);
    run(&mut player, 1, vec![key(ACTION_CROUCH, true)]);
    run(&mut player, 1, vec![key(ACTION_CROUCH, false)]);
    run(&mut player, 60, Vec::new());
    let from = player.feet();
    run(&mut player, 120, Vec::new());
    report("sneak 1s", &player, from, 1.0);
    run(&mut player, 1, vec![key(ACTION_CROUCH, true)]);
    run(&mut player, 1, vec![key(ACTION_CROUCH, false)]);
    run(&mut player, 60, Vec::new());
    // Face the longest clear run (rays at 0.5 m and 1.7 m) for the sprint and slide.
    let feet = player.feet();
    let clear = |yaw: f32| {
        let direction = Vec3::new(yaw.cos(), yaw.sin(), 0.0);
        [0.5f32, 1.7]
            .iter()
            .map(|h| world.cast_ray(feet + Vec3::Z * *h, direction, 40.0).map_or(40.0, |hit| hit.distance))
            .fold(40.0f32, f32::min)
    };
    let yaw = (0..16).map(|k| k as f32 * std::f32::consts::TAU / 16.0).max_by(|a, b| clear(*a).total_cmp(&clear(*b))).unwrap();
    println!("NAV PROBE sprint direction yaw={yaw:.3} clear={:.2}m", clear(yaw));
    player.teleport(feet, yaw);
    run(&mut player, 60, Vec::new());
    for frame in 0..8 {
        let events = match frame {
            0 => vec![key(ACTION_SPRINT, true)],
            1 => vec![key(ACTION_SPRINT, false)],
            _ => Vec::new(),
        };
        run(&mut player, 1, events);
        println!("NAV PROBE sprint frame {frame}: states={:?} speed={:.3}", player.machine.active_states(), player.physics.velocity.truncate().length());
    }
    for step in 0..5 {
        run(&mut player, 10, Vec::new());
        println!(
            "NAV PROBE sprint +{}: feet={:?} velocity={:?} support={:?} contacts={} gravity={:?} states={:?}",
            (step + 1) * 10,
            player.feet(),
            player.physics.velocity,
            player.physics.ground.state,
            player.physics.proxy.contact_count(),
            player.physics.gravity_velocity,
            player.machine.active_states(),
        );
    }
    run(&mut player, 1, vec![key(ACTION_CROUCH, true)]);
    run(&mut player, 1, vec![key(ACTION_CROUCH, false)]);
    for step in 0..24 {
        run(&mut player, 15, Vec::new());
        println!(
            "NAV PROBE slide t={:.3}s: states={:?} speed={:.3} posture={} feet={:?}",
            (step + 1) as f32 * 15.0 / 120.0,
            player.machine.active_states(),
            player.physics.velocity.truncate().length(),
            player.physics.posture,
            player.feet(),
        );
    }
    let from = player.feet();
    run(&mut player, 120, Vec::new());
    report("after slide 1s", &player, from, 1.0);
    run(&mut player, 1, vec![key(ACTION_MOVE_FORWARD, false)]);
    run(&mut player, 240, Vec::new());
    let ground = player.feet().z;
    let standing = player.view().origin.z - player.feet().z;
    run(&mut player, 1, vec![key(ACTION_JUMP, true)]);
    run(&mut player, 1, vec![key(ACTION_JUMP, false)]);
    // The jump/land camera additive: eye above the feet every 0.05 s.
    let mut peak = player.feet().z;
    let mut trace = Vec::new();
    for frame in 0..240 {
        run(&mut player, 1, Vec::new());
        peak = peak.max(player.feet().z);
        if frame % 6 == 5 {
            let eye = player.view().origin.z - player.feet().z - standing;
            trace.push(format!("{:.2}s {:?} {:?} {:+.4}", (frame + 1) as f32 / 120.0, player.machine.active_states(), player.body.additives.playing(), eye));
        }
    }
    println!("NAV PROBE jump eye offset from standing {standing:.4}: {}", trace.join(", "));
    println!("NAV PROBE jump: peak +{:.3}m above {ground:.3}, then feet={:?} states={:?}", peak - ground, player.feet(), player.machine.active_states());
    Ok(())
}

/// `DH2_ANIM_PROBE`: catfall, mantle and vault on synthetic collision
/// (a floor plus one box), traced headlessly.
pub(super) fn probe_animation(catalog: &Catalog) -> Result<(), String> {
    use crate::gameplay::player::controller::{ACTION_JUMP, ACTION_MOVE_FORWARD, ACTION_SPRINT};
    let quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| [[to_view(a), to_view(b), to_view(c)], [to_view(a), to_view(c), to_view(d)]];
    let block = |min: Vec3, max: Vec3| {
        let p = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let (a, b) = (min, max);
        let mut t = Vec::new();
        t.extend(quad(p(a.x, a.y, b.z), p(b.x, a.y, b.z), p(b.x, b.y, b.z), p(a.x, b.y, b.z)));
        t.extend(quad(p(a.x, a.y, a.z), p(a.x, a.y, b.z), p(a.x, b.y, b.z), p(a.x, b.y, a.z)));
        t.extend(quad(p(b.x, a.y, a.z), p(b.x, b.y, a.z), p(b.x, b.y, b.z), p(b.x, a.y, b.z)));
        t.extend(quad(p(a.x, a.y, a.z), p(b.x, a.y, a.z), p(b.x, a.y, b.z), p(a.x, a.y, b.z)));
        t.extend(quad(p(a.x, b.y, a.z), p(a.x, b.y, b.z), p(b.x, b.y, b.z), p(b.x, b.y, a.z)));
        t
    };
    let floor = quad(Vec3::new(-60.0, -60.0, 0.0), Vec3::new(60.0, -60.0, 0.0), Vec3::new(60.0, 60.0, 0.0), Vec3::new(-60.0, 60.0, 0.0)).to_vec();
    let settings = look_settings();
    let dt = 1.0 / 120.0;
    let key = |action: i32, down: bool| ActionEvent { action, key: 0, value: [0.0; 2], flags: down as u32 };
    let trace = |player: &mut Player, line: &str, t: f32| {
        let eye = player.view().origin;
        println!(
            "ANIM PROBE {line} t={t:.3}: states={:?} feet={:?} eye-feet={:.3} mode={}/{} pitch={:.3} yaw={:.3} scene={:?}",
            player.machine.active_states(),
            player.feet(),
            eye.z - player.feet().z,
            player.camera.current_mode,
            player.camera.target_mode,
            player.camera.pitch,
            player.physics.yaw,
            player.body.midnight.playing(),
        );
    };
    let scenario = |label: &str, box_triangles: Vec<[Vec3; 3]>, start: Vec3, seconds: f32, input: &dyn Fn(usize, &Player) -> Vec<ActionEvent>| -> Result<(), String> {
        let mut triangles = floor.clone();
        triangles.extend(box_triangles);
        let world = CollisionWorld::new(triangles);
        let collision = GameCollision(&world);
        let setup = load_player_setup(catalog, super::pause_menu::debug_character())?;
        let mut player = Player::new(setup, start, 0.0, base_fov())?;
        for frame in 0..(seconds / dt) as usize {
            let events = input(frame, &player);
            player.frame(&collision, &events, &settings, dt, dt, 80.0);
            player.view();
            if frame % 6 == 5 {
                trace(&mut player, label, (frame + 1) as f32 * dt);
            }
        }
        Ok(())
    };
    // Walk off a pillar.
    let walk = |frame: usize, _: &Player| if frame == 30 { vec![key(ACTION_MOVE_FORWARD, true)] } else { Vec::new() };
    scenario("catfall 7m", block(Vec3::new(-2.0, -2.0, 0.0), Vec3::new(0.6, 2.0, 7.0)), Vec3::new(0.0, 0.0, 7.0), 3.0, &walk)?;
    scenario("catfall 12m", block(Vec3::new(-2.0, -2.0, 0.0), Vec3::new(0.6, 2.0, 12.0)), Vec3::new(0.0, 0.0, 12.0), 3.5, &walk)?;
    let climb = |frame: usize, _: &Player| match frame {
        60 => vec![key(ACTION_MOVE_FORWARD, true), key(ACTION_JUMP, true)],
        _ => Vec::new(),
    };
    scenario("mantle high", block(Vec3::new(1.2, -5.0, 0.0), Vec3::new(6.0, 5.0, 1.7)), Vec3::new(0.0, 0.0, 0.0), 2.5, &climb)?;
    scenario("mantle mid", block(Vec3::new(1.2, -5.0, 0.0), Vec3::new(6.0, 5.0, 1.0)), Vec3::new(0.0, 0.0, 0.0), 2.5, &climb)?;
    let vault = |frame: usize, player: &Player| {
        let mut events = Vec::new();
        match frame {
            30 => events.push(key(ACTION_MOVE_FORWARD, true)),
            90 => events.push(key(ACTION_SPRINT, true)),
            91 => events.push(key(ACTION_SPRINT, false)),
            _ => {}
        }
        if player.vault_sensor.can_vault && frame % 2 == 0 {
            events.push(key(ACTION_JUMP, true));
        } else if frame % 2 == 1 {
            events.push(key(ACTION_JUMP, false));
        }
        events
    };
    scenario("vault", block(Vec3::new(10.0, -5.0, 0.0), Vec3::new(10.4, 5.0, 0.9)), Vec3::new(0.0, 0.0, 0.0), 4.0, &vault)?;
    Ok(())
}
