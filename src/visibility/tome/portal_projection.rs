//! Portal projection.
use super::{PortalTileMask, native_projected_corner_bounds};

/// Query-context matrix and six face-vector sets prepared by FUN_140a81500.
/// Keep setup errors deferred until a clipped portal actually consumes them.
pub struct PortalQueryProjection {
    matrix: [[f32; 4]; 4],
    winding_flip: bool,
    matrix_finite: bool,
    faces: Result<[[[f32; 4]; 3]; 6], String>,
}

impl PortalQueryProjection {
    pub fn new(matrix: [[f32; 4]; 4], winding_flip: bool) -> Self {
        Self {
            matrix,
            winding_flip,
            matrix_finite: matrix.iter().flatten().all(|value| value.is_finite()),
            faces: portal_face_vectors_from_query_matrix(matrix, winding_flip),
        }
    }
}

/// Transform a world-space portal corner with the row-major matrix stored in
/// the Umbra query context at +0x40..+0x7c. FUN_140a85bd0 computes each output
/// component as x*M[0][j] + y*M[1][j] + z*M[2][j] + M[3][j]. This is the
/// projective clip-space input used before portal rectangle clipping; it does
/// not perform the original clipping or viewport conversion itself.
pub fn transform_portal_corner_by_query_matrix(
    point: [f32; 3],
    matrix: [[f32; 4]; 4],
) -> Result<[f32; 4], String> {
    if point
        .iter()
        .chain(matrix.iter().flatten())
        .any(|value| !value.is_finite())
    {
        return Err("nonfinite Tome portal projection input".into());
    }
    let projected = std::array::from_fn(|column| {
        point[0] * matrix[0][column]
            + point[1] * matrix[1][column]
            + point[2] * matrix[2][column]
            + matrix[3][column]
    });
    if projected.iter().any(|value| !value.is_finite()) {
        return Err("nonfinite projected Tome portal corner".into());
    }
    Ok(projected)
}

/// Build the exact four SIMD vectors that FUN_140a85bd0 places at
/// RBP+0x120..0x150 before calling FUN_140a8a080: four corner X values,
/// four corner Y values, four corner W values, and the projected normal
/// extent. `face_code` is the 0..5
/// face selector assembled in EBX; `winding_flip` is query-context +0x4b4.
/// The caller supplies the already padded low/high world bounds and the
/// query matrix at +0x40. This ports the caller-side corner construction;
/// it does not run the Tome traversal or rasterizer.
pub fn portal_raster_inputs_from_bounds(
    bounds: [[f32; 3]; 2],
    face_code: u8,
    matrix: [[f32; 4]; 4],
    winding_flip: bool,
) -> Result<([[f32; 4]; 4], [[f32; 4]; 3]), String> {
    if face_code >= 6 {
        return Err("invalid Tome portal raster face".into());
    }
    let (projected, face, _) =
        portal_raster_and_clip_inputs_from_bounds(bounds, face_code, matrix, winding_flip)?;
    Ok((projected, face))
}

pub(super) fn portal_raster_and_clip_inputs_from_bounds(
    bounds: [[f32; 3]; 2],
    face_code: u8,
    matrix: [[f32; 4]; 4],
    winding_flip: bool,
) -> Result<([[f32; 4]; 4], [[f32; 4]; 3], [[f32; 4]; 8]), String> {
    portal_raster_and_clip_inputs_prepared(
        bounds,
        face_code,
        &PortalQueryProjection::new(matrix, winding_flip),
    )
}

#[inline(always)]
pub(super) fn portal_raster_and_clip_inputs_prepared(
    bounds: [[f32; 3]; 2],
    face_code: u8,
    projection: &PortalQueryProjection,
) -> Result<([[f32; 4]; 4], [[f32; 4]; 3], [[f32; 4]; 8]), String> {
    let matrix = projection.matrix;
    let winding_flip = projection.winding_flip;
    if face_code >= 6
        || !projection.matrix_finite
        || bounds.iter().flatten().any(|v| !v.is_finite())
        || (0..3).any(|axis| bounds[0][axis] > bounds[1][axis])
    {
        return Err("invalid Tome portal raster inputs".into());
    }

    // The original SSE path starts from the low corner, forms each extent
    // vector, then selects the two cyclic transverse axes for this face.
    let base: [f32; 4] = std::array::from_fn(|column| {
        ((bounds[0][2] * matrix[2][column] + matrix[3][column]) + bounds[0][1] * matrix[1][column])
            + bounds[0][0] * matrix[0][column]
    });
    let extents: [[f32; 4]; 3] = std::array::from_fn(|axis| {
        let extent = bounds[1][axis] - bounds[0][axis];
        std::array::from_fn(|column| extent * matrix[axis][column])
    });

    let axis = (face_code / 2) as usize;
    let mut u = extents[(axis + 1) % 3];
    let mut v = extents[(axis + 2) % 3];
    if (face_code & 1 != 0) != winding_flip {
        std::mem::swap(&mut u, &mut v);
    }

    // FUN_140a85bd0 places the low-face cycle into three four-lane X/Y/W
    // vectors and the perpendicular extent into the fourth vector. The
    // rasterizer selects the parallel face per edge using that extent.
    let normal = extents[axis];
    let low_u: [f32; 4] = std::array::from_fn(|lane| base[lane] + u[lane]);
    let low_uv: [f32; 4] = std::array::from_fn(|lane| low_u[lane] + v[lane]);
    let low_v: [f32; 4] = std::array::from_fn(|lane| base[lane] + v[lane]);
    let corners = [base, low_u, low_uv, low_v];
    let projected = [
        std::array::from_fn(|corner| corners[corner][0]),
        std::array::from_fn(|corner| corners[corner][1]),
        std::array::from_fn(|corner| corners[corner][3]),
        normal,
    ];
    let face = projection.faces.as_ref().map_err(Clone::clone)?[face_code as usize];
    if projected.iter().flatten().any(|v| !v.is_finite()) {
        return Err("nonfinite projected Tome portal corner".into());
    }
    let clip_corners = std::array::from_fn(|corner| {
        if corner < 4 {
            corners[corner]
        } else {
            std::array::from_fn(|axis| normal[axis] + corners[corner - 4][axis])
        }
    });
    Ok((projected, face, clip_corners))
}

/// Build FUN_140a81500's six query-face vector rows from the matrix stored at
/// query context +0x40. The original stores three float4 vectors per face in
/// the +0x13e table. `winding_flip` is the query-context byte at +0x4b4.
/// These rows are consumed by FUN_140a8a080 when a clipped portal is rasterized.
pub fn portal_face_vectors_from_query_matrix(
    matrix: [[f32; 4]; 4],
    winding_flip: bool,
) -> Result<[[[f32; 4]; 3]; 6], String> {
    if matrix.iter().flatten().any(|value| !value.is_finite()) {
        return Err("nonfinite Tome portal query matrix".into());
    }

    // FUN_140a81500 reads columns x, y, and w from the first three stored
    // matrix rows, then writes the three cyclic cross products to its stack
    // cofactor table. Preserve the executable's scalar multiply/subtract order.
    let rows =
        std::array::from_fn::<_, 3, _>(|row| [matrix[row][0], matrix[row][1], matrix[row][3]]);
    let [a, b, c] = rows;
    let cofactors = [
        [
            b[1] * c[2] - b[2] * c[1],
            c[0] * b[2] - c[2] * b[0],
            b[0] * c[1] - b[1] * c[0],
        ],
        [
            c[1] * a[2] - c[2] * a[1],
            c[2] * a[0] - c[0] * a[2],
            c[0] * a[1] - c[1] * a[0],
        ],
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ],
    ];
    if cofactors.iter().flatten().any(|value| !value.is_finite()) {
        return Err("nonfinite Tome portal face cofactor".into());
    }

    let sign_flip = |value: f32| f32::from_bits(value.to_bits() ^ 0x8000_0000);
    let mut faces = [[[0.0; 4]; 3]; 6];
    // FUN_140a81500 rotates the one-hot axis sequence 1, 2, 0 and pairs it
    // with (1 << axis) & 3, yielding (1,2), (2,0), and (0,1).
    for (pair, (current, next)) in [(1usize, 2usize), (2, 0), (0, 1)].into_iter().enumerate() {
        let mut primary = [[0.0; 4]; 3];
        let mut opposite = [[0.0; 4]; 3];
        for component in 0..3 {
            let current_value = cofactors[current][component];
            let next_value = cofactors[next][component];
            primary[component] = [
                current_value,
                sign_flip(next_value),
                sign_flip(current_value),
                next_value,
            ];
            opposite[component] = [
                sign_flip(next_value),
                current_value,
                next_value,
                sign_flip(current_value),
            ];
        }

        // The byte at query +0x4b4 selects which member of each face pair
        // receives the primary versus opposite vector ordering.
        let even = pair * 2;
        let (even_vectors, odd_vectors) = if winding_flip {
            (primary, opposite)
        } else {
            (opposite, primary)
        };
        faces[even] = even_vectors;
        faces[even + 1] = odd_vectors;
    }
    Ok(faces)
}

/// FUN_140a85bd0 converts its clipped portal pixel rectangle to the tile
/// rectangle supplied to FUN_140a89d80 (eight pixels wide, four high).
///
/// Before that tile conversion, FUN_140a85bd0 maps the clipped NDC rectangle
/// to viewport pixels with the vectors at 0x141e68090 and 0x141e680a0. The
/// first vector is [32, 32, 32, 32]; the second is [32, 32, 32.999996,
/// 32.999996]. CVTTPS2DQ truncates the four results, then the function clamps
/// the minimum edges upward and maximum edges downward to its query viewport.
#[inline(always)]
pub fn projected_portal_pixel_rect(
    projected: [f32; 4],
    viewport: [i32; 4],
) -> Result<Option<[i32; 4]>, String> {
    if projected.iter().any(|value| !value.is_finite())
        || viewport[0] > viewport[2]
        || viewport[1] > viewport[3]
    {
        return Err("invalid projected Tome portal rectangle or viewport".into());
    }
    let [min_x, min_y, max_x, max_y] = projected;
    let pixels = [
        min_x * 32.0 + 32.0,
        min_y * 32.0 + 32.0,
        max_x * 32.0 + f32::from_bits(0x4203_ffff),
        max_y * 32.0 + f32::from_bits(0x4203_ffff),
    ];
    if pixels
        .iter()
        .any(|value| !value.is_finite() || *value < i32::MIN as f32 || *value >= i32::MAX as f32)
    {
        return Err("projected Tome portal pixel coordinate exceeds integer range".into());
    }
    #[cfg(target_arch = "x86_64")]
    let mut pixels = super::simd_predicates::truncate_i32x4(pixels);
    #[cfg(not(target_arch = "x86_64"))]
    let mut pixels = pixels.map(|value| value as i32);
    pixels[0] = pixels[0].max(viewport[0]);
    pixels[1] = pixels[1].max(viewport[1]);
    pixels[2] = pixels[2].min(viewport[2]);
    pixels[3] = pixels[3].min(viewport[3]);
    Ok((pixels[2] > pixels[0] && pixels[3] > pixels[1]).then_some(pixels))
}

/// FUN_140a85bd0's fallback extrema branch (LAB_140a866e5) projects both
/// parallel faces of the padded portal box, then reduces their eight
/// homogeneous corners to [min x, min y, max x, max y]. The active-plane
/// crossing branch must not use this helper; it adds edge-derived extrema.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub fn projected_portal_box_fallback_bounds(corners: [[f32; 4]; 8]) -> Result<[f32; 4], String> {
    use std::arch::x86_64::*;

    if corners.iter().flatten().any(|value| !value.is_finite()) {
        return Err("nonfinite homogeneous Tome portal box corner".into());
    }

    let mut ndc = [[0.0f32; 2]; 8];
    for group in 0..2 {
        let base = group * 4;
        let mut x_lanes = [0.0f32; 4];
        let mut y_lanes = [0.0f32; 4];
        unsafe {
            let w = _mm_setr_ps(
                corners[base][3],
                corners[base + 1][3],
                corners[base + 2][3],
                corners[base + 3][3],
            );
            let zero_mask = _mm_cmpeq_ps(w, _mm_setzero_ps());
            // FUN_140a85bd0 substitutes 1.0 for exactly-zero W lanes before RCPPS.
            let denominator = _mm_or_ps(
                _mm_and_ps(zero_mask, _mm_set1_ps(1.0)),
                _mm_andnot_ps(zero_mask, w),
            );
            let reciprocal = _mm_rcp_ps(denominator);
            let x = _mm_mul_ps(
                _mm_setr_ps(
                    corners[base][0],
                    corners[base + 1][0],
                    corners[base + 2][0],
                    corners[base + 3][0],
                ),
                reciprocal,
            );
            let y = _mm_mul_ps(
                _mm_setr_ps(
                    corners[base][1],
                    corners[base + 1][1],
                    corners[base + 2][1],
                    corners[base + 3][1],
                ),
                reciprocal,
            );
            _mm_storeu_ps(x_lanes.as_mut_ptr(), x);
            _mm_storeu_ps(y_lanes.as_mut_ptr(), y);
        }
        for lane in 0..4 {
            ndc[base + lane] = [x_lanes[lane], y_lanes[lane]];
        }
    }

    let mut bounds = [ndc[0][0], ndc[0][1], ndc[0][0], ndc[0][1]];
    for point in &ndc[1..] {
        bounds[0] = bounds[0].min(point[0]);
        bounds[1] = bounds[1].min(point[1]);
        bounds[2] = bounds[2].max(point[0]);
        bounds[3] = bounds[3].max(point[1]);
    }
    if bounds.iter().any(|value| !value.is_finite()) {
        return Err("nonfinite projected Tome portal bounds".into());
    }
    Ok(bounds)
}

/// FUN_140a85bd0 uses conservative per-corner homogeneous masks when W
/// crosses zero. Preserve RCPPS and the native +/-1 mask substitutions;
/// geometric polygon clipping would discard coverage retained by the game.
pub fn projected_portal_box_crossing_bounds(
    corners: [[f32; 4]; 8],
) -> Result<Option<[f32; 4]>, String> {
    if corners.iter().flatten().any(|value| !value.is_finite()) {
        return Err("nonfinite homogeneous Tome portal box corner".into());
    }

    Ok(Some(native_projected_corner_bounds(corners, true)))
}

/// Select the two FUN_140a85bd0 projected-bound paths from the record's
/// retained active-plane indices and the projected box W lanes.
#[inline(always)]
pub fn projected_portal_box_bounds(
    corners: [[f32; 4]; 8],
    active_planes: &[usize],
) -> Result<Option<([f32; 4], bool)>, String> {
    //a85bd0 pairs corresponding low/high-face corners. At least one
    //pair must reach Z<=W with nonnegative maximum W before projection.
    if portal_box_depth_mask(corners) == 0 {
        return Ok(None);
    }
    let crossing_path =
        active_planes.first() == Some(&0) && corners.iter().any(|corner| corner[3] < 0.0);
    if crossing_path {
        Ok(projected_portal_box_crossing_bounds(corners)?.map(|bounds| (bounds, true)))
    } else {
        projected_portal_box_fallback_bounds(corners).map(|bounds| Some((bounds, false)))
    }
}

pub fn portal_box_depth_mask(corners: [[f32; 4]; 8]) -> u32 {
    (0..4).fold(0, |mask, i| {
        let low = corners[i];
        let high = corners[i + 4];
        let maximum_w = if low[3] > high[3] { low[3] } else { high[3] };
        mask | if (low[2] <= low[3] || high[2] <= high[3]) && 0.0 <= maximum_w {
            1 << i
        } else {
            0
        }
    })
}

/// Compose FUN_140a85bd0's selected portal-bound branch with its NDC-to-pixel
/// conversion and viewport clamp. The boolean reports whether the active
/// plane-zero crossing branch supplied the bounds.
#[inline(always)]
pub fn projected_portal_box_pixel_rect(
    corners: [[f32; 4]; 8],
    active_planes: &[usize],
    viewport: [i32; 4],
) -> Result<(Option<[i32; 4]>, bool), String> {
    if viewport[0] > viewport[2] || viewport[1] > viewport[3] {
        return Err("invalid projected Tome portal viewport".into());
    }
    let Some((bounds, used_crossing_path)) = projected_portal_box_bounds(corners, active_planes)?
    else {
        return Ok((None, active_planes.first() == Some(&0)));
    };
    Ok((
        projected_portal_pixel_rect(bounds, viewport)?,
        used_crossing_path,
    ))
}

#[cfg(not(target_arch = "x86_64"))]
#[inline(always)]
pub fn projected_portal_box_fallback_bounds(_: [[f32; 4]; 8]) -> Result<[f32; 4], String> {
    Err("original Tome portal projection requires x86_64 SSE".into())
}

pub fn portal_tile_rect(pixels: [i32; 4]) -> Result<[u8; 4], String> {
    let [left, top, right, bottom] = pixels;
    if left < 0 || top < 0 || right <= left || bottom <= top {
        return Err("invalid clipped Tome portal rectangle".into());
    }
    let edges = [
        left >> 3,
        top >> 2,
        right.checked_add(7).ok_or("Tome tile edge overflow")? >> 3,
        bottom.checked_add(3).ok_or("Tome tile edge overflow")? >> 2,
    ];
    if edges.iter().any(|&edge| edge > u8::MAX as i32) {
        return Err("Tome portal tile exceeds byte coordinates".into());
    }
    Ok(edges.map(|edge| edge as u8))
}

impl PortalTileMask {
    pub fn new(rect: [u8; 4]) -> Result<Self, String> {
        let width = rect[2]
            .checked_sub(rect[0])
            .ok_or("invalid Tome tile rectangle")?;
        let height = rect[3]
            .checked_sub(rect[1])
            .ok_or("invalid Tome tile rectangle")?;
        if width == 0 || height == 0 {
            return Err("empty Tome tile rectangle".into());
        }
        Ok(Self {
            rect,
            bits: vec![0; width as usize * height as usize],
        })
    }

    /// Build the initial query coverage mask from FUN_140a81500 and
    /// FUN_140a89f90. The game initializes every per-tile coverage word in
    /// the viewport rectangle to all ones before seeding the camera's cell.
    pub fn full(rect: [u8; 4]) -> Result<Self, String> {
        let mut mask = Self::new(rect)?;
        mask.bits.fill(u32::MAX);
        Ok(mask)
    }

    /// The original function expands to the coordinate-wise union and
    /// zero-fills newly added rows and columns around the old row-major mask.
    pub fn grow_to_include(&mut self, rect: [u8; 4]) -> Result<(), String> {
        if rect[2] <= rect[0] || rect[3] <= rect[1] {
            return Err("invalid Tome tile rectangle".into());
        }
        let old = self.rect;
        let expanded = [
            old[0].min(rect[0]),
            old[1].min(rect[1]),
            old[2].max(rect[2]),
            old[3].max(rect[3]),
        ];
        if expanded == old {
            return Ok(());
        }
        let mut grown = Self::new(expanded)?;
        let old_width = (old[2] - old[0]) as usize;
        let grown_width = (expanded[2] - expanded[0]) as usize;
        let x_offset = (old[0] - expanded[0]) as usize;
        let y_offset = (old[1] - expanded[1]) as usize;
        for row in 0..(old[3] - old[1]) as usize {
            let start = (row + y_offset) * grown_width + x_offset;
            grown.bits[start..start + old_width]
                .copy_from_slice(&self.bits[row * old_width..(row + 1) * old_width]);
        }
        *self = grown;
        Ok(())
    }

    pub fn merge_new_coverage(&mut self, incoming: &Self) -> Result<bool, String> {
        let [left, top, right, bottom] = incoming.rect;
        if left < self.rect[0]
            || top < self.rect[1]
            || right > self.rect[2]
            || bottom > self.rect[3]
        {
            return Err("incoming Tome tile mask exceeds destination".into());
        }
        let width = (self.rect[2] - self.rect[0]) as usize;
        let incoming_width = (right - left) as usize;
        let mut changed = 0u32;
        for row in 0..(bottom - top) as usize {
            let destination = (top - self.rect[1]) as usize * width
                + row * width
                + (left - self.rect[0]) as usize;
            let source = row * incoming_width;
            for column in 0..incoming_width {
                let old = self.bits[destination + column];
                let merged = old | incoming.bits[source + column];
                changed |= merged ^ old;
                self.bits[destination + column] = merged;
            }
        }
        Ok(changed != 0)
    }

    /// FUN_140a8b2d0 tests a projected object's pixel rectangle against the
    /// portal coverage mask. These four tables are the original executable's
    /// constants at 0x141e68530, 0x141e68550, 0x141e68570, 0x141e68580.
    pub fn intersects_pixels(&self, pixels: [i32; 4]) -> Result<bool, String> {
        const LEFT: [u32; 8] = [
            0xffff_ffff,
            0xffff_eeee,
            0xffff_cccc,
            0xffff_8888,
            0xffff_0000,
            0xeeee_0000,
            0xcccc_0000,
            0x8888_0000,
        ];
        const RIGHT: [u32; 8] = [
            0xffff_ffff,
            0x0000_1111,
            0x0000_3333,
            0x0000_7777,
            0x0000_ffff,
            0x1111_ffff,
            0x3333_ffff,
            0x7777_ffff,
        ];
        const TOP: [u32; 4] = [0xffff_ffff, 0xfff0_fff0, 0xff00_ff00, 0xf000_f000];
        const BOTTOM: [u32; 4] = [0xffff_ffff, 0x000f_000f, 0x00ff_00ff, 0x0fff_0fff];
        let tile = portal_tile_rect(pixels)?;
        if tile[0] < self.rect[0]
            || tile[1] < self.rect[1]
            || tile[2] > self.rect[2]
            || tile[3] > self.rect[3]
        {
            return Err("Tome object rectangle exceeds portal tile mask".into());
        }
        let width = (self.rect[2] - self.rect[0]) as usize;
        for y in tile[1]..tile[3] {
            let y_mask = (if y == tile[1] {
                TOP[pixels[1] as usize & 3]
            } else {
                u32::MAX
            }) & (if y + 1 == tile[3] {
                BOTTOM[pixels[3] as usize & 3]
            } else {
                u32::MAX
            });
            for x in tile[0]..tile[2] {
                let x_mask = (if x == tile[0] {
                    LEFT[pixels[0] as usize & 7]
                } else {
                    u32::MAX
                }) & (if x + 1 == tile[2] {
                    RIGHT[pixels[2] as usize & 7]
                } else {
                    u32::MAX
                });
                let index = (y - self.rect[1]) as usize * width + (x - self.rect[0]) as usize;
                if self.bits[index] & x_mask & y_mask != 0 {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// FUN_140a8b430 accepts an object only where its projected rectangle
    /// overlaps a set portal bit and that same pixel's query depth reaches
    /// the supplied threshold. The depth grid uses 4x4 tiles; this mask uses
    /// one u32 for each 8x4 tile.
    pub fn intersects_depth_pixels(
        &self,
        depths: &[f32],
        pixels: [i32; 4],
        threshold: f32,
    ) -> Result<bool, String> {
        if depths.len() != 64 * 64 {
            return Err("invalid Tome depth grid length".into());
        }
        let tile = portal_tile_rect(pixels)?;
        if tile[0] < self.rect[0]
            || tile[1] < self.rect[1]
            || tile[2] > self.rect[2]
            || tile[3] > self.rect[3]
            || pixels[2] > 64
            || pixels[3] > 64
        {
            return Err("Tome object rectangle exceeds portal mask or depth grid".into());
        }
        let width = (self.rect[2] - self.rect[0]) as usize;
        for y in pixels[1] as usize..pixels[3] as usize {
            for x in pixels[0] as usize..pixels[2] as usize {
                let mask_index =
                    (y / 4 - self.rect[1] as usize) * width + (x / 8 - self.rect[0] as usize);
                let mask_bit = ((x / 4) & 1) * 16 + (y & 3) * 4 + (x & 3);
                if self.bits[mask_index] & (1 << mask_bit) == 0 {
                    continue;
                }
                let depth_index = ((y / 4) * 16 + x / 4) * 16 + (y & 3) * 4 + (x & 3);
                if threshold <= depths[depth_index] {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// FUN_140a8a080's projected portal raster. `projected` is its four
    /// float4 vectors and `face` its three float4 vectors.
    pub fn rasterize_projected_box(
        &mut self,
        incoming: &Self,
        pixels: [i32; 4],
        projected: [[f32; 4]; 4],
        face: [[f32; 4]; 3],
    ) -> Result<bool, String> {
        let [left, top, right, bottom] = pixels;
        let tile = portal_tile_rect(pixels)?;
        if tile[0] < self.rect[0]
            || tile[1] < self.rect[1]
            || tile[2] > self.rect[2]
            || tile[3] > self.rect[3]
            || tile[0] < incoming.rect[0]
            || tile[1] < incoming.rect[1]
            || tile[2] > incoming.rect[2]
            || tile[3] > incoming.rect[3]
        {
            return Err("projected Tome portal exceeds tile masks".into());
        }
        let x0 = left >> 2;
        let y0 = top >> 2;
        let x1 = right.checked_add(3).ok_or("Tome raster edge overflow")? >> 2;
        let y1 = bottom.checked_add(3).ok_or("Tome raster edge overflow")? >> 2;
        let width = x1 - x0;
        let height = y1 - y0;
        let medium = width * height >= 11 && width != 1 && height != 1;
        let large = medium && width * height > 40;
        if projected
            .iter()
            .flatten()
            .chain(face.iter().flatten())
            .any(|value| !value.is_finite())
        {
            return Err("nonfinite Tome portal raster input".into());
        }
        let mut edge_x = [0.0f32; 4];
        let mut edge_y = [0.0f32; 4];
        let mut edge_origin = [0.0f32; 4];
        for lane in 0..4 {
            let positive = 0.0
                < face[2][lane] * projected[2][lane]
                    + face[1][lane] * projected[1][lane]
                    + face[0][lane] * projected[0][lane];
            let next = (lane + 1) & 3;
            let offset_x = if positive { projected[3][0] } else { 0.0 };
            let offset_y = if positive { projected[3][1] } else { 0.0 };
            let offset_z = if positive { projected[3][3] } else { 0.0 };
            let px = projected[0][lane] + offset_x;
            let py = projected[1][lane] + offset_y;
            let pz = projected[2][lane] + offset_z;
            let qx = projected[0][next] + offset_x;
            let qy = projected[1][next] + offset_y;
            let qz = projected[2][next] + offset_z;
            edge_x[lane] = (qz * py - qy * pz) * (1.0 / 32.0);
            edge_y[lane] = (qx * pz - qz * px) * (1.0 / 32.0);
            let constant = qy * px - qx * py;
            let bias =
                (edge_x[lane].abs() + edge_y[lane].abs()).max(f32::from_bits(0x3400_0000)) * -0.5;
            edge_origin[lane] = constant + edge_x[lane] * -31.5 + edge_y[lane] * -31.5 + bias;
        }
        let macro_x0 = x0 >> 2;
        let macro_y0 = y0 >> 2;
        let macro_width = ((x1 + 3) >> 2) - macro_x0;
        let mut macro_classes = Vec::new();
        if large {
            for macro_y in macro_y0..((y1 + 3) >> 2) {
                for macro_x in macro_x0..((x1 + 3) >> 2) {
                    let mut possible = true;
                    let mut full = true;
                    for lane in 0..4 {
                        let base = edge_origin[lane]
                            + edge_x[lane] * (macro_x * 16) as f32
                            + edge_y[lane] * (macro_y * 16) as f32;
                        let dx = edge_x[lane] * 15.0;
                        let dy = edge_y[lane] * 15.0;
                        let min = base + dx.min(0.0) + dy.min(0.0);
                        let max = base + dx.max(0.0) + dy.max(0.0);
                        possible &= min.to_bits() & 0x8000_0000 != 0;
                        full &= max.to_bits() & 0x8000_0000 != 0;
                    }
                    macro_classes.push(if !possible {
                        0
                    } else if full {
                        2
                    } else {
                        1
                    });
                }
            }
        }
        let destination_width = (self.rect[2] - self.rect[0]) as usize;
        let source_width = (incoming.rect[2] - incoming.rect[0]) as usize;
        let mut changed = 0u32;
        for half_y in y0..y1 {
            // The original large-block copy operates on whole u32 mask words
            // (two 4x4 cells), even when the rectangle ends in one half.
            let iteration_x0 = if large { x0 & !1 } else { x0 };
            let iteration_x1 = if large { (x1 + 1) & !1 } else { x1 };
            for half_x in iteration_x0..iteration_x1 {
                let mut coverage = None;
                if large {
                    let class = macro_classes[(((half_y >> 2) - macro_y0) * macro_width
                        + ((half_x >> 2) - macro_x0))
                        as usize];
                    if class == 0 || (half_x < x0 || half_x >= x1) && class != 2 {
                        coverage = Some(0);
                    } else if class == 2 {
                        coverage = Some(u16::MAX);
                    }
                }
                if medium && coverage.is_none() {
                    let mut possible = true;
                    let mut full = true;
                    for lane in 0..4 {
                        let base = edge_origin[lane]
                            + edge_x[lane] * (half_x * 4) as f32
                            + edge_y[lane] * (half_y * 4) as f32;
                        let dx = edge_x[lane] * 3.0;
                        let dy = edge_y[lane] * 3.0;
                        let min = base + dx.min(0.0) + dy.min(0.0);
                        let max = base + dx.max(0.0) + dy.max(0.0);
                        possible &= min.to_bits() & 0x8000_0000 != 0;
                        full &= max.to_bits() & 0x8000_0000 != 0;
                    }
                    if !possible {
                        coverage = Some(0);
                    } else if full {
                        coverage = Some(u16::MAX);
                    }
                }
                let coverage = coverage.unwrap_or_else(|| {
                    super::portal_raster::coverage_4x4(
                        edge_origin,
                        edge_x,
                        edge_y,
                        half_x * 4,
                        half_y * 4,
                    )
                });
                let tx = (half_x >> 1) as u8;
                let ty = half_y as u8;
                let shift = ((half_x & 1) * 16) as u32;
                let destination =
                    (ty - self.rect[1]) as usize * destination_width + (tx - self.rect[0]) as usize;
                let source = (ty - incoming.rect[1]) as usize * source_width
                    + (tx - incoming.rect[0]) as usize;
                let old = self.bits[destination];
                let merged = old | ((coverage as u32) << shift & incoming.bits[source]);
                changed |= merged ^ old;
                self.bits[destination] = merged;
            }
        }
        Ok(changed != 0)
    }
}

/// FUN_140a784c0 tests a projected pixel rectangle against the original
/// 64x64 query-depth array at context +0x5a0. Each 4x4 tile stores 16 floats
/// in row-major pixel order; tiles form a 16x16 grid. The caller supplies
/// the projected depth threshold.
pub fn depth_grid_intersects_pixels(
    depths: &[f32],
    lower: [i32; 2],
    upper: [i32; 2],
    threshold: f32,
) -> Result<bool, String> {
    if depths.len() != 64 * 64 {
        return Err("invalid Tome depth grid length".into());
    }
    let x0 = lower[0].max(0);
    let y0 = lower[1].max(0);
    let x1 = upper[0].min(64);
    let y1 = upper[1].min(64);
    for y in y0..y1 {
        for x in x0..x1 {
            let tile = (y as usize >> 2) * 16 + (x as usize >> 2);
            let pixel = ((y as usize & 3) << 2) + (x as usize & 3);
            if threshold <= depths[tile * 16 + pixel] {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// FUN_140a8b660 expands an original 8x4-pixel mask rectangle into the
/// query's tiled 64x64 float grid. The installed executable confirms one
/// little-endian u32 mask per tile, with bit N mapping to depth slot N.
/// The context also tracks the greatest depth value passed to this writer.
pub fn rasterize_depth_mask(
    depths: &mut [f32],
    rect: [u8; 4],
    bits: &[u32],
    value: f32,
    previous_max: f32,
) -> Result<f32, String> {
    if depths.len() != 64 * 64 {
        return Err("invalid Tome depth grid length".into());
    }
    let [left, top, right, bottom] = rect.map(usize::from);
    if left >= right || top >= bottom || right > 8 || bottom > 16 {
        return Err("invalid Tome depth mask rectangle".into());
    }
    let width = right - left;
    if bits.len() != width * (bottom - top) {
        return Err("invalid Tome depth mask word count".into());
    }
    if !value.is_finite() || !previous_max.is_finite() {
        return Err("nonfinite Tome depth mask value".into());
    }
    for (tile, &word) in bits.iter().enumerate() {
        let tile_x = left + tile % width;
        let tile_y = top + tile / width;
        let start = tile_y * 256 + tile_x * 32;
        let mut remaining = word;
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            depths[start + bit] = depths[start + bit].max(value);
            remaining &= remaining - 1;
        }
    }
    Ok(previous_max.max(value))
}
