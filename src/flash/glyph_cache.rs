//! Iggy's glyph bitmap cache (swf_fontcache.c): small glyphs are drawn as
//! textured quads from glyphs rasterised into a cache page.
//!
//! - Lookup (`FUN_14107b980`): an entry of the glyph whose pixel scale is
//!   within [0.909, 1.1] of the wanted one is reused (`player+0x1eb0/0x1eb4`
//!   from `FUN_14107b910(player, 1)`).
//! - Entry (`FUN_14107ad10`): the glyph's points rasterised at
//!   `(k + 1)` times its pixel scale (`k = player+0x1eac = 1`) with a border
//!   of `k + 1` zero pixels, then blurred in place by a 2×2 box
//!   (`(a + b + c + d + 2) >> 2`), placed on the page (`FUN_14107a980`
//!   shelves) with UVs and a quad (`FUN_14107b2d0`) in em units.
//! - Rasteriser: stb_truetype's curve flattening (`FUN_141098dc0`,
//!   `FUN_14109a480`) and version-2 scanline rasteriser (`FUN_1410999c0`,
//!   `FUN_141099cc0`, `FUN_1410998b0`, `FUN_141099120`, `FUN_141099770`) as
//!   Iggy embeds them, at tolerance `player+0x1e80 = 0.25`
//!   (`FUN_14102b160`).
//!
//! Page texels are `(c, c, c, c)`: GDraw's glyph shaders (exe 0x229dac0,
//! 0x229dd30) multiply the page's `.x` by the premultiplied colour, which the
//! textured shader does with that texel.
use crate::assets::iggy::shape::{PathPoint, Segment};
use std::collections::HashMap;
use std::sync::Arc;

/// `player+0x1eac`: rasterisation at (K + 1)× with the box blur.
const K: i32 = 1;
/// `player+0x1eb0` / `player+0x1eb4`.
const MATCH: (f32, f32) = (0.909_090_9, 1.1);
/// `player+0x1e80`.
const TOLERANCE: f32 = 0.25;
/// At most 64 shelves (`FUN_14107a980`).
const MAX_SHELVES: usize = 64;

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// `IggyPlayerSetBitmapFontCaching` from the movie's `m_vectorFontBuffer`
/// (`FUN_14002c440`): page size and the largest cached glyph in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontCaching {
    pub page: (u32, u32),
    /// Width and height limits (`player+0x1ea0`, `player+0x1e9c`).
    pub max_glyph: (f32, f32),
}

impl FontCaching {
    /// `FUN_14002c440`: {count, width, height} -> page sides
    /// `nextpow2(side · trunc(√count))`, doubled when the display is above
    /// 1920×1080, and the glyph limits.
    pub fn from_buffer(count: u32, width: u32, height: u32, display: (u32, u32)) -> Option<FontCaching> {
        if count == 0 || width == 0 || height == 0 {
            return None;
        }
        let root = (count as f32).sqrt() as u32;
        let mut page = ((width * root).next_power_of_two(), (height * root).next_power_of_two());
        if display.0 > 1920 || display.1 > 1080 {
            page = (page.0 * 2, page.1 * 2);
        }
        Some(FontCaching { page, max_glyph: (width as f32, height as f32) })
    }
}

/// The cache page shared with the renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// Distinct per cache.
    pub id: u64,
    pub width: u32,
    pub height: u32,
    /// RGBA8, `(c, c, c, c)` per texel.
    pub rgba: Vec<u8>,
    /// Changes whenever texels change.
    pub version: u64,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    /// Pixel scales it was made for (`+0x30`, `+0x34`).
    scale: (f32, f32),
    /// UVs (`+0x20..0x2c`) and the quad in em units (`+0x38..0x44`).
    uv: [f32; 4],
    quad: [f32; 4],
}

/// A shelf: its y, height and filled width.
#[derive(Clone, Copy, Debug)]
struct Shelf {
    y: i32,
    height: i32,
    x: i32,
}

/// A drawable cached glyph: the quad in em units and its page UVs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CachedGlyph {
    pub quad: [f32; 4],
    pub uv: [f32; 4],
}

pub struct GlyphCache {
    caching: FontCaching,
    page: Arc<Page>,
    shelves: Vec<Shelf>,
    /// By (font address, glyph index).
    entries: HashMap<(usize, usize), Vec<Entry>>,
}

impl GlyphCache {
    pub fn new(caching: FontCaching) -> GlyphCache {
        let (w, h) = caching.page;
        GlyphCache {
            caching,
            page: Arc::new(Page { id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed), width: w, height: h, rgba: vec![0; (w * h * 4) as usize], version: 0 }),
            shelves: Vec::new(),
            entries: HashMap::new(),
        }
    }

    pub fn caching(&self) -> FontCaching {
        self.caching
    }

    pub fn page(&self) -> &Arc<Page> {
        &self.page
    }

    /// `FUN_141098c20`: whether a glyph of these em extents is drawn from the
    /// cache at pixel scales `scale` (`size · sx`, `size · sy`).
    pub fn allowed(&self, extent: (f32, f32), scale: (f32, f32)) -> bool {
        extent.0 > 0.0 && extent.1 > 0.0 && extent.0 * scale.0 < self.caching.max_glyph.0 && extent.1 * scale.1 < self.caching.max_glyph.1
    }

    /// `FUN_14107b980`: the entry for glyph `key` at pixel scales `scale`,
    /// rasterising it when none matches. `None` when the glyph has no
    /// coverage or the page is full.
    pub fn glyph(&mut self, key: (usize, usize), points: &[PathPoint], scale: (f32, f32)) -> Option<CachedGlyph> {
        if let Some(e) = self.entries.get(&key).and_then(|list| {
            list.iter().find(|e| {
                MATCH.0 * scale.0 <= e.scale.0 && e.scale.0 <= MATCH.1 * scale.0 && MATCH.0 * scale.1 <= e.scale.1 && e.scale.1 <= MATCH.1 * scale.1
            })
        }) {
            return Some(CachedGlyph { quad: e.quad, uv: e.uv });
        }
        let entry = self.make(points, scale)?;
        self.entries.entry(key).or_default().push(entry);
        Some(CachedGlyph { quad: entry.quad, uv: entry.uv })
    }

    /// `FUN_14107ad10` and `FUN_14107b2d0`.
    fn make(&mut self, points: &[PathPoint], scale: (f32, f32)) -> Option<Entry> {
        let factor = (K + 1) as f32;
        let (sx, sy) = (factor * scale.0, factor * scale.1);
        let (x0, y0, x1, y1) = bounds(points, sx, sy)?;
        // The raster starts (k + 2) / 2 pixels left of the glyph.
        let origin_x = x0 - (K + 2) / 2;
        let (w, h) = (x1 - origin_x, y1 - y0);
        if w <= 0 || h <= 0 {
            return None;
        }
        let border = K + 1;
        let stride = w + 2 * border;
        let rows = h + 2 * border;
        let mut pixels = vec![0u8; (stride * rows) as usize];
        {
            let offset = (border * stride + border) as usize;
            rasterize(&mut pixels[offset..], w, h, stride, TOLERANCE, points, sx, sy, origin_x, y0);
        }
        if K != 0 {
            // In place over rows 0..=h, columns 0..=w of the interior's
            // top-left: each texel the rounded mean of itself, its right,
            // lower and lower-right neighbours, read one row/column up-left.
            let base = (K * stride + K) as usize;
            for row in 0..=h as usize {
                for col in 0..=w as usize {
                    let at = base + row * stride as usize + col;
                    let (a, b, c, d) = (pixels[at], pixels[at + 1], pixels[at + stride as usize], pixels[at + stride as usize + 1]);
                    pixels[at] = ((a as u32 + b as u32 + c as u32 + d as u32 + 2) >> 2) as u8;
                }
            }
        }
        let (px, py) = self.place(w, h)?;
        // Copy the bordered raster to (px - border, py - border).
        let page = Arc::make_mut(&mut self.page);
        for row in 0..rows {
            for col in 0..stride {
                let (tx, ty) = (px - border + col, py - border + row);
                if tx < 0 || ty < 0 || tx >= page.width as i32 || ty >= page.height as i32 {
                    continue;
                }
                let c = pixels[(row * stride + col) as usize];
                let at = ((ty as u32 * page.width + tx as u32) * 4) as usize;
                page.rgba[at..at + 4].copy_from_slice(&[c; 4]);
            }
        }
        page.version += 1;
        let (pw, ph) = (page.width as f32, page.height as f32);
        let half = factor * 0.5;
        let uv = [(px as f32 - half) / pw, (py as f32 - half) / ph, ((w + px) as f32 + half) / pw, ((h + py) as f32 + half) / ph];
        let k_half = K as f32 * 0.5;
        let quad = [
            ((origin_x as f32 + k_half) / factor - 0.5) / scale.0,
            ((y0 as f32 + k_half) / factor - 0.5) / scale.1,
            ((origin_x as f32 + k_half + w as f32) / factor + 0.5) / scale.0,
            ((y0 as f32 + k_half + h as f32) / factor + 0.5) / scale.1,
        ];
        Some(Entry { scale, uv, quad })
    }

    /// `FUN_14107a980`'s shelves: the lowest shelf tall enough with room,
    /// else a new shelf of `k + 2 + (h + 2) / 5 + h` (clipped to the page);
    /// a full page starts over (the original evicts entries unused this
    /// frame from fitting shelves; not ported).
    fn place(&mut self, w: i32, h: i32) -> Option<(i32, i32)> {
        let (pw, ph) = (self.page.width as i32, self.page.height as i32);
        let reach = K + 1;
        let (need_w, need_h) = (w + reach, h + reach);
        if pw < need_w * 2 || ph < need_h * 2 {
            return None;
        }
        let fits = |s: &Shelf| need_h <= s.height && s.x + need_w <= pw;
        let best = self.shelves.iter().enumerate().filter(|(_, s)| fits(s)).min_by_key(|(_, s)| s.height).map(|(i, _)| i);
        let best = best.filter(|&i| self.shelves[i].height <= (need_h * 2).max(5));
        let index = match best {
            Some(i) => i,
            None => {
                let top = self.shelves.last().map_or(reach, |s| s.y + s.height + reach);
                let mut height = K + 2 + (h + 2) / 5 + h;
                if !self.shelves.is_empty() && ph < top + height {
                    height = ph - top;
                }
                if self.shelves.len() == MAX_SHELVES || height < need_h {
                    // Start over: every entry is dropped with its texels.
                    self.shelves.clear();
                    self.entries.clear();
                    let page = Arc::make_mut(&mut self.page);
                    page.rgba.iter_mut().for_each(|b| *b = 0);
                    page.version += 1;
                    return self.place(w, h);
                }
                self.shelves.push(Shelf { y: top, height, x: reach });
                self.shelves.len() - 1
            }
        };
        let shelf = &mut self.shelves[index];
        let at = (shelf.x, shelf.y);
        shelf.x += w + reach;
        Some(at)
    }
}

/// `FUN_14107b4f0`: the pixel box of every point and control point at the
/// scales: floor of the minimums, ceiling of the maximums.
fn bounds(points: &[PathPoint], sx: f32, sy: f32) -> Option<(i32, i32, i32, i32)> {
    let first = points.first()?;
    let (mut minx, mut miny, mut maxx, mut maxy) = (first.x, first.y, first.x, first.y);
    for p in &points[1..] {
        minx = minx.min(p.x);
        miny = miny.min(p.y);
        maxx = maxx.max(p.x);
        maxy = maxy.max(p.y);
        if p.segment == Segment::CurveTo {
            minx = minx.min(p.cx);
            miny = miny.min(p.cy);
            maxx = maxx.max(p.cx);
            maxy = maxy.max(p.cy);
        }
    }
    Some(((minx * sx).floor() as i32, (miny * sy).floor() as i32, (maxx * sx).ceil() as i32, (maxy * sy).ceil() as i32))
}

/// `FUN_14109a480` (stbtt__tesselate_curve).
#[allow(clippy::too_many_arguments)]
fn tesselate(out: &mut Vec<[f32; 2]>, x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32, flatness_squared: f32, n: i32) {
    let mx = (x0 + 2.0 * x1 + x2) * 0.25;
    let my = (y0 + 2.0 * y1 + y2) * 0.25;
    let dx = (x0 + x2) * 0.5 - mx;
    let dy = (y0 + y2) * 0.5 - my;
    if n > 16 {
        return;
    }
    if dx * dx + dy * dy > flatness_squared {
        tesselate(out, x0, y0, (x0 + x1) * 0.5, (y0 + y1) * 0.5, mx, my, flatness_squared, n + 1);
        tesselate(out, mx, my, (x1 + x2) * 0.5, (y1 + y2) * 0.5, x2, y2, flatness_squared, n + 1);
    } else {
        out.push([x2, y2]);
    }
}

/// `FUN_141098dc0` (stbtt_FlattenCurves): contours of flattened points.
fn flatten(points: &[PathPoint], flatness: f32) -> Vec<Vec<[f32; 2]>> {
    let mut contours: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut pen = [0f32; 2];
    for p in points {
        match p.segment {
            Segment::MoveTo => {
                contours.push(vec![[p.x, p.y]]);
            }
            Segment::LineTo => {
                if let Some(c) = contours.last_mut() {
                    c.push([p.x, p.y]);
                }
            }
            Segment::CurveTo => {
                if let Some(c) = contours.last_mut() {
                    tesselate(c, pen[0], pen[1], p.cx, p.cy, p.x, p.y, flatness * flatness, 0);
                }
            }
        }
        pen = [p.x, p.y];
    }
    contours
}

#[derive(Clone, Copy)]
struct Edge {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    invert: bool,
}

struct Active {
    fx: f32,
    fdx: f32,
    fdy: f32,
    direction: f32,
    sy: f32,
    ey: f32,
}

/// `FUN_141099000` with `FUN_1410999c0`: rasterise into `pixels` (`w`×`h`,
/// `stride`), points scaled by (sx, sy), the raster's top-left at
/// (off_x, off_y) pixels.
#[allow(clippy::too_many_arguments)]
fn rasterize(pixels: &mut [u8], w: i32, h: i32, stride: i32, tolerance: f32, points: &[PathPoint], sx: f32, sy: f32, off_x: i32, off_y: i32) {
    let contours = flatten(points, tolerance / sx.min(sy));
    let mut edges = Vec::new();
    for c in &contours {
        let n = c.len();
        let mut j = n.wrapping_sub(1);
        for k in 0..n {
            let (a, b) = (c[j], c[k]);
            j = k;
            if a[1] == b[1] {
                continue;
            }
            // From the lower y to the higher; invert when the contour goes up.
            let (from, to, invert) = if a[1] < b[1] { (a, b, true) } else { (b, a, false) };
            edges.push(Edge { x0: from[0] * sx, y0: from[1] * sy, x1: to[0] * sx, y1: to[1] * sy, invert });
        }
    }
    // stbtt__sort_edges: by y0 (the quick and insertion sorts give an order
    // the scanline pass does not depend on beyond y0).
    edges.sort_by(|a, b| a.y0.partial_cmp(&b.y0).unwrap_or(std::cmp::Ordering::Equal));
    rasterize_sorted(pixels, w, h, stride, &edges, off_x, off_y);
}

/// `FUN_141099cc0` (stbtt__rasterize_sorted_edges, version 2).
fn rasterize_sorted(pixels: &mut [u8], w: i32, h: i32, stride: i32, edges: &[Edge], off_x: i32, off_y: i32) {
    let len = w as usize;
    let mut scanline = vec![0f32; len];
    let mut fill = vec![0f32; len + 1];
    let mut active: Vec<Active> = Vec::new();
    let mut next = 0;
    for j in 0..h {
        let y = (off_y + j) as f32;
        let (top, bottom) = (y, y + 1.0);
        scanline.iter_mut().for_each(|v| *v = 0.0);
        fill.iter_mut().for_each(|v| *v = 0.0);
        active.retain(|z| z.ey > top);
        while next < edges.len() && edges[next].y0 <= bottom {
            let e = edges[next];
            next += 1;
            // `FUN_1410998b0` (stbtt__new_active).
            let dxdy = (e.x1 - e.x0) / (e.y1 - e.y0);
            active.push(Active { fdx: dxdy, fdy: 1.0 / dxdy, fx: e.x0 + dxdy * (top - e.y0) - off_x as f32, direction: if e.invert { 1.0 } else { -1.0 }, sy: e.y0, ey: e.y1 });
        }
        // New edges go to the front of the original's list.
        for z in active.iter().rev() {
            fill_active(&mut scanline, &mut fill, len, z, top);
        }
        let mut sum = 0.0;
        for i in 0..len {
            sum += fill[i];
            let k = (scanline[i] + sum).abs() * 255.0 + 0.5;
            pixels[(j * stride) as usize + i] = (k as i32).min(255) as u8;
        }
        for z in &mut active {
            z.fx += z.fdx;
        }
    }
}

/// `FUN_141099770` (stbtt__handle_clipped_edge) on `line[x]`.
#[allow(clippy::too_many_arguments)]
fn clipped(line: &mut [f32], x: i32, e: &Active, mut x0: f32, mut y0: f32, mut x1: f32, mut y1: f32) {
    if y0 == y1 || y0 > e.ey || y1 < e.sy {
        return;
    }
    if y0 < e.sy {
        x0 += (x1 - x0) * (e.sy - y0) / (y1 - y0);
        y0 = e.sy;
    }
    if y1 > e.ey {
        x1 += (x1 - x0) * (e.ey - y1) / (y1 - y0);
        y1 = e.ey;
    }
    let xf = x as f32;
    let Some(slot) = line.get_mut(x as usize) else { return };
    if x0 <= xf && x1 <= xf {
        *slot += e.direction * (y1 - y0);
    } else if x0 < xf + 1.0 || x1 < xf + 1.0 {
        *slot += e.direction * (y1 - y0) * (1.0 - ((x0 - xf) + (x1 - xf)) * 0.5);
    }
}

/// `FUN_141099120` (stbtt__fill_active_edges_new, early version 2).
fn fill_active(scanline: &mut [f32], fill: &mut [f32], len: usize, e: &Active, y_top: f32) {
    let y_bottom = y_top + 1.0;
    let lenf = len as f32;
    if e.fdx == 0.0 {
        let x0 = e.fx;
        if x0 < lenf {
            if x0 >= 0.0 {
                clipped(scanline, x0 as i32, e, x0, y_top, x0, y_bottom);
                // The original's `scanline_fill - 1` at x0 + 1 is fill[x0 + 1].
                clipped(fill, x0 as i32 + 1, e, x0, y_top, x0, y_bottom);
            } else {
                clipped(fill, 0, e, x0, y_top, x0, y_bottom);
            }
        }
        return;
    }
    let mut x0 = e.fx;
    let mut dx = e.fdx;
    let mut xb = x0 + dx;
    let mut dy = e.fdy;
    let (mut x_top, mut sy0) = if e.sy > y_top { (x0 + dx * (e.sy - y_top), e.sy) } else { (x0, y_top) };
    let (mut x_bottom, mut sy1) = if e.ey < y_bottom { (x0 + dx * (e.ey - y_top), e.ey) } else { (xb, y_bottom) };
    if x_top >= 0.0 && x_bottom >= 0.0 && x_top < lenf && x_bottom < lenf {
        if x_top as i32 == x_bottom as i32 {
            let x = x_top as i32;
            let height = sy1 - sy0;
            scanline[x as usize] += e.direction * (1.0 - ((x_top - x as f32) + (x_bottom - x as f32)) * 0.5) * height;
            fill[x as usize + 1] += e.direction * height;
        } else {
            if x_top > x_bottom {
                // Flip vertically; the signed area is the same.
                sy0 = y_bottom - (sy0 - y_top);
                sy1 = y_bottom - (sy1 - y_top);
                std::mem::swap(&mut sy0, &mut sy1);
                std::mem::swap(&mut x_bottom, &mut x_top);
                dx = -dx;
                dy = -dy;
                std::mem::swap(&mut x0, &mut xb);
            }
            let x1 = x_top as i32;
            let x2 = x_bottom as i32;
            let mut y_crossing = (x1 as f32 + 1.0 - x0) * dy + y_top;
            let sign = e.direction;
            let mut area = sign * (y_crossing - sy0);
            scanline[x1 as usize] += area * (1.0 - ((x_top - x1 as f32) + 1.0) * 0.5);
            let step = sign * dy;
            for x in x1 + 1..x2 {
                scanline[x as usize] += area + step * 0.5;
                area += step;
            }
            y_crossing += dy * (x2 - (x1 + 1)) as f32;
            scanline[x2 as usize] += area + sign * (1.0 - (x_bottom - x2 as f32) * 0.5) * (sy1 - y_crossing);
            fill[x2 as usize + 1] += sign * (sy1 - sy0);
        }
        return;
    }
    // Clipping: the brute-force path over every pixel.
    for x in 0..len as i32 {
        let y0 = y_top;
        let x1 = x as f32;
        let x2 = x as f32 + 1.0;
        let x3 = xb;
        let y3 = y_bottom;
        let y1 = (x as f32 - x0) / dx + y_top;
        let y2 = (x as f32 + 1.0 - x0) / dx + y_top;
        if x0 < x1 && x3 > x2 {
            clipped(scanline, x, e, x0, y0, x1, y1);
            clipped(scanline, x, e, x1, y1, x2, y2);
            clipped(scanline, x, e, x2, y2, x3, y3);
        } else if x3 < x1 && x0 > x2 {
            clipped(scanline, x, e, x0, y0, x2, y2);
            clipped(scanline, x, e, x2, y2, x1, y1);
            clipped(scanline, x, e, x1, y1, x3, y3);
        } else if x0 < x1 && x3 > x1 {
            clipped(scanline, x, e, x0, y0, x1, y1);
            clipped(scanline, x, e, x1, y1, x3, y3);
        } else if x3 < x1 && x0 > x1 {
            clipped(scanline, x, e, x0, y0, x1, y1);
            clipped(scanline, x, e, x1, y1, x3, y3);
        } else if x0 < x2 && x3 > x2 {
            clipped(scanline, x, e, x0, y0, x2, y2);
            clipped(scanline, x, e, x2, y2, x3, y3);
        } else if x3 < x2 && x0 > x2 {
            clipped(scanline, x, e, x0, y0, x2, y2);
            clipped(scanline, x, e, x2, y2, x3, y3);
        } else {
            clipped(scanline, x, e, x0, y0, x3, y3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<PathPoint> {
        let p = |x, y, segment| PathPoint { x, y, cx: 0.0, cy: 0.0, segment, fill0: 0, fill1: 1, line: 0 };
        vec![p(x0, y0, Segment::MoveTo), p(x1, y0, Segment::LineTo), p(x1, y1, Segment::LineTo), p(x0, y1, Segment::LineTo), p(x0, y0, Segment::LineTo)]
    }

    #[test]
    fn a_pixel_aligned_square_covers_its_pixels_exactly() {
        let mut pixels = vec![0u8; 8 * 8];
        rasterize(&mut pixels, 8, 8, 8, TOLERANCE, &square(2.0, 2.0, 5.0, 6.0), 1.0, 1.0, 0, 0);
        for y in 0..8 {
            for x in 0..8 {
                let inside = (2..5).contains(&x) && (2..6).contains(&y);
                assert_eq!(pixels[y * 8 + x], if inside { 255 } else { 0 }, "({x}, {y})");
            }
        }
    }

    #[test]
    fn a_half_pixel_edge_gives_half_coverage() {
        let mut pixels = vec![0u8; 4 * 4];
        rasterize(&mut pixels, 4, 4, 4, TOLERANCE, &square(1.5, 0.0, 3.0, 4.0), 1.0, 1.0, 0, 0);
        assert_eq!(pixels[1], 128);
        assert_eq!(pixels[2], 255);
        assert_eq!(pixels[0], 0);
    }

    #[test]
    fn entries_are_reused_within_ten_percent() {
        let caching = FontCaching::from_buffer(128, 64, 64, (1920, 1080)).unwrap();
        assert_eq!(caching.page, (1024, 1024));
        let mut cache = GlyphCache::new(caching);
        let glyph = square(0.0, -0.7, 0.5, 0.0);
        let a = cache.glyph((1, 2), &glyph, (40.0, 40.0)).unwrap();
        let version = cache.page().version;
        let b = cache.glyph((1, 2), &glyph, (42.0, 42.0)).unwrap();
        assert_eq!(a, b);
        assert_eq!(cache.page().version, version);
        cache.glyph((1, 2), &glyph, (60.0, 60.0)).unwrap();
        assert!(cache.page().version > version);
    }
}
