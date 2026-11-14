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

const PIBY2_1: f64 = f64::from_bits(0x3ff9_21fb_5440_0000);
const PIBY2_1TAIL: f64 = f64::from_bits(0x3dd0_b461_1a62_6331);
const PIBY2_2: f64 = f64::from_bits(0x3dd0_b461_1a60_0000);
const PIBY2_2TAIL: f64 = f64::from_bits(0x3ba3_198a_2e03_7073);
const TWOBYPI: f64 = f64::from_bits(0x3fe4_5f30_6dc9_c883);
const SIXTH: f64 = f64::from_bits(0x3fc5_5555_5555_5555);

/// The CRT's reduction by pi/2 for |x| below 5e5 (1411ae130/1411ade30):
/// the quadrant, the reduced argument and its biased exponent.
fn reduce_piby2(x: f64, magnitude: u64) -> (u32, f64, u64) {
    let n = (x * TWOBYPI + 0.5) as i32 as u32;
    let dn = n as i32 as f64;
    let first = x - PIBY2_1 * dn;
    let mut r = first - PIBY2_1TAIL * dn;
    // The exponent of the first remainder decides both the refinement and
    // the small-argument shortcuts.
    let exponent = r.abs().to_bits() >> 52;
    if 15 < (magnitude >> 52) as i64 - exponent as i64 {
        r = first - PIBY2_2 * dn;
        r = r - (PIBY2_2TAIL * dn - ((first - r) - PIBY2_2 * dn));
    }
    (n, r, exponent)
}

/// approximate_: 1411c6e50/1411c6cb8 handle infinities and NaNs, and
/// __remainder_piby2f_inline reduces |x| >= 5e5; neither is ported, so
/// those inputs go through f64.
fn crt_large(x: f32, cosine: bool) -> f32 {
    let d = x as f64;
    (if cosine { d.cos() } else { d.sin() }) as f32
}

/// sinf (1411ae130): f64 polynomials after a three-part pi/2 reduction.
pub fn sinf(x: f32) -> f32 {
    if x.to_bits() & 0x7f80_0000 == 0x7f80_0000 {
        return crt_large(x, false);
    }
    let mut d = x as f64;
    let magnitude = d.abs().to_bits();
    let sine = |d: f64, x2: f64| {
        let c = |bits: u64| f64::from_bits(bits);
        d + ((((c(0x3ec7_1de3_a556_c734) * x2 + c(0xbf2a_01a0_1a01_a01a)) * x2) * x2 + c(0x3f81_1111_1111_1111) * x2)
            + c(0xbfc5_5555_5555_5555))
            * d
            * x2
    };
    if magnitude < 0x3fe9_21fb_5444_2d19 {
        if magnitude < 0x3f80_0000_0000_0000 {
            if 0x3f1f_ffff_ffff_ffff < magnitude {
                d = d - d * d * d * SIXTH;
            }
        } else {
            d = sine(d, d * d);
        }
        return d as f32;
    }
    let negative = d != f64::from_bits(magnitude);
    if negative {
        d = 0. - d;
    }
    if !(magnitude < 500_000f64.to_bits()) {
        return crt_large(x, false);
    }
    let (n, r, exponent) = reduce_piby2(d, magnitude);
    let mut d = r;
    if exponent <= 0x3f1 {
        if exponent < 0x3df {
            if n & 1 != 0 {
                d = 1.;
            }
        } else if n & 1 == 0 {
            d = d - SIXTH * d * d * d;
        } else {
            d = 1. - d * d * 0.5;
        }
    } else {
        let x2 = d * d;
        let c = |bits: u64| f64::from_bits(bits);
        d = if n & 1 == 0 {
            sine(d, x2)
        } else {
            (-0.5 * x2 + 1.)
                + ((((c(0xbe92_7e4f_b778_9f5c) * x2 + c(0x3efa_01a0_1a01_a019)) * x2) * x2 + c(0xbf56_c16c_16c1_6c16) * x2)
                    + c(0x3fa5_5555_5555_5555))
                    * x2
                    * x2
        };
    }
    if ((n >> 1) & 1 != 0) != negative {
        d = 0. - d;
    }
    d as f32
}

/// cosf (1411ade30).
pub fn cosf(x: f32) -> f32 {
    if x.to_bits() & 0x7f80_0000 == 0x7f80_0000 {
        return crt_large(x, true);
    }
    let d = x as f64;
    let magnitude = d.abs().to_bits();
    let c = |bits: u64| f64::from_bits(bits);
    let (n, mut r, x2) = if magnitude < 0x3fe9_21fb_5444_2d19 {
        (0, d, d * d)
    } else {
        let positive = if d != f64::from_bits(magnitude) { 0. - d } else { d };
        if !(magnitude < 500_000f64.to_bits()) {
            return crt_large(x, true);
        }
        let (n, r, exponent) = reduce_piby2(positive, magnitude);
        let x2 = r * r;
        if exponent < 0x3f2 {
            let r = if exponent < 0x3df {
                if n & 1 == 0 { 1. } else { r }
            } else if n & 1 == 0 {
                1. - x2 * 0.5
            } else {
                r - SIXTH * r * x2
            };
            return (if (n.wrapping_add(1)) & 2 != 0 { 0. - r } else { r }) as f32;
        }
        (n, r, x2)
    };
    r = if n & 1 == 0 {
        ((((c(0xbe92_7e4f_a17f_65f6) * x2 + c(0x3efa_01a0_19f4_ec90)) * x2) * x2 + c(0xbf56_c16c_16c1_6967) * x2)
            + c(0x3fa5_5555_5555_5555))
            * x2
            * x2
            - (0.5 * x2 - 1.)
    } else {
        ((((c(0x3ec7_1de3_796c_de01) * x2 + c(0xbf2a_01a0_19e8_3e5c)) * x2) * x2 + c(0x3f81_1111_1111_0bb3) * x2)
            + c(0xbfc5_5555_5555_5555))
            * x2
            * r
            + r
    };
    (if (n.wrapping_add(1)) & 2 != 0 { 0. - r } else { r }) as f32
}

#[cfg(test)]
mod crt_tests {
    /// The installed sinf/cosf (analysis/probe_crt_sincos.py).
    #[test]
    fn sine_cosine_match_installed_crt() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/crt_sincos_cases.txt")).unwrap();
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let w: Vec<u32> = line.split_whitespace().map(|h| u32::from_str_radix(h, 16).unwrap()).collect();
            let x = f32::from_bits(w[0]);
            assert_eq!([super::sinf(x).to_bits(), super::cosf(x).to_bits()], [w[1], w[2]], "x = {x}");
        }
    }
}
