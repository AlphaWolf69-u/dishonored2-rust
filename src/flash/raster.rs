//! CPU reference rasterizer for draw lists (diagnostics and tests).
use super::render::{mode, DrawList, Texture};
use super::player::CharRef;
use crate::assets::archive::Catalog;
use crate::assets::iggy::shape::TextureEntry;
use crate::assets::iggy::IggyFile;
use std::collections::HashMap;

/// Decoded RGBA8 images of library bitmaps (mip 0 of each iggytex entry).
#[derive(Default)]
pub struct Images {
    pub images: HashMap<CharRef, (u32, u32, Vec<u8>)>,
}

/// Decode mip 0 of a raw iggytex entry (`FUN_1402e2c00` formats).
pub fn decode_entry(data: &[u8], e: &TextureEntry) -> Result<Vec<u8>, String> {
    let (w, h) = (e.width as usize, e.height as usize);
    let start = e.offset as usize;
    match e.format {
        13 => {
            let blocks = w.div_ceil(4) * h.div_ceil(4);
            let src = data.get(start..start + blocks * 16).ok_or("iggytex entry outside file")?;
            let mut out = vec![0u8; w.div_ceil(4) * 4 * h.div_ceil(4) * 4 * 4];
            let pitch = w.div_ceil(4) * 4 * 4;
            for by in 0..h.div_ceil(4) {
                for bx in 0..w.div_ceil(4) {
                    let block = &src[(by * w.div_ceil(4) + bx) * 16..][..16];
                    let offset = by * 4 * pitch + bx * 16;
                    bcdec_rs::bc7(block, &mut out[offset..], pitch);
                }
            }
            // Crop the 4×4-aligned decode to the image size.
            let mut cropped = Vec::with_capacity(w * h * 4);
            for y in 0..h {
                cropped.extend_from_slice(&out[y * pitch..y * pitch + w * 4]);
            }
            Ok(cropped)
        }
        0 => Ok(data.get(start..start + w * h * 4).ok_or("iggytex entry outside file")?.to_vec()),
        f => Err(format!("iggytex format {f} is not decoded by the preview")),
    }
}

impl Images {
    pub fn load(&mut self, catalog: &Catalog, library: usize, resource: &str, file: &IggyFile) -> Result<(), String> {
        let entries = file.textures().map_err(|e| e.to_string())?;
        if entries.is_empty() {
            return Ok(());
        }
        let path = resource.replace(".iggy", ".raw.iggytex");
        let data = catalog.read(&path).map_err(|e| e.to_string())?.ok_or_else(|| format!("{path} not indexed"))?;
        for e in &entries {
            let rgba = decode_entry(&data, e)?;
            self.images.insert(CharRef { library, id: e.character }, (e.width as u32, e.height as u32, rgba));
        }
        Ok(())
    }
}

fn sample(image: &(u32, u32, Vec<u8>), u: f32, v: f32, repeat: bool, smooth: bool) -> [f32; 4] {
    let (w, h, data) = image;
    let fetch = |x: i64, y: i64| -> [f32; 4] {
        let (x, y) = if repeat {
            (x.rem_euclid(*w as i64), y.rem_euclid(*h as i64))
        } else {
            (x.clamp(0, *w as i64 - 1), y.clamp(0, *h as i64 - 1))
        };
        let i = ((y as usize) * (*w as usize) + x as usize) * 4;
        [data[i] as f32 / 255.0, data[i + 1] as f32 / 255.0, data[i + 2] as f32 / 255.0, data[i + 3] as f32 / 255.0]
    };
    let (x, y) = (u * *w as f32 - 0.5, v * *h as f32 - 0.5);
    if !smooth {
        return fetch((x + 0.5).floor() as i64, (y + 0.5).floor() as i64);
    }
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let (a, b, c, d) = (fetch(x0, y0), fetch(x0 + 1, y0), fetch(x0, y0 + 1), fetch(x0 + 1, y0 + 1));
    std::array::from_fn(|k| (a[k] * (1.0 - fx) + b[k] * fx) * (1.0 - fy) + (c[k] * (1.0 - fx) + d[k] * fx) * fy)
}

/// Rasterize `list` into an RGBA8 image of `width`×`height` over `background`.
pub fn rasterize(list: &DrawList, images: &Images, width: u32, height: u32, background: [u8; 4]) -> Vec<u8> {
    let mut px: Vec<[f32; 4]> = vec![background.map(|v| v as f32 / 255.0); (width * height) as usize];
    let sx = width as f32 / list.stage.0;
    let sy = height as f32 / list.stage.1;
    let white = (1, 1, vec![255u8; 4]);
    for batch in &list.batches {
        let image: Option<(u32, u32, Vec<u8>)> = match &batch.texture {
            Texture::White => None,
            Texture::Bitmap(c) => images.images.get(c).cloned(),
            Texture::Ramp(i) => Some((64, 1, list.ramps[*i].to_vec())),
        };
        let premultiplied = matches!(batch.texture, Texture::Ramp(_));
        let verts = &list.vertices[batch.first as usize..(batch.first + batch.count) as usize];
        for tri in verts.chunks_exact(3) {
            let p: Vec<[f32; 2]> = tri.iter().map(|v| [v.position[0] * sx, v.position[1] * sy]).collect();
            let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
            if area.abs() < 1e-9 {
                continue;
            }
            let (minx, maxx) = (p.iter().map(|q| q[0]).fold(f32::MAX, f32::min), p.iter().map(|q| q[0]).fold(f32::MIN, f32::max));
            let (miny, maxy) = (p.iter().map(|q| q[1]).fold(f32::MAX, f32::min), p.iter().map(|q| q[1]).fold(f32::MIN, f32::max));
            let (x0, x1) = (minx.floor().max(0.0) as u32, (maxx.ceil() as i64).clamp(0, width as i64) as u32);
            let (y0, y1) = (miny.floor().max(0.0) as u32, (maxy.ceil() as i64).clamp(0, height as i64) as u32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((p[1][0] - cx) * (p[2][1] - cy) - (p[2][0] - cx) * (p[1][1] - cy)) / area;
                    let w1 = ((p[2][0] - cx) * (p[0][1] - cy) - (p[0][0] - cx) * (p[2][1] - cy)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    // Perspective-correct weights (vertex w, 1 in 2D).
                    let (q0, q1, q2) = (w0 / tri[0].w, w1 / tri[1].w, w2 / tri[2].w);
                    let sum = q0 + q1 + q2;
                    let (w0, w1, w2) = (q0 / sum, q1 / sum, q2 / sum);
                    let lerp = |f: &dyn Fn(usize) -> f32| w0 * f(0) + w1 * f(1) + w2 * f(2);
                    let u = lerp(&|i| tri[i].uv[0]);
                    let v = lerp(&|i| tri[i].uv[1]);
                    let sampling = tri[0].params[0] as u32;
                    let (u, v) = match sampling {
                        mode::RADIAL => ((u * u + v * v).sqrt(), 0.5),
                        mode::FOCAL => {
                            let f = tri[0].params[1];
                            // Distance ratio from the focal point to the unit circle.
                            let (dx, dy) = (u - f, v);
                            let a = dx * dx + dy * dy;
                            let b = 2.0 * (f * dx);
                            let c = f * f - 1.0;
                            let t = (-b + (b * b - 4.0 * a * c).max(0.0).sqrt()) / (2.0 * a).max(1e-9);
                            (1.0 / t.max(1e-9), 0.5)
                        }
                        _ => (u, v),
                    };
                    let mut texel = match &image {
                        Some(img) => sample(img, u, v, batch.repeat, batch.smooth),
                        None => sample(&white, 0.5, 0.5, false, false),
                    };
                    if premultiplied && texel[3] > 0.0 {
                        for k in 0..3 {
                            texel[k] /= texel[3];
                        }
                    }
                    let mul = tri[0].mul;
                    let add = tri[0].add;
                    let src: [f32; 4] = std::array::from_fn(|k| (texel[k] * mul[k] + add[k]).clamp(0.0, 1.0));
                    let dst = &mut px[(y * width + x) as usize];
                    match batch.blend {
                        7 => {
                            for k in 0..3 {
                                dst[k] = (dst[k] + src[k] * src[3]).min(1.0);
                            }
                        }
                        _ => {
                            for k in 0..3 {
                                dst[k] = src[k] * src[3] + dst[k] * (1.0 - src[3]);
                            }
                            dst[3] = src[3] + dst[3] * (1.0 - src[3]);
                        }
                    }
                }
            }
        }
    }
    px.iter().flat_map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect()
}
