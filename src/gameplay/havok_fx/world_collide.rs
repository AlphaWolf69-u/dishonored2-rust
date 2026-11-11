//! hkfxEffects::WorldCollide (constructor 140ffb110, vtable 141f1e228): the
//! per-body contacts the collide effects write and the contact solvers read.
//! Channels (140ffb440): "WorldCollide/StaticContact/Plane",
//! ".../LinearVelocity", ".../AngularVelocity", ".../COM", and the same for
//! DynamicContact.

/// One 0x40-byte contact record.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Contact {
    /// Normal and plane distance: n . x + w = 0 on the surface.
    pub plane: [f32; 4],
    /// The hit body's linear and angular velocity and centre of mass.
    pub linear_velocity: [f32; 4],
    pub angular_velocity: [f32; 4],
    pub centre_of_mass: [f32; 4],
}

/// A contact's friction and restitution (+0x40 / +0x60); -1 without one.
pub const NO_MATERIAL: [f32; 2] = [-1., -1.];

pub struct WorldCollide {
    /// +0x30 / +0x40.
    pub static_contacts: Vec<Contact>,
    pub static_materials: Vec<[f32; 2]>,
    /// +0x50 / +0x60.
    pub dynamic_contacts: Vec<Contact>,
    pub dynamic_materials: Vec<[f32; 2]>,
}

impl WorldCollide {
    /// Slot 6 (140ffb180) sizes the channels to the body capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            static_contacts: vec![Contact::default(); capacity],
            static_materials: vec![NO_MATERIAL; capacity],
            dynamic_contacts: vec![Contact::default(); capacity],
            dynamic_materials: vec![NO_MATERIAL; capacity],
        }
    }

    /// Slot 8 (140ffb2e0): new bodies have no contacts.
    pub fn allocate(&mut self, bodies: &[i32]) {
        for &body in bodies {
            let body = body as usize;
            self.static_contacts[body] = Contact::default();
            self.static_materials[body] = NO_MATERIAL;
            self.dynamic_contacts[body] = Contact::default();
            self.dynamic_materials[body] = NO_MATERIAL;
        }
    }
}

/// The combination policies of an hknp material (+0x38 friction, +0x39
/// restitution) as PhysicsCollide applies them (140fc1370:3600-3640).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combine {
    GeometricMean,
    Minimum,
    Maximum,
    ArithmeticMean,
}

impl Combine {
    pub fn from_policy(policy: u8) -> Option<Self> {
        match policy {
            0 => Some(Self::GeometricMean),
            1 => Some(Self::Minimum),
            2 => Some(Self::Maximum),
            3 => Some(Self::ArithmeticMean),
            _ => None,
        }
    }

    /// The hit material's value `surface` with the particle's `own`.
    pub fn apply(self, surface: f32, own: f32) -> f32 {
        use crate::float::{add, multiply};
        match self {
            Self::GeometricMean => multiply(surface, own).sqrt(),
            Self::Minimum => {
                if own <= surface { own } else { surface }
            }
            Self::Maximum => {
                if surface <= own { own } else { surface }
            }
            Self::ArithmeticMean => multiply(add(surface, own), 0.5),
        }
    }
}

/// The hknp material of a world surface as the game builds it from an
/// arkPhysicsMaterial (140092f80: constructor 140c9c210 then 14008b6e0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceMaterial {
    /// Truncated to their top 16 bits (+0x32, +0x34, +0x36).
    pub dynamic_friction: f32,
    pub static_friction: f32,
    pub restitution: f32,
    /// +0x38 / +0x39.
    pub friction_policy: u8,
    pub restitution_policy: u8,
    /// +0x3c: the maximum contact impulse; a negative one ignores the
    /// material.
    pub maximum_impulse: f32,
}

fn truncate16(value: f32) -> f32 {
    f32::from_bits(value.to_bits() & 0xffff_0000)
}

impl SurfaceMaterial {
    pub fn from_physics(material: &crate::assets::entities::physics_material::PhysicsMaterial) -> Self {
        // 140c9c210's defaults: 0.5 frictions, restitution 0, policies 1/2.
        let mut result = Self {
            dynamic_friction: f32::from_bits(0x3f00_0000),
            static_friction: f32::from_bits(0x3f00_0000),
            restitution: 0.,
            friction_policy: 1,
            restitution_policy: 2,
            maximum_impulse: f32::from_bits(0x7f7f_ffee),
        };
        // 14008b6e0: flag 0x20 materials keep the defaults.
        if material.flags & 0x20 == 0 {
            result.dynamic_friction = truncate16(material.dynamic_friction);
            result.static_friction = truncate16(material.static_friction);
            result.restitution = truncate16(material.restitution);
            result.friction_policy = 2;
        }
        result
    }
}

/// 140fc1370:3588-3660: the contact's (friction, restitution) from the
/// particle solver's own values and the hit surface's material. A resting
/// body (solver state >= 3) uses the static friction when it is larger.
pub fn contact_material(surface: Option<SurfaceMaterial>, own: [f32; 2], resting: bool) -> Result<[f32; 2], String> {
    let Some(surface) = surface.filter(|surface| !(surface.maximum_impulse <= 0. && surface.maximum_impulse != 0.)) else {
        return Ok(own);
    };
    let friction = if !resting || surface.static_friction <= surface.dynamic_friction {
        surface.dynamic_friction
    } else {
        surface.static_friction
    };
    let friction = crate::float::multiply(friction, f32::from_bits(0x4019_999a));
    let restitution = Combine::from_policy(surface.restitution_policy)
        .ok_or("unknown hknp restitution combine policy")?
        .apply(surface.restitution, own[1]);
    let friction = Combine::from_policy(surface.friction_policy)
        .ok_or("unknown hknp friction combine policy")?
        .apply(friction, own[0]);
    Ok([friction, restitution])
}
