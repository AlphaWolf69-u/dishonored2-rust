//! Shared FX timing and scalar-parameter evaluation. Definitions remain owned
//! by their asset loader; running effects consume the caller's shared RNG seed.
pub mod curve;
pub mod decal;
pub mod environment;
pub mod envelope;
pub mod groups;
pub mod parameters;
pub mod particle;
pub mod world_node;
pub mod pool;
pub mod sound;
pub mod timing;
pub mod vector_curve;

use crate::float::add;

pub(crate) fn normalize_small(value: f32) -> f32 {
    if f32::from_bits(value.to_bits() & 0x7fff_ffff) <= f32::from_bits(0x2193_92ef) {
        0.
    } else {
        value
    }
}

#[cfg(test)]
#[path = "effects/tests.rs"]
mod tests;
