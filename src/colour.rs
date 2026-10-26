//! Packed engine RGBA colours shared by scene, animation and effect records.
use crate::float::{multiply, truncate_i32};

/// 164340 truncates scaled channels before integer clamping. Converting after
/// a float clamp would change the integer-indefinite path for invalid inputs.
pub fn pack_rgba8(colour: [f32; 4]) -> u32 {
    u32::from_le_bytes(colour.map(|v| truncate_i32(multiply(v, 255.)).clamp(0, 255) as u8))
}

/// 164620 uses the stored single-precision reciprocal of 255 for every byte.
pub fn unpack_rgba8(colour: u32) -> [f32; 4] {
    colour
        .to_le_bytes()
        .map(|v| multiply(f32::from(v), f32::from_bits(0x3b80_8081)))
}
