//! The four-byte packed vectors Integrate keeps per body (140ff9fd0 at
//! 140ffa32d-140ffa449): three signed mantissa bytes sharing one exponent
//! byte (Havok's hkPackedVector3 layout).
use crate::float::{add, multiply};

/// 1423cce40: the w lane every packed vector is given before packing.
pub const PACK_W: f32 = f32::from_bits(0x2880_0000);

/// 140ffa32d: each byte becomes the high byte of a lane; the exponent
/// lane shifted right by one is the scale.
pub fn unpack(bytes: [u8; 4]) -> [f32; 4] {
    let lanes = bytes.map(|byte| (u32::from(byte) << 24) as i32);
    let scale = f32::from_bits((lanes[3] as u32) >> 1);
    lanes.map(|lane| multiply(scale, lane as f32))
}

/// 140ffa373: the shared exponent is the largest exponent of v x 1.00417;
/// each lane is rescaled to 2^30 of it, truncated, and its top byte kept.
pub fn pack(v: [f32; 4]) -> [u8; 4] {
    let exponents = v.map(|lane| multiply(f32::from_bits(0x3f80_8889), lane).to_bits() as i32 & 0x7f80_0000);
    // pshufd 0x4e / 0xb1 with signed PCMPGTD selects the maximum.
    let pair = [exponents[0].max(exponents[2]), exponents[1].max(exponents[3])];
    let maximum = pair[0].max(pair[1]);
    let bias = 0x4e80_0000_i32;
    let bytes: [u8; 3] = std::array::from_fn(|i| {
        let scaled = f32::from_bits((bias.wrapping_add(v[i].to_bits() as i32)).wrapping_sub(maximum) as u32);
        // CVTTPS2DQ gives 0x80000000 out of range; XOR with the >= 2^31
        // mask turns the positive overflow into 0x7fffffff.
        let truncated = if scaled.is_nan() || !(scaled.abs() < 2_147_483_648.) { i32::MIN } else { scaled as i32 };
        let fixed = if f32::from_bits(0x4f00_0000) <= scaled { !truncated } else { truncated };
        (fixed.wrapping_add(0x0080_0000) >> 24) as u8
    });
    let exponent = (0x7f00_0000_i32.wrapping_add(maximum.wrapping_sub(bias).wrapping_mul(2)) >> 24) as u8;
    [bytes[0], bytes[1], bytes[2], exponent]
}

/// The w lane replaced as 140ffa376/140ffa4ee do before packing.
pub fn with_pack_w(v: [f32; 4]) -> [f32; 4] {
    [v[0], v[1], v[2], PACK_W]
}

/// a x b + c per lane, in the original's MULPS/ADDPS order.
pub fn multiply_add(a: [f32; 4], b: [f32; 4], c: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|i| add(multiply(a[i], b[i]), c[i]))
}
