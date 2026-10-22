//! Floating-point operations whose operand order affects stored engine words.
// ADDSS preserves its first operand's NaN payload. Rust may commute ordinary
// addition, so retain stored NaN words explicitly instead of doing arithmetic
// on them (which can also canonicalize the sign of a single NaN).
pub(crate) fn add(first: f32, second: f32) -> f32 {
    if first.is_nan() {
        f32::from_bits(first.to_bits() | 0x0040_0000)
    } else if second.is_nan() {
        f32::from_bits(second.to_bits() | 0x0040_0000)
    } else {
        first + second
    }
}

/// MULSS has the same stored NaN precedence as ADDSS.
pub(crate) fn multiply(first: f32, second: f32) -> f32 {
    if first.is_nan() {
        f32::from_bits(first.to_bits() | 0x0040_0000)
    } else if second.is_nan() {
        f32::from_bits(second.to_bits() | 0x0040_0000)
    } else {
        first * second
    }
}
