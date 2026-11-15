//! arkPlayerStateGrapple's movement projection (140838d90/140839b80).
//! Target acquisition and state admission must finish before constructing a
//! motion. The canonical player physics owner integrates its velocity.
use super::{
    character_proxy::{PlayerWorld, WorldHit},
    physics::PlayerPhysics,
};
use crate::gameplay::collision_filter::QueryFilter;
use crate::{assets::entities::Value, gameplay::options::decl::atom};
use bevy_math::{Mat3, Vec3};
use std::sync::Arc;

fn number(fields: &[(String, Value)], name: &str, default: f32) -> Result<f32, String> {
    match atom(fields, name) {
        None => Ok(default),
        Some(value) => value
            .trim_end_matches('f')
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("{name}: invalid number {value:?}")),
    }
}

/// Reflected arkDeclPlayerStateGrapple fields (141782d70); constructor
/// defaults are 140837390. Scene paths remain with the Midnight owner.
#[derive(Clone, Debug, PartialEq)]
pub struct Decl {
    pub distance_to_wall: f32,
    pub start_speed_to_wall: f32,
    pub end_speed_to_wall: f32,
    pub start_speed_to_ceil: f32,
    pub end_speed_to_ceil: f32,
    pub start_speed_to_ground: f32,
    pub end_speed_to_ground: f32,
    pub start_speed_to_ground_no_jump: f32,
    pub end_speed_to_ground_no_jump: f32,
    pub start_speed_to_mantle: f32,
    pub end_speed_to_mantle: f32,
    pub start_speed_to_mantle_no_jump: f32,
    pub end_speed_to_mantle_no_jump: f32,
    pub max_inertia_blend_duration: f32,
    pub start_inertia: f32,
    pub end_inertia: f32,
    pub camera_start_turn_position: f32,
    pub camera_full_turn_position: f32,
    pub max_turn_speed_joystick: f32,
    pub max_turn_speed_mouse: f32,
    pub max_turn_speed_joystick_wall: f32,
    pub max_turn_speed_mouse_wall: f32,
    pub max_turn_speed_joystick_ceil: f32,
    pub max_turn_speed_mouse_ceil: f32,
    pub max_angular_acceleration_joystick: f32,
    pub max_angular_deceleration_joystick: f32,
    pub max_angular_acceleration_mouse: f32,
    pub max_angular_deceleration_mouse: f32,
    pub camera_recenter_angle: f32,
    pub camera_ceil_max_pitch_up: f32,
    pub camera_ceil_max_pitch_down: f32,
    pub camera_blend_duration: f32,
    pub min_speed_to_jump_on_ground: f32,
    pub jump_height: f32,
    pub min_jump_height_snapped_to_ground: f32,
    pub max_jump_height_snapped_to_ground: f32,
    pub min_jump_steepness: f32,
    pub ground_jump_velocity: f32,
    pub max_forward_speed: f32,
    pub max_strafe_speed: f32,
    pub push_delay: f32,
    pub extra_jump_velocity: f32,
    pub camera_constraints_wall: [f32; 4],
    pub ceil_jump_impulse: Impulse,
    pub translate_midnight: Option<String>,
    pub wall_midnight: Option<String>,
    pub ceil_midnight: Option<String>,
    pub stick_sound_event: Option<String>,
    pub unstick_sound_event: Option<String>,
    pub leave_wall_fx: Option<String>,
    pub shoved_damage: Option<String>,
}
impl Decl {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let path = |name| {
            atom(fields, name)
                .filter(|p| !p.is_empty() && !p.eq_ignore_ascii_case("NULL"))
                .map(str::to_owned)
        };
        Ok(Self {
            distance_to_wall: number(fields, "m_distanceToWall", 0.15)?,
            start_speed_to_wall: number(fields, "m_startSpeedToWall", 15.0)?,
            end_speed_to_wall: number(fields, "m_endSpeedToWall", 7.5)?,
            start_speed_to_ceil: number(fields, "m_startSpeedToCeil", 30.0)?,
            end_speed_to_ceil: number(fields, "m_endSpeedToCeil", 15.5)?,
            start_speed_to_ground: number(fields, "m_startSpeedToGround", 30.0)?,
            end_speed_to_ground: number(fields, "m_endSpeedToGround", 10.0)?,
            start_speed_to_ground_no_jump: number(fields, "m_startSpeedToGroundNoJump", 40.0)?,
            end_speed_to_ground_no_jump: number(fields, "m_endSpeedToGroundNoJump", 15.0)?,
            start_speed_to_mantle: number(fields, "m_startSpeedToMantle", 30.0)?,
            end_speed_to_mantle: number(fields, "m_endSpeedToMantle", 10.0)?,
            start_speed_to_mantle_no_jump: number(fields, "m_startSpeedToMantleNoJump", 40.0)?,
            end_speed_to_mantle_no_jump: number(fields, "m_endSpeedToMantleNoJump", 10.0)?,
            max_inertia_blend_duration: number(fields, "m_maxInertiaBlendDuration", 0.25)?,
            start_inertia: number(fields, "m_startInertia", 1.0)?,
            end_inertia: number(fields, "m_endInertia", 200.0)?,
            camera_start_turn_position: number(fields, "m_cameraStartTurnPosition", 0.7)?,
            camera_full_turn_position: number(fields, "m_cameraFullTurnPosition", 0.99)?,
            max_turn_speed_joystick: number(fields, "m_maxTurnSpeedJoystick", 350.0)?,
            max_turn_speed_mouse: number(fields, "m_maxTurnSpeedMouse", 700.0)?,
            max_turn_speed_joystick_wall: number(fields, "m_maxTurnSpeedJoystickWall", 500.0)?,
            max_turn_speed_mouse_wall: number(fields, "m_maxTurnSpeedMouseWall", 1000.0)?,
            max_turn_speed_joystick_ceil: number(fields, "m_maxTurnSpeedJoystickCeil", 200.0)?,
            max_turn_speed_mouse_ceil: number(fields, "m_maxTurnSpeedMouseCeil", 400.0)?,
            max_angular_acceleration_joystick: number(
                fields,
                "m_maxAngularAccelerationJoystick",
                525.0,
            )?,
            max_angular_deceleration_joystick: number(
                fields,
                "m_maxAngularDecelerationJoystick",
                2000.0,
            )?,
            max_angular_acceleration_mouse: number(
                fields,
                "m_maxAngularAccelerationMouse",
                5000.0,
            )?,
            max_angular_deceleration_mouse: number(
                fields,
                "m_maxAngularDecelerationMouse",
                20000.0,
            )?,
            camera_recenter_angle: number(fields, "m_cameraRecenterAngle", 20.0)?,
            camera_ceil_max_pitch_up: number(fields, "m_cameraCeilMaxPitchUp", 0.0)?,
            camera_ceil_max_pitch_down: number(fields, "m_cameraCeilMaxPitchDown", 80.0)?,
            camera_blend_duration: number(fields, "m_cameraBlendDuration", 0.25)?,
            min_speed_to_jump_on_ground: number(fields, "m_minSpeedToJumpOnGround", 2.5)?,
            jump_height: number(fields, "m_jumpHeight", 1.5)?,
            min_jump_height_snapped_to_ground: number(
                fields,
                "m_minJumpHeightSnappedToGround",
                2.0,
            )?,
            max_jump_height_snapped_to_ground: number(
                fields,
                "m_maxJumpHeightSnappedToGround",
                6.0,
            )?,
            min_jump_steepness: number(fields, "m_minJumpSteepness", 0.25)?,
            ground_jump_velocity: number(fields, "m_groundJumpVelocity", 10.0)?,
            max_forward_speed: number(fields, "m_maxForwardSpeed", 6.0)?,
            max_strafe_speed: number(fields, "m_maxStrafeSpeed", 6.0)?,
            push_delay: number(fields, "m_pushDelay", 0.25)?,
            extra_jump_velocity: number(fields, "m_extraJumpVelocity", 50.0)?,
            camera_constraints_wall: super::decls::constraint_params(
                fields,
                "m_cameraConstraintsWall",
                [90.0, 90.0, 130.0, 130.0],
            )?,
            ceil_jump_impulse: {
                let nested = fields
                    .iter()
                    .find_map(|(key, value)| {
                        if key == "m_ceilJumpImpulse" {
                            if let Value::Scope(fields) = value {
                                Some(fields.as_slice())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .unwrap_or(&[]);
                Impulse {
                    strength: number(nested, "m_strength", 6.0)?,
                    duration: number(nested, "m_duration", 0.75)?,
                }
            },
            translate_midnight: path("m_translateMidnight"),
            wall_midnight: path("m_wallMidnight"),
            ceil_midnight: path("m_ceilMidnight"),
            stick_sound_event: path("m_stickSoundEvent"),
            unstick_sound_event: path("m_unstickSoundEvent"),
            leave_wall_fx: path("m_leaveWallFX"),
            shoved_damage: path("m_shovedDamage"),
        })
    }
}

/// arkPlayerPhysicsImpulseSettings (14157be20, metadata registration
/// 1414302fe..141430344); second float is duration, not jump angle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impulse {
    pub strength: f32,
    pub duration: f32,
}

/// Sensor27 constructor 1403b1c80, reflected by 1417b9e20. Snap's base
/// fields are acquired separately by the canonical sensor owner.
#[derive(Clone, Debug, PartialEq)]
pub struct SensorDecl {
    pub shape_radius: f32,
    pub hole_width: f32,
    pub hole_height: f32,
    pub jump_shape_height: f32,
    pub extra_horizontal_snap_distance: f32,
    pub extra_ground_snap_distance: f32,
    pub extra_ceiling_snap_distance: f32,
    pub max_angle_snapping: f32,
    pub max_distance_snapping: f32,
    pub num_extra_samples: usize,
    pub num_extra_samples_horiz: usize,
    pub minimum_travel_distance: f32,
    pub minimum_camera_distance: f32,
    pub movement_tolerancy: f32,
    pub collision_layers: u32,
}
impl SensorDecl {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let count = |key, default| -> Result<usize, String> {
            match atom(fields, key) {
                None => Ok(default),
                Some(text) => text
                    .parse()
                    .map_err(|_| format!("{key}: invalid count {text:?}")),
            }
        };
        Ok(Self {
            shape_radius: number(fields, "m_shapeRadius", 0.01)?,
            hole_width: number(fields, "m_holeWidth", 0.5)?,
            hole_height: number(fields, "m_holeHeight", 0.5)?,
            jump_shape_height: number(fields, "m_jumpShapeHeight", 3.0)?,
            extra_horizontal_snap_distance: number(fields, "m_extraHorizontalSnapDistance", 0.5)?,
            extra_ground_snap_distance: number(fields, "m_extraGroundSnapDistance", 1.8)?,
            extra_ceiling_snap_distance: number(fields, "m_extraCeilingSnapDistance", 0.0)?,
            max_angle_snapping: number(fields, "m_maxAngleSnapping", 5.0)?,
            max_distance_snapping: number(fields, "m_maxDistanceSnapping", 0.1)?,
            num_extra_samples: count("m_numExtraSamples", 16)?,
            num_extra_samples_horiz: count("m_numExtraSamplesHoriz", 10)?,
            minimum_travel_distance: number(fields, "m_minimumTravelDistance", 2.0)?,
            minimum_camera_distance: number(fields, "m_minimumCameraDistance", 1.0)?,
            movement_tolerancy: number(fields, "m_movementTolerancy", 0.2)?,
            collision_layers: super::decls::layer_mask(fields, "m_collisionLayers", 0x20822b)?,
        })
    }
    /// 1403b3d10 allocates these shapes once at component Start. The host
    /// retains their convex vertices and borrows the canonical world query
    /// owner for every sweep. No collision bodies are created.
    pub fn shapes(&self) -> [Vec<Vec3>; 7] {
        let r = self.shape_radius;
        let w = self.hole_width;
        let h = self.hole_height;
        let bounds = [
            (Vec3::splat(-r), Vec3::splat(r)),
            (
                Vec3::new(-r, -w * 0.5, -h * 0.5),
                Vec3::new(r, w * 0.5, h * 0.5),
            ),
            (Vec3::new(-r, -w, -r), Vec3::new(r, -r, r)),
            (Vec3::new(-r, r, -r), Vec3::new(r, w, r)),
            (Vec3::splat(-r), Vec3::new(r, r, h)),
            (Vec3::new(-r, -r, -h), Vec3::new(r, r, -r)),
        ];
        std::array::from_fn(|slot| {
            if slot < 6 {
                let (min, max) = bounds[slot];
                (0..8)
                    .map(|corner| {
                        Vec3::new(
                            if corner & 1 == 0 { min.x } else { max.x },
                            if corner & 2 == 0 { min.y } else { max.y },
                            if corner & 4 == 0 { min.z } else { max.z },
                        )
                    })
                    .collect()
            } else {
                // 1403b3f69 supplies jumpShapeHeight, shapeRadius, 0 to
                // 1400efc60. Its eight polygon coefficients at141ae36e0
                // are deliberately the executable's rounded .38/.92.
                const RING: [(f32, f32); 8] = [
                    (-0.92, 0.38),
                    (-0.38, 0.92),
                    (0.38, 0.92),
                    (0.92, 0.38),
                    (0.92, -0.38),
                    (0.38, -0.92),
                    (-0.38, -0.92),
                    (-0.92, -0.38),
                ];
                [self.jump_shape_height, 0.0]
                    .into_iter()
                    .flat_map(|z| {
                        RING.into_iter()
                            .map(move |(x, y)| Vec3::new(x * r, y * r, z))
                    })
                    .collect()
            }
        })
    }
}

/// Material projection read from the collision world's actual resolved
/// arkPhysicsMaterial. `blocks_vision` is collisionSettings+0x7c bit3;
/// `through` is the separate flags byte+0x80 bit3.
#[derive(Clone, Copy, Debug)]
pub struct Material {
    pub through: bool,
    pub blocks_vision: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct TargetHit {
    pub hit: WorldHit,
    pub position: Vec3,
    pub distance: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct GeometryTarget {
    pub position: Vec3,
    pub normal: Vec3,
    pub safe_position: Vec3,
    pub posture: usize,
    pub snapped_ground: bool,
    pub can_jump: bool,
    pub hit: WorldHit,
}
#[derive(Clone, Copy, Debug)]
pub struct SafePosition {
    pub position: Vec3,
    pub posture: usize,
    pub distance: f32,
    pub shape_changed: bool,
}

/// Persistent sensor query scratch (Start1403b3d10, Finish1403b1d40).
#[derive(Clone, Debug)]
pub struct PreparedSensor {
    pub decl: SensorDecl,
    shapes: [Vec<Vec3>; 7],
    samples: Vec<Vec3>,
}
impl PreparedSensor {
    pub fn new(decl: SensorDecl) -> Self {
        let shapes = decl.shapes();
        let mut samples = vec![Vec3::X];
        let tangent = decl.max_angle_snapping.to_radians().tan();
        // 1403b3d10: rings of four cardinal offsets; the horizontal
        // samples stop at numExtraSamplesHoriz while verticals continue.
        for ring in 0..decl.num_extra_samples {
            let radius = tangent / decl.num_extra_samples as f32 * (ring + 1) as f32;
            for quadrant in 0..4 {
                if quadrant & 1 != 0 || ring < decl.num_extra_samples_horiz {
                    let angle = std::f32::consts::FRAC_PI_2 * quadrant as f32;
                    samples.push(
                        Vec3::new(1.0, angle.cos() * radius, angle.sin() * radius)
                            .normalize_or_zero(),
                    );
                }
            }
        }
        Self {
            decl,
            shapes,
            samples,
        }
    }
    /// 1403b2280's camera-motion clearance. Retain no extra query world or
    /// collision shape; callers supply the existing per-state hit scratch.
    pub fn clear(
        &self,
        world: &dyn PlayerWorld,
        physics: &PlayerPhysics,
        eye: Vec3,
        predicted: Vec3,
        hits: &mut Vec<WorldHit>,
    ) -> bool {
        let forward = physics.axis().x_axis;
        let origin = eye - forward * forward.dot(eye - physics.position());
        let delta = predicted - origin;
        let length = delta.length();
        let result = cast_target(
            &self.decl,
            &self.shapes[0],
            hits,
            world,
            origin,
            physics.axis(),
            delta.normalize_or_zero(),
            length,
            0,
            |hit| {
                world.physics_material(hit).map(|value| Material {
                    through: value.can_grapple_through(),
                    blocks_vision: value.collision.legacy_blocking(
                        crate::gameplay::collision_settings::CollisionBlock::Vision,
                    ),
                })
            },
        );
        result.is_none_or(|hit| hit.position.distance_squared(predicted) <= 0.01)
    }
}
pub struct Sensor {
    pub prepared: Arc<PreparedSensor>,
    hits: Vec<WorldHit>,
    crossings: Vec<Crossing>,
}
impl Sensor {
    pub fn new(decl: SensorDecl) -> Self {
        Self {
            prepared: Arc::new(PreparedSensor::new(decl)),
            hits: Vec::new(),
            crossings: Vec::new(),
        }
    }
    pub fn from_prepared(prepared: Arc<PreparedSensor>) -> Self {
        Self {
            prepared,
            hits: Vec::new(),
            crossings: Vec::new(),
        }
    }
    /// State27 14083b910: compare the three real navigation shapes,
    /// retry 5cm higher only for zero horizontal progress, and retain the
    /// furthest safe point measured from the cast's starting origin.
    pub fn state_safe_position(
        &mut self,
        world: &dyn PlayerWorld,
        physics: &PlayerPhysics,
        source_start: Vec3,
        target_position: Vec3,
        target_normal: Vec3,
        grappling_posture: usize,
        previous: Vec3,
        previous_distance: f32,
    ) -> SafePosition {
        for attempt in 0..2 {
            let mut positions = [Vec3::ZERO; 3];
            let mut distances = [0.0; 3];
            for slot in 0..3 {
                positions[slot] = self.safe_position(
                    world,
                    physics,
                    target_position,
                    target_normal,
                    slot,
                    attempt as f32 * 0.05,
                );
                distances[slot] = positions[slot].distance(physics.position());
            }
            let mut posture = grappling_posture;
            let slot = match grappling_posture {
                10 => {
                    if distances[2] + 0.1 <= distances[1] || distances[2] + 0.1 <= distances[0] {
                        if distances[0] < distances[1] + 0.1 {
                            1
                        } else {
                            0
                        }
                    } else {
                        2
                    }
                }
                9 => {
                    if distances[1] + 0.1 <= distances[0] || distances[1] + 0.1 <= distances[2] {
                        if distances[0] < distances[2] + 0.1 {
                            posture = 10;
                            2
                        } else {
                            0
                        }
                    } else {
                        1
                    }
                }
                _ => {
                    if distances[0] + 0.1 <= distances[1] || distances[0] + 0.1 <= distances[2] {
                        if distances[1] + 0.1 <= distances[2] {
                            posture = 10;
                            2
                        } else {
                            posture = 9;
                            1
                        }
                    } else {
                        0
                    }
                }
            };
            let distance = (positions[slot] - source_start).truncate().length();
            if distance > f32::EPSILON || attempt == 1 {
                return if distance > previous_distance {
                    SafePosition {
                        position: positions[slot],
                        posture,
                        distance,
                        shape_changed: posture != grappling_posture,
                    }
                } else {
                    SafePosition {
                        position: previous,
                        posture: grappling_posture,
                        distance: previous_distance,
                        shape_changed: false,
                    }
                };
            }
        }
        unreachable!()
    }
    /// 1403b6c90 projects the camera onto the player-forward plane, and
    /// limits its view vector independently by the horizontal/vertical
    /// range attributes. DAT141ac8ffc is 100, verified in the image.
    pub fn query_segment(
        physics: &PlayerPhysics,
        camera_position: Vec3,
        camera_axis: Mat3,
        horizontal_range: f32,
        vertical_range: f32,
    ) -> (Vec3, Vec3, f32) {
        let forward = physics.axis().x_axis;
        let origin = camera_position - forward * forward.dot(camera_position - physics.position());
        let direction = camera_axis.x_axis;
        let mut displacement = direction * 100.0;
        if vertical_range < displacement.z.abs() {
            displacement *= vertical_range / displacement.z.abs();
        }
        let horizontal = displacement.truncate().length();
        if horizontal_range < horizontal {
            displacement *= horizontal_range / horizontal;
        }
        (origin, direction, displacement.length())
    }
    /// 1403b4610 /1403b2810 ordinary world-geometry branch. The small
    /// authored sensor hull is swept by the canonical player query owner.
    /// Through surfaces are skipped before nearest selection; a nearest
    /// surface which does not block vision rejects targeting beyond it.
    pub fn cast_target(
        &mut self,
        world: &dyn PlayerWorld,
        origin: Vec3,
        axis: Mat3,
        direction: Vec3,
        length: f32,
        mut material: impl FnMut(&WorldHit) -> Option<Material>,
    ) -> Option<TargetHit> {
        cast_target(
            &self.prepared.decl,
            &self.prepared.shapes[0],
            &mut self.hits,
            world,
            origin,
            axis,
            direction,
            length,
            0x4400,
            &mut material,
        )
    }
    /// 1403b6c90 quantizes sample orientation by maxAngle/numSamples;
    /// 1403b49c0 accepts the first sample whose real hull sweep succeeds.
    pub fn sampled_target(
        &mut self,
        world: &dyn PlayerWorld,
        physics: &PlayerPhysics,
        origin: Vec3,
        direction: Vec3,
        length: f32,
        snapping: bool,
    ) -> Option<TargetHit> {
        let material = |hit: &WorldHit| {
            world.physics_material(hit).map(|value| Material {
                through: value.can_grapple_through(),
                blocks_vision: value
                    .collision
                    .legacy_blocking(crate::gameplay::collision_settings::CollisionBlock::Vision),
            })
        };
        if let Some(hit) =
            self.cast_target(world, origin, physics.axis(), direction, length, material)
        {
            return Some(hit);
        }
        if !snapping || self.prepared.decl.num_extra_samples == 0 {
            return None;
        }
        let step =
            self.prepared.decl.max_angle_snapping / self.prepared.decl.num_extra_samples as f32;
        if step == 0.0 {
            return None;
        }
        let pitch = (-direction.z)
            .atan2(direction.truncate().length())
            .to_degrees();
        let yaw = direction.y.atan2(direction.x).to_degrees();
        let pitch = (pitch - pitch % step).to_radians();
        let yaw = (yaw - yaw % step).to_radians();
        let axis = Mat3::from_axis_angle(Vec3::Z, yaw) * Mat3::from_axis_angle(Vec3::Y, pitch);
        for sample in &self.prepared.samples {
            let sample_direction = axis * *sample;
            let Some(ray) = world.cast_ray(
                QueryFilter::layers(self.prepared.decl.collision_layers, 0x40),
                origin,
                sample_direction,
                length,
            ) else {
                continue;
            };
            let delta = ray.position - origin;
            if let Some(hit) = cast_target(
                &self.prepared.decl,
                &self.prepared.shapes[0],
                &mut self.hits,
                world,
                origin,
                physics.axis(),
                delta.normalize_or_zero(),
                delta.length(),
                0,
                material,
            ) {
                return Some(hit);
            }
        }
        None
    }
    fn material(world: &dyn PlayerWorld, hit: &WorldHit) -> Option<Material> {
        world.physics_material(hit).map(|value| Material {
            through: value.can_grapple_through(),
            blocks_vision: value
                .collision
                .legacy_blocking(crate::gameplay::collision_settings::CollisionBlock::Vision),
        })
    }
    fn shape_target(
        &mut self,
        world: &dyn PlayerWorld,
        slot: usize,
        origin: Vec3,
        axis: Mat3,
        direction: Vec3,
        length: f32,
    ) -> Option<TargetHit> {
        cast_target(
            &self.prepared.decl,
            &self.prepared.shapes[slot],
            &mut self.hits,
            world,
            origin,
            axis,
            direction,
            length,
            0,
            |hit| Self::material(world, hit),
        )
    }
    fn travel_clear(
        &mut self,
        world: &dyn PlayerWorld,
        shape: &[Vec3],
        axis: Mat3,
        origin: Vec3,
        displacement: Vec3,
        target_body: u32,
    ) -> bool {
        self.hits.clear();
        world.cast_shape(
            QueryFilter::layers(self.prepared.decl.collision_layers, 0x40),
            shape,
            axis,
            origin,
            displacement,
            0.0,
            &mut self.hits,
            None,
        );
        self.hits.iter().all(|hit| {
            hit.value >= 1.0
                || hit.body == target_body
                || Self::material(world, hit).is_some_and(|material| material.through)
        })
    }
    fn vertical_distances(
        &self,
        world: &dyn PlayerWorld,
        physics: &PlayerPhysics,
        position: Vec3,
        normal: Vec3,
        range: f32,
    ) -> (Option<f32>, Option<f32>) {
        let decl = &self.prepared.decl;
        let filter = QueryFilter::layers(decl.collision_layers, 0x40);
        let mut down = world
            .cast_ray(
                filter,
                position,
                -Vec3::Z,
                decl.extra_ground_snap_distance + range,
            )
            .map(|hit| hit.distance);
        let mut up = world
            .cast_ray(
                filter,
                position,
                Vec3::Z,
                decl.extra_ceiling_snap_distance + range,
            )
            .map(|hit| hit.distance);
        if Target::from_normal(normal) == Target::Wall {
            let outside = position
                + Vec3::new(normal.x, normal.y, 0.0).normalize_or_zero()
                    * physics.current_settings().radius;
            if let Some(hit) = world.cast_ray(
                filter,
                outside,
                -Vec3::Z,
                decl.extra_ground_snap_distance + range,
            ) {
                down = Some(down.map_or(hit.distance, |distance| distance.min(hit.distance)));
            }
            if let Some(hit) = world.cast_ray(
                filter,
                outside,
                Vec3::Z,
                decl.extra_ceiling_snap_distance + range,
            ) {
                up = Some(up.map_or(hit.distance, |distance| distance.min(hit.distance)));
            }
        }
        (down, up)
    }
    /// The ordinary world-surface branch of sensor27. Uses its authored
    /// hole probes, actual player postures and forward/reverse safe casts.
    /// Actor catch targets have a separate entity protocol and never enter
    /// this geometry projection.
    pub fn geometry_target(
        &mut self,
        world: &dyn PlayerWorld,
        physics: &PlayerPhysics,
        eye: Vec3,
        view: Mat3,
        horizontal_range: f32,
        vertical_range: f32,
        snapping: bool,
        sneaking: bool,
        crouching: bool,
    ) -> Option<GeometryTarget> {
        let vertical_range = vertical_range
            + if view.x_axis.z > 0.0 {
                (physics.posture_settings(0).height - physics.current_settings().height).max(0.0)
            } else {
                0.0
            };
        let (origin, direction, length) =
            Self::query_segment(physics, eye, view, horizontal_range, vertical_range);
        let mut target = if let Some(target) =
            self.sampled_target(world, physics, origin, direction, length, snapping)
        {
            target
        } else {
            if !snapping {
                return None;
            }
            // 1403b49c0: no forward hit may snap to a floor/ceiling at
            // the range-limited endpoint, followed by the same hull sweep.
            let mut endpoint = origin + direction * length;
            let (down, up) =
                self.vertical_distances(world, physics, endpoint, Vec3::Z, vertical_range);
            let floor = match (down, up) {
                (Some(down), Some(up)) => {
                    down - self.prepared.decl.extra_ground_snap_distance
                        <= up - self.prepared.decl.extra_ceiling_snap_distance
                }
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => return None,
            };
            let vertical = if floor { down? } else { up? };
            endpoint.z += if floor { -vertical } else { vertical };
            let delta = endpoint - physics.position();
            let horizontal = horizontal_range + self.prepared.decl.extra_horizontal_snap_distance;
            let extra = if endpoint.z >= physics.position().z {
                self.prepared.decl.extra_ceiling_snap_distance
            } else {
                self.prepared.decl.extra_ground_snap_distance
            };
            if delta.truncate().length_squared() > horizontal * horizontal
                || delta.z.abs() > vertical_range + extra
            {
                return None;
            }
            let delta = endpoint - origin;
            self.cast_target(
                world,
                origin,
                physics.axis(),
                delta.normalize_or_zero(),
                delta.length(),
                |hit| Self::material(world, hit),
            )?
        };
        let mut position = target.position;
        let mut normal = target.hit.normal;
        let (mut down, mut up) =
            self.vertical_distances(world, physics, position, normal, vertical_range);
        // 1403b49c0: centre/horizontal/vertical hole probes determine the
        // nearest admissible point along the real target segment.
        let delta = position - origin;
        let distance = delta.length();
        let along = delta.normalize_or_zero();
        let centre = self
            .shape_target(world, 1, origin, view, along, distance)
            .map_or(distance, |hit| hit.distance);
        let mut sides = [distance; 4];
        for (slot, value) in sides.iter_mut().enumerate() {
            if let Some(hit) = self.shape_target(world, slot + 2, origin, view, along, distance) {
                if hit.distance > 0.0 {
                    *value = hit.distance;
                }
            }
        }
        let width = if sides[0] < distance && sides[1] < distance {
            sides[0].max(sides[1])
        } else {
            distance
        };
        let mut height = if !crouching && sides[2] < distance && sides[3] < distance {
            sides[2].max(sides[3])
        } else {
            distance
        };
        if let (Some(down), Some(up)) = (down, up) {
            let gap = down + up;
            if gap > physics.posture_settings(2).height && gap < physics.posture_settings(1).height
            {
                height = distance;
            }
        }
        let clearance = centre.max(width.min(height));
        if (clearance - distance).abs() >= 0.1 {
            position = origin + along * clearance;
            normal = -direction;
            (down, up) = self.vertical_distances(world, physics, position, normal, vertical_range);
        }
        // Near targets are constrained to the source's minimum travel
        // distance using its larger hole hull, rather than teleporting to
        // an arbitrary ray hit immediately in front of the camera.
        if position.distance_squared(physics.position())
            < self.prepared.decl.minimum_travel_distance.powi(2)
        {
            let forward = physics.axis().x_axis;
            let distance = self.prepared.decl.minimum_travel_distance + 0.1;
            let point = physics.position() + forward * distance;
            let delta = point - origin;
            let hit = self.shape_target(
                world,
                1,
                origin,
                physics.axis(),
                delta.normalize_or_zero(),
                delta.length(),
            );
            let travel = hit
                .as_ref()
                .map_or(self.prepared.decl.minimum_travel_distance, |hit| {
                    hit.distance.min(self.prepared.decl.minimum_travel_distance)
                });
            position = physics.position() + forward * travel;
            if let Some(hit) = hit {
                normal = hit.hit.normal;
                target.hit = hit.hit;
            }
            let previous_down = down;
            (down, up) = self.vertical_distances(world, physics, position, normal, vertical_range);
            if let (Some(before), Some(after)) = (previous_down, down) {
                if (before - after).abs() > 1.0 {
                    return None;
                }
            }
        }
        // 1403b2370: the body origin is below a ceiling/wall hit; a floor
        // hit is already an origin. Preserve the source .7071 split.
        let shape = physics.current_settings();
        let mut snapped_ground = false;
        match Target::from_normal(normal) {
            Target::Wall => {
                normal = Vec3::new(normal.x, normal.y, 0.0).normalize_or_zero();
                position += normal * shape.radius;
                {
                    let distance = down.unwrap_or(f32::MAX);
                    let lower = shape.height.min(distance);
                    let candidate = position - Vec3::Z * lower;
                    if self
                        .prepared
                        .clear(world, physics, eye, candidate, &mut self.hits)
                    {
                        position = candidate;
                        if distance < shape.height {
                            normal = Vec3::Z;
                            snapped_ground = true;
                        }
                    }
                }
            }
            Target::Ceil => {
                if let Some(distance) = down.filter(|distance| *distance < shape.height) {
                    position.z -= distance;
                    normal = Vec3::Z;
                    snapped_ground = true;
                } else {
                    position.z -= shape.height;
                    normal = -Vec3::Z;
                }
            }
            Target::Ground => normal = Vec3::Z,
        }
        let _ = up;
        // Native minimum-camera gate precedes the posture casts.
        if position.distance_squared(origin) < self.prepared.decl.minimum_camera_distance.powi(2) {
            return None;
        }
        let delta = position - Vec3::new(origin.x, origin.y, physics.position().z);
        let distance = delta.length();
        let travel =
            delta.normalize_or_zero() * (distance - physics.posture_settings(8).extrude_length);
        let from = Vec3::new(origin.x, origin.y, physics.position().z);
        let mut free = [false; 3];
        for (slot, clear) in free.iter_mut().enumerate() {
            *clear = self.travel_clear(
                world,
                physics.posture_body(slot + 8),
                physics.axis(),
                from,
                travel,
                target.hit.body,
            );
        }
        let order = if sneaking {
            [1, 0, 2]
        } else if crouching {
            [2, 1, 0]
        } else {
            [0, 1, 2]
        };
        let posture = order.into_iter().find(|slot| free[*slot])? + 8;
        let safe = self.state_safe_position(
            world,
            physics,
            physics.position(),
            position,
            normal,
            posture,
            physics.position(),
            0.0,
        );
        let posture = safe.posture;
        let safe_position = safe.position;
        if safe_position == physics.position() {
            return None;
        }
        let can_jump = {
            let prepared = self.prepared.clone();
            self.travel_clear(
                world,
                &prepared.shapes[6],
                Mat3::IDENTITY,
                from + Vec3::Z * physics.posture_settings(8).z_offset,
                travel,
                target.hit.body,
            )
        };
        target.position = position;
        Some(GeometryTarget {
            position,
            normal,
            safe_position,
            posture,
            snapped_ground,
            can_jump,
            hit: target.hit,
        })
    }
    /// 1403b2000 checks the actual player posture shape at the candidate
    /// safe position, with source collisionLayers and required flag0x40.
    pub fn posture_clear(
        &mut self,
        world: &dyn PlayerWorld,
        physics: &PlayerPhysics,
        posture: usize,
        position: Vec3,
    ) -> bool {
        self.hits.clear();
        world.cast_shape(
            QueryFilter::layers(self.prepared.decl.collision_layers, 0x40),
            physics.posture_body(posture),
            physics.axis(),
            position,
            Vec3::ZERO,
            0.0,
            &mut self.hits,
            None,
        );
        self.hits.is_empty()
    }
    /// Actual forward/reverse casts for 1403b3450's safe-interval scan.
    /// `vertical_offset` is the admitted target posture's z offset supplied
    /// by the existing physics shape owner.
    pub fn safe_position(
        &mut self,
        world: &dyn PlayerWorld,
        physics: &PlayerPhysics,
        target_position: Vec3,
        target_normal: Vec3,
        posture: usize,
        vertical_offset: f32,
    ) -> Vec3 {
        let limit = f32::from_bits(0x3f35_04f2);
        let adjustment = if target_normal.z > limit {
            0.01
        } else if target_normal.z < -limit {
            -0.01
        } else {
            0.0
        };
        let origin = physics.position() + Vec3::Z * (vertical_offset + adjustment);
        let endpoint = target_position + Vec3::Z * (vertical_offset + adjustment);
        let delta = endpoint - origin;
        let length = delta.length();
        let direction = delta.normalize_or_zero();
        self.crossings.clear();
        let filter = QueryFilter::layers(self.prepared.decl.collision_layers, 0x40);
        for reverse in [false, true] {
            self.hits.clear();
            world.cast_shape(
                filter,
                physics.posture_body(physics.posture),
                physics.axis(),
                if reverse { endpoint } else { origin },
                if reverse { -delta } else { delta },
                0.0,
                &mut self.hits,
                None,
            );
            self.crossings.extend(self.hits.iter().map(|hit| Crossing {
                distance: if reverse {
                    length - hit.value * length
                } else {
                    hit.value * length
                },
                // arkGrappleSafePosCollector1403b77a0 writes +/-1 by
                // query direction; it deliberately ignores hit normals.
                normal_dot_direction: if reverse { -1.0 } else { 1.0 },
            }));
        }
        // Borrow the already allocated scratch separately from crossing
        // storage; every candidate uses the same canonical posture shape.
        let hits = &mut self.hits;
        let result = safe_position(origin, direction, length, &mut self.crossings, |position| {
            hits.clear();
            world.cast_shape(
                filter,
                physics.posture_body(posture),
                physics.axis(),
                position,
                Vec3::ZERO,
                0.0,
                hits,
                None,
            );
            hits.is_empty()
        });
        result - Vec3::Z * (vertical_offset + adjustment)
    }
}

fn cast_target(
    decl: &SensorDecl,
    shape: &[Vec3],
    hits: &mut Vec<WorldHit>,
    world: &dyn PlayerWorld,
    origin: Vec3,
    axis: Mat3,
    direction: Vec3,
    length: f32,
    extra_layers: u32,
    mut material: impl FnMut(&WorldHit) -> Option<Material>,
) -> Option<TargetHit> {
    hits.clear();
    world.cast_shape(
        QueryFilter::layers(decl.collision_layers | extra_layers, 0x40),
        shape,
        axis,
        origin,
        direction * length,
        0.0,
        hits,
        None,
    );
    let mut nearest = None;
    let mut distance = length + 0.001;
    let mut targetable = false;
    for hit in hits {
        let info = material(hit);
        if info.is_some_and(|value| value.through) {
            continue;
        }
        let candidate = hit.value * length;
        if candidate < distance {
            distance = candidate;
            nearest = Some(*hit);
            targetable = info.is_none_or(|value| value.blocks_vision);
        }
    }
    if !targetable {
        return None;
    }
    let hit = nearest?;
    Some(TargetHit {
        hit,
        position: hit.position + hit.normal * decl.shape_radius,
        distance,
    })
}

/// arkTarget_t as consumed by 140839b80. Actor catching is a separate
/// target family and must not be represented by a world point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Wall,
    Ceil,
    Ground,
}

impl Target {
    /// 140837a40 uses DAT141ae8a84, not the player's walkable slope.
    pub fn from_normal(normal: Vec3) -> Self {
        let limit = f32::from_bits(0x3f35_04f2);
        if normal.z > limit {
            Self::Ground
        } else if normal.z < -limit {
            Self::Ceil
        } else {
            Self::Wall
        }
    }
}

/// A signed surface crossing along a forward safe-position sweep. Reverse
/// query distances are transformed to the same coordinate before insertion.
#[derive(Clone, Copy, Debug)]
pub struct Crossing {
    pub distance: f32,
    pub normal_dot_direction: f32,
}

/// 1403b26d0 + the free-interval scan in 1403b3450. `clear` runs the real
/// player posture overlap query (1403b2000); an interval never substitutes
/// for that check. Both query directions must have been collected first.
pub fn safe_position(
    start: Vec3,
    direction: Vec3,
    length: f32,
    crossings: &mut Vec<Crossing>,
    mut clear: impl FnMut(Vec3) -> bool,
) -> Vec3 {
    crossings.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    let mut index = 0;
    while index + 1 < crossings.len() {
        let a = crossings[index].normal_dot_direction;
        let b = crossings[index + 1].normal_dot_direction;
        if a > 0.0 && b > 0.0 {
            crossings.remove(index + 1);
        } else if a < 0.0 && b < 0.0 {
            crossings.remove(index);
            index = index.saturating_sub(1);
        } else {
            index += 1;
        }
    }
    let keep_distance = 0.01; // DAT141ab3810, used by 1403b3450.
    if crossings.is_empty() {
        return start + direction * length;
    }
    let last = crossings.last().unwrap();
    if length - last.distance > keep_distance
        && last.normal_dot_direction < 0.0
        && clear(start + direction * (length - keep_distance))
    {
        return start + direction * length;
    }
    for pair in crossings.windows(2).rev() {
        if pair[1].distance - pair[0].distance > keep_distance
            && pair[0].normal_dot_direction < 0.0
            && pair[1].normal_dot_direction > 0.0
            && clear(start + direction * (pair[1].distance - keep_distance))
        {
            return start + direction * pair[1].distance;
        }
    }
    let first = &crossings[0];
    if first.distance > keep_distance
        && first.normal_dot_direction > 0.0
        && clear(start + direction * (first.distance - keep_distance))
    {
        start + direction * first.distance
    } else {
        start
    }
}

#[derive(Clone, Debug)]
pub struct Motion {
    pub target: Target,
    pub safe_position: Vec3,
    pub velocity: Vec3,
    pub direction: Vec3,
    pub distance_blend_ratio: f32,
    pub camera_blend_ratio: f32,
    pub goal_reached: bool,
    distance: f32,
    start_speed: f32,
    end_speed: f32,
    inertia: f32,
    inertia_blend_timer: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct JumpInput {
    pub position: Vec3,
    pub target_position: Vec3,
    pub velocity: Vec3,
    pub player_forward: Vec3,
    pub camera_direction: Vec3,
    pub was_sneaking: bool,
    pub snapped_ground: bool,
    pub to_mantle: bool,
    pub can_jump: bool,
    pub initial_target: Target,
}
#[derive(Clone, Copy, Debug)]
pub struct JumpSetup {
    pub can_jump: bool,
    pub vertical: Option<(f32, f32, f32)>,
    pub extra_horizontal_speed: f32,
}
/// Ground-travel initialization in140839b80. Native call14083a100 passes
/// the chosen vertical goal in XMM1, speed XMM2, steepness XMM3; XMM0 is
/// unrelated scratch and is not a fourth physics argument.
pub fn jump_setup(decl: &Decl, input: JumpInput) -> JumpSetup {
    let horizontal_velocity = Vec3::new(input.velocity.x, input.velocity.y, 0.0);
    let speed = horizontal_velocity.dot(input.player_forward);
    let minimum_goal = input.position.z + decl.jump_height;
    let can_jump = (input.can_jump
        && ((!input.was_sneaking && speed > decl.min_speed_to_jump_on_ground)
            || (input.to_mantle && minimum_goal <= input.target_position.z)
            || input.snapped_ground))
        || input.initial_target == Target::Ceil;
    if !can_jump {
        return JumpSetup {
            can_jump: false,
            vertical: None,
            extra_horizontal_speed: 0.0,
        };
    }
    let (ratio, target_z) = if input.snapped_ground {
        let ratio = input.camera_direction.z;
        (
            ratio,
            input.target_position.z
                + (1.0 - ratio) * decl.min_jump_height_snapped_to_ground
                + ratio * decl.max_jump_height_snapped_to_ground,
        )
    } else {
        (
            (input.target_position - input.position)
                .normalize_or_zero()
                .z
                .max(0.0),
            input.target_position.z,
        )
    };
    let mut goal = minimum_goal;
    let max_speed = if minimum_goal <= target_z
        || speed <= decl.min_speed_to_jump_on_ground
        || input.to_mantle
        || input.snapped_ground
    {
        goal = if minimum_goal <= target_z {
            target_z
        } else {
            minimum_goal
        };
        ratio * decl.extra_jump_velocity
    } else {
        decl.ground_jump_velocity
    };
    let gain = decl.min_jump_steepness.max(ratio.min(1.0));
    let extra_horizontal_speed = if speed > decl.min_speed_to_jump_on_ground && !input.to_mantle {
        speed - decl.min_speed_to_jump_on_ground
    } else {
        0.0
    };
    JumpSetup {
        can_jump: true,
        vertical: Some((goal, max_speed, gain)),
        extra_horizontal_speed,
    }
}
impl Motion {
    pub fn set_reference_distance(&mut self, distance: f32) {
        self.distance = distance;
    }
    pub fn add_jump_speed(&mut self, speed: f32) {
        self.start_speed += speed;
        self.end_speed += speed;
    }
    /// 140839b80 selects speed pairs after the sensor and jump/mantle
    /// owners have made their real decisions.
    pub fn new(
        decl: &Decl,
        target: Target,
        start: Vec3,
        safe_position: Vec3,
        velocity: Vec3,
        to_mantle: bool,
        can_jump: bool,
    ) -> Self {
        let (start_speed, end_speed) = match (target, to_mantle, can_jump) {
            (Target::Wall, _, _) => (decl.start_speed_to_wall, decl.end_speed_to_wall),
            (Target::Ceil, _, _) => (decl.start_speed_to_ceil, decl.end_speed_to_ceil),
            (Target::Ground, true, true) => (decl.start_speed_to_mantle, decl.end_speed_to_mantle),
            (Target::Ground, true, false) => (
                decl.start_speed_to_mantle_no_jump,
                decl.end_speed_to_mantle_no_jump,
            ),
            (Target::Ground, false, true) => (decl.start_speed_to_ground, decl.end_speed_to_ground),
            (Target::Ground, false, false) => (
                decl.start_speed_to_ground_no_jump,
                decl.end_speed_to_ground_no_jump,
            ),
        };
        Self {
            target,
            safe_position,
            velocity,
            direction: Vec3::ZERO,
            distance_blend_ratio: 0.0,
            camera_blend_ratio: 0.0,
            goal_reached: false,
            distance: start.distance(safe_position),
            start_speed,
            end_speed,
            inertia: decl.start_inertia,
            inertia_blend_timer: decl.max_inertia_blend_duration,
        }
    }
    /// 140838d90, excluding the camera and proxy setters which belong to
    /// their established owners. Returns the relative travel velocity;
    /// caller adds the actual target's world velocity exactly once.
    pub fn interpolate_to_target(
        &mut self,
        decl: &Decl,
        position: Vec3,
        dt: f32,
        jump_to_target: bool,
        to_mantle: bool,
    ) -> Vec3 {
        if !self.goal_reached {
            let mut delta = self.safe_position - position;
            if jump_to_target {
                delta.z = 0.0;
            }
            let remaining = delta.length();
            self.direction = delta.normalize_or_zero();
            self.distance_blend_ratio = if self.distance > f32::EPSILON {
                self.distance_blend_ratio
                    .max(1.0 - (remaining / self.distance).clamp(0.0, 1.0))
            } else {
                1.0
            };
            let ratio = self.distance_blend_ratio;
            let speed = (1.0 - ratio) * self.start_speed + ratio * self.end_speed;
            if remaining <= dt * speed {
                self.goal_reached = true;
                self.distance_blend_ratio = 1.0;
                self.velocity = if self.target == Target::Ground && !to_mantle {
                    self.direction * self.end_speed
                } else {
                    Vec3::ZERO
                };
            } else {
                let desired = self.direction * speed;
                let change = desired - self.velocity;
                // Ground travel intentionally bypasses the wall/ceiling
                // inertia limiter in the executable.
                self.velocity = if self.target != Target::Ground
                    && change.length_squared() > (dt * self.inertia).powi(2)
                {
                    self.velocity + change.normalize_or_zero() * (dt * self.inertia)
                } else {
                    desired
                };
            }
            let elapsed_ratio = if decl.max_inertia_blend_duration > 0.0 {
                1.0 - self.inertia_blend_timer / decl.max_inertia_blend_duration
            } else {
                1.0
            };
            let blend = self.distance_blend_ratio.max(elapsed_ratio);
            self.inertia = (1.0 - blend) * decl.start_inertia + blend * decl.end_inertia;
            self.inertia_blend_timer = (self.inertia_blend_timer - dt).max(0.0);
        }
        if self.target == Target::Wall {
            let span = decl.camera_full_turn_position - decl.camera_start_turn_position;
            let ratio = if span > 0.0 {
                (self.distance_blend_ratio - decl.camera_start_turn_position) / span
            } else {
                1.0
            };
            self.camera_blend_ratio = self.camera_blend_ratio.max(ratio).min(1.0);
        }
        self.velocity
    }
}
