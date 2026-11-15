//! Far Reach's retained typed acquisition. World targeting is admitted only
//! after sensor27's actual safe-position and player-state checks succeed.
use crate::{
    assets::entities::Value,
    gameplay::player::{
        Player,
        camera::View,
        character_proxy::PlayerWorld,
        grapple::{Decl, GeometryTarget, Sensor, SensorDecl, Target},
        grapple_state::Request,
        physics::PlayerPhysics,
        sensors::{MantleDecl, MantleSensor},
        states::StateId,
    },
};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct PlayerSettings {
    pub motion: Decl,
    pub sensor: SensorDecl,
    pub mantle: Option<MantleDecl>,
}

pub(super) struct Runtime {
    sensor: Sensor,
    target: Option<GeometryTarget>,
    pending: bool,
    was_active: bool,
    mantle: Option<MantleSensor>,
    mantle_exit: Option<StateId>,
    held_snapshot: Option<(GeometryTarget, bevy_math::Vec3)>,
    snapshot_broken: bool,
    aiming: bool,
    elapsed: f32,
    moved: bool,
    previous_position: Option<bevy_math::Vec3>,
    cast_mode: i32,
    snap_hold_duration: f32,
    snap_action: i32,
    clearance_hits: Vec<crate::gameplay::player::character_proxy::WorldHit>,
    request: Option<Request>,
    beam_destination: bevy_math::Vec3,
    beam_speed: f32,
    start_pull_distance: f32,
    world_impact_fx: Option<String>,
    world_impact: Option<(String, bevy_math::Vec3, bevy_math::Mat3)>,
}
impl Runtime {
    pub fn new(prepared: Prepared, physics: &PlayerPhysics) -> Self {
        Self {
            mantle: prepared
                .player
                .mantle
                .map(|decl| MantleSensor::new(decl, physics)),
            sensor: Sensor::new(prepared.player.sensor),
            target: None,
            pending: false,
            was_active: false,
            mantle_exit: None,
            held_snapshot: None,
            snapshot_broken: false,
            aiming: false,
            elapsed: 0.0,
            moved: true,
            previous_position: None,
            cast_mode: 0,
            snap_hold_duration: prepared.snap_hold_duration,
            snap_action: prepared.snap_action,
            clearance_hits: Vec::new(),
            request: None,
            beam_destination: bevy_math::Vec3::ZERO,
            beam_speed: prepared.beam_speed,
            start_pull_distance: prepared.start_pull_distance,
            world_impact_fx: prepared.world_impact_fx,
            world_impact: None,
        }
    }
    pub fn begin_cast(&mut self) {
        self.aiming = true;
        self.elapsed = 0.0;
        self.moved = true;
        self.previous_position = None;
        self.snapshot_broken = false;
    }
    pub fn finish_cast(&mut self) {
        self.aiming = false;
    }
    /// 442710's source hold accumulator runs only while the actual power
    /// lifecycle is aiming; moving the target resets it. It never admits a
    /// cast or replaces an animation boundary.
    pub fn update_cast(&mut self, _player: &Player, dt: f32, cast_mode: i32) {
        self.cast_mode = cast_mode;
        if self.aiming {
            self.elapsed = if self.moved { 0.0 } else { self.elapsed + dt };
        }
    }
    /// Source sensor27 queries the existing geometry owner after the real
    /// player camera exists; no synthetic grapple target actor is spawned.
    pub fn aim(
        &mut self,
        player: &Player,
        view: &View,
        queries: &dyn PlayerWorld,
        snapping: bool,
    ) -> bool {
        if self.has_activity() {
            return false;
        }
        let previous = self.target;
        let previous_mantle = self.mantle_exit;
        self.target = self.sensor.geometry_target(
            queries,
            &player.physics,
            view.origin,
            view.axis,
            player
                .attributes
                .get("ARK_ATTRIBUTE_GRAPPLE_HORIZONTAL_RANGE"),
            player
                .attributes
                .get("ARK_ATTRIBUTE_GRAPPLE_VERTICAL_RANGE"),
            snapping,
            player.machine.is_active(StateId::SNEAKING),
            player.machine.is_active(StateId::AUTOCROUCH),
        );
        self.mantle_exit = None;
        if let Some(mut target) = self.target {
            self.moved = self.previous_position.is_none_or(|position| {
                position.distance_squared(target.sensor_position)
                    > self.sensor.prepared.decl.movement_tolerancy.powi(2)
            });
            self.previous_position = Some(target.sensor_position);
            if let Some(mantle) = self.mantle.as_mut() {
                // 442cc0 supplies sensor3's position/axis/ceiling overrides.
                let axis = if target.sensor_normal.z.abs() >= f32::from_bits(0x3f3504f2) {
                    player.physics.axis()
                } else {
                    crate::gameplay::reference_frame::direction_basis(
                        -bevy_math::Vec3::new(target.sensor_normal.x, target.sensor_normal.y, 0.0)
                            .normalize_or_zero(),
                    )
                };
                let origin = target.sensor_position
                    - axis.x_axis * (player.physics.current_settings().radius + 0.2)
                    - bevy_math::Vec3::Z
                        * mantle
                            .decl
                            .forward_cast_height_min
                            .min(target.forward_ground_distance);
                mantle.update_at_with_ceiling(
                    queries,
                    &player.physics,
                    origin,
                    axis,
                    Some(target.ceiling_distance),
                );
                if mantle.can_mantle()
                    && mantle.destination_surface.is_some_and(|hit| {
                        hit.body == 0 && queries.physics_material(&hit).is_some()
                    })
                    && !(snapping && target.sensor_normal.z > f32::from_bits(0x3f3504f2))
                {
                    let destination = mantle.destination + mantle.destination_axis.x_axis * 0.01;
                    let probe = target.sensor_position
                        + target.sensor_normal.cross(view.axis.y_axis)
                            * (destination.z - target.sensor_position.z);
                    if self.sensor.prepared.clear(
                        queries,
                        &player.physics,
                        view.origin,
                        probe,
                        &mut self.clearance_hits,
                    ) {
                        if let Some((posture, state)) = mantle.final_posture() {
                            self.mantle_exit = Some(StateId(state));
                            self.held_snapshot = Some((target, probe));
                            self.snapshot_broken = false;
                            target.position = destination;
                            target.safe_position = destination;
                            target.normal = bevy_math::Vec3::Z;
                            target.posture = posture + 8;
                            self.target = Some(target);
                        }
                    }
                }
            }
        } else if snapping && previous_mantle.is_some() && !self.snapshot_broken {
            // 4439f0 retains a previously accepted ledge only within the
            // source cone/range and a fresh material-aware clearance query.
            if let Some((snapshot, probe)) = self.held_snapshot {
                let delta = snapshot.sensor_position - view.origin;
                let (_, _, range) = Sensor::query_segment(
                    &player.physics,
                    view.origin,
                    view.axis,
                    player
                        .attributes
                        .get("ARK_ATTRIBUTE_GRAPPLE_HORIZONTAL_RANGE"),
                    player
                        .attributes
                        .get("ARK_ATTRIBUTE_GRAPPLE_VERTICAL_RANGE"),
                );
                if delta.normalize_or_zero().dot(view.axis.x_axis)
                    >= self
                        .sensor
                        .prepared
                        .decl
                        .max_angle_snapping
                        .to_radians()
                        .cos()
                    && delta.length() <= range + self.sensor.prepared.decl.max_distance_snapping
                    && self.sensor.prepared.clear(
                        queries,
                        &player.physics,
                        view.origin,
                        probe,
                        &mut self.clearance_hits,
                    )
                {
                    self.target = previous;
                    self.mantle_exit = previous_mantle;
                } else {
                    self.snapshot_broken = true;
                }
            }
        }
        if self.target.is_none() {
            self.moved = true;
            self.previous_position = None;
        }
        self.target.is_some()
    }
    pub fn execute(&mut self, player: &mut Player, _view: &View) -> Result<bool, String> {
        if self.has_activity() {
            return Ok(false);
        }
        let Some(target) = self.target else {
            return Ok(false);
        };
        if !player.machine.grapple_request_allowed()
            || player.machine.is_active(StateId::GRAPPLE)
            || player.grapple_state().is_some_and(|run| !run.finished)
        {
            return Ok(false);
        }
        let exit_state = if let Some(state) = self.mantle_exit {
            state
        } else if player.machine.is_active(StateId::SNEAKING) {
            StateId::SNEAKING
        } else if player.machine.is_active(StateId::AUTOCROUCH) {
            StateId::AUTOCROUCH
        } else {
            StateId::RUNNING
        };
        self.beam_destination = target.position;
        if target.normal.z < f32::from_bits(0x3f3504f2) {
            self.beam_destination.z += player.physics.current_settings().height;
        }
        if !player.start_grapple_beam(self.beam_destination, self.beam_speed)? {
            return Ok(false);
        }
        self.request = Some(Request {
            target: Target::from_normal(target.normal),
            travel_target: if self.cast_mode == 1 && self.elapsed >= self.snap_hold_duration
                || self.cast_mode == 2 && player.controller.down(self.snap_action)
            {
                Target::from_normal(target.normal)
            } else {
                Target::Ground
            },
            target_position: target.position,
            target_normal: target.normal,
            safe_position: target.safe_position,
            target_velocity: bevy_math::Vec3::ZERO,
            start_posture: target.posture,
            exit_state,
            to_mantle: self.mantle_exit.is_some() && !target.through_surface,
            can_jump: target.can_jump,
            snapped_ground: target.snapped_ground,
            jump_to_target: false,
            sensor: Arc::clone(&self.sensor.prepared),
            snap_action: self.snap_action,
            cast_mode: self.cast_mode,
            mantle: self.mantle.take(),
        });
        self.pending = true;
        Ok(true)
    }
    pub fn has_activity(&self) -> bool {
        self.pending || self.was_active
    }
    pub fn cancel(&mut self, _player: &mut Player) {
        if !self.has_activity() {
            self.target = None;
        }
    }
    pub fn service(&mut self, player: &mut Player) -> Result<bool, String> {
        if self.request.is_some() {
            let remaining = player.retarget_grapple_beam(self.beam_destination)?;
            if remaining.is_some_and(|distance| distance <= self.start_pull_distance)
                && !player.machine.is_active(StateId::USE_POWER)
            {
                let request = self.request.take().unwrap();
                if let Some(path) = &self.world_impact_fx {
                    let target = self.target.unwrap();
                    self.world_impact = Some((
                        path.clone(),
                        if self.mantle_exit.is_some() {
                            target.position
                        } else {
                            target.sensor_position
                        },
                        crate::gameplay::reference_frame::direction_basis(target.normal),
                    ));
                }
                if let Err(request) = player.begin_grapple_checked(request) {
                    self.mantle = request.mantle;
                    self.pending = false;
                    player.stop_grapple_beam();
                    return Ok(false);
                }
            }
        }
        let active = player.machine.is_active(StateId::GRAPPLE);
        if active {
            self.pending = false;
            self.was_active = true;
        }
        let finished = player.take_finished_grapple();
        if let Some(request) = finished {
            player.stop_grapple_beam();
            self.mantle = request.mantle;
            self.pending = false;
            self.was_active = false;
            self.target = None;
        }
        if self.was_active && !active {
            self.pending = false;
            self.was_active = false;
            self.target = None;
        }
        Ok(self.has_activity())
    }
    pub fn retire(&mut self, player: &mut Player) {
        player.stop_grapple_beam();
        if let Some(request) = self.request.take() {
            self.mantle = request.mantle;
        }
        if self.has_activity() {
            player.cancel_grapple();
        }
        self.pending = false;
        self.was_active = false;
        self.target = None;
    }
    pub fn take_world_impact(&mut self) -> Option<(String, bevy_math::Vec3, bevy_math::Mat3)> {
        self.world_impact.take()
    }
}
#[derive(Clone)]
pub(super) struct Prepared {
    pub player: PlayerSettings,
    pub snap_hold_duration: f32,
    pub snap_action: i32,
    pub beam_speed: f32,
    pub start_pull_distance: f32,
    pub world_impact_fx: Option<String>,
}
impl Prepared {
    pub fn player(
        grapple: &[(String, Value)],
        sensor: &[(String, Value)],
    ) -> Result<PlayerSettings, String> {
        if grapple.is_empty() || sensor.is_empty() {
            return Err("Far Reach requires its Grapple state and Grapple sensor component".into());
        }
        Ok(PlayerSettings {
            motion: Decl::read(grapple)?,
            sensor: SensorDecl::read(sensor)?,
            mantle: None,
        })
    }
    pub fn player_with_mantle(
        grapple: &[(String, Value)],
        sensor: &[(String, Value)],
        mantle: &[(String, Value)],
    ) -> Result<PlayerSettings, String> {
        let mut settings = Self::player(grapple, sensor)?;
        if !mantle.is_empty() {
            settings.mantle = Some(crate::gameplay::player::decls::mantle_sensor(mantle)?);
        }
        Ok(settings)
    }
    pub fn with_power(
        component: &[(String, Value)],
        player: &PlayerSettings,
    ) -> Result<Self, String> {
        Ok(Self {
            player: player.clone(),
            snap_hold_duration: crate::gameplay::player::grapple::seconds(
                component,
                "m_snapHoldDuration",
                0.5,
            )?,
            snap_action: match crate::gameplay::options::decl::atom(component, "m_snapAction") {
                None => 20,
                Some(name) => crate::platform::bindings::ACTIONS
                    .iter()
                    .find(|(_, tag, _)| *tag == name.trim_start_matches("ACTION_"))
                    .map(|(id, _, _)| *id)
                    .ok_or_else(|| format!("unknown m_snapAction {name}"))?,
            },
            beam_speed: crate::gameplay::options::decl::float(
                component,
                "m_castGrappleSpeed",
                50.0,
            ),
            start_pull_distance: crate::gameplay::options::decl::float(
                component,
                "m_startPullDistance",
                3.0,
            ),
            world_impact_fx: crate::gameplay::options::decl::atom(component, "m_worldImpactFX")
                .filter(|path| !path.is_empty() && !path.eq_ignore_ascii_case("NULL"))
                .map(str::to_owned),
        })
    }
    /// Traversal scenes are canonical Midnight resources and are acquired
    /// alongside the player state. Power FX stay with their established owner.
    pub fn scene_paths(&self) -> impl Iterator<Item = &str> {
        [
            self.player.motion.translate_midnight.as_deref(),
            self.player.motion.wall_midnight.as_deref(),
            self.player.motion.ceil_midnight.as_deref(),
        ]
        .into_iter()
        .flatten()
    }
    pub fn effect_paths(&self) -> impl Iterator<Item = &str> {
        [
            self.world_impact_fx.as_deref(),
            self.player.motion.leave_wall_fx.as_deref(),
        ]
        .into_iter()
        .flatten()
    }
}
