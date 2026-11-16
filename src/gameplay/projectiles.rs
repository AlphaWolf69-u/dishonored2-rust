//! arkCpntProjectile finite flight (629590), surface response selection
//! (624300/6262d0), and hit parameter construction (624610).
use crate::{
    assets::projectiles::Declaration,
    gameplay::{damage::Parameters, entity_lifetime::EntityId},
};
use bevy_math::{Mat3, Vec3};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default)]
pub struct Flags {
    pub disable_roll: bool,
    pub can_pin_limbs: bool,
    pub ignore_armor: bool,
    pub ignore_knocked_off_armor: bool,
    pub pass_through_npcs: bool,
    pub pass_through_swarm_agents: bool,
    pub electrified_damage: bool,
    pub bounce_once_instead_of_breaking: bool,
    pub bypass_big_hit_immunity: bool,
    pub always_break_on_hit: bool,
    pub disable_trail: bool,
    pub always_stick_to_npcs: bool,
}
#[derive(Clone, Debug)]
pub struct HomingTarget {
    pub entity: EntityId,
    pub tag: Option<Arc<str>>,
    pub offset: Vec3,
}
#[derive(Clone, Debug)]
pub struct SpawnRequest {
    pub attacker: Option<EntityId>,
    pub equipment: Option<EntityId>,
    pub frame: crate::gameplay::reference_frame::ReferenceFrame,
    pub velocity: Vec3,
    /// None denotes the constructor's -1 sentinel; an authored attribute
    /// supplies its actual factor before Initialize626eb0 multiplies it.
    pub gravity_multiplier: Option<f32>,
    pub damage_increment: f32,
    pub sleep_timer_multiplier: f32,
    pub flags: Flags,
    pub homing_target: Option<HomingTarget>,
    pub explosion_fx_override: Option<Arc<str>>,
}

#[derive(Clone, Copy, Debug)]
pub struct RenderState {
    pub hidden: bool,
    pub current: bool,
    pub previous: bool,
}

/// removeWhenHidden877140 is a delayed callback with a finite retry count,
/// not an unbounded "wait until invisible" lifespan or a frame countdown.
#[derive(Clone, Copy, Debug)]
pub struct Removal {
    pub deadline: f64,
    pub retry_delay: f32,
    pub retries: i32,
}
impl Removal {
    pub fn after_hit(declaration: &Declaration, now: f64) -> Self {
        Self {
            deadline: now + f64::from(declaration.lifespan_after_hit),
            retry_delay: declaration.lifespan_if_visible,
            retries: 10,
        }
    }
    pub fn poll(&mut self, now: f64, model: Option<RenderState>) -> bool {
        if now < self.deadline {
            return false;
        }
        if self.retries != 0
            && model.is_some_and(|model| !model.hidden && (model.current || model.previous))
        {
            self.retries = if self.retries < 1 {
                -1
            } else {
                self.retries - 1
            };
            self.deadline = now + f64::from(self.retry_delay);
            false
        } else {
            true
        }
    }
}

/// The real projectile entity owns this state from successful preparation to
/// shared entity retirement. It is not a weapon ray or a second physics world.
pub struct Projectile {
    pub declaration: Arc<Declaration>,
    pub identity: EntityId,
    pub attacker: Option<EntityId>,
    pub origin: Vec3,
    pub position: Vec3,
    pub axis: Mat3,
    pub velocity: Vec3,
    pub maximum_speed: f32,
    pub gravity_multiplier: f32,
    pub damage_increment: f32,
    pub movement_enabled: bool,
    pub flags: Flags,
    pub sleep_timer_multiplier: f32,
    pub homing_target: Option<HomingTarget>,
    pub explosion_fx_override: Option<Arc<str>>,
    pub forward_extent: f32,
    pub unbent_distance: f32,
    pub has_hit_and_stopped: bool,
    /// Source+a5 bit80, distinct from a Stick/Break response.629590 sets
    ///it for the impact-stopping (explosion FX) contact mode;626ff0 also
    ///sets it when an external deflection transfers motion ownership.
    pub impact_stopped: bool,
    /// Source+a4 bit20:627110 enables repeated contact processing for NPC
    ///launchers or a declaration without m_explosionFx (+180).
    pub process_all_contacts: bool,
    pub picked_up: bool,
    pub pass_through_entities: Vec<EntityId>,
}
impl Projectile {
    /// 626eb0 preserves an explicitly supplied velocity; only zero velocity
    /// initializes from authored speed and the entity's actual forward axis.
    /// 639970 applies the weapon's upgrade speed factor before this callback.
    pub fn start(
        declaration: Arc<Declaration>,
        identity: EntityId,
        attacker: Option<EntityId>,
        position: Vec3,
        axis: Mat3,
        velocity: Vec3,
        gravity_factor: Option<f32>,
    ) -> Self {
        let velocity = if velocity == Vec3::ZERO {
            axis.x_axis * declaration.linear_velocity
        } else {
            velocity
        };
        let gravity_multiplier = gravity_factor.unwrap_or(1.) * declaration.gravity_multiplier;
        let process_all_contacts = declaration.effects.explosion.is_none();
        Self {
            declaration,
            identity,
            attacker,
            origin: position,
            position,
            axis,
            velocity,
            maximum_speed: f32::from_bits(0x7149f2ca),
            gravity_multiplier,
            damage_increment: 0.,
            movement_enabled: true,
            flags: Flags::default(),
            sleep_timer_multiplier: 1.,
            homing_target: None,
            explosion_fx_override: None,
            forward_extent: 0.,
            unbent_distance: 0.,
            has_hit_and_stopped: false,
            impact_stopped: false,
            process_all_contacts,
            picked_up: false,
            pass_through_entities: Vec::new(),
        }
    }

    pub fn from_request(
        declaration: Arc<Declaration>,
        identity: EntityId,
        request: SpawnRequest,
        forward_extent: f32,
    ) -> Self {
        let mut projectile = Self::start(
            declaration,
            identity,
            request.attacker,
            Vec3::from_array(request.frame.position),
            Mat3::from_cols_array_2d(&request.frame.basis),
            request.velocity,
            request.gravity_multiplier,
        );
        projectile.damage_increment = request.damage_increment;
        projectile.sleep_timer_multiplier = request.sleep_timer_multiplier;
        projectile.flags = request.flags;
        projectile.homing_target = request.homing_target;
        projectile.explosion_fx_override = request.explosion_fx_override;
        projectile.forward_extent = forward_extent;
        projectile
    }

    /// 6291a0 rotates the actual velocity toward a resolved live target only
    /// while that target is in the projectile's forward hemisphere.
    pub fn home(&mut self, dt: f32, target: Option<Vec3>) {
        let Some(target) = target else {
            return;
        };
        let toward = target - self.position;
        let toward = toward / toward.length_squared().max(f32::MIN_POSITIVE).sqrt();
        let current = self.velocity / self.velocity.length_squared().max(f32::MIN_POSITIVE).sqrt();
        let dot = current.dot(toward);
        if dot <= 0. {
            return;
        }
        let angle = dot.clamp(-1., 1.).acos();
        let angle =
            angle.min(self.declaration.maximum_homing_angle * dt * f32::from_bits(0x3c8efa35));
        let axis = current.cross(toward);
        if angle == 0. || axis.length_squared() <= f32::MIN_POSITIVE {
            return;
        }
        //111cf0 uses target×current with row-vector rotation. Bevy's
        //column-vector quaternion uses the opposite axis for that same turn.
        self.velocity = bevy_math::Quat::from_axis_angle(axis.normalize(), angle) * self.velocity;
    }

    /// 629590 stops the model's forward extent short of the witness point,
    /// capped by distance from its original spawn position.
    pub fn impact_position(&self, hit: Hit) -> Vec3 {
        hit.position - hit.direction * hit.position.distance(self.origin).min(self.forward_extent)
    }

    /// Predict the finite segment for this entity's actual time-space delta.
    /// The caller resolves that segment against canonical body/shape owners
    /// before publishing the accepted frame. Entity coordinates are Z-up,
    /// matching player/equipment frames and canonical WorldStorage; a Bevy
    /// collision adapter performs its ordinary conversion at the boundary.
    pub fn flight(&self, dt: f32, world_gravity: Vec3) -> Option<Flight> {
        if !self.movement_enabled || self.impact_stopped || dt == 0. {
            return None;
        }
        let mut velocity = self.velocity;
        // 628d40 clamps speed before computing translation; the stored final
        // velocity retains the unclamped input plus this step's gravity.
        let speed_squared = velocity.length_squared();
        if self.maximum_speed == 0. {
            velocity = Vec3::ZERO;
        } else if self.maximum_speed * self.maximum_speed < speed_squared {
            velocity *= self.maximum_speed / speed_squared.sqrt();
        }
        let gravity = Vec3::Z * (world_gravity.z * self.gravity_multiplier);
        let end = self.position + velocity * dt + gravity * (dt * dt * 0.5);
        Some(Flight {
            start: self.position,
            end,
            velocity: self.velocity + gravity * dt,
            out_of_range: self.declaration.maximum_range > 0.
                && end.distance_squared(self.origin)
                    > self.declaration.maximum_range * self.declaration.maximum_range,
        })
    }

    ///110210/629590 preserve current roll, add authored degrees per second,
    ///and align the forward axis with this actual flight segment.
    pub fn orient_for_segment(&mut self, dt: f32, direction: Vec3) {
        let forward = self.axis.x_axis;
        let roll = if (forward.x * forward.x + forward.y * forward.y).sqrt() > f32::EPSILON {
            self.axis.y_axis.z.atan2(self.axis.z_axis.z) * f32::from_bits(0x42652ee0)
        } else {
            0.
        };
        let roll = (roll
            + if self.flags.disable_roll {
                0.
            } else {
                dt * self.declaration.roll_speed
            })
            * f32::from_bits(0x3c8efa35);
        self.axis = crate::gameplay::reference_frame::direction_basis(direction)
            * Mat3::from_quat(bevy_math::Quat::from_rotation_x(roll));
    }

    pub fn publish_flight(&mut self, flight: Flight) {
        self.position = flight.end;
        self.velocity = flight.velocity;
    }

    /// The caller supplies the victim's real class exemption and actual live
    /// inflicted-damage attribute. No NPC/class/armor state is inferred here.
    pub fn hit_parameters(
        &self,
        hit: Hit,
        damage_factor: f32,
        falloff_exempt: bool,
    ) -> Option<Parameters> {
        let declaration = self.declaration.damage.as_ref()?;
        let mut fraction = 1.;
        let distance = hit.position.distance(self.origin);
        if !falloff_exempt && self.declaration.falloff_minimum < self.declaration.falloff_maximum {
            fraction = (1.
                - (distance - self.declaration.falloff_minimum)
                    / (self.declaration.falloff_maximum - self.declaration.falloff_minimum))
                .clamp(0., 1.);
        }
        let mut maximum = (declaration.maximum + self.damage_increment) * damage_factor;
        let mut minimum = declaration.minimum.min(maximum);
        let epsilon = f32::from_bits(0x219392ef); // 141ab2294
        if maximum.abs() <= epsilon {
            maximum = 0.;
        }
        if minimum.abs() <= epsilon {
            minimum = 0.;
        }
        let mut parameters = Parameters::from_declaration(declaration);
        parameters.set_amount(
            declaration,
            (1. - fraction) * minimum + maximum * fraction,
            false,
        );
        parameters.hit_location = Some(hit.position);
        parameters.hit_normal = Some(hit.normal);
        parameters.hit_direction = hit.direction;
        parameters.hit_region = hit.region;
        parameters.affected_regions = 1u32.checked_shl(hit.region).unwrap_or(0);
        parameters.hit_distance = distance;
        parameters.attacker = self.attacker;
        parameters.inflictor = Some(self.identity);
        parameters.ignore_armor = self.flags.ignore_armor;
        Some(parameters)
    }
}

#[derive(Clone, Copy)]
pub struct Flight {
    pub start: Vec3,
    pub end: Vec3,
    pub velocity: Vec3,
    pub out_of_range: bool,
}
#[derive(Clone, Copy)]
pub struct Hit {
    pub position: Vec3,
    pub normal: Vec3,
    pub direction: Vec3,
    pub region: u32,
}

/// 6262d0 uses asin(-normal dot velocity), so a head-on hit is 90 degrees.
pub fn incidence_angle(normal: Vec3, velocity: Vec3) -> f32 {
    let direction = velocity / velocity.length_squared().max(f32::MIN_POSITIVE).sqrt();
    (-normal.dot(direction)).clamp(-1., 1.).asin() * f32::from_bits(0x42652ee0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response {
    None,
    BounceOnce,
    Stick,
    Break,
    UncannyDeflect,
    Bounce,
}

/// Real contact facts gathered by the collision/component owners in 6262d0.
/// All special state is explicit rather than guessed from body names.
pub struct Contact {
    pub player: bool,
    pub npc: bool,
    pub world: bool,
    pub can_stick: bool,
    pub can_break: bool,
    pub angle: f32,
    pub active_water_trigger: bool,
    pub absorbed_by_armor: bool,
    pub desired_deflection: u32,
    pub npc_break_probability: Option<f32>,
}
pub struct ContactFlags {
    pub always_break_on_hit: bool,
    pub always_stick_to_npcs: bool,
    pub pinned_limb: bool,
    pub bounce_once: bool,
}
/// 624300. `random` is the game's actual random draw in [0,1); this function
/// does not create an independent RNG or consume a draw on a non-break path.
pub fn response(
    declaration: &Declaration,
    contact: &Contact,
    flags: &mut ContactFlags,
    mut random: impl FnMut() -> f32,
) -> Response {
    let mut result = Response::Break;
    if !contact.absorbed_by_armor {
        if contact.desired_deflection == 0 {
            if !contact.player {
                result = Response::Bounce;
                let required = if contact.npc {
                    declaration.required_npc_stick_angle
                } else {
                    declaration.required_stick_angle
                };
                if contact.can_stick && contact.angle >= required && !contact.active_water_trigger {
                    result = Response::Stick;
                }
                if contact.can_break && declaration.can_break {
                    let unbroken = result;
                    if flags.always_break_on_hit {
                        result = Response::Break;
                    }
                    if !flags.always_break_on_hit || contact.npc && flags.always_stick_to_npcs {
                        let chance = if !contact.can_stick {
                            if contact.angle < declaration.bounce_angle {
                                declaration.break_chance_not_stickable
                            } else {
                                100.
                            }
                        } else if contact.npc {
                            contact
                                .npc_break_probability
                                .unwrap_or(declaration.break_chance_npc)
                        } else {
                            declaration.break_chance_stickable
                        };
                        result = if random() < chance * 0.01 {
                            Response::Break
                        } else {
                            unbroken
                        };
                    }
                }
            }
        } else if contact.desired_deflection == 3 {
            result = Response::None;
        } else if contact.desired_deflection == 4 {
            result = Response::UncannyDeflect;
        }
    }
    if flags.pinned_limb && (!contact.world || result != Response::Stick) {
        result = Response::Break;
    }
    if flags.bounce_once
        && !contact.can_stick
        && result == Response::Break
        && !contact.player
        && (!contact.npc || !contact.absorbed_by_armor)
    {
        flags.bounce_once = false;
        result = Response::BounceOnce;
    }
    result
}

/// Contact result publication629590. A normal bounce relinquishes the finite
/// reference-frame owner to the SAME Throwable body; BounceOnce keeps it.
#[derive(Clone, Copy, Debug)]
pub enum Handoff {
    None,
    Throwable,
    Stick,
}
#[derive(Clone, Copy, Debug)]
pub struct ImpactMotion {
    pub handoff: Handoff,
    pub impulse: Option<Vec3>,
    pub hidden: bool,
}
impl Projectile {
    pub fn frame(&self) -> crate::gameplay::reference_frame::ReferenceFrame {
        crate::gameplay::reference_frame::ReferenceFrame {
            position: self.position.to_array(),
            basis: self.axis.to_cols_array_2d(),
        }
    }
    ///629590 uses the velocity AFTER this step's gravity when determining
    ///the bounce speed, while reflection uses the actual incoming cast ray.
    pub fn apply_response(
        &mut self,
        response: Response,
        hit: Hit,
        final_velocity: Vec3,
        mut random: impl FnMut() -> f32,
    ) -> ImpactMotion {
        let backed_off = self.impact_position(hit);
        self.position = backed_off;
        self.velocity = final_velocity;
        let mut result = ImpactMotion {
            handoff: Handoff::None,
            impulse: None,
            hidden: false,
        };
        match response {
            Response::BounceOnce | Response::Bounce => {
                let reflected = hit.direction - 2. * hit.normal.dot(hit.direction) * hit.normal;
                let speed = final_velocity.length();
                let separation = f32::from_bits(0x3d4ccccd); //141ab3814
                if response == Response::BounceOnce {
                    self.position = hit.position + reflected * separation;
                    // The incoming roll is preserved relative to its +X frame.
                    let incoming = crate::gameplay::reference_frame::direction_basis(hit.direction);
                    let roll = incoming.transpose() * self.axis;
                    self.axis = crate::gameplay::reference_frame::direction_basis(reflected) * roll;
                    self.velocity = reflected * speed;
                    self.flags.bounce_once_instead_of_breaking = false;
                } else {
                    let factor = 1. - (incidence_angle(hit.normal, hit.direction) / 90.).abs();
                    self.velocity = reflected
                        * (factor * factor * self.declaration.side_hit_restitution * 0.01 * speed);
                    self.position = backed_off - hit.direction * separation;
                    let magnitude = speed * self.declaration.bounce_impulse_factor;
                    let angle = random()
                        * self.declaration.bounce_impulse_random_angle
                        * f32::from_bits(0x3c8efa35);
                    let local = Vec3::new(angle.cos() * magnitude, angle.sin() * magnitude, 0.);
                    let roll =
                        bevy_math::Quat::from_rotation_x(random() * f32::from_bits(0x40c90fdb));
                    result.impulse = Some(
                        crate::gameplay::reference_frame::direction_basis(hit.normal)
                            * (roll * local),
                    );
                    result.handoff = Handoff::Throwable;
                    self.movement_enabled = false;
                }
            }
            Response::Stick => {
                self.movement_enabled = false;
                self.has_hit_and_stopped = true;
                result.handoff = Handoff::Stick;
            }
            Response::Break | Response::UncannyDeflect => {
                self.movement_enabled = false;
                self.has_hit_and_stopped = true;
                result.hidden = true;
            }
            Response::None => {}
        }
        result
    }
    ///627f60 authored sink depth, before the target's actual shape/attachment
    ///owner resolves the optional geometric correction and relative binding.
    pub fn stick_frame(
        &self,
        hit: Hit,
        axis: Mat3,
        npc: bool,
        pinned_limb: bool,
        random: f32,
    ) -> crate::gameplay::reference_frame::ReferenceFrame {
        let settings = &self.declaration.stick;
        let mut depth = (if npc {
            settings.sink_npc
        } else {
            settings.sink
        }) + random * settings.sink_random_range;
        depth = depth.max(settings.minimum_sink);
        if pinned_limb {
            depth = settings.sink_pinned_limb;
        }
        crate::gameplay::reference_frame::ReferenceFrame {
            position: (hit.position + axis.x_axis * (depth - self.forward_extent)).to_array(),
            basis: axis.to_cols_array_2d(),
        }
    }
}

fn wrap_angle(mut angle: f32) -> f32 {
    if angle >= 360. || angle < 0. {
        angle -= (angle / 360.).floor() * 360.;
    }
    if angle > 180. {
        angle -= 360.;
    }
    angle
}
///624930 checks the opposite 180-degree orientation before choosing the
///nearest circular endpoint, retaining the original tie choice (maximum).
fn clamp_stick_angle(angle: f32, minimum: f32, maximum: f32) -> f32 {
    if angle >= minimum && angle <= maximum {
        return angle;
    }
    let alternative = wrap_angle(angle + 180.);
    if alternative >= minimum && alternative <= maximum {
        return alternative;
    }
    let reference = if angle.abs() > 90. {
        alternative
    } else {
        angle
    };
    if wrap_angle(minimum - reference).abs() < wrap_angle(maximum - reference).abs() {
        minimum
    } else {
        maximum
    }
}
impl Projectile {
    ///624b90/625cb0 project the incoming transverse axes onto the actual
    ///contact plane, then apply authored pitch/yaw constraints in that frame.
    pub fn sticking_axis(&self, normal: Vec3, mut random: impl FnMut() -> f32) -> Mat3 {
        let incoming = self.axis;
        let project = |v: Vec3| v - normal * v.dot(normal);
        let left = project(incoming.y_axis);
        let up = project(incoming.z_axis);
        let forward = -normal;
        let plane = if left.length_squared() > f32::from_bits(0x38d1b717) {
            let left = left / left.length_squared().max(f32::MIN_POSITIVE).sqrt();
            Mat3::from_cols(forward, left, forward.cross(left))
        } else {
            let up = up / up.length_squared().max(f32::MIN_POSITIVE).sqrt();
            Mat3::from_cols(forward, up.cross(forward), up)
        };
        let settings = &self.declaration.stick;
        if settings.minimum_yaw == 0.
            && settings.maximum_yaw == 0.
            && settings.minimum_pitch == 0.
            && settings.maximum_pitch == 0.
        {
            return incoming;
        }
        let relative = plane.transpose() * incoming;
        let a = relative.to_cols_array();
        let horizontal = (a[0] * a[0] + a[1] * a[1]).sqrt();
        let degrees = f32::from_bits(0x42652ee0);
        let (mut pitch, mut yaw, roll) = if horizontal <= f32::EPSILON {
            (
                if a[2] < 0. { 90. } else { -90. },
                -a[3].atan2(a[4]) * degrees,
                0.,
            )
        } else {
            (
                -a[2].atan2(horizontal) * degrees,
                a[1].atan2(a[0]) * degrees,
                a[5].atan2(a[8]) * degrees,
            )
        };
        let adjust =
            |angle: f32, min: f32, max: f32, randomize: bool, random: &mut dyn FnMut() -> f32| {
                if angle < min || angle > max {
                    if randomize {
                        min + random() * (max - min)
                    } else {
                        clamp_stick_angle(angle, min, max)
                    }
                } else {
                    angle
                }
            };
        yaw = adjust(
            yaw,
            settings.minimum_yaw,
            settings.maximum_yaw,
            settings.random_yaw,
            &mut random,
        );
        pitch = -adjust(
            -pitch,
            settings.minimum_pitch,
            settings.maximum_pitch,
            settings.random_pitch,
            &mut random,
        );
        let radians = f32::from_bits(0x3c8efa35);
        let (sy, cy) = (yaw * radians).sin_cos();
        let (sp, cp) = (pitch * radians).sin_cos();
        let (sr, cr) = (roll * radians).sin_cos();
        plane
            * Mat3::from_cols(
                Vec3::new(cp * cy, cp * sy, -sp),
                Vec3::new(sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, sr * cp),
                Vec3::new(cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp),
            )
    }
}
