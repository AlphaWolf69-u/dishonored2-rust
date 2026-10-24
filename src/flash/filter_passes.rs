//! GDraw D3D11 filter passes (notes: filter research in the session log;
//! functions in chunk_000114/115): the separable blur `FUN_1402e15d0` and its
//! draws `FUN_1402e1d80` / `FUN_1402e1f10`, the colour matrix layout
//! (`FUN_14100d950`, the shader at exe 0x22a95d0).
//!
//! Rectangles are pixel rectangles `[x0, y0, x1, y1]` of a filter render
//! target of `size` pixels; a pass draws its destination rectangle, reading
//! the source at `uv` (the source coordinates of the destination corners)
//! with each tap's offset, clamped to `clamp` (half a texel inside the
//! source rectangle), and sums tap weight × sample (the blur pixel shaders
//! at exe 0x22a8eb0 and siblings).

/// One draw of the tap shader.
#[derive(Clone, Debug, PartialEq)]
pub struct TapPass {
    pub destination: [i32; 4],
    pub uv: [f32; 4],
    pub clamp: [f32; 4],
    /// (offset x, offset y, weight) in source coordinates.
    pub taps: Vec<[f32; 3]>,
}

/// `FUN_1411b5ad8`.
fn ceil(v: f32) -> f32 {
    v.ceil()
}

/// The rectangle grown by `x`/`y`, within the target.
fn grow(r: [i32; 4], x: i32, y: i32, size: (i32, i32)) -> [i32; 4] {
    [(r[0] - x).max(0), (r[1] - y).max(0), (r[2] + x).min(size.0), (r[3] + y).min(size.1)]
}

/// `FUN_1402e1d80`: a same-size pass over `rect`, clamped to `clamp_rect`.
fn same_size(rect: [i32; 4], clamp_rect: [i32; 4], size: (i32, i32), taps: Vec<[f32; 3]>) -> TapPass {
    let (tx, ty) = (1.0 / size.0 as f32, 1.0 / size.1 as f32);
    TapPass {
        destination: rect,
        uv: [rect[0] as f32 * tx, rect[1] as f32 * ty, rect[2] as f32 * tx, rect[3] as f32 * ty],
        clamp: [(clamp_rect[0] as f32 + 0.5) * tx, (clamp_rect[1] as f32 + 0.5) * ty, (clamp_rect[2] as f32 - 0.5) * tx, (clamp_rect[3] as f32 - 0.5) * ty],
        taps,
    }
}

/// `FUN_1402e1f10`: along `axis`, the destination is the rectangle divided by
/// `factor` (the downsampling pass, `factor` > 1) and the source coordinates
/// are normalized by `dims` (the physical size times the factor on the
/// upsampling pass).
#[allow(clippy::too_many_arguments)]
fn scaled(rect: [i32; 4], axis: usize, factor: i32, dims: (i32, i32), clamp_rect: [i32; 4], taps: Vec<[f32; 3]>) -> TapPass {
    let (tx, ty) = (1.0 / dims.0 as f32, 1.0 / dims.1 as f32);
    let (destination, src) = if axis == 0 {
        let x0 = rect[0] / factor;
        let x1 = (rect[2] - 1) / factor + 1;
        ([x0, rect[1], x1, rect[3]], [(x0 as f32 - 0.5) * factor as f32 + 0.5, rect[1] as f32, (x1 as f32 - 0.5) * factor as f32 + 0.5, rect[3] as f32])
    } else {
        let y0 = rect[1] / factor;
        let y1 = (rect[3] - 1) / factor + 1;
        ([rect[0], y0, rect[2], y1], [rect[0] as f32, (y0 as f32 - 0.5) * factor as f32 + 0.5, rect[2] as f32, (y1 as f32 - 0.5) * factor as f32 + 0.5])
    };
    TapPass {
        destination,
        uv: [src[0] * tx, src[1] * ty, src[2] * tx, src[3] * ty],
        clamp: [(clamp_rect[0] as f32 + 0.5) * tx, (clamp_rect[1] as f32 + 0.5) * ty, (clamp_rect[2] as f32 - 0.5) * tx, (clamp_rect[3] as f32 - 0.5) * ty],
        taps,
    }
}

fn tap(axis: usize, offset: f32, weight: f32) -> [f32; 3] {
    if axis == 0 {
        [offset, 0.0, weight]
    } else {
        [0.0, offset, weight]
    }
}

/// `FUN_1402e15d0`: one axis of a blur of `size` pixels over the content
/// `rect` (grown here) with sampling clamp `clamp_rect` (updated to the grown
/// rectangle plus one pixel). Returns the passes to draw in order, or none
/// when the blur is under two taps.
pub fn blur_axis(axis: usize, size: f32, target: (i32, i32), rect: &mut [i32; 4], clamp_rect: &mut [i32; 4]) -> Vec<TapPass> {
    let texel = 1.0 / if axis == 0 { target.0 } else { target.1 } as f32;
    let c = ceil((size - 1.0) * 0.5);
    let inverse = 1.0 / size;
    let half = c as i32;
    let n = half + 1;
    let edge = 1.0 - ((c * 2.0 + 1.0) - size) * 0.5;
    if n < 2 {
        return Vec::new();
    }
    let along = |v: i32| if axis == 0 { (v, 0) } else { (0, v) };
    let mut passes = Vec::new();
    if n < 10 {
        let mut taps: Vec<[f32; 3]> = (0..n).map(|i| tap(axis, (2 * i) as f32 + (1.5 - n as f32), inverse * 2.0)).collect();
        taps[(n - 1) as usize] = tap(axis, half as f32, inverse * edge);
        taps[0] = tap(axis, (1.0 - n as f32) + 1.0 / (edge + 1.0), (edge + 1.0) * inverse);
        // The offsets above are in pixels.
        for t in &mut taps {
            t[axis] *= texel;
        }
        let (gx, gy) = along(half);
        *rect = grow(*rect, gx, gy, target);
        passes.push(same_size(*rect, *clamp_rect, target, taps));
    } else {
        // Wide blurs: downsample along the axis, then blur the reduced image.
        let k = ceil(((half - 9) as f32 / 71.0) * 7.0 + 2.0) as i32;
        let factor = k.max(2);
        let (m, spacing) = if k > 9 { (9, (k / 9) as f32 * texel) } else { (k, texel) };
        let taps: Vec<[f32; 3]> = (0..m).map(|i| tap(axis, ((1 - m + 2 * i) as f32 + 0.5) * spacing, (1.0 / (m * 2) as f32) * 2.0)).collect();
        let q = (n * 2) / m;
        let e = (((q + 1) >> 1) + 1) * m + 1;
        let (gx, gy) = along(e);
        *rect = grow(*rect, gx, gy, target);
        passes.push(scaled(*rect, axis, factor, target, *clamp_rect, taps));
        let mut texel = texel;
        let mut q = q;
        if q > 18 {
            texel = ((q - 1) as f32 * texel) / 17.0;
            q = 18;
        }
        let mut taps = Vec::new();
        if q % 2 == 0 {
            let h = q >> 1;
            for i in 0..h {
                taps.push(tap(axis, ((2 * i) as f32 + (1 - h) as f32) * texel, (1.0 / q as f32) * 2.0));
            }
        } else {
            let h = (q + 1) >> 1;
            let w = 1.0 / q as f32;
            for i in 0..h {
                taps.push(tap(axis, ((2 * i) as f32 + (1 - h) as f32 + 0.5) * texel, w * 2.0));
            }
            if h > 0 {
                taps[(h - 1) as usize] = tap(axis, ((h * 2 - 2) as f32 + (1 - h) as f32) * texel, w);
            }
        }
        let dims = if axis == 0 { (target.0 * factor, target.1) } else { (target.0, target.1 * factor) };
        let reduced_clamp = grow(*rect, 1, 1, target);
        passes.push(scaled(*rect, axis, 1, dims, reduced_clamp, taps));
    }
    *clamp_rect = grow(*rect, 1, 1, target);
    passes
}

/// `FUN_14109b1f0`: the blur size in target pixels for a filter blur in
/// stage pixels, with the stage-to-target scale (`P+0x1f54` / `P+0x1f58`).
pub fn blur_pixels(blur: f32, scale: f32) -> f32 {
    (blur - 1.0) * scale + 1.0
}

/// `FUN_14100d950`: a script `ColorMatrixFilter.matrix` (row-major 4×5, up
/// to 20 entries, missing ones zero) in the record layout the shader reads:
/// the four coefficient rows, then the four offsets scaled by 1/256
/// (`DAT_141b0fd80`).
pub fn colour_matrix(values: &[f32]) -> [f32; 20] {
    const ORDER: [usize; 20] = [0, 1, 2, 3, 16, 4, 5, 6, 7, 17, 8, 9, 10, 11, 18, 12, 13, 14, 15, 19];
    let mut out = [0.0f32; 20];
    for (i, &slot) in ORDER.iter().enumerate() {
        out[slot] = values.get(i).copied().unwrap_or(0.0);
    }
    for v in &mut out[16..20] {
        *v *= f32::from_bits(0x3b80_0000);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_blur_is_a_box_of_its_size() {
        // B = 4: c = ceil(1.5) = 2, three taps covering 5 pixels with
        // fractional edges: weights per pixel 1/8, 1/4, 1/4, 1/4, 1/8.
        let mut rect = [10, 10, 20, 20];
        let mut clamp = [9, 9, 21, 21];
        let passes = blur_axis(0, 4.0, (100, 100), &mut rect, &mut clamp);
        assert_eq!(passes.len(), 1);
        let taps = &passes[0].taps;
        assert_eq!(taps.len(), 3);
        let total: f32 = taps.iter().map(|t| t[2]).sum();
        assert!((total - 1.0).abs() < 1e-6, "{total}");
        assert_eq!(rect, [8, 10, 22, 20]);
        assert_eq!(clamp, [7, 9, 23, 21]);
    }

    #[test]
    fn a_wide_blur_downsamples_then_blurs() {
        let mut rect = [100, 100, 200, 200];
        let mut clamp = [99, 99, 201, 201];
        let passes = blur_axis(1, 25.0, (400, 400), &mut rect, &mut clamp);
        assert_eq!(passes.len(), 2);
        let sum = |p: &TapPass| p.taps.iter().map(|t| t[2]).sum::<f32>();
        assert!((sum(&passes[0]) - 1.0).abs() < 1e-6);
        assert!((sum(&passes[1]) - 1.0).abs() < 1e-5, "{}", sum(&passes[1]));
        // The first pass writes the reduced rows.
        assert!(passes[0].destination[3] < rect[3]);
    }

    #[test]
    fn script_colour_matrices_take_the_record_layout() {
        let identity: Vec<f32> = vec![1., 0., 0., 0., 255., 0., 1., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 0., 1., 0.];
        let m = colour_matrix(&identity);
        assert_eq!(&m[0..4], &[1., 0., 0., 0.]);
        assert_eq!(m[15], 1.0);
        assert!((m[16] - 255.0 / 256.0).abs() < 1e-6);
    }
}

/// A display object's filter record (the 0x38-byte records at `DO+0xb0`) as
/// far as the render-target path draws it.
#[derive(Clone, Debug, PartialEq)]
pub enum FilterRecord {
    /// Type 1: blur in stage pixels, passes (the quality byte).
    Blur { blur: [f32; 2], passes: u8 },
    /// Type 6: the shader's layout ([`colour_matrix`]).
    ColourMatrix([f32; 20]),
    /// Type 0, as the `DropShadowFilter` setters store it: colour bytes and
    /// alpha (`FUN_1410126f0`, `FUN_141012600`), angle in radians
    /// (`FUN_141012690`), flags inner/knockout/hideObject (bits 0..2 of +2).
    DropShadow { colour: [u8; 4], blur: [f32; 2], angle: f32, distance: f32, strength: f32, passes: u8, inner: bool, knockout: bool, hide_object: bool },
    /// Other types (shadows, glows, bevels, convolution): not drawn by the
    /// render-target path yet.
    Unsupported,
}
