//! Display-list traversal into GPU-ready triangle batches (Iggy's GDraw
//! input). Fill texgen follows `FUN_141089760`/`FUN_141088fa0`/`FUN_141088c80`.
//!
//! Not yet drawn: masks and clip layers (mask objects are skipped), filters,
//! cacheAsBitmap, non-normal blend modes beyond the batch flag, and 3D
//! transforms.
use super::three_d;
use super::avm2::object::Obj;
use super::avm2::Vm;
use super::display::{children, display_ref, DisplayKind};
use super::geom::{Affine, Colour};
use super::player::{self, CharRef};
use super::tessellate::{fill_edges, triangulate};
use crate::assets::iggy::shape::{Fill, Gradient, Shape};
use std::collections::HashMap;

/// Fill sampling mode in the shader.
pub mod mode {
    pub const TEXTURE: u32 = 0;
    pub const RADIAL: u32 = 1;
    pub const FOCAL: u32 = 2;
    /// Texture coordinates into a premultiplied gradient ramp.
    pub const LINEAR_RAMP: u32 = 3;
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Vertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub mul: [f32; 4],
    pub add: [f32; 4],
    /// Sampling mode and focal point.
    pub params: [f32; 2],
    /// Perspective divisor `(Z + f·Wh) / f` of 3D content, 1 in 2D: the
    /// GDraw vertex shader divides by it and interpolates texture
    /// coordinates perspective-correctly.
    pub w: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Texture {
    White,
    Bitmap(CharRef),
    /// Index into `DrawList::ramps`.
    Ramp(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    pub texture: Texture,
    pub repeat: bool,
    pub smooth: bool,
    /// Iggy blend code (1 normal).
    pub blend: u8,
    pub first: u32,
    pub count: u32,
}

#[derive(Clone, Debug, Default)]
pub struct DrawList {
    pub vertices: Vec<Vertex>,
    pub batches: Vec<Batch>,
    /// 64×1 RGBA8 gradient ramps.
    pub ramps: Vec<[u8; 256]>,
    pub stage: (f32, f32),
}

/// `FUN_1410947c0`: a 64-texel ramp sampled at ratio `i*4 + 2`, then
/// premultiplied with `(c*a + 0x80 + ((c*a + 0x80) >> 8)) >> 8`.
pub fn gradient_ramp(g: &Gradient) -> [u8; 256] {
    let mut out = [0u8; 256];
    for i in 0..64 {
        let ratio = (i * 4 + 2) as f32;
        let c = match g.stops.iter().position(|(_, r)| *r as f32 >= ratio) {
            Some(0) => g.stops[0].0.map(|v| v as f32),
            Some(k) => {
                let (c0, r0) = g.stops[k - 1];
                let (c1, r1) = g.stops[k];
                let t = if r1 > r0 { (ratio - r0 as f32) / (r1 as f32 - r0 as f32) } else { 0.0 };
                std::array::from_fn(|j| c0[j] as f32 + (c1[j] as f32 - c0[j] as f32) * t)
            }
            None => g.stops.last().map_or([0.0; 4], |s| s.0.map(|v| v as f32)),
        };
        let a = c[3].round() as u32;
        for j in 0..3 {
            let v = c[j].round() as u32 * a + 0x80;
            out[i * 4 + j] = ((v + (v >> 8)) >> 8) as u8;
        }
        out[i * 4 + 3] = a as u8;
    }
    out
}

fn matrix(m: &[f32; 6]) -> Affine {
    Affine { a: m[0] as f64, c: m[1] as f64, b: m[2] as f64, d: m[3] as f64, tx: m[4] as f64, ty: m[5] as f64 }
}

struct Builder<'a> {
    vm: &'a Vm,
    /// Diagnostic: draw objects whose `visible` is false.
    debug_ignore_visibility: bool,
    out: DrawList,
    ramps: HashMap<Vec<u8>, usize>,
    /// The current 3D world (`P+0x2488`) in stage pixels, inside a 3D subtree.
    world: Option<three_d::Mat4>,
    projection: three_d::Projection,
}

impl Builder<'_> {
    fn push(&mut self, texture: Texture, repeat: bool, smooth: bool, blend: u8, vertices: impl IntoIterator<Item = Vertex>) {
        let first = self.out.vertices.len() as u32;
        match self.world {
            // 3D content reaches the projection in local units (`FUN_14103b040`).
            Some(world) => {
                let projection = self.projection;
                self.out.vertices.extend(vertices.into_iter().map(|mut v| {
                    let (p, w) = projection.project(&world, v.position[0], v.position[1]);
                    v.position = p;
                    v.w = w;
                    v
                }));
            }
            None => self.out.vertices.extend(vertices),
        }
        let count = self.out.vertices.len() as u32 - first;
        if count == 0 {
            return;
        }
        if let Some(last) = self.out.batches.last_mut() {
            if last.texture == texture && last.repeat == repeat && last.smooth == smooth && last.blend == blend && last.first + last.count == first {
                last.count += count;
                return;
            }
        }
        self.out.batches.push(Batch { texture, repeat, smooth, blend, first, count });
    }

    fn ramp(&mut self, g: &Gradient) -> usize {
        let ramp = gradient_ramp(g);
        let key = ramp.to_vec();
        if let Some(&i) = self.ramps.get(&key) {
            return i;
        }
        self.out.ramps.push(ramp);
        self.ramps.insert(key, self.out.ramps.len() - 1);
        self.out.ramps.len() - 1
    }

    /// `script` names the library bitmaps of script bitmap fills
    /// (`Graphics.beginBitmapFill`): (layer, fill number, character).
    fn shape(&mut self, shape: &Shape, library: usize, m: &Affine, c: &Colour, blend: u8, script: &[(usize, u16, CharRef)]) {
        let scale = ((m.a * m.a + m.b * m.b).sqrt().max((m.c * m.c + m.d * m.d).sqrt())) as f32;
        let file = player::state(self.vm).borrow().file(library);
        for (layer_index, layer) in shape.layers.iter().enumerate() {
            for (i, fill) in layer.fills.iter().enumerate() {
                // A script bitmap fill draws its BitmapData's character.
                let resolved;
                let (fill, file) = match fill {
                    Fill::Bitmap { matrix, bitmap: None, repeat, smooth } => {
                        match script.iter().find(|s| s.0 == layer_index && s.1 == i as u16 + 1) {
                            Some(&(_, _, ch)) => {
                                resolved = Fill::Bitmap { matrix: *matrix, bitmap: Some(ch.id), repeat: *repeat, smooth: *smooth };
                                (&resolved, player::state(self.vm).borrow().file(ch.library))
                            }
                            None => continue,
                        }
                    }
                    other => (other, file.clone()),
                };
                let library = match fill {
                    Fill::Bitmap { .. } => script.iter().find(|s| s.0 == layer_index && s.1 == i as u16 + 1).map_or(library, |s| s.2.library),
                    _ => library,
                };
                let tris = triangulate(&fill_edges(layer, i as u16 + 1, scale));
                if tris.is_empty() {
                    continue;
                }
                let (texture, repeat, smooth, colour, texgen): (Texture, bool, bool, [f32; 4], Box<dyn Fn([f32; 2]) -> ([f32; 2], [f32; 2])>) = match fill {
                    Fill::Solid(rgba) => (Texture::White, false, true, rgba.map(|v| v as f32 / 255.0), Box::new(|_| ([0.0, 0.0], [mode::TEXTURE as f32, 0.0]))),
                    Fill::Bitmap { matrix: fm, bitmap: Some(id), repeat, smooth } => {
                        let Ok(Some(b)) = file.bitmap(*id as usize) else { continue };
                        let inv = matrix(fm).invert();
                        let (w, h, uv) = (b.width as f64, b.height as f64, b.uv);
                        (
                            Texture::Bitmap(CharRef { library, id: *id }),
                            *repeat,
                            *smooth,
                            [1.0; 4],
                            Box::new(move |p| {
                                let (x, y) = inv.apply(p[0] as f64, p[1] as f64);
                                ([(x / w) as f32 * uv[0] + uv[2], (y / h) as f32 * uv[1] + uv[3]], [mode::TEXTURE as f32, 0.0])
                            }),
                        )
                    }
                    Fill::Bitmap { bitmap: None, .. } => continue,
                    Fill::Linear { matrix: fm, gradient } => {
                        let ramp = self.ramp(gradient);
                        let inv = matrix(fm).invert();
                        (Texture::Ramp(ramp), false, true, [1.0; 4], Box::new(move |p| {
                            let (x, _) = inv.apply(p[0] as f64, p[1] as f64);
                            ([(x / 32768.0 + 0.5) as f32, 0.5], [mode::LINEAR_RAMP as f32, 0.0])
                        }))
                    }
                    Fill::Radial { matrix: fm, gradient } | Fill::Focal { matrix: fm, gradient } => {
                        let ramp = self.ramp(gradient);
                        let inv = matrix(fm).invert();
                        let (sampling, focal) = match fill {
                            Fill::Focal { gradient, .. } => (mode::FOCAL, gradient.focal * -0.999),
                            _ => (mode::RADIAL, 0.0),
                        };
                        (Texture::Ramp(ramp), false, true, [1.0; 4], Box::new(move |p| {
                            let (x, y) = inv.apply(p[0] as f64, p[1] as f64);
                            ([(x / 16384.0) as f32, (y / 16384.0) as f32], [sampling as f32, focal])
                        }))
                    }
                };
                let mul = std::array::from_fn(|k| colour[k] * c.mul[k] as f32);
                let add = c.add.map(|v| v as f32 / 255.0);
                let vertices: Vec<Vertex> = tris
                    .into_iter()
                    .map(|p| {
                        let (uv, params) = texgen(p);
                        let (x, y) = m.apply(p[0] as f64, p[1] as f64);
                        Vertex { position: [x as f32, y as f32], uv, mul, add, params, w: 1.0 }
                    })
                    .collect();
                self.push(texture, repeat, smooth, blend, vertices);
            }
        }
    }

    fn bitmap(&mut self, c: CharRef, w: f32, h: f32, uv: [f32; 4], m: &Affine, col: &Colour, smooth: bool, blend: u8) {
        let mul = col.mul.map(|v| v as f32);
        let add = col.add.map(|v| v as f32 / 255.0);
        let corner = |x: f32, y: f32| {
            let (px, py) = m.apply(x as f64, y as f64);
            Vertex { position: [px as f32, py as f32], uv: [x / w * uv[0] + uv[2], y / h * uv[1] + uv[3]], mul, add, params: [0.0; 2], w: 1.0 }
        };
        let (a, b, cc, d) = (corner(0.0, 0.0), corner(w, 0.0), corner(w, h), corner(0.0, h));
        self.push(Texture::Bitmap(c), false, smooth, blend, [a, b, cc, a, cc, d]);
    }

    /// `FUN_14102dcd0`: the engine's visible glyphs, tessellated as vectors
    /// (the glyph bitmap cache is not ported).
    fn text(&mut self, o: &Obj, m: &Affine, c: &Colour, blend: u8) {
        let glyphs = super::text::text_object(o).map(|t| t.glyphs()).unwrap_or_default();
        for glyph in glyphs {
            let colour = [((glyph.colour >> 16) & 0xff) as f32 / 255.0, ((glyph.colour >> 8) & 0xff) as f32 / 255.0, (glyph.colour & 0xff) as f32 / 255.0, 1.0];
            let gm = Affine { a: glyph.size as f64, b: 0.0, c: 0.0, d: glyph.size as f64, tx: glyph.x as f64, ty: glyph.y as f64 }.then(m);
            let scale = ((gm.a * gm.a + gm.b * gm.b).sqrt()) as f32;
            for layer in &glyph.font.glyphs[glyph.glyph].layers {
                // Glyph paths reference fill 1 and carry no fill list.
                let tris = triangulate(&fill_edges(layer, 1, scale));
                let mul = std::array::from_fn(|k| colour[k] * c.mul[k] as f32);
                let add = c.add.map(|v| v as f32 / 255.0);
                let vertices: Vec<Vertex> = tris
                    .into_iter()
                    .map(|p| {
                        let (x, y) = gm.apply(p[0] as f64, p[1] as f64);
                        Vertex { position: [x as f32, y as f32], uv: [0.0; 2], mul, add, params: [0.0; 2], w: 1.0 }
                    })
                    .collect();
                self.push(Texture::White, false, true, blend, vertices);
            }
        }
    }

    fn node(&mut self, o: &Obj, parent_m: &Affine, parent_c: &Colour, masks: &[Obj]) {
        let Some(d) = display_ref(o) else { return };
        if (!d.visible && !self.debug_ignore_visibility) || masks.contains(o) || d.clip_depth > 0 {
            return;
        }
        let own = d.matrix;
        let c = d.colour.then(parent_c);
        let (kind, character, graphics, blend) = (d.kind, d.character, d.graphics.clone(), d.blend);
        drop(d);
        // `FUN_141037930`: the first object with a Matrix3D starts the 3D
        // world from its parent's 2D world and ignores its own 2D matrix;
        // deeper objects concatenate their Matrix3D or their 2D matrix.
        // Inside 3D the 2D matrices are identity.
        let outer = self.world;
        let (m, world) = match (three_d::matrix(o), outer) {
            (Some(m3), None) => (Affine::default(), Some(three_d::multiply(&three_d::from_affine(parent_m), &m3))),
            (Some(m3), Some(w)) => (Affine::default(), Some(three_d::multiply(&w, &m3))),
            (None, Some(w)) => (Affine::default(), Some(three_d::multiply(&w, &three_d::from_affine(&own)))),
            (None, None) => (own.then(parent_m), None),
        };
        self.world = world;
        if c.mul[3] <= 0.0 && c.add[3] <= 0.0 {
            self.world = outer;
            return;
        }
        if let Some(g) = graphics {
            if let Some(state) = super::graphics::state(&g) {
                let shape = state.shape();
                let script = state.bitmap_chars.clone();
                drop(state);
                self.shape(&shape, 0, &m, &c, blend, &script);
            }
        }
        match (kind, character) {
            (DisplayKind::Shape, Some(ch)) => {
                let file = player::state(self.vm).borrow().file(ch.library);
                if let Ok(Some(shape)) = file.shape(ch.id as usize) {
                    self.shape(&shape, ch.library, &m, &c, blend, &[]);
                }
            }
            (DisplayKind::Bitmap, _) => {
                // A placed bitmap draws its character record (DO +0xa8);
                // script bitmaps draw their BitmapData's character.
                let data = o.host::<super::Native>().and_then(|n| n.target_value.clone());
                let smooth = o.host::<super::Native>().is_some_and(|n| n.flag);
                let from_data = data.as_ref().and_then(|v| v.as_object()?.host::<super::Native>()?.bitmap_data.clone()).and_then(|s| s.character);
                if let Some(ch) = character.or(from_data) {
                    {
                        let file = player::state(self.vm).borrow().file(ch.library);
                        if let Ok(Some(b)) = file.bitmap(ch.id as usize) {
                            self.bitmap(ch, b.width as f32, b.height as f32, b.uv, &m, &c, smooth, blend);
                        }
                    }
                }
            }
            (DisplayKind::Text, _) => self.text(o, &m, &c, blend),
            _ => {}
        }
        // Objects used as masks are not drawn themselves.
        let child_masks: Vec<Obj> = children(o).iter().filter_map(|k| display_ref(k).and_then(|d| d.mask.clone())).collect();
        let mut all = masks.to_vec();
        all.extend(child_masks);
        for child in children(o) {
            self.node(&child, &m, &c, &all);
        }
        self.world = outer;
    }
}

/// Build the draw list for the stage.
pub fn draw_list(vm: &Vm) -> DrawList {
    build(vm, false)
}

/// Diagnostic draw list that ignores `visible` (inspection of hidden screens).
pub fn debug_draw_all(vm: &Vm) -> DrawList {
    build(vm, true)
}

fn build(vm: &Vm, debug_ignore_visibility: bool) -> DrawList {
    let (stage, size, projection) = {
        let shared = player::state(vm);
        let p = shared.borrow();
        (p.stage.clone(), p.stage_size, p.stage_projection)
    };
    // `FUN_14103a4f0`: the Stage projection, rebuilt every draw.
    let projection = three_d::Projection::stage(size, projection.0, projection.1);
    let mut b = Builder {
        vm,
        debug_ignore_visibility,
        out: DrawList { stage: (size.0 as f32, size.1 as f32), ..DrawList::default() },
        ramps: HashMap::new(),
        world: None,
        projection,
    };
    if let Some(stage) = stage {
        b.node(&stage, &Affine::default(), &Colour::default(), &[]);
    }
    b.out
}
