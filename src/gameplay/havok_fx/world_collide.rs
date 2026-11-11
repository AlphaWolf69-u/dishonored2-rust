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
