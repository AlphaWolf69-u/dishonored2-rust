//! arkCpntProjectile finite flight (629590), surface response selection
//! (624300/6262d0), and hit parameter construction (624610).
use crate::{assets::projectiles::Declaration, gameplay::{damage::Parameters, entity_lifetime::EntityId}};
use bevy_math::{Mat3, Vec3};
use std::sync::Arc;

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
}
impl Projectile {
    /// 626eb0 preserves an explicitly supplied velocity; only zero velocity
    /// initializes from authored speed and the entity's actual forward axis.
    /// 639970 applies the weapon's upgrade speed factor before this callback.
    pub fn start(declaration: Arc<Declaration>, identity: EntityId, attacker: Option<EntityId>,
        position: Vec3, axis: Mat3, velocity: Vec3, gravity_factor: Option<f32>) -> Self {
        let velocity = if velocity == Vec3::ZERO { axis.x_axis * declaration.linear_velocity } else { velocity };
        let gravity_multiplier = gravity_factor.unwrap_or(1.) * declaration.gravity_multiplier;
        Self { declaration, identity, attacker, origin: position, position, axis, velocity,
            maximum_speed: f32::from_bits(0x7149f2ca), gravity_multiplier,
            damage_increment: 0., movement_enabled: true }
    }

    /// Predict the finite segment for this entity's actual time-space delta.
    /// The caller resolves that segment against canonical body/shape owners
    /// before publishing the accepted frame. Entity coordinates are Z-up,
    /// matching player/equipment frames and canonical WorldStorage; a Bevy
    /// collision adapter performs its ordinary conversion at the boundary.
    pub fn flight(&self, dt: f32, world_gravity: Vec3) -> Option<Flight> {
        if !self.movement_enabled || dt == 0. { return None; }
        let mut velocity = self.velocity;
        // 628d40 clamps speed before computing translation; the stored final
        // velocity retains the unclamped input plus this step's gravity.
        let speed_squared = velocity.length_squared();
        if self.maximum_speed == 0. { velocity = Vec3::ZERO; }
        else if self.maximum_speed * self.maximum_speed < speed_squared {
            velocity *= self.maximum_speed / speed_squared.sqrt();
        }
        let gravity = Vec3::Z * (world_gravity.z * self.gravity_multiplier);
        let end = self.position + velocity * dt + gravity * (dt * dt * 0.5);
        Some(Flight { start: self.position, end, velocity: self.velocity + gravity * dt,
            out_of_range: self.declaration.maximum_range > 0.
                && end.distance_squared(self.origin) > self.declaration.maximum_range * self.declaration.maximum_range })
    }

    pub fn publish_flight(&mut self, flight: Flight) {
        self.position = flight.end;
        self.velocity = flight.velocity;
    }

    /// The caller supplies the victim's real class exemption and actual live
    /// inflicted-damage attribute. No NPC/class/armor state is inferred here.
    pub fn hit_parameters(&self, hit: Hit, damage_factor: f32, falloff_exempt: bool) -> Option<Parameters> {
        let declaration = self.declaration.damage.as_ref()?;
        let mut fraction = 1.;
        let distance = hit.position.distance(self.origin);
        if !falloff_exempt && self.declaration.falloff_minimum < self.declaration.falloff_maximum {
            fraction = (1. - (distance - self.declaration.falloff_minimum)
                / (self.declaration.falloff_maximum - self.declaration.falloff_minimum)).clamp(0., 1.);
        }
        let mut maximum = (declaration.maximum + self.damage_increment) * damage_factor;
        let mut minimum = declaration.minimum.min(maximum);
        let epsilon = f32::from_bits(0x219392ef); // 141ab2294
        if maximum.abs() <= epsilon { maximum = 0.; }
        if minimum.abs() <= epsilon { minimum = 0.; }
        let mut parameters = Parameters::from_declaration(declaration);
        parameters.set_amount(declaration, (1. - fraction) * minimum + maximum * fraction, false);
        parameters.hit_location = Some(hit.position);
        parameters.hit_normal = Some(hit.normal);
        parameters.hit_direction = hit.direction;
        parameters.hit_region = hit.region;
        parameters.affected_regions = 1u32.checked_shl(hit.region).unwrap_or(0);
        parameters.hit_distance = distance;
        parameters.attacker = self.attacker;
        parameters.inflictor = Some(self.identity);
        Some(parameters)
    }
}

#[derive(Clone, Copy)]
pub struct Flight { pub start: Vec3, pub end: Vec3, pub velocity: Vec3, pub out_of_range: bool }
#[derive(Clone, Copy)]
pub struct Hit { pub position: Vec3, pub normal: Vec3, pub direction: Vec3, pub region: u32 }

/// 6262d0 uses asin(-normal dot velocity), so a head-on hit is 90 degrees.
pub fn incidence_angle(normal: Vec3, velocity: Vec3) -> f32 {
    let direction = velocity / velocity.length_squared().max(f32::MIN_POSITIVE).sqrt();
    (-normal.dot(direction)).clamp(-1., 1.).asin() * f32::from_bits(0x42652ee0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response { None, BounceOnce, Stick, Break, UncannyDeflect, Bounce }

/// Real contact facts gathered by the collision/component owners in 6262d0.
/// All special state is explicit rather than guessed from body names.
pub struct Contact {
    pub player: bool,
    pub npc: bool,
    pub world: bool,
    pub can_stick: bool,
    pub can_break: bool,
    pub angle: f32,
    pub stick_blocked_by_pickable: bool,
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
pub fn response(declaration: &Declaration, contact: &Contact, flags: &mut ContactFlags,
    mut random: impl FnMut() -> f32) -> Response {
    let mut result = Response::Break;
    if !contact.absorbed_by_armor {
        if contact.desired_deflection == 0 {
            if !contact.player {
                result = Response::Bounce;
                let required = if contact.npc { declaration.required_npc_stick_angle } else { declaration.required_stick_angle };
                if contact.can_stick && contact.angle >= required && !contact.stick_blocked_by_pickable { result = Response::Stick; }
                if contact.can_break && declaration.can_break {
                    let unbroken = result;
                    if flags.always_break_on_hit { result = Response::Break; }
                    if !flags.always_break_on_hit || contact.npc && flags.always_stick_to_npcs {
                    let chance = if !contact.can_stick {
                        if contact.angle < declaration.bounce_angle { declaration.break_chance_not_stickable } else { 100. }
                    } else if contact.npc { contact.npc_break_probability.unwrap_or(declaration.break_chance_npc) }
                    else { declaration.break_chance_stickable };
                    result = if random() < chance * 0.01 { Response::Break } else { unbroken };
                    }
                }
            }
        } else if contact.desired_deflection == 3 { result = Response::None; }
        else if contact.desired_deflection == 4 { result = Response::UncannyDeflect; }
    }
    if flags.pinned_limb && (!contact.world || result != Response::Stick) { result = Response::Break; }
    if flags.bounce_once && !contact.can_stick && result == Response::Break && !contact.player
        && (!contact.npc || !contact.absorbed_by_armor) {
        flags.bounce_once = false;
        result = Response::BounceOnce;
    }
    result
}
