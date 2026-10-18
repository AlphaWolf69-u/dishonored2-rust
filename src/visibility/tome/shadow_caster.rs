//! Original-backed candidate classification and caster-mask selection.
//!
//! `ShadowCasterTome` is built once from the immutable level Tome. Each frame,
//! `ShadowCasterSelectionScratch::select` consumes the deduplicated indices
//! from the ordinary tiled visibility query and caller-supplied receiver,
//! caster-flag, and parent-suppression state. It deliberately does not invent
//! the native receiver initializer or decode unverified instance flag bits.

use super::{
    active_query_planes_into, append_object_ids_for_index, classify_native_receiver_bounds, float,
    object_bounds, object_count, object_distance_bounds, object_distance_rejection_mask,
    query_bounds_accepts_aabb, recorded_bounds, word, NativeReceiverBoundsClass,
    VisibleObjectIndexOutput,
};
use std::ops::Range;

/// Typed ownership of the fields FUN_140a71f80 modifies in its opaque owner.
/// The mask is persistent caller-owned storage; this function does not clear it.
pub struct NativeReceiverDepthProjection {
    pub receiver_rows: [[f32; 4]; 4],
    pub transposed_rows: [[f32; 4]; 4],
    pub mask: Box<[u16; 128 * 128]>,
}

/// FUN_140a71f80: build four receiver-space depth segments per coarse tile.
pub fn fill_receiver_depth_projection(
    state: &mut NativeReceiverDepthProjection,
    depths: &[f32; 4096],
    camera_inverse_rows: [[f32; 4]; 4],
    receiver_rows: [[f32; 4]; 4],
    cutoff: f32,
) {
    state.receiver_rows = receiver_rows;
    state.transposed_rows =
        std::array::from_fn(|row| std::array::from_fn(|column| receiver_rows[column][row]));
    let unproject = |point: [f32; 3]| -> [f32; 3] {
        let row = camera_inverse_rows[3];
        let denominator = ((point[0] * row[0] + point[1] * row[1]) + point[2] * row[2]) + row[3];
        let reciprocal = 1.0f64 / f64::from(denominator);
        std::array::from_fn(|axis| {
            let row = camera_inverse_rows[axis];
            let numerator = ((point[0] * row[0] + point[1] * row[1]) + point[2] * row[2]) + row[3];
            (f64::from(numerator) * reciprocal) as f32
        })
    };
    let transform = |point: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|axis| {
            let row = receiver_rows[axis];
            ((point[0] * row[0] + point[1] * row[1]) + point[2] * row[2]) + row[3]
        })
    };
    let coarse = coarse_receiver_depth(depths);
    let mut near = unproject([-1.0, -1.0, 0.0]);
    let right = unproject([1.0, -1.0, 0.0]);
    let up = unproject([-1.0, 1.0, 0.0]);
    let dx: [f32; 3] = std::array::from_fn(|axis| (right[axis] - near[axis]) * (1.0 / 16.0));
    let dy: [f32; 3] = std::array::from_fn(|axis| (up[axis] - near[axis]) * (1.0 / 16.0));
    for y in 0..16 {
        let next_row: [f32; 3] = std::array::from_fn(|axis| near[axis] + dy[axis]);
        let v = (y as f32 * 0.125 + 0.0625) - 1.0;
        for x in 0..16 {
            let next_x: [f32; 3] = std::array::from_fn(|axis| near[axis] + dx[axis]);
            let depth = if coarse[y * 16 + x] < cutoff {
                coarse[y * 16 + x]
            } else {
                cutoff
            };
            if depth != 0.0 {
                let near_corners = [
                    near,
                    std::array::from_fn(|axis| near[axis] + dy[axis]),
                    std::array::from_fn(|axis| next_x[axis] + dy[axis]),
                    next_x,
                ];
                let u = (x as f32 * 0.125 + 0.0625) - 1.0;
                let far_corners = [
                    unproject([u - 0.0625, v - 0.0625, depth]),
                    unproject([u - 0.0625, v + 0.0625, depth]),
                    unproject([u + 0.0625, v + 0.0625, depth]),
                    unproject([u + 0.0625, v - 0.0625, depth]),
                ];
                let rays: [[f32; 3]; 4] = std::array::from_fn(|corner| {
                    std::array::from_fn(|axis| {
                        far_corners[corner][axis] - near_corners[corner][axis]
                    })
                });
                for segment in 0..4 {
                    let mut minimum = [f32::MAX; 3];
                    let mut maximum = [-f32::MAX; 3];
                    for corner in 0..4 {
                        for fraction in [segment as f32 * 0.25, (segment as f32 + 1.0) * 0.25] {
                            let point = transform(std::array::from_fn(|axis| {
                                rays[corner][axis] * fraction + near_corners[corner][axis]
                            }));
                            for axis in 0..3 {
                                minimum[axis] = if minimum[axis] < point[axis] {
                                    minimum[axis]
                                } else {
                                    point[axis]
                                };
                                maximum[axis] = if point[axis] < maximum[axis] {
                                    maximum[axis]
                                } else {
                                    point[axis]
                                };
                            }
                        }
                    }
                    rasterize_receiver_depth_mask(&mut state.mask, minimum, maximum);
                }
            }
            near = next_x;
        }
        near = next_row;
    }
}

/// FUN_140a70200: max-rasterize a receiver-space AABB into the 128x128
/// U16 depth mask. The native AL return reports whether a rectangle was covered.
pub fn rasterize_receiver_depth_mask(
    mask: &mut [u16; 128 * 128],
    minimum: [f32; 3],
    maximum: [f32; 3],
) -> bool {
    // Use the native CVTTSS2SI integer-indefinite behavior, not a saturating cast.
    let cvtt = |value: f32| unsafe {
        core::arch::x86_64::_mm_cvttss_si32(core::arch::x86_64::_mm_set_ss(value))
    };
    let x0 = cvtt(((minimum[0] + 1.0) * 0.5) * 128.0).max(0);
    let y0 = cvtt(((minimum[1] + 1.0) * 0.5) * 128.0).max(0);
    let x1 = 1i32
        .wrapping_sub(cvtt(((maximum[0] + 1.0) * -0.5) * 128.0))
        .min(128);
    let y1 = 1i32
        .wrapping_sub(cvtt(((maximum[1] + 1.0) * -0.5) * 128.0))
        .min(128);
    // COMISS zero,Z / JA rejects strictly negative Z, but admits unordered Z.
    if x0 >= x1 || y0 >= y1 || maximum[2] < 0.0 {
        return false;
    }
    let depth = 1i32.wrapping_sub(cvtt(maximum[2] * -65534.0)).min(65535) as u16;
    for y in y0..y1 {
        for value in &mut mask[y as usize * 128 + x0 as usize..y as usize * 128 + x1 as usize] {
            *value = (*value).max(depth);
        }
    }
    true
}

/// FUN_140a73bf0, 1050–1164: receiver-mask rejection of a world AABB.
/// This is only the enabled depth-mask branch, not the full caster query.
/// Native factors are 64 / 128 / 65534, with UV endpoint offsets [0,0,1,1]
/// at 0x141e68080. Native acceptance is STRICTLY threshold < stored depth.
pub fn receiver_depth_allows_aabb(
    state: &NativeReceiverDepthProjection,
    bounds: [[f32; 3]; 2],
) -> Result<bool, String> {
    let mut lower = [0.0; 3];
    let mut upper = [0.0; 3];
    for axis in 0..3 {
        let row: [f32; 4] = std::array::from_fn(|component| state.transposed_rows[component][axis]);
        let mut minimum = [0.0; 3];
        let mut maximum = [0.0; 3];
        for component in 0..3 {
            let a = row[component] * bounds[0][component];
            let b = row[component] * bounds[1][component];
            // CMPPS/LT selects the second operand on unordered/equal inputs.
            (minimum[component], maximum[component]) = if a < b { (a, b) } else { (b, a) };
        }
        lower[axis] = ((row[3] + minimum[0]) + minimum[1]) + minimum[2];
        upper[axis] = ((row[3] + maximum[0]) + maximum[1]) + maximum[2];
    }
    let cvtt_clamped = |value: f32, limit: f32| unsafe {
        use core::arch::x86_64::*;
        let value = _mm_max_ss(_mm_setzero_ps(), _mm_set_ss(value));
        let value = _mm_min_ss(_mm_set_ss(limit), value);
        _mm_cvttss_si32(value)
    };
    let rect = [
        cvtt_clamped(lower[0] * 64.0 + 64.0, 128.0),
        cvtt_clamped(lower[1] * 64.0 + 64.0, 128.0),
        cvtt_clamped((upper[0] * 64.0 + 1.0) + 64.0, 128.0),
        cvtt_clamped((upper[1] * 64.0 + 1.0) + 64.0, 128.0),
    ];
    let threshold = cvtt_clamped(lower[2] * 65534.0, 65534.0);
    // Preserve valid native inputs; report malformed owners rather than index
    // arbitrary memory when CVTT returns integer-indefinite for an unordered UV.
    if rect.iter().any(|&value| value < 0) || threshold < 0 {
        return Err("nonfinite native receiver depth query".into());
    }
    if rect[0] >= rect[2] || rect[1] >= rect[3] {
        return Ok(false);
    }
    for y in rect[1]..rect[3] {
        let row = y as usize * 128;
        if state.mask[row + rect[0] as usize..row + rect[2] as usize]
            .iter()
            .any(|&depth| threshold < i32::from(depth))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// FUN_140a6f640: reduce the native tiled 64x64 visibility depth to 16x16.
/// Its decompiled signature omits a fifth stack argument: machine code shows
/// the caller passes the state/context there, and the native helper loads the
/// source pointer from `context+0x590` (valid byte `+0x598`). This Rust helper
/// takes the extracted 4096 samples directly. A tile is 16 contiguous samples;
/// ignore exact 1.0 sky sentinel and retain the greatest positive depth.
pub fn coarse_receiver_depth(depths: &[f32; 4096]) -> [f32; 256] {
    std::array::from_fn(|tile| {
        let mut maximum = 0.0;
        for &depth in &depths[tile * 16..tile * 16 + 16] {
            if depth != 1.0 && maximum < depth {
                maximum = depth;
            }
        }
        maximum
    })
}

/// FUN_140a872a0: the original normalized camera planes and finite-far
/// flag used by the receiver-volume initializer. Preserve its unordered
/// comparison behavior and coefficient operand order.
pub fn native_normalized_frustum_planes(rows: [[f32; 4]; 4]) -> ([[f32; 4]; 6], bool) {
    let mut planes = super::query_planes_from_rows(rows, rows);
    let [x, y, z, _] = planes[1];
    let far_length = (y * y + x * x + z * z).sqrt();
    let near = planes[0];
    let depth_dot = near[1] * y + near[0] * x + near[2] * z;
    let finite_far = !(far_length <= 0.0 || depth_dot >= 0.0);
    for (index, plane) in planes.iter_mut().enumerate() {
        let [x, y, z, _] = *plane;
        let reciprocal = 1.0 / (x * x + y * y + z * z).sqrt();
        for (axis, value) in plane.iter_mut().enumerate() {
            *value = if index >= 4 && axis < 3 {
                native_mulss(*value, reciprocal)
            } else {
                native_mulss(reciprocal, *value)
            };
        }
    }
    (planes, finite_far)
}

/// FUN_140a87720: intersect the six frustum planes in the original corner
/// order. Keep the cofactor order and the two final swaps; the extrusion's
/// fixed face/edge topology consumes precisely this layout.
pub fn native_frustum_plane_corners(planes: &[[f32; 4]; 6]) -> [[f32; 3]; 8] {
    let mut corners = [[0.0; 3]; 8];
    for (index, point) in corners.iter_mut().enumerate() {
        let a = planes[index >> 2];
        let b = planes[2 + ((index >> 1) & 1)];
        let c = planes[4 + (index & 1)];
        let cross_x = c[2] * b[1] - c[1] * b[2];
        let cross_y = c[0] * b[2] - c[2] * b[0];
        let cross_z = c[1] * b[0] - c[0] * b[1];
        // ADDSS at 140a878cd/140a878db and 140a8790a/140a8790f
        // sums the last two terms FIRST. Decompiled text flattens this into
        // a left-associated expression, which fails oblique gameplay planes.
        let numerator_z = (b[1] * a[0] - b[0] * a[1]) * c[3]
            + ((c[0] * a[1] - c[1] * a[0]) * b[3] + cross_z * a[3]);
        let numerator_y = (b[0] * a[2] - b[2] * a[0]) * c[3]
            + ((c[2] * a[0] - c[0] * a[2]) * b[3] + cross_y * a[3]);
        let reciprocal = 1.0 / (cross_x * a[0] + cross_y * a[1] + cross_z * a[2]);
        let numerator_x = (b[2] * a[1] - b[1] * a[2]) * c[3]
            + ((c[1] * a[2] - c[2] * a[1]) * b[3] + cross_x * a[3]);
        *point = [
            -numerator_x * reciprocal,
            -numerator_y * reciprocal,
            -numerator_z * reciprocal,
        ];
    }
    corners.swap(0, 1);
    corners.swap(4, 5);
    corners
}

/// FUN_140a88460: retain facing frustum planes and add the silhouette-edge
/// planes of its extrusion along the light direction. No guessed shadow box.
/// Only the returned prefix is written; the caller owns all other plane slots.
pub fn extrude_native_receiver_planes(
    direction: [f32; 3],
    planes: &[[f32; 4]; 6],
    output: &mut [[f32; 4]; 32],
) -> usize {
    let dot_yxz = |normal: [f32; 3]| {
        direction[1] * normal[1] + direction[0] * normal[0] + direction[2] * normal[2]
    };
    let mut count = 0;
    for plane in planes {
        if dot_yxz([plane[0], plane[1], plane[2]]) < 0.0 {
            output[count] = *plane;
            count += 1;
        }
    }
    let corners = native_frustum_plane_corners(planes);
    let difference = |a: usize, b: usize| {
        std::array::from_fn::<_, 3, _>(|axis| corners[a][axis] - corners[b][axis])
    };
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let normalized = |normal: [f32; 3], order: [usize; 3]| {
        let [a, b, c] = order;
        let reciprocal =
            1.0 / (normal[a] * normal[a] + normal[b] * normal[b] + normal[c] * normal[c]).sqrt();
        normal.map(|value| value * reciprocal)
    };
    let faces = [
        (3, 2, 1),
        (4, 5, 6),
        (0, 1, 5),
        (1, 2, 6),
        (2, 3, 7),
        (3, 0, 4),
    ];
    let normals: [[f32; 3]; 6] = std::array::from_fn(|index| {
        let (a, origin, b) = faces[index];
        normalized(
            cross(difference(a, origin), difference(b, origin)),
            [1, 0, 2],
        )
    });
    let center: [f32; 3] = std::array::from_fn(|axis| {
        corners.iter().fold(0.0, |sum, point| sum + point[axis]) * 0.125
    });
    // Cube-edge topology used by DAT_141e68450..DAT_141e68500:
    // (endpoint A, endpoint B, adjacent face A, adjacent face B).
    // Native stack publication order, not address order of the table rows.
    let edges = [
        (0, 1, 0, 2),
        (1, 2, 0, 3),
        (2, 3, 0, 4),
        (3, 0, 0, 5),
        (7, 6, 1, 4),
        (6, 5, 1, 3),
        (5, 4, 1, 2),
        (4, 7, 1, 5),
        (7, 3, 4, 5),
        (5, 1, 2, 3),
        (2, 6, 4, 3),
        (0, 4, 2, 5),
    ];
    for (a, b, face_a, face_b) in edges {
        let facing_a = dot_yxz(normals[face_a]);
        let facing_b = dot_yxz(normals[face_b]);
        if !((facing_a > 0.0 && facing_b <= 0.0) || (facing_a <= 0.0 && facing_b > 0.0)) {
            continue;
        }
        let mut normal = normalized(cross(difference(b, a), direction), [1, 0, 2]);
        let from_edge = difference_from_point(center, corners[a]);
        if from_edge[0] * normal[0] + from_edge[1] * normal[1] + from_edge[2] * normal[2] < 0.0 {
            normal = normal.map(|value| -value);
        }
        let p = corners[a];
        let offset = -(p[0] * normal[0] + p[1] * normal[1] + p[2] * normal[2]);
        let length = (normal[0] * normal[0]
            + normal[1] * normal[1]
            + normal[2] * normal[2]
            + offset * offset)
            .sqrt();
        let mut plane = [normal[0], normal[1], normal[2], offset];
        if length != 0.0 {
            let reciprocal = (1.0f64 / f64::from(length)) as f32;
            plane = plane.map(|value| native_mulss(reciprocal, value));
        }
        output[count] = plane;
        count += 1;
    }
    count
}

/// Preserve the native MULSS destination/source NaN selection. Rust's scalar
/// multiplication permits operand commuting; 140a88fa4..140a88fb2 does not.
#[inline]
fn native_mulss(mut left: f32, right: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "mulss {left}, {right}",
            left = inout(xmm_reg) left,
            right = in(xmm_reg) right,
            options(nomem, nostack, preserves_flags),
        );
        left
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (left, right);
        panic!("Original receiver MULSS normalization requires x86_64")
    }
}

fn difference_from_point(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] - b[axis])
}

/// FUN_140a73bf0's baked caster-direction lookup at 0x140a74732.
/// The receiver's direction is negated before selecting the dominant axis.
/// Keep the original tie order, CVTT truncation, and un-clamped 16x16 bins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeCasterDirection {
    pub face: u32,
    pub x: i32,
    pub y: i32,
}

impl NativeCasterDirection {
    pub fn from_receiver_direction(direction: [f32; 3]) -> Result<Self, String> {
        if direction.iter().any(|value| !value.is_finite()) {
            return Err("nonfinite native caster direction".into());
        }
        let direction = direction.map(|value| f32::from_bits(value.to_bits() ^ 0x8000_0000));
        let absolute = direction.map(|value| value.to_bits() & 0x7fff_ffff);
        let axis = if absolute[0] < absolute[1] {
            if absolute[1] < absolute[2] {
                2
            } else {
                1
            }
        } else if absolute[2] <= absolute[0] {
            0
        } else {
            2
        };
        if absolute[axis] == 0 {
            return Ok(Self {
                face: 0,
                x: 0,
                y: 0,
            });
        }
        let next = (1usize << axis) & 3;
        let last = (1usize << next) & 3;
        let scale = 8.0 / f32::from_bits(absolute[axis]); // DAT_141ac8fe4 = 8
        let cvtt = |value: f32| {
            if value >= -2147483648.0 && value < 2147483648.0 {
                value.trunc() as i32
            } else {
                i32::MIN
            }
        };
        Ok(Self {
            face: ((!direction[axis].to_bits()) >> 31) | ((axis as u32) * 2),
            x: cvtt(scale * direction[next] + 8.0),
            y: cvtt(scale * direction[last] + 8.0),
        })
    }

    /// Both packed forms used by FUN_140a73bf0. This consumes the original
    /// node/global Tome bytes without duplicating the large baked tables.
    pub fn allows_object(self, bytes: &[u8], index: u32) -> Result<bool, String> {
        let count = object_count(bytes)?;
        if index as usize >= count
            || self.face >= 6
            || !(0..=16).contains(&self.x)
            || !(0..=16).contains(&self.y)
        {
            return Err("native caster direction/object index out of range".into());
        }
        let flags = word(bytes, 0x10)? & 3;
        if flags == 0 {
            return Ok(true);
        }
        let bin = i64::from(self.y) * 16 + i64::from(self.x);
        let bit_at = if flags & 2 != 0 {
            bin * count as i64 + i64::from(self.face) * count as i64 * 256 + i64::from(index)
        } else {
            let records = word(bytes, 0x13c)? as usize;
            if records == 0 {
                return Err("native caster direction record table is absent".into());
            }
            let record = records + index as usize * 0x40 + self.face as usize * 8;
            bin * i64::from(word(bytes, 0x148)?) * 4 + i64::from(word(bytes, record)? as i32) * 4
        };
        let bit_at =
            usize::try_from(bit_at).map_err(|_| "negative native caster direction bit offset")?;
        let bits = word(bytes, 0x140)? as usize;
        if bits == 0 {
            return Err("native caster direction bit table is absent".into());
        }
        let packed = word(bytes, bits + (bit_at >> 5) * 4)? >> (bit_at & 31);
        if flags & 2 != 0 {
            return Ok(packed & 1 != 0);
        }
        let records = word(bytes, 0x13c)? as usize;
        let record = records + index as usize * 0x40 + self.face as usize * 8;
        let palette_at = word(bytes, 0x144)? as usize;
        if palette_at == 0 {
            return Err("native caster direction palette is absent".into());
        }
        let palette_index = u64::from(word(bytes, record + 4)?) + u64::from(packed & 15);
        let at = palette_at
            .checked_add(
                usize::try_from(palette_index * 2)
                    .map_err(|_| "native caster direction palette offset overflow")?,
            )
            .ok_or("native caster direction palette offset overflow")?;
        let value = bytes
            .get(at..at + 2)
            .ok_or("truncated native caster direction palette")?;
        Ok(u16::from_le_bytes(value.try_into().unwrap()) == 0x7f7f)
    }
}

///a82710's perspective baked-occlusion test, using the same64-byte
///direction records/palette as a73bf0. Unlike directional caster admission,
///the palette word is a truncated float distance (bits shifted left16).
///The query's near-plane displacement is added before the radial test.
pub fn native_point_occlusion_visible(
    bytes: &[u8],
    index: u32,
    point: [f32; 3],
    near_offset: f32,
) -> Result<bool, String> {
    if word(bytes, 0x10)? & 1 == 0 {
        return Ok(true);
    }
    if index as usize >= object_count(bytes)? {
        return Err("native point occlusion object is outside the Tome".into());
    }
    let records = word(bytes, 0x13c)? as usize;
    if records == 0 {
        return Err("native point occlusion record table is absent".into());
    }
    let record = records
        .checked_add(index as usize * 64)
        .ok_or("native point occlusion record overflow")?;
    let origin = [
        float(bytes, record + 48)?,
        float(bytes, record + 52)?,
        float(bytes, record + 56)?,
    ];
    let delta: [f32; 3] = std::array::from_fn(|axis| point[axis] - origin[axis]);
    let absolute = delta.map(|value| value.to_bits() & 0x7fffffff);
    let axis = if absolute[0] < absolute[1] {
        if absolute[1] < absolute[2] {
            2
        } else {
            1
        }
    } else {
        if absolute[2] <= absolute[0] {
            0
        } else {
            2
        }
    };
    let direction = NativeCasterDirection::from_receiver_direction(
        delta.map(|value| f32::from_bits(value.to_bits() ^ 0x80000000)),
    )?;
    let face_record = record + direction.face as usize * 8;
    let bin = i64::from(direction.y) * 16 + i64::from(direction.x);
    let bit =
        (bin * i64::from(word(bytes, 0x148)?) + i64::from(word(bytes, face_record)? as i32)) * 4;
    let bit = usize::try_from(bit).map_err(|_| "negative native point occlusion bit offset")?;
    let bits = word(bytes, 0x140)? as usize;
    let palette = word(bytes, 0x144)? as usize;
    if bits == 0 || palette == 0 {
        return Err("native point occlusion bit/palette table is absent".into());
    }
    let nibble = (word(bytes, bits + (bit >> 5) * 4)? >> (bit & 31)) & 15;
    let entry = u64::from(word(bytes, face_record + 4)?) + u64::from(nibble);
    let at = palette
        .checked_add(
            usize::try_from(entry * 2)
                .map_err(|_| "native point occlusion palette offset overflow")?,
        )
        .ok_or("native point occlusion palette overflow")?;
    let value = bytes
        .get(at..at + 2)
        .ok_or("truncated native point occlusion palette")?;
    let distance = f32::from_bits(u32::from(u16::from_le_bytes(value.try_into().unwrap())) << 16);
    //COMISS/JBE also rejects an unordered palette distance.
    Ok(distance + near_offset > f32::from_bits(absolute[axis]))
}

/// FUN_140a70620: distribute aligned blocks, then assign the unaligned tail
/// to the original last nonempty worker. Shadow gather passes alignment 2.
pub fn native_aligned_job_range(
    job: u32,
    jobs: u32,
    count: u32,
    alignment: u32,
) -> Result<Range<usize>, String> {
    if alignment == 0
        || (jobs != 0 && job >= jobs)
        || [job, jobs, count, alignment]
            .iter()
            .any(|&value| value > i32::MAX as u32)
    {
        return Err("invalid native aligned job range".into());
    }
    if jobs == 0 || count == 0 {
        return Ok(0..0);
    }
    let aligned = count - count % alignment;
    let base = (aligned / jobs) / alignment * alignment;
    let extra = (aligned - base * jobs) / alignment;
    let start = base * job + job.min(extra) * alignment;
    let mut length = base + if job < extra { alignment } else { 0 };
    let tail_job = if base == 0 {
        extra.min(jobs - 1)
    } else {
        jobs - 1
    };
    if job == tail_job {
        length += count % alignment;
    }
    if length == 0 {
        Ok(0..0)
    } else {
        Ok(start as usize..(start + length) as usize)
    }
}

/// FUN_140a73bf0's optional 20-byte object-distance descriptor. The final
/// flag selects the descriptor's center instead of the receiver camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativeShadowDistanceDescriptor {
    pub center: [f32; 3],
    pub radius: f32,
    pub override_center: bool,
}

/// The distance test alone, not a replacement for the full caster query.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativeShadowObjectDistanceQuery {
    pub point: [f32; 3],
    pub scale_squared: [f32; 4],
}

impl NativeShadowObjectDistanceQuery {
    /// FUN_140a73bf0: receiver camera at +0x86a0, not its ray at +0x86ac.
    /// Nonpositive radii select the original default 1.0. Unordered radii
    /// survive the native ordered <= comparison; do not sanitize their NaNs.
    pub fn from_descriptor(
        receiver_camera: [f32; 3],
        descriptor: Option<NativeShadowDistanceDescriptor>,
    ) -> Self {
        let radius = match descriptor {
            Some(value) if !(value.radius <= 0.0) => value.radius,
            _ => 1.0,
        };
        Self {
            point: descriptor
                .filter(|value| value.override_center)
                .map_or(receiver_camera, |value| value.center),
            scale_squared: [radius * radius; 4],
        }
    }

    /// The original +0x48 record is lower XYZ / near², upper XYZ / far².
    /// Reuse the independently verified ordered SIMD comparison rule.
    pub fn accepts(self, record: Option<[[f32; 4]; 2]>) -> bool {
        record.is_none_or(|record| {
            object_distance_rejection_mask(record, self.point, self.scale_squared) == 0
        })
    }
}

#[derive(Debug, Clone)]
struct TomeObject {
    bounds: [[f32; 3]; 2],
    object_ids: Range<usize>,
    distance_record: Option<[[f32; 4]; 2]>,
}

/// Load-time cache of Tome object bounds and authored render IDs. Build it
/// alongside the other immutable Tome query data, not inside the frame loop.
#[derive(Debug)]
pub struct ShadowCasterTome {
    objects: Vec<TomeObject>,
    object_ids: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowCasterObject<'a> {
    pub tome_index: u32,
    pub bounds: [[f32; 3]; 2],
    pub object_ids: &'a [u32],
    pub distance_record: Option<[[f32; 4]; 2]>,
}

impl ShadowCasterTome {
    /// Cache the original FUN_140a767e0 object-ID mapping and
    /// FUN_140a82710 six-float AABBs once for this Tome resource.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let count = object_count(bytes)?;
        let mut objects = Vec::with_capacity(count);
        let mut object_ids = Vec::new();
        for index in 0..count {
            let start = object_ids.len();
            append_object_ids_for_index(bytes, index as u32, &mut object_ids)?;
            let bounds = object_bounds(bytes, index as u32)?;
            objects.push(TomeObject {
                bounds,
                object_ids: start..object_ids.len(),
                distance_record: object_distance_bounds(bytes, index as u32)?,
            });
        }
        Ok(Self {
            objects,
            object_ids,
        })
    }

    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    pub fn object(&self, index: u32) -> Option<ShadowCasterObject<'_>> {
        let object = self.objects.get(index as usize)?;
        Some(ShadowCasterObject {
            tome_index: index,
            bounds: object.bounds,
            object_ids: &self.object_ids[object.object_ids.clone()],
            distance_record: object.distance_record,
        })
    }
}

/// A receiver state after the caller has constructed the original query
/// volume. Bounds and planes must be in Tome/game coordinates and correspond
/// to the same original receiver/cascade test. This type does not derive the
/// receiver extrusion, distance flags, or +0x1840/+0x1850 instance inputs.
#[derive(Debug, Clone, Copy)]
pub struct VerifiedShadowReceiverState<'a> {
    pub bounds: [[f32; 3]; 2],
    pub planes: &'a [[f32; 4]],
    pub active_planes: &'a [usize],
}

impl<'a> VerifiedShadowReceiverState<'a> {
    /// FUN_140a707e0 passes the extruded planes to FUN_140a77b10. That
    /// initializer leaves the world-space query bounds at +/-FLT_MAX.
    /// The receiver's finite fitting box is in light space and cannot be
    /// used as a world-space broad phase for this query.
    pub fn from_native_planes(planes: &'a [[f32; 4]], active_planes: &'a [usize]) -> Self {
        assert!(planes.len() <= 22 && active_planes.len() == planes.len());
        Self {
            bounds: [[-f32::MAX; 3], [f32::MAX; 3]],
            planes,
            active_planes,
        }
    }
}

/// Plane fields of FUN_140a72df0's fresh cascade contexts. The original
/// calls FUN_140a77560 with offset=[0,0,0], job=0, jobs=1 and partition=0.
/// This owns only the geometry consumed by caster tests, not its portal scratch.
#[derive(Clone, Copy, Debug)]
pub struct NativeShadowFrustumQuery {
    pub planes: [[f32; 4]; 6],
    pub bounds: [[f32; 4]; 2],
}

impl NativeShadowFrustumQuery {
    pub fn from_projection_rows(rows: [[f32; 4]; 4]) -> Result<Self, String> {
        let (planes, bounds) = super::query_plane_geometry_from_rows(rows, rows, [0.0; 3])?;
        Ok(Self { planes, bounds })
    }

    pub fn as_receiver(&self) -> VerifiedShadowReceiverState<'_> {
        const FRESH_PLANES: [usize; 6] = [0, 1, 2, 3, 4, 5];
        VerifiedShadowReceiverState {
            bounds: self.bounds.map(|row| [row[0], row[1], row[2]]),
            planes: &self.planes,
            active_planes: &FRESH_PLANES,
        }
    }
}

/// A global-mode FUN_140a73bf0 hit and the parallel auxiliary output mask.
/// The index is global even when the caller dispatches several aligned jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeGlobalShadowCasterHit {
    pub tome_index: u32,
    pub cascade_mask: u32,
}

/// Per-instance bytes published by FUN_14032a690 after each gatherCaster job.
/// These are independent of ordinary camera visibility. Storage is retained
/// across frames; the caller clears it once, merges every job, then applies
/// FUN_140330c10's ordered parent suppression before submitting any draws.
pub struct NativeShadowInstanceMasks {
    masks: Vec<Vec<u8>>,
}

impl NativeShadowInstanceMasks {
    pub fn new(instance_count: usize, cascade_count: usize) -> Self {
        Self {
            masks: (0..cascade_count)
                .map(|_| vec![0; instance_count])
                .collect(),
        }
    }

    pub fn clear(&mut self) {
        for mask in &mut self.masks {
            mask.fill(0);
        }
    }

    pub fn masks(&self) -> &[Vec<u8>] {
        &self.masks
    }

    ///32c2a0 intersects the WORLD tree result with the published Tome bytes.
    pub fn intersect_world_query(&mut self, cascade: usize, world: &[u8]) -> Result<(), String> {
        let mask = self.masks.get_mut(cascade).ok_or("invalid WORLD query cascade")?;
        if mask.len() != world.len() {
            return Err("WORLD and Tome instance mask lengths differ".into());
        }
        for (selected, &broad) in mask.iter_mut().zip(world) {
            *selected &= broad;
        }
        Ok(())
    }

    /// Native world+0x408 is the same Tome-index-to-BWM-instance list already
    /// decoded as WorldMesh::tome_object_instances. Empty lists stay empty:
    /// authored object IDs are not a replacement for these renderer indices.
    pub fn publish(
        &mut self,
        hits: &[NativeGlobalShadowCasterHit],
        tome_object_instances: &[Vec<usize>],
    ) -> Result<(), String> {
        let result = (|| {
            for hit in hits {
                let instances = tome_object_instances
                    .get(hit.tome_index as usize)
                    .ok_or("shadow caster Tome index exceeds world instance groups")?;
                for &instance in instances {
                    let mut bit = 1u32;
                    for mask in &mut self.masks {
                        let selected = mask
                            .get_mut(instance)
                            .ok_or("shadow caster world instance index exceeds mask storage")?;
                        if hit.cascade_mask & bit != 0 {
                            *selected = 1;
                        }
                        // SHL / sign-bit OR in the original is a rotate, not
                        // a shift that becomes undefined after 32 cascades.
                        bit = bit.rotate_left(1);
                    }
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.clear();
        }
        result
    }

    /// Native suppression visits groups in serialized order. A selected
    /// parent clears its children in THAT cascade only; it does not clear the
    /// parent, test main-view visibility, or recursively traverse a graph.
    pub fn suppress_parents<'a>(
        &mut self,
        groups: impl IntoIterator<Item = (usize, &'a [usize])>,
    ) -> Result<(), String> {
        let result = (|| {
            for (parent, children) in groups {
                for mask in &mut self.masks {
                    let selected = *mask
                        .get(parent)
                        .ok_or("shadow suppression parent exceeds instance mask storage")?;
                    if selected == 1 {
                        for &child in children {
                            *mask.get_mut(child).ok_or(
                                "shadow suppression child exceeds instance mask storage",
                            )? = 0;
                        }
                    }
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.clear();
        }
        result
    }
}

/// The global, no-visibility-bitset, accumulating-mask path in
/// FUN_140a73bf0, with auxiliary output and receiver+0x86bc bit4 clear.
/// Receiver+0x86ba is not read by this function: it is NOT the switch for
/// containment-stop mode. Do not reject a valid owner based on that byte.
/// The mode-bit4 and spatial-node branches remain separate paths.
/// `source` must be the same immutable Tome from which `tome` was parsed.
/// Receiver states must be freshly initialized (all plane indices in order),
/// not the reduced lists produced for a previously visited spatial node.
pub struct NativeGlobalShadowCasterInput<'a> {
    pub source: &'a [u8],
    pub primary: VerifiedShadowReceiverState<'a>,
    pub cascades: &'a [VerifiedShadowReceiverState<'a>],
    pub distance: NativeShadowObjectDistanceQuery,
    pub direction: NativeCasterDirection,
    pub depth: Option<&'a NativeReceiverDepthProjection>,
    pub job: u32,
    pub jobs: u32,
}

/// Original global gather scratch, retained across camera updates. Unlike the
/// older AABB-only candidate query, this evaluates distance records, baked
/// direction tables and receiver depth before publishing one index/mask pair.
pub struct NativeGlobalShadowCasterScratch {
    object_count: usize,
    primary_active: Vec<usize>,
    cascade_active: Vec<Vec<usize>>,
    cascade_enabled: Vec<bool>,
    output: Vec<NativeGlobalShadowCasterHit>,
}

impl NativeGlobalShadowCasterScratch {
    pub fn new(object_count: usize, cascade_count: usize) -> Self {
        Self {
            object_count,
            primary_active: Vec::with_capacity(22),
            cascade_active: (0..cascade_count).map(|_| Vec::with_capacity(22)).collect(),
            cascade_enabled: vec![false; cascade_count],
            output: Vec::with_capacity(object_count),
        }
    }

    /// FUN_140a73bf0 global branch (ctx null or ctx+30 false), with auxiliary
    /// masks and receiver mode bit4 clear. No ordinary-camera visibility list
    /// is consulted: off-camera objects can cast into a visible receiver.
    pub fn gather<'a>(
        &'a mut self,
        tome: &ShadowCasterTome,
        input: &NativeGlobalShadowCasterInput<'_>,
    ) -> Result<&'a [NativeGlobalShadowCasterHit], String> {
        self.output.clear();
        if tome.object_count() != self.object_count
            || object_count(input.source)? != self.object_count
        {
            return Err("global shadow gather belongs to a different Tome object domain".into());
        }
        if input.cascades.is_empty() || input.cascades.len() != self.cascade_active.len() {
            return Err(
                "global shadow auxiliary output requires matching nonempty cascades".into(),
            );
        }
        let bounds4 = |bounds: [[f32; 3]; 2]| bounds.map(|p| [p[0], p[1], p[2], 1.0]);
        let root = bounds4(recorded_bounds(input.source, 0x14)?);
        let fresh = |receiver: VerifiedShadowReceiverState<'_>| {
            receiver.planes.len() <= 22
                && receiver.active_planes.len() == receiver.planes.len()
                && receiver
                    .active_planes
                    .iter()
                    .enumerate()
                    .all(|(slot, &index)| slot == index)
        };
        if !fresh(input.primary) || input.cascades.iter().any(|&receiver| !fresh(receiver)) {
            return Err(
                "global shadow gather requires unreduced native receiver plane states".into(),
            );
        }
        if !query_bounds_accepts_aabb(
            bounds4(input.primary.bounds),
            root,
            input.primary.planes,
            input.primary.active_planes,
        )? {
            return Ok(&self.output);
        }
        active_query_planes_into(input.primary.planes, root, &mut self.primary_active);
        for (index, &receiver) in input.cascades.iter().enumerate() {
            self.cascade_enabled[index] = query_bounds_accepts_aabb(
                bounds4(receiver.bounds),
                root,
                receiver.planes,
                receiver.active_planes,
            )?;
            active_query_planes_into(receiver.planes, root, &mut self.cascade_active[index]);
        }
        let range = native_aligned_job_range(input.job, input.jobs, self.object_count as u32, 2)?;
        for index in range {
            let object = &tome.objects[index];
            if !input.distance.accepts(object.distance_record) {
                continue;
            }
            let bounds = bounds4(object.bounds);
            if !query_bounds_accepts_aabb(
                bounds4(input.primary.bounds),
                bounds,
                input.primary.planes,
                &self.primary_active,
            )? || !input.direction.allows_object(input.source, index as u32)?
            {
                continue;
            }
            if let Some(depth) = input.depth {
                if !receiver_depth_allows_aabb(depth, object.bounds)? {
                    continue;
                }
            }
            let mut cascade_mask = 0;
            for (cascade, &receiver) in input.cascades.iter().enumerate() {
                if self.cascade_enabled[cascade]
                    && query_bounds_accepts_aabb(
                        bounds4(receiver.bounds),
                        bounds,
                        receiver.planes,
                        &self.cascade_active[cascade],
                    )?
                {
                    // The original rotates the mask bit after bit31, not a
                    // Rust shift that would panic on a larger supplied count.
                    cascade_mask |= 1u32.rotate_left(cascade as u32);
                }
            }
            if cascade_mask != 0 {
                self.output.push(NativeGlobalShadowCasterHit {
                    tome_index: index as u32,
                    cascade_mask,
                });
            }
        }
        Ok(&self.output)
    }
}

/// A hierarchy edge already decoded with the original object's suppression
/// semantics: FUN_140330c10 removes `suppressed_child_index` from a receiver's
/// caster mask when `parent_index` is selected. Its raw parent-flag byte and
/// child-link list at +0x7c are intentionally not decoded or guessed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParentSuppressionEdge {
    pub parent_index: u32,
    pub suppressed_child_index: u32,
}

/// Reusable frame scratch. Its masks are indexed by Tome object index and its
/// output rows by receiver index; capacities are retained across `select` calls.
#[derive(Debug)]
pub struct ShadowCasterSelectionScratch {
    object_count: usize,
    seen: Vec<u32>,
    parent_nodes: Vec<u32>,
    suppressed_children: Vec<u32>,
    candidate_order: Vec<u32>,
    selected_by_receiver: Vec<Vec<u32>>,
    pre_suppression_by_receiver: Vec<Vec<u32>>,
    output_by_receiver: Vec<Vec<u32>>,
}

impl ShadowCasterSelectionScratch {
    pub fn new(object_count: usize, receiver_count: usize) -> Result<Self, String> {
        let word_count = object_count
            .checked_add(31)
            .ok_or("shadow caster mask size overflow")?
            / 32;
        Ok(Self {
            object_count,
            seen: vec![0; word_count],
            parent_nodes: vec![0; word_count],
            suppressed_children: vec![0; word_count],
            candidate_order: Vec::with_capacity(object_count),
            selected_by_receiver: (0..receiver_count).map(|_| vec![0; word_count]).collect(),
            pre_suppression_by_receiver: (0..receiver_count).map(|_| vec![0; word_count]).collect(),
            output_by_receiver: (0..receiver_count).map(|_| Vec::new()).collect(),
        })
    }

    /// Classify visible Tome candidates against verified receiver volumes,
    /// apply caller-decoded native caster eligibility and parent suppression,
    /// then expose reusable per-receiver object-index lists. `visible.count()`
    /// must fit its stored index buffer; a truncated list fails closed.
    ///
    /// `caster_enabled` is aligned to Tome object indices and must come from
    /// the original-backed object-flag path. `parent_suppression_edges` must
    /// contain the complete, already-decoded parent-to-suppressed-child edges;
    /// empty is valid only when the verified hierarchy has no such edges.
    /// Contained boxes stop testing later receiver volumes. Parent/child chains
    /// are rejected until their native traversal ordering is verified.
    pub fn select<'a>(
        &'a mut self,
        tome: &ShadowCasterTome,
        visible: &VisibleObjectIndexOutput,
        caster_enabled: &[bool],
        receivers: &[VerifiedShadowReceiverState<'_>],
        parent_suppression_edges: &[ParentSuppressionEdge],
    ) -> Result<&'a [Vec<u32>], String> {
        self.seen.fill(0);
        self.parent_nodes.fill(0);
        self.suppressed_children.fill(0);
        self.candidate_order.clear();
        for mask in &mut self.selected_by_receiver {
            mask.fill(0);
        }
        for output in &mut self.output_by_receiver {
            output.clear();
        }

        if tome.object_count() != self.object_count {
            return Err("shadow caster scratch belongs to a different Tome object domain".into());
        }
        if receivers.len() != self.selected_by_receiver.len() {
            return Err("shadow caster scratch receiver count mismatch".into());
        }
        if caster_enabled.len() != self.object_count {
            return Err("shadow caster flag table does not match Tome object count".into());
        }
        if visible.count() != visible.indices().len() {
            return Err(
                "ordinary Tome visibility output is truncated; caster query refused".into(),
            );
        }
        for edge in parent_suppression_edges {
            let parent = edge.parent_index as usize;
            let child = edge.suppressed_child_index as usize;
            if parent >= self.object_count || child >= self.object_count || parent == child {
                return Err("invalid shadow caster parent-suppression edge".into());
            }
            self.parent_nodes[parent / 32] |= 1u32 << (parent % 32);
            self.suppressed_children[child / 32] |= 1u32 << (child % 32);
        }
        if self
            .parent_nodes
            .iter()
            .zip(&self.suppressed_children)
            .any(|(parents, children)| parents & children != 0)
        {
            return Err("chained shadow parent-suppression semantics are unsupported".into());
        }

        for &index in visible.indices() {
            let object_index = index as usize;
            if object_index >= self.object_count {
                return Err("ordinary Tome visibility index exceeds caster object table".into());
            }
            let bit = 1u32 << (index % 32);
            let word = &mut self.seen[object_index / 32];
            if *word & bit != 0 {
                continue;
            }
            *word |= bit;
            self.candidate_order.push(index);
            if !caster_enabled[object_index] {
                continue;
            }

            let bounds = tome.objects[object_index].bounds;
            let input_bounds = [
                [bounds[0][0], bounds[0][1], bounds[0][2], 1.0],
                [bounds[1][0], bounds[1][1], bounds[1][2], 1.0],
            ];
            for (receiver_index, receiver) in receivers.iter().enumerate() {
                let receiver_bounds = [
                    [
                        receiver.bounds[0][0],
                        receiver.bounds[0][1],
                        receiver.bounds[0][2],
                        1.0,
                    ],
                    [
                        receiver.bounds[1][0],
                        receiver.bounds[1][1],
                        receiver.bounds[1][2],
                        1.0,
                    ],
                ];
                match classify_native_receiver_bounds(
                    receiver_bounds,
                    input_bounds,
                    receiver.planes,
                    receiver.active_planes,
                )? {
                    NativeReceiverBoundsClass::Outside => continue,
                    NativeReceiverBoundsClass::Intersecting => {
                        self.selected_by_receiver[receiver_index][object_index / 32] |= bit;
                    }
                    NativeReceiverBoundsClass::Contained => {
                        self.selected_by_receiver[receiver_index][object_index / 32] |= bit;
                        // FUN_140a76f30's containment result stops this
                        // candidate's later receiver/cascade tests.
                        break;
                    }
                }
            }
        }

        // Evaluate the verified one-level suppression edges against the
        // unsuppressed mask; chained graphs fail closed above.
        for (selected, original) in self
            .selected_by_receiver
            .iter_mut()
            .zip(&mut self.pre_suppression_by_receiver)
        {
            original.copy_from_slice(selected);
            for edge in parent_suppression_edges {
                let parent = edge.parent_index as usize;
                let child = edge.suppressed_child_index as usize;
                let parent_selected = original[parent / 32] & (1u32 << (parent % 32)) != 0;
                if parent_selected {
                    selected[child / 32] &= !(1u32 << (child % 32));
                }
            }
        }

        for (receiver_index, mask) in self.selected_by_receiver.iter().enumerate() {
            for &index in &self.candidate_order {
                if mask[index as usize / 32] & (1u32 << (index % 32)) != 0 {
                    self.output_by_receiver[receiver_index].push(index);
                }
            }
        }
        Ok(&self.output_by_receiver)
    }

    pub fn selected_indices(&self, receiver_index: usize) -> Option<&[u32]> {
        self.output_by_receiver
            .get(receiver_index)
            .map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caster_publication_uses_world_groups_and_keeps_cascades_separate() {
        let mut masks = NativeShadowInstanceMasks::new(6, 3);
        let groups = vec![vec![4, 1], vec![], vec![0, 2, 5]];
        masks
            .publish(
                &[
                    NativeGlobalShadowCasterHit {
                        tome_index: 0,
                        cascade_mask: 0b101,
                    },
                    NativeGlobalShadowCasterHit {
                        tome_index: 1,
                        cascade_mask: 7,
                    },
                ],
                &groups,
            )
            .unwrap();
        // A second job accumulates; the empty group did not invent an ID.
        masks
            .publish(
                &[NativeGlobalShadowCasterHit {
                    tome_index: 2,
                    cascade_mask: 0b010,
                }],
                &groups,
            )
            .unwrap();
        assert_eq!(
            masks.masks(),
            &[
                vec![0, 1, 0, 0, 1, 0],
                vec![1, 0, 1, 0, 0, 1],
                vec![0, 1, 0, 0, 1, 0],
            ]
        );
        masks
            .suppress_parents([(4, &[1, 5][..]), (0, &[2][..])])
            .unwrap();
        assert_eq!(
            masks.masks(),
            &[
                vec![0, 0, 0, 0, 1, 0],
                vec![1, 0, 0, 0, 0, 1],
                vec![0, 0, 0, 0, 1, 0],
            ]
        );
        masks.clear();
        assert!(masks.masks().iter().flatten().all(|value| *value == 0));
    }

    #[test]
    fn caster_parent_suppression_preserves_serialized_order() {
        let groups = vec![vec![0, 1, 2]];
        let hits = [NativeGlobalShadowCasterHit {
            tome_index: 0,
            cascade_mask: 1,
        }];
        let mut forward = NativeShadowInstanceMasks::new(3, 1);
        forward.publish(&hits, &groups).unwrap();
        forward
            .suppress_parents([(0, &[1][..]), (1, &[2][..])])
            .unwrap();
        assert_eq!(forward.masks(), &[vec![1, 0, 1]]);
        let mut reverse = NativeShadowInstanceMasks::new(3, 1);
        reverse.publish(&hits, &groups).unwrap();
        reverse
            .suppress_parents([(1, &[2][..]), (0, &[1][..])])
            .unwrap();
        assert_eq!(reverse.masks(), &[vec![1, 0, 0]]);
    }

    #[test]
    fn malformed_caster_publication_cannot_leave_partial_or_old_masks() {
        let mut masks = NativeShadowInstanceMasks::new(2, 1);
        let hit = [NativeGlobalShadowCasterHit {
            tome_index: 0,
            cascade_mask: 1,
        }];
        masks.publish(&hit, &[vec![0]]).unwrap();
        assert!(masks.publish(&hit, &[vec![1, 2]]).is_err());
        assert_eq!(masks.masks(), &[vec![0, 0]]);
        masks.publish(&hit, &[vec![0]]).unwrap();
        assert!(masks
            .publish(
                &[NativeGlobalShadowCasterHit {
                    tome_index: 1,
                    cascade_mask: 1
                },],
                &[vec![0]]
            )
            .is_err());
        assert_eq!(masks.masks(), &[vec![0, 0]]);
        masks.publish(&hit, &[vec![0]]).unwrap();
        assert!(masks.suppress_parents([(0, &[2][..])]).is_err());
        assert_eq!(masks.masks(), &[vec![0, 0]]);
    }

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
        write_u32(bytes, offset, value.to_bits());
    }

    fn fixture_tome() -> Vec<u8> {
        let count = 3usize;
        let bounds_at = 0x60;
        let ids_at = bounds_at + count * 0x18;
        let mut bytes = vec![0; ids_at + count * 4];
        write_u32(&mut bytes, 0x40, count as u32);
        write_u32(&mut bytes, 0x44, bounds_at as u32);
        write_u32(&mut bytes, 0x50, ids_at as u32);
        let centers = [3.0, 0.3, 0.5];
        for (index, center_x) in centers.into_iter().enumerate() {
            let at = bounds_at + index * 0x18;
            for (axis, center) in [center_x, 0.0, 0.0].into_iter().enumerate() {
                write_f32(&mut bytes, at + axis * 4, center - 0.1);
                write_f32(&mut bytes, at + 12 + axis * 4, center + 0.1);
            }
            write_u32(&mut bytes, ids_at + index * 4, 100 + index as u32);
        }
        bytes
    }

    #[test]
    #[ignore = "requires local original cascade context snapshots"]
    fn native_cascade_plane_geometry_matches_original_contexts() {
        let source =
            std::fs::read_to_string("artifacts/original-cascade-plane-geometry.txt").unwrap();
        let mut count = 0;
        for line in source.lines() {
            let words = line
                .split_whitespace()
                .map(|field| u32::from_str_radix(field, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 48);
            let rows = std::array::from_fn(|row| {
                std::array::from_fn(|column| f32::from_bits(words[row * 4 + column]))
            });
            let query = NativeShadowFrustumQuery::from_projection_rows(rows).unwrap();
            let actual = query
                .planes
                .into_iter()
                .flatten()
                .chain(query.bounds.into_iter().flatten())
                .map(f32::to_bits)
                .collect::<Vec<_>>();
            assert_eq!(actual, words[16..], "native cascade geometry {count}");
            assert_eq!(query.as_receiver().active_planes, [0, 1, 2, 3, 4, 5]);
            count += 1;
        }
        assert_eq!(count, 4);
    }

    /// Entire query input and ALL output IDs/masks from one invocation.
    /// Local original data is deliberately outside the source tree.
    #[test]
    #[ignore = "requires paired original gameplay caster capture and converter"]
    fn global_shadow_gather_matches_paired_original_gameplay() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let capture = std::env::var("DH2_SHADOW_CASTER_CAPTURE")
            .unwrap_or_else(|_| "artifacts/original-shadow-caster-paired-input.bin".into());
        let bytes = std::fs::read(root.join(capture))
            .expect("run analysis/convert_shadow_caster_capture.py");
        assert_eq!(&bytes[..4], b"DHC1");
        let u32_at = |data: &[u8], offset: usize| {
            u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
        };
        let f32_at = |data: &[u8], offset: usize| f32::from_bits(u32_at(data, offset));
        let path_len = u32_at(&bytes, 4) as usize;
        let receiver_len = u32_at(&bytes, 8) as usize;
        let descriptor_len = u32_at(&bytes, 12) as usize;
        let job = u32_at(&bytes, 16);
        let jobs = u32_at(&bytes, 20);
        let count = u32_at(&bytes, 24) as usize;
        let output_count = u32_at(&bytes, 28) as usize;
        let path = std::str::from_utf8(&bytes[32..32 + path_len]).unwrap();
        let source = std::fs::read(root.join(path)).unwrap();
        let tome = ShadowCasterTome::parse(&source).unwrap();
        let at = 32 + path_len;
        let receiver = &bytes[at..at + receiver_len];
        assert_eq!(receiver_len, 0x86d0 + count * 0x590);
        assert_eq!(u32_at(receiver, 0x86bc) & 16, 0, "alternate gather mode");
        let at = at + receiver_len;
        let descriptor = &bytes[at..at + descriptor_len];
        let descriptor = if descriptor.is_empty() {
            None
        } else {
            assert_eq!(descriptor.len(), 20);
            Some(NativeShadowDistanceDescriptor {
                center: std::array::from_fn(|axis| f32_at(descriptor, axis * 4)),
                radius: f32_at(descriptor, 12),
                override_center: descriptor[16] != 0,
            })
        };
        let read_context = |context: &[u8]| {
            assert_eq!(context.len(), 0x590);
            let count = u32_at(context, 0x4ac) as usize;
            assert!(count <= 22);
            let bounds = std::array::from_fn(|side| {
                std::array::from_fn(|axis| f32_at(context, 0x530 + side * 16 + axis * 4))
            });
            let planes: Vec<[f32; 4]> = (0..count)
                .map(|index| {
                    std::array::from_fn(|axis| f32_at(context, 0x80 + index * 16 + axis * 4))
                })
                .collect();
            let active: Vec<usize> = (0..count)
                .map(|index| u32_at(context, 0x4d4 + index * 4) as usize)
                .collect();
            (bounds, planes, active)
        };
        let (bounds, planes, active) = read_context(&receiver[0x8110..0x86a0]);
        let primary = VerifiedShadowReceiverState::from_native_planes(&planes, &active);
        assert_eq!(
            primary.bounds, bounds,
            "live plane-only handoff must match the original query, not the receiver fitting box"
        );
        let contexts: Vec<_> = (0..count)
            .map(|index| {
                let at = 0x86d0 + index * 0x590;
                read_context(&receiver[at..at + 0x590])
            })
            .collect();
        let cascades: Vec<_> = contexts
            .iter()
            .map(|(bounds, planes, active)| VerifiedShadowReceiverState {
                bounds: *bounds,
                planes,
                active_planes: active,
            })
            .collect();
        let depth = (receiver[0x86b8] != 0).then(|| NativeReceiverDepthProjection {
            receiver_rows: std::array::from_fn(|row| {
                std::array::from_fn(|column| f32_at(receiver, row * 16 + column * 4))
            }),
            transposed_rows: std::array::from_fn(|row| {
                std::array::from_fn(|column| f32_at(receiver, 0x40 + row * 16 + column * 4))
            }),
            mask: Box::new(std::array::from_fn(|index| {
                u16::from_le_bytes(
                    receiver[0xf0 + index * 2..0xf2 + index * 2]
                        .try_into()
                        .unwrap(),
                )
            })),
        });
        let input = NativeGlobalShadowCasterInput {
            source: &source,
            primary,
            cascades: &cascades,
            distance: NativeShadowObjectDistanceQuery::from_descriptor(
                std::array::from_fn(|axis| f32_at(receiver, 0x86a0 + axis * 4)),
                descriptor,
            ),
            direction: NativeCasterDirection::from_receiver_direction(std::array::from_fn(
                |axis| f32_at(receiver, 0x86ac + axis * 4),
            ))
            .unwrap(),
            depth: depth.as_ref(),
            job,
            jobs,
        };
        let mut scratch = NativeGlobalShadowCasterScratch::new(tome.object_count(), count);
        let actual = scratch.gather(&tome, &input).unwrap();
        let output_at = at + descriptor_len;
        assert_eq!(bytes.len(), output_at + output_count * 8);
        let expected: Vec<_> = (0..output_count)
            .map(|index| NativeGlobalShadowCasterHit {
                tome_index: u32_at(&bytes, output_at + index * 4),
                cascade_mask: u32_at(&bytes, output_at + output_count * 4 + index * 4),
            })
            .collect();
        assert_eq!(
            actual, expected,
            "paired full native gather output, not a sampled count"
        );
    }

    #[test]
    fn global_shadow_gather_accumulates_masks_and_uses_full_object_domain() {
        let mut bytes = fixture_tome();
        for (row, values) in [[-4.0, -2.0, -2.0], [4.0, 2.0, 2.0]]
            .into_iter()
            .enumerate()
        {
            for (axis, value) in values.into_iter().enumerate() {
                write_f32(&mut bytes, 0x14 + row * 12 + axis * 4, value);
            }
        }
        let tome = ShadowCasterTome::parse(&bytes).unwrap();
        let primary_bounds = [[-1.0; 3], [1.0; 3]];
        let second_bounds = [[0.45, -1.0, -1.0], [1.0; 3]];
        let primary_planes = box_planes(primary_bounds);
        let second_planes = box_planes(second_bounds);
        let indices = [0, 1, 2, 3, 4, 5];
        let primary = VerifiedShadowReceiverState {
            bounds: primary_bounds,
            planes: &primary_planes,
            active_planes: &indices,
        };
        let cascades = [
            primary,
            VerifiedShadowReceiverState {
                bounds: second_bounds,
                planes: &second_planes,
                active_planes: &indices,
            },
        ];
        let mut input = NativeGlobalShadowCasterInput {
            source: &bytes,
            primary,
            cascades: &cascades,
            distance: NativeShadowObjectDistanceQuery::from_descriptor([0.0; 3], None),
            direction: NativeCasterDirection::from_receiver_direction([0.0; 3]).unwrap(),
            depth: None,
            job: 0,
            jobs: 1,
        };
        let mut scratch = NativeGlobalShadowCasterScratch::new(tome.object_count(), cascades.len());
        let expected = vec![
            NativeGlobalShadowCasterHit {
                tome_index: 1,
                cascade_mask: 1,
            },
            NativeGlobalShadowCasterHit {
                tome_index: 2,
                cascade_mask: 3,
            },
        ];
        assert_eq!(scratch.gather(&tome, &input).unwrap(), expected);
        let capacities = (
            scratch.output.capacity(),
            scratch.primary_active.capacity(),
            scratch
                .cascade_active
                .iter()
                .map(Vec::capacity)
                .collect::<Vec<_>>(),
        );
        let mut combined = Vec::new();
        input.jobs = 8;
        for job in 0..8 {
            input.job = job;
            combined.extend_from_slice(scratch.gather(&tome, &input).unwrap());
        }
        assert_eq!(combined, expected);
        assert_eq!(
            capacities,
            (
                scratch.output.capacity(),
                scratch.primary_active.capacity(),
                scratch
                    .cascade_active
                    .iter()
                    .map(Vec::capacity)
                    .collect::<Vec<_>>()
            )
        );
        // No ordinary-camera visible list is supplied or consulted.
        let identity = std::array::from_fn(|row| {
            std::array::from_fn(|column| if row == column { 1.0 } else { 0.0 })
        });
        let depth = NativeReceiverDepthProjection {
            receiver_rows: identity,
            transposed_rows: identity,
            mask: Box::new([0; 128 * 128]),
        };
        input.job = 0;
        input.jobs = 1;
        input.depth = Some(&depth);
        assert!(scratch.gather(&tome, &input).unwrap().is_empty());
        let unoccluded = NativeReceiverDepthProjection {
            receiver_rows: identity,
            transposed_rows: identity,
            mask: Box::new([u16::MAX; 128 * 128]),
        };
        input.depth = Some(&unoccluded);
        assert_eq!(scratch.gather(&tome, &input).unwrap(), expected);
        input.jobs = 0;
        assert!(scratch.gather(&tome, &input).unwrap().is_empty());
    }

    #[test]
    fn global_shadow_gather_applies_distance_and_baked_direction_before_masks() {
        let mut bytes = fixture_tome();
        for (row, values) in [[-4.0, -2.0, -2.0], [4.0, 2.0, 2.0]]
            .into_iter()
            .enumerate()
        {
            for (axis, value) in values.into_iter().enumerate() {
                write_f32(&mut bytes, 0x14 + row * 12 + axis * 4, value);
            }
        }
        let distances = 0x180;
        let bits = distances + 3 * 32;
        bytes.resize(bits + 6 * 256 * 3 / 8, 0);
        write_u32(&mut bytes, 0x48, distances as u32);
        write_u32(&mut bytes, 0x10, 2);
        write_u32(&mut bytes, 0x140, bits as u32);
        write_u32(&mut bytes, bits, 0b110);
        for object in 0..3 {
            for (index, value) in [-1.0, -1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 10.0]
                .into_iter()
                .enumerate()
            {
                write_f32(&mut bytes, distances + object * 32 + index * 4, value);
            }
        }
        // A zero far² rejects index1 even though its baked bit is present.
        write_f32(&mut bytes, distances + 32 + 28, 0.0);
        let tome = ShadowCasterTome::parse(&bytes).unwrap();
        let bounds = [[-4.0, -2.0, -2.0], [4.0, 2.0, 2.0]];
        let planes = box_planes(bounds);
        let indices = [0, 1, 2, 3, 4, 5];
        let receiver = VerifiedShadowReceiverState {
            bounds,
            planes: &planes,
            active_planes: &indices,
        };
        let cascades = [receiver];
        let mut input = NativeGlobalShadowCasterInput {
            source: &bytes,
            primary: receiver,
            cascades: &cascades,
            distance: NativeShadowObjectDistanceQuery::from_descriptor([0.0; 3], None),
            direction: NativeCasterDirection::from_receiver_direction([0.0; 3]).unwrap(),
            depth: None,
            job: 0,
            jobs: 1,
        };
        let mut scratch = NativeGlobalShadowCasterScratch::new(3, 1);
        assert_eq!(
            scratch.gather(&tome, &input).unwrap(),
            [NativeGlobalShadowCasterHit {
                tome_index: 2,
                cascade_mask: 1
            }]
        );
        input.cascades = &[];
        assert!(scratch.gather(&tome, &input).is_err());
        input.cascades = &cascades;
        input.primary.active_planes = &[1, 0, 2, 3, 4, 5];
        assert!(scratch.gather(&tome, &input).is_err());
    }

    #[test]
    fn receiver_depth_query_uses_strict_integer_depth_and_one_pixel_endpoints() {
        let identity = std::array::from_fn(|row| {
            std::array::from_fn(|column| if row == column { 1.0 } else { 0.0 })
        });
        let mut state = NativeReceiverDepthProjection {
            receiver_rows: identity,
            transposed_rows: identity,
            mask: Box::new([0; 128 * 128]),
        };
        let center = [[0.0, 0.0, 0.5]; 2];
        assert!(!receiver_depth_allows_aabb(&state, center).unwrap());
        state.mask[64 * 128 + 64] = 32768;
        assert!(receiver_depth_allows_aabb(&state, center).unwrap());
        state.mask[64 * 128 + 64] = 32767;
        assert!(!receiver_depth_allows_aabb(&state, center).unwrap());
        state.mask[64 * 128 + 64] = 65534;
        assert!(!receiver_depth_allows_aabb(&state, [[0.0, 0.0, 1.0]; 2]).unwrap());
        state.mask[64 * 128 + 64] = 65535;
        assert!(receiver_depth_allows_aabb(&state, [[0.0, 0.0, 1.0]; 2]).unwrap());
        state.mask[0] = 1;
        assert!(receiver_depth_allows_aabb(&state, [[-1.0, -1.0, 0.0]; 2]).unwrap());
        assert!(!receiver_depth_allows_aabb(&state, [[1.0, 1.0, 0.0]; 2]).unwrap());
    }

    #[test]
    fn receiver_depth_query_reads_transposed_owner_and_rejects_invalid_uv() {
        let identity = std::array::from_fn(|row| {
            std::array::from_fn(|column| if row == column { 1.0 } else { 0.0 })
        });
        let mut state = NativeReceiverDepthProjection {
            receiver_rows: identity,
            transposed_rows: identity,
            mask: Box::new([65535; 128 * 128]),
        };
        state.transposed_rows[3][0] = 3.0;
        assert!(!receiver_depth_allows_aabb(&state, [[0.0, 0.0, 0.5]; 2]).unwrap());
        state.transposed_rows[3][0] = f32::NAN;
        assert!(receiver_depth_allows_aabb(&state, [[0.0, 0.0, 0.5]; 2]).is_err());
    }

    #[test]
    fn shadow_distance_descriptor_preserves_native_defaults_and_center() {
        let camera = [19.0, -23.0, 7.0];
        let mut descriptor = NativeShadowDistanceDescriptor {
            center: [-1.0, 2.0, 3.0],
            radius: 0.6,
            override_center: false,
        };
        let query = NativeShadowObjectDistanceQuery::from_descriptor(camera, Some(descriptor));
        assert_eq!(query.point, camera);
        assert_eq!(query.scale_squared, [0.6f32 * 0.6; 4]);
        descriptor.override_center = true;
        assert_eq!(
            NativeShadowObjectDistanceQuery::from_descriptor(camera, Some(descriptor)).point,
            descriptor.center
        );
        for radius in [0.0, -0.0, -1.0] {
            descriptor.radius = radius;
            assert_eq!(
                NativeShadowObjectDistanceQuery::from_descriptor(camera, Some(descriptor))
                    .scale_squared,
                [1.0; 4]
            );
        }
        assert_eq!(
            NativeShadowObjectDistanceQuery::from_descriptor(camera, None).point,
            camera
        );
        descriptor.radius = f32::from_bits(0x7fc00123);
        assert!(
            NativeShadowObjectDistanceQuery::from_descriptor(camera, Some(descriptor))
                .scale_squared
                .iter()
                .all(|value| value.is_nan())
        );
    }

    #[test]
    fn shadow_distance_filter_reuses_original_executable_cases() {
        let mut count = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/object_distance_cases.txt"
        ))
        .lines()
        {
            let words = line
                .split_whitespace()
                .map(|field| field.parse::<u32>().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 17);
            let point = std::array::from_fn(|axis| f32::from_bits(words[axis]));
            let record = std::array::from_fn(|row| {
                std::array::from_fn(|axis| f32::from_bits(words[4 + row * 4 + axis]))
            });
            let scale_squared = std::array::from_fn(|axis| f32::from_bits(words[12 + axis]));
            let query = NativeShadowObjectDistanceQuery {
                point,
                scale_squared,
            };
            assert_eq!(
                query.accepts(Some(record)),
                words[16] == 0,
                "shadow distance case {count}"
            );
            assert!(query.accepts(None));
            count += 1;
        }
        assert_eq!(count, 822);
    }

    #[test]
    fn shadow_tome_caches_optional_distance_records_without_sanitizing() {
        let mut bytes = fixture_tome();
        let table = bytes.len();
        bytes.resize(table + 3 * 32, 0);
        write_u32(&mut bytes, 0x48, table as u32);
        let record = [
            [-1.0, -2.0, -3.0, f32::from_bits(0x7fc00123)],
            [4.0, 5.0, 6.0, 25.0],
        ];
        for object in 0..3 {
            for (index, value) in record.into_iter().flatten().enumerate() {
                write_f32(&mut bytes, table + object * 32 + index * 4, value);
            }
        }
        let tome = ShadowCasterTome::parse(&bytes).unwrap();
        for object in 0..3 {
            let actual = tome.object(object).unwrap().distance_record.unwrap();
            assert_eq!(
                actual.map(|row| row.map(f32::to_bits)),
                record.map(|row| row.map(f32::to_bits))
            );
        }
        let absent = ShadowCasterTome::parse(&fixture_tome()).unwrap();
        assert!(absent.object(0).unwrap().distance_record.is_none());
        bytes.truncate(table + 3 * 32 - 1);
        assert!(ShadowCasterTome::parse(&bytes).is_err());
    }

    fn box_planes(bounds: [[f32; 3]; 2]) -> [[f32; 4]; 6] {
        [
            [1.0, 0.0, 0.0, -bounds[0][0]],
            [-1.0, 0.0, 0.0, bounds[1][0]],
            [0.0, 1.0, 0.0, -bounds[0][1]],
            [0.0, -1.0, 0.0, bounds[1][1]],
            [0.0, 0.0, 1.0, -bounds[0][2]],
            [0.0, 0.0, -1.0, bounds[1][2]],
        ]
    }

    fn bounds4(bounds: [[f32; 3]; 2]) -> [[f32; 4]; 2] {
        [
            [bounds[0][0], bounds[0][1], bounds[0][2], 1.0],
            [bounds[1][0], bounds[1][1], bounds[1][2], 1.0],
        ]
    }

    #[test]
    fn receiver_depth_projection_matches_original_executable() {
        let mut count = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/receiver_depth_projection_cases.txt"
        ))
        .lines()
        {
            let mut fields = line.split_whitespace();
            let seed = fields.next().unwrap().parse::<u16>().unwrap();
            let words = fields
                .map(|field| u32::from_str_radix(field, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 32 + 1 + 4096 + 32 + 16384);
            let rows = |offset| {
                std::array::from_fn(|row| {
                    std::array::from_fn(|column| f32::from_bits(words[offset + row * 4 + column]))
                })
            };
            let depths = std::array::from_fn(|index| f32::from_bits(words[33 + index]));
            let mut state = NativeReceiverDepthProjection {
                receiver_rows: [[12.25; 4]; 4],
                transposed_rows: [[12.25; 4]; 4],
                mask: Box::new([seed; 128 * 128]),
            };
            fill_receiver_depth_projection(
                &mut state,
                &depths,
                rows(0),
                rows(16),
                f32::from_bits(words[32]),
            );
            for (index, value) in state
                .receiver_rows
                .into_iter()
                .flatten()
                .chain(state.transposed_rows.into_iter().flatten())
                .enumerate()
            {
                assert_eq!(
                    value.to_bits(),
                    words[4129 + index],
                    "projection {count} matrix word {index}"
                );
            }
            for (index, value) in state.mask.iter().copied().enumerate() {
                assert_eq!(
                    u32::from(value),
                    words[4161 + index],
                    "projection {count} mask sample {index}"
                );
            }
            count += 1;
        }
        assert_eq!(count, 8);
    }

    #[test]
    fn receiver_depth_mask_matches_original_executable() {
        let mut count = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/receiver_mask_cases.txt"
        ))
        .lines()
        {
            let mut fields = line.split_whitespace();
            let seed = fields.next().unwrap().parse::<u16>().unwrap();
            let expected_result = fields.next().unwrap().parse::<u32>().unwrap() != 0;
            let bounds: [f32; 6] = std::array::from_fn(|_| {
                f32::from_bits(u32::from_str_radix(fields.next().unwrap(), 16).unwrap())
            });
            let mut mask = [seed; 128 * 128];
            assert_eq!(
                rasterize_receiver_depth_mask(
                    &mut mask,
                    [bounds[0], bounds[1], bounds[2]],
                    [bounds[3], bounds[4], bounds[5]]
                ),
                expected_result,
                "mask case {count}"
            );
            for (index, value) in mask.into_iter().enumerate() {
                let expected = u16::from_str_radix(fields.next().unwrap(), 16).unwrap();
                assert_eq!(value, expected, "mask case {count} sample {index}");
            }
            assert!(fields.next().is_none());
            count += 1;
        }
        assert_eq!(count, 32);
    }

    #[test]
    fn coarse_receiver_depth_matches_original_executable() {
        let mut count = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/receiver_coarse_depth_cases.txt"
        ))
        .lines()
        {
            let words = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 4096 + 256);
            let input = std::array::from_fn(|index| f32::from_bits(words[index]));
            let actual = coarse_receiver_depth(&input);
            for (tile, value) in actual.into_iter().enumerate() {
                assert_eq!(
                    value.to_bits(),
                    words[4096 + tile],
                    "original coarse depth case {count} tile {tile}"
                );
            }
            count += 1;
        }
        assert_eq!(count, 32);
    }

    #[test]
    fn native_normalized_frustum_matches_original_executable() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/shadow_frustum_plane_cases.txt");
        let rows = std::fs::read_to_string(path)
            .expect("run analysis/probe_original_shadow_receiver_geometry.py");
        let mut cases = 0;
        for (case, row) in rows.lines().enumerate() {
            let words: Vec<u32> = row
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect();
            assert_eq!(words.len(), 41, "original frustum case {case}");
            let matrix = std::array::from_fn(|row| {
                std::array::from_fn(|column| f32::from_bits(words[row * 4 + column]))
            });
            let (planes, finite_far) = native_normalized_frustum_planes(matrix);
            assert_eq!(
                u32::from(finite_far),
                words[40],
                "original far flag case {case}"
            );
            for (index, value) in planes.iter().flatten().enumerate() {
                assert_eq!(
                    value.to_bits(),
                    words[16 + index],
                    "original normalized frustum case {case}, coefficient {index}"
                );
            }
            cases += 1;
        }
        assert_eq!(cases, 512);
    }

    #[test]
    fn native_receiver_geometry_matches_original_executable() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/shadow_receiver_geometry_cases.txt");
        let rows = std::fs::read_to_string(path)
            .expect("run analysis/probe_original_shadow_receiver_geometry.py");
        let mut cases = 0;
        for (case, row) in rows.lines().enumerate() {
            let words: Vec<u32> = row
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect();
            assert_eq!(words.len(), 180, "original receiver case {case}");
            let direction = std::array::from_fn(|axis| f32::from_bits(words[axis]));
            let planes = std::array::from_fn(|plane| {
                std::array::from_fn(|axis| f32::from_bits(words[3 + plane * 4 + axis]))
            });
            let corners = native_frustum_plane_corners(&planes);
            for (index, value) in corners.iter().flatten().enumerate() {
                assert_eq!(
                    value.to_bits(),
                    words[27 + index],
                    "original corner case {case}, word {index}"
                );
            }
            let mut output = [[12.25; 4]; 32];
            let count = extrude_native_receiver_planes(direction, &planes, &mut output);
            assert_eq!(
                count, words[51] as usize,
                "original receiver count case {case}"
            );
            for (index, value) in output.iter().flatten().enumerate() {
                assert_eq!(value.to_bits(), words[52 + index],
                    "original receiver case {case}, word {index}, actual plane {:?}, expected plane {:?}",
                    output[index / 4].map(f32::to_bits),
                    &words[52 + (index / 4) * 4..56 + (index / 4) * 4]);
            }
            cases += 1;
        }
        assert_eq!(cases, 512);
    }

    #[test]
    fn receiver_bounds_classification_distinguishes_outside_partial_and_contained() {
        let receiver = [[-1.0; 3], [1.0; 3]];
        let planes = box_planes(receiver);
        let active = [0, 1, 2, 3, 4, 5];
        assert_eq!(
            classify_native_receiver_bounds(
                bounds4(receiver),
                bounds4([[-0.25; 3], [0.25; 3]]),
                &planes,
                &active,
            )
            .unwrap(),
            NativeReceiverBoundsClass::Contained
        );
        assert_eq!(
            classify_native_receiver_bounds(
                bounds4(receiver),
                bounds4([[0.5, -0.1, -0.1], [1.5, 0.1, 0.1]]),
                &planes,
                &active,
            )
            .unwrap(),
            NativeReceiverBoundsClass::Intersecting
        );
        assert_eq!(
            classify_native_receiver_bounds(
                bounds4(receiver),
                bounds4([[2.0, -0.1, -0.1], [2.5, 0.1, 0.1]]),
                &planes,
                &active,
            )
            .unwrap(),
            NativeReceiverBoundsClass::Outside
        );
    }

    #[test]
    fn native_direction_keeps_axis_ties_and_unclamped_bins() {
        assert_eq!(
            NativeCasterDirection::from_receiver_direction([0.0; 3]).unwrap(),
            NativeCasterDirection {
                face: 0,
                x: 0,
                y: 0
            }
        );
        assert_eq!(
            NativeCasterDirection::from_receiver_direction([1.0, 0.0, 0.0]).unwrap(),
            NativeCasterDirection {
                face: 0,
                x: 8,
                y: 8
            }
        );
        assert_eq!(
            NativeCasterDirection::from_receiver_direction([-1.0, -1.0, -1.0]).unwrap(),
            NativeCasterDirection {
                face: 1,
                x: 16,
                y: 16
            }
        );
        assert_eq!(
            NativeCasterDirection::from_receiver_direction([0.0, -1.0, -1.0]).unwrap(),
            NativeCasterDirection {
                face: 3,
                x: 16,
                y: 8
            }
        );
        assert_eq!(
            NativeCasterDirection::from_receiver_direction([0.07612236, -0.4317113, -0.89879405,])
                .unwrap(),
            NativeCasterDirection {
                face: 5,
                x: 7,
                y: 11
            }
        );
        assert!(NativeCasterDirection::from_receiver_direction([f32::NAN, 0.0, 0.0]).is_err());
    }

    #[test]
    fn native_direction_decodes_palette_and_dense_forms() {
        let direction = NativeCasterDirection {
            face: 2,
            x: 3,
            y: 1,
        };
        let mut bytes = vec![0u8; 0x1100];
        let put = |bytes: &mut [u8], at: usize, value: u32| {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };
        put(&mut bytes, 0x40, 2);
        assert!(direction.allows_object(&bytes, 1).unwrap());
        put(&mut bytes, 0x10, 1);
        put(&mut bytes, 0x13c, 0x180);
        put(&mut bytes, 0x140, 0x240);
        put(&mut bytes, 0x144, 0x400);
        put(&mut bytes, 0x148, 2);
        let record = 0x180 + 0x40 + 2 * 8;
        put(&mut bytes, record, 5);
        put(&mut bytes, record + 4, 8);
        let bit = (19 * 2 + 5) * 4;
        put(&mut bytes, 0x240 + (bit / 32) * 4, 6 << (bit % 32));
        bytes[0x400 + 14 * 2..0x400 + 15 * 2].copy_from_slice(&0x7f7fu16.to_le_bytes());
        assert!(direction.allows_object(&bytes, 1).unwrap());
        bytes[0x400 + 14 * 2] = 0;
        assert!(!direction.allows_object(&bytes, 1).unwrap());
        put(&mut bytes, 0x10, 2);
        let bit = 19 * 2 + 2 * 2 * 256 + 1;
        put(&mut bytes, 0x240 + (bit / 32) * 4, 1 << (bit % 32));
        assert!(direction.allows_object(&bytes, 1).unwrap());
        assert!(!direction.allows_object(&bytes, 0).unwrap());
        assert!(direction.allows_object(&bytes[..0x240], 1).is_err());
    }

    #[test]
    fn native_jobs_match_original_executable_cases() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/shadow_job_cases.txt");
        let rows = std::fs::read_to_string(path).expect("missing tracked shadow job fixture");
        let mut count = 0;
        for row in rows.lines() {
            let values: Vec<u32> = row.split_whitespace().map(|v| v.parse().unwrap()).collect();
            let expected = if values[5] == 0 {
                0..0
            } else {
                values[4] as usize..(values[4] + values[5]) as usize
            };
            assert_eq!(
                native_aligned_job_range(values[0], values[1], values[2], values[3]).unwrap(),
                expected,
                "original job case {row}"
            );
            count += 1;
        }
        assert_eq!(count, 2081);
    }

    #[test]
    fn native_jobs_preserve_alignment_and_short_worker_tail() {
        assert_eq!(native_aligned_job_range(0, 8, 70753, 2).unwrap(), 0..8844);
        assert_eq!(
            native_aligned_job_range(7, 8, 70753, 2).unwrap(),
            61908..70753
        );
        assert_eq!(native_aligned_job_range(0, 8, 1, 2).unwrap(), 0..1);
        assert_eq!(native_aligned_job_range(7, 8, 1, 2).unwrap(), 0..0);
        assert_eq!(native_aligned_job_range(2, 8, 5, 2).unwrap(), 4..5);
        assert_eq!(native_aligned_job_range(7, 8, 5, 2).unwrap(), 0..0);
        assert_eq!(native_aligned_job_range(0, 0, 5, 2).unwrap(), 0..0);
        assert!(native_aligned_job_range(8, 8, 70753, 2).is_err());
        assert!(native_aligned_job_range(0, 8, 70753, 0).is_err());
    }

    #[test]
    fn cached_tome_selection_classifies_receivers_and_suppresses_verified_children() {
        let bytes = fixture_tome();
        let tome = ShadowCasterTome::parse(&bytes).unwrap();
        assert_eq!(tome.object_count(), 3);
        assert_eq!(tome.object(2).unwrap().object_ids, [102]);

        let narrow = [[-0.25; 3], [0.25; 3]];
        let broad = [[-1.0; 3], [1.0; 3]];
        let narrow_planes = box_planes(narrow);
        let broad_planes = box_planes(broad);
        let receivers = [
            VerifiedShadowReceiverState {
                bounds: narrow,
                planes: &narrow_planes,
                active_planes: &[0, 1, 2, 3, 4, 5],
            },
            VerifiedShadowReceiverState {
                bounds: broad,
                planes: &broad_planes,
                active_planes: &[0, 1, 2, 3, 4, 5],
            },
        ];
        let mut visible = VisibleObjectIndexOutput::new(3, 3);
        visible.push_unique(2).unwrap();
        visible.push_unique(1).unwrap();
        visible.push_unique(0).unwrap();
        visible.push_unique(2).unwrap();

        let edges = [ParentSuppressionEdge {
            parent_index: 2,
            suppressed_child_index: 1,
        }];
        let mut scratch = ShadowCasterSelectionScratch::new(3, receivers.len()).unwrap();
        let selected = scratch
            .select(&tome, &visible, &[true; 3], &receivers, &edges)
            .unwrap();
        assert_eq!(selected, &vec![vec![1], vec![2]]);
        assert_eq!(tome.object(selected[0][0]).unwrap().object_ids, [101]);
        assert_eq!(tome.object(selected[1][0]).unwrap().object_ids, [102]);

        let chained_edges = [
            ParentSuppressionEdge {
                parent_index: 2,
                suppressed_child_index: 1,
            },
            ParentSuppressionEdge {
                parent_index: 1,
                suppressed_child_index: 0,
            },
        ];
        assert!(scratch
            .select(&tome, &visible, &[true; 3], &receivers, &chained_edges)
            .unwrap_err()
            .contains("chained"));
        assert_eq!(scratch.selected_indices(0), Some(&[][..]));
    }

    #[test]
    fn caster_flags_and_truncated_visible_outputs_fail_closed() {
        let bytes = fixture_tome();
        let tome = ShadowCasterTome::parse(&bytes).unwrap();
        let bounds = [[-1.0; 3], [1.0; 3]];
        let planes = box_planes(bounds);
        let receivers = [VerifiedShadowReceiverState {
            bounds,
            planes: &planes,
            active_planes: &[0, 1, 2, 3, 4, 5],
        }];
        let mut visible = VisibleObjectIndexOutput::new(3, 1);
        visible.push_unique(1).unwrap();
        visible.push_unique(2).unwrap();
        let mut scratch = ShadowCasterSelectionScratch::new(3, 1).unwrap();
        assert!(scratch
            .select(&tome, &visible, &[true; 3], &receivers, &[])
            .unwrap_err()
            .contains("truncated"));

        let mut visible = VisibleObjectIndexOutput::new(3, 3);
        visible.push_unique(1).unwrap();
        assert!(scratch
            .select(&tome, &visible, &[true], &receivers, &[])
            .unwrap_err()
            .contains("flag table"));
        assert_eq!(
            scratch
                .select(&tome, &visible, &[false; 3], &receivers, &[])
                .unwrap(),
            &[Vec::<u32>::new()]
        );
    }
}
