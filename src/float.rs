//! Floating-point operations whose operand order affects stored engine words.
// ADDSS preserves its first operand's NaN payload. Rust may commute ordinary
// addition, so retain stored NaN words explicitly instead of doing arithmetic
// on them (which can also canonicalize the sign of a single NaN).
/// Truncated half conversion shared by 2fa690's spectrum upload and 1ee880's
/// particle instance data. Tiny values become positive zero; overflow saturates
/// to the signed largest finite half instead of producing an IEEE infinity.
pub(crate) fn pack_half_truncated(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 255) as i32 - 112;
    if exponent <= 0 {
        0
    } else if exponent > 30 {
        sign | 0x7bff
    } else {
        sign | ((exponent as u16) << 10) | ((bits & 0x7fffff) >> 13) as u16
    }
}

/// CVTTSS2SI's signed conversion, including integer-indefinite on invalid input.
pub(crate) fn truncate_i32(value: f32) -> i32 {
    if value >= -2147483648.0 && value < 2147483648.0 {
        value as i32
    } else {
        i32::MIN
    }
}

/// The64-bit CVTTSS2SI variant, used before narrowing coherence controls.
pub(crate) fn truncate_i64(value: f32) -> i64 {
    if value >= -9223372036854775808.0 && value < 9223372036854775808.0 {
        value as i64
    } else {
        i64::MIN
    }
}

/// DIVSS fixes the stored sign of an invalid result such as 0/0. LLVM
/// arithmetic leaves that NaN sign unspecified; mass diagonalization stores it.
#[cfg(target_arch = "x86_64")]
pub(crate) fn divide(mut first: f32, second: f32) -> f32 {
    unsafe {
        std::arch::asm!("divss {first}, {second}", first=inout(xmm_reg) first,
            second=in(xmm_reg) second, options(pure, nomem, nostack));
    }
    first
}
#[cfg(not(target_arch = "x86_64"))]
pub(crate) fn divide(_first: f32, _second: f32) -> f32 {
    unimplemented!("DH2 stored division words require x86-64 DIVSS")
}

pub(crate) fn add(first: f32, second: f32) -> f32 {
    if first.is_nan() {
        f32::from_bits(first.to_bits() | 0x0040_0000)
    } else if second.is_nan() {
        f32::from_bits(second.to_bits() | 0x0040_0000)
    } else {
        first + second
    }
}

/// SUBSS propagates a NaN operand before subtraction; rewriting it as addition
/// with a negated operand can change the stored NaN sign.
pub(crate) fn subtract(first: f32, second: f32) -> f32 {
    if first.is_nan() {
        f32::from_bits(first.to_bits() | 0x0040_0000)
    } else if second.is_nan() {
        f32::from_bits(second.to_bits() | 0x0040_0000)
    } else {
        first - second
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
