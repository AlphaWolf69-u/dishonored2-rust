//! Display-list traversal into GPU-ready triangle batches (Iggy's GDraw
//! input). Fill texgen follows `FUN_141089760`/`FUN_141088fa0`/`FUN_141088c80`.
//!
//! Masks and clip layers follow `FUN_141037100`: stencil bits per mask slot,
//! carried on each batch as GDraw's stencil test and set bits.
//!
//! Not yet drawn: filters, cacheAsBitmap, non-normal blend modes beyond the
//! batch flag.
use super::three_d;
use super::avm2::object::Obj;
use super::avm2::Vm;
use super::display::{children, concatenated, display_ref, DisplayKind};
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
    /// GDraw stencil state: the bits that must be set where this draws (the
    /// finished masks), and the bits it writes instead of colour (a mask
    /// being drawn). Both 0: no stencil.
    pub stencil_test: u8,
    pub stencil_set: u8,
    /// Non-zero: a full-target draw that clears these stencil bits (GDraw
    /// slot `+0x28`), not colour.
    pub stencil_clear: u8,
}

/// Mask slots: GDraw's stencil bits (`FUN_14109a670` resets eight).
const MASK_SLOTS: usize = 8;
/// A free slot whose stencil bit is clean (`-1`) or still set (`-2`).
const FREE: i16 = -1;
const DIRTY: i16 = -2;

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

type Triangles = std::rc::Rc<Vec<[f32; 2]>>;

/// What draw lists of one player reuse from frame to frame: parsed shape
/// characters, and the triangles of each shape fill and glyph layer at the
/// exact scale they were tessellated for (so they are the same triangles a
/// fresh tessellation gives). Script `Graphics` shapes change and are not
/// kept.
#[derive(Default)]
pub struct Cache {
    shapes: HashMap<(usize, u16), Option<std::rc::Rc<Shape>>>,
    /// (library, character, layer, fill, scale bits).
    fills: HashMap<(usize, u16, usize, usize, u32), Triangles>,
    /// (font, glyph, layer, scale bits); the font is held so its address
    /// stays its identity.
    glyphs: HashMap<(usize, usize, usize, u32), (std::rc::Rc<crate::assets::iggy::font::Font>, Triangles)>,
}

/// Entries kept before the cache starts over (animated scales add new ones).
const CACHE_LIMIT: usize = 50_000;

impl Cache {
    fn trim(&mut self) {
        if self.fills.len() > CACHE_LIMIT {
            self.fills.clear();
        }
        if self.glyphs.len() > CACHE_LIMIT {
            self.glyphs.clear();
        }
    }
}

struct Builder<'a> {
    vm: &'a Vm,
    cache: &'a mut Cache,
    /// Per slot: the clip depth it lasts to (`P+0x2290`; FREE / DIRTY) and
    /// the parent whose clip layer claimed it (`P+0x2250`; none for a
    /// script mask).
    slots: [(i16, Option<Obj>); MASK_SLOTS],
    /// Level bytes `P+0x1f94..0x1f97`: allocated, test, set, dirty bits.
    allocated: u8,
    test: u8,
    set: u8,
    dirty: u8,
    /// Objects that are another object's script mask (`DO+0x70`).
    script_masks: Vec<Obj>,
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
        let (stencil_test, stencil_set) = (self.test, self.set);
        if let Some(last) = self.out.batches.last_mut() {
            if last.texture == texture
                && last.repeat == repeat
                && last.smooth == smooth
                && last.blend == blend
                && last.first + last.count == first
                && last.stencil_test == stencil_test
                && last.stencil_set == stencil_set
                && last.stencil_clear == 0
            {
                last.count += count;
                return;
            }
        }
        self.out.batches.push(Batch { texture, repeat, smooth, blend, first, count, stencil_test, stencil_set, stencil_clear: 0 });
    }

    /// `FUN_14109a670`: clear every dirty bit (GDraw `+0x28`); dirty slots
    /// become clean.
    fn clear_dirty(&mut self) {
        push_clear(&mut self.out, self.dirty);
        for slot in &mut self.slots {
            if slot.0 == DIRTY {
                slot.0 = FREE;
            }
        }
        self.dirty = 0;
    }

    /// `FUN_141036460`: the first clean slot, else the last dirty one after
    /// clearing the dirty bits. Claiming sets its allocated and set bits.
    fn allocate(&mut self, owner: Option<Obj>, limit: i16) -> Option<usize> {
        let slot = match self.slots.iter().position(|s| s.0 == FREE) {
            Some(i) => i,
            None => {
                let i = self.slots.iter().rposition(|s| s.0 == DIRTY);
                if i.is_some() {
                    self.clear_dirty();
                }
                match i {
                    Some(i) => i,
                    None => {
                        player::state(self.vm).borrow_mut().uncaught.push("Iggy error 0x6a: Too many masks are nested inside each other".into());
                        return None;
                    }
                }
            }
        };
        self.slots[slot] = (limit, owner);
        self.allocated |= 1 << slot;
        self.set |= 1 << slot;
        Some(slot)
    }

    /// `FUN_14103b5b0`: the mask is drawn; later draws test its bit.
    fn finish_mask(&mut self, slot: usize) {
        self.set &= !(1 << slot);
        self.test |= 1 << slot;
    }

    /// Release a slot: its bit stays set in the stencil until cleared.
    fn release(&mut self, slot: usize) {
        self.slots[slot] = (DIRTY, None);
        self.dirty |= 1 << slot;
        self.test &= !(1 << slot);
        self.allocated &= !(1 << slot);
    }

    /// Release the slots `owner`'s clip layers claimed whose clip depth is
    /// below `depth` (all of them for `None`).
    fn release_owned(&mut self, owner: &Obj, depth: Option<i32>) {
        for i in 0..MASK_SLOTS {
            let (limit, ref slot_owner) = self.slots[i];
            if limit >= 0 && slot_owner.as_ref() == Some(owner) && depth.is_none_or(|d| (limit as i32) < d) {
                self.release(i);
            }
        }
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
    /// `character` is the shape's character when it is one (its fills'
    /// triangles are cached).
    fn shape(&mut self, shape: &Shape, library: usize, character: Option<u16>, m: &Affine, c: &Colour, blend: u8, script: &[(usize, u16, CharRef)]) {
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
                let tris: Triangles = match character {
                    Some(id) => self
                        .cache
                        .fills
                        .entry((library, id, layer_index, i, scale.to_bits()))
                        .or_insert_with(|| std::rc::Rc::new(triangulate(&fill_edges(layer, i as u16 + 1, scale))))
                        .clone(),
                    None => std::rc::Rc::new(triangulate(&fill_edges(layer, i as u16 + 1, scale))),
                };
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
                    .iter()
                    .map(|&p| {
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
        // `FUN_14102e950`: text filters are emulated by drawing the glyphs
        // again, before the text, around the drop shadow's offset
        // ((cos, sin)(angle) × distance in stage pixels).
        let filters = display_ref(o).filter(|d| d.filters.is_empty()).map(|d| d.timeline_filters.clone()).unwrap_or_default();
        for filter in &filters {
            if let crate::assets::iggy::timeline::Filter::DropShadow { colour, blur, angle, distance, strength, .. } = filter {
                let base = ((angle.cos() * distance) as f64, (angle.sin() * distance) as f64);
                for (offset, alpha) in shadow_copies(*blur, *strength, colour[3]) {
                    let rgba = [colour[0] as f32 / 255.0, colour[1] as f32 / 255.0, colour[2] as f32 / 255.0, alpha as f32 / 255.0];
                    self.glyphs(&glyphs, m, c, blend, (base.0 + offset.0 as f64, base.1 + offset.1 as f64), Some(rgba));
                }
            }
        }
        self.glyphs(&glyphs, m, c, blend, (0.0, 0.0), None);
    }

    /// `FUN_14102dcd0`: the glyphs in their colours, or all in `colour`
    /// (a filter pass), moved by `offset` stage pixels.
    fn glyphs(&mut self, glyphs: &[super::text_engine::PlacedGlyph], m: &Affine, c: &Colour, blend: u8, offset: (f64, f64), colour: Option<[f32; 4]>) {
        for glyph in glyphs {
            let colour = colour.unwrap_or([((glyph.colour >> 16) & 0xff) as f32 / 255.0, ((glyph.colour >> 8) & 0xff) as f32 / 255.0, (glyph.colour & 0xff) as f32 / 255.0, 1.0]);
            let mut gm = Affine { a: glyph.size as f64, b: 0.0, c: 0.0, d: glyph.size as f64, tx: glyph.x as f64, ty: glyph.y as f64 }.then(m);
            gm.tx += offset.0;
            gm.ty += offset.1;
            let scale = ((gm.a * gm.a + gm.b * gm.b).sqrt()) as f32;
            for (li, layer) in glyph.font.glyphs[glyph.glyph].layers.iter().enumerate() {
                // Glyph paths reference fill 1 and carry no fill list.
                let key = (std::rc::Rc::as_ptr(&glyph.font) as usize, glyph.glyph, li, scale.to_bits());
                let font = glyph.font.clone();
                let tris = self.cache.glyphs.entry(key).or_insert_with(|| (font, std::rc::Rc::new(triangulate(&fill_edges(layer, 1, scale))))).1.clone();
                let mul = std::array::from_fn(|k| colour[k] * c.mul[k] as f32);
                let add = c.add.map(|v| v as f32 / 255.0);
                let vertices: Vec<Vertex> = tris
                    .iter()
                    .map(|&p| {
                        let (x, y) = gm.apply(p[0] as f64, p[1] as f64);
                        Vertex { position: [x as f32, y as f32], uv: [0.0; 2], mul, add, params: [0.0; 2], w: 1.0 }
                    })
                    .collect();
                self.push(Texture::White, false, true, blend, vertices);
            }
        }
    }

    /// `FUN_141037930` for a child: its matrix and colour from the parent's.
    fn node(&mut self, o: &Obj, parent_m: &Affine, parent_c: &Colour) {
        let Some(d) = display_ref(o) else { return };
        let own = d.matrix;
        let c = d.colour.then(parent_c);
        drop(d);
        // The first object with a Matrix3D starts the 3D world from its
        // parent's 2D world and ignores its own 2D matrix; deeper objects
        // concatenate their Matrix3D or their 2D matrix. Inside 3D the 2D
        // matrices are identity.
        let outer = self.world;
        let (m, world) = match (three_d::matrix(o), outer) {
            (Some(m3), None) => (Affine::default(), Some(three_d::multiply(&three_d::from_affine(parent_m), &m3))),
            (Some(m3), Some(w)) => (Affine::default(), Some(three_d::multiply(&w, &m3))),
            (None, Some(w)) => (Affine::default(), Some(three_d::multiply(&w, &three_d::from_affine(&own)))),
            (None, None) => (own.then(parent_m), None),
        };
        self.world = world;
        self.draw(o, &m, &c);
        self.world = outer;
    }

    /// Draw `o` at `m` with colour `c`, then its children.
    fn draw(&mut self, o: &Obj, m: &Affine, c: &Colour) {
        if c.mul[3] <= 0.0 && c.add[3] <= 0.0 {
            return;
        }
        let Some(d) = display_ref(o) else { return };
        let (kind, character, graphics, blend) = (d.kind, d.character, d.graphics.clone(), d.blend);
        drop(d);
        let (m, c) = (*m, *c);
        if let Some(g) = graphics {
            if let Some(state) = super::graphics::state(&g) {
                let shape = state.shape();
                let script = state.bitmap_chars.clone();
                drop(state);
                self.shape(&shape, 0, None, &m, &c, blend, &script);
            }
        }
        match (kind, character) {
            (DisplayKind::Shape, Some(ch)) => {
                let vm = self.vm;
                let shape = self
                    .cache
                    .shapes
                    .entry((ch.library, ch.id))
                    .or_insert_with(|| player::state(vm).borrow().file(ch.library).shape(ch.id as usize).ok().flatten().map(std::rc::Rc::new))
                    .clone();
                if let Some(shape) = shape {
                    self.shape(&shape, ch.library, Some(ch.id), &m, &c, blend, &[]);
                }
            }
            (DisplayKind::Bitmap, _) => {
                // A placed bitmap draws its character record (DO +0xa8);
                // script bitmaps draw their BitmapData's character.
                let data = o.host::<super::Native>().and_then(|n| n.target_value.clone());
                let smooth = o.host::<super::Native>().is_some_and(|n| n.flag);
                let from_data = data.as_ref().and_then(|v| v.as_object()?.host::<super::Native>()?.bitmap_data.clone()).and_then(|s| s.character);
                if let Some(ch) = character.or(from_data) {
                    let file = player::state(self.vm).borrow().file(ch.library);
                    if let Ok(Some(b)) = file.bitmap(ch.id as usize) {
                        self.bitmap(ch, b.width as f32, b.height as f32, b.uv, &m, &c, smooth, blend);
                    }
                }
            }
            (DisplayKind::Text, _) => self.text(o, &m, &c, blend),
            _ => {}
        }
        self.children(o, &m, &c);
    }

    /// `FUN_141037100`: the children in display order with their clip
    /// layers and script masks.
    fn children(&mut self, o: &Obj, m: &Affine, c: &Colour) {
        let mut depth = -1;
        for child in children(o) {
            let Some(d) = display_ref(&child) else { continue };
            let (visible, clip_depth, child_depth, mask) = (d.visible, d.clip_depth, d.depth, d.mask.clone());
            let has_content = !d.children.is_empty() || d.graphics.is_some() || matches!(d.kind, DisplayKind::Shape | DisplayKind::Bitmap);
            drop(d);
            // The depth index (DO+0xc4): the last placed depth.
            if let Some(cd) = child_depth.filter(|&cd| cd >= 0) {
                depth = cd;
            }
            // Clip layers of this parent end below the current depth.
            self.release_owned(o, Some(depth));
            let clip_layer = clip_depth > 0;
            // Drawn when visible or a clip layer, unless it is another
            // object's script mask.
            if !(visible || self.debug_ignore_visibility || clip_layer) || self.script_masks.contains(&child) {
                continue;
            }
            // A clip layer is drawn into a new slot, which then masks the
            // siblings up to its clip depth.
            let clip_slot = if clip_layer && depth < clip_depth as i32 && has_content { self.allocate(Some(o.clone()), clip_depth as i16) } else { None };
            // A script mask is drawn at its own world (its matrices up to the
            // Stage) into a slot that lasts for this child only.
            let mask_slot = match mask {
                Some(mask) => {
                    let slot = self.allocate(None, 0x7fff);
                    if let Some(slot) = slot {
                        let outer = self.world.take();
                        self.draw(&mask, &concatenated(&mask), &Colour::default());
                        self.world = outer;
                        self.finish_mask(slot);
                    }
                    slot
                }
                None => None,
            };
            self.node(&child, m, c);
            if let Some(slot) = clip_slot {
                self.finish_mask(slot);
            }
            if let Some(slot) = mask_slot {
                self.release(slot);
            }
        }
        self.release_owned(o, None);
    }
}

/// The copies `FUN_14102e950` draws for a drop shadow: (offset from the
/// shadow position, alpha byte). Both blurs at most 2 (`DAT_141ab3b20`): one
/// copy in the filter's alpha. Otherwise three rings around the shadow
/// position, b = max(blurX, blurY): counts (1.5b, b, 0.5b) capped at 30, 20
/// and 10; radii r = 0.5b + strength, r - r/3, r - 2r/3; copy j of a ring
/// of n at angle j·2π/n, offset (sin, cos) × radius; each copy's alpha
/// A / (n · k · {3.5, 2.5, 1.3}) with k = min(1/strength, 1), truncated to a
/// byte.
pub fn shadow_copies(blur: [f32; 2], strength: f32, alpha: u8) -> Vec<((f32, f32), u8)> {
    if blur[0] <= 2.0 && blur[1] <= 2.0 {
        return vec![((0.0, 0.0), alpha)];
    }
    let b = blur[0].max(blur[1]);
    let counts = [((b * 1.5) as i64 as u8).min(30), (b as i64 as u8).min(20), ((b * 0.5) as i64 as u8).min(10)];
    let k = (1.0 / strength).clamp(0.0, 1.0);
    let r = b * 0.5 + strength;
    let radii = [r, r - r * (1.0 / 3.0), r - r * 2.0 * (1.0 / 3.0)];
    let divisors = [3.5f32, 2.5, 1.3];
    let mut out = Vec::new();
    for ((&n, &radius), &divisor) in counts.iter().zip(&radii).zip(&divisors) {
        if n == 0 {
            continue;
        }
        let a = (alpha as f32 / (n as f32 * k * divisor)) as i64 as u8;
        let step = std::f32::consts::TAU / n as f32;
        for j in 0..n {
            let angle = j as f32 * step;
            out.push(((angle.sin() * radius, angle.cos() * radius), a));
        }
    }
    out
}

/// A full-target stencil clear of `bits` (GDraw `+0x28` / the render start's
/// `ClearDepthStencilView`): a quad over the stage, drawn with no colour.
fn push_clear(out: &mut DrawList, bits: u8) {
    if bits == 0 {
        return;
    }
    let (w, h) = out.stage;
    let first = out.vertices.len() as u32;
    let corner = |x: f32, y: f32| Vertex { position: [x, y], uv: [0.0; 2], mul: [1.0; 4], add: [0.0; 4], params: [0.0; 2], w: 1.0 };
    // Generous: the stage may be letterboxed inside the target.
    let (x0, y0, x1, y1) = (-w, -h, 2.0 * w, 2.0 * h);
    out.vertices.extend([corner(x0, y0), corner(x1, y0), corner(x1, y1), corner(x0, y0), corner(x1, y1), corner(x0, y1)]);
    out.batches.push(Batch { texture: Texture::White, repeat: false, smooth: true, blend: 1, first, count: 6, stencil_test: 0, stencil_set: 0, stencil_clear: bits });
}

/// Every object some display object uses as its script mask (`DO+0x70`).
fn script_masks(o: &Obj, out: &mut Vec<Obj>) {
    for child in children(o) {
        if let Some(mask) = display_ref(&child).and_then(|d| d.mask.clone()) {
            out.push(mask);
        }
        script_masks(&child, out);
    }
}

/// Build the draw list for the stage.
pub fn draw_list(vm: &Vm) -> DrawList {
    build(vm, &mut Cache::default(), false)
}

/// The draw list reusing `cache` (one cache per player, kept across frames).
pub fn draw_list_cached(vm: &Vm, cache: &mut Cache) -> DrawList {
    build(vm, cache, false)
}

/// Diagnostic draw list that ignores `visible` (inspection of hidden screens).
pub fn debug_draw_all(vm: &Vm) -> DrawList {
    build(vm, &mut Cache::default(), true)
}

fn build(vm: &Vm, cache: &mut Cache, debug_ignore_visibility: bool) -> DrawList {
    cache.trim();
    let (stage, size, projection) = {
        let shared = player::state(vm);
        let p = shared.borrow();
        (p.stage.clone(), p.stage_size, p.stage_projection)
    };
    // `FUN_14103a4f0`: the Stage projection, rebuilt every draw.
    let projection = three_d::Projection::stage(size, projection.0, projection.1);
    let mut masks = Vec::new();
    if let Some(stage) = &stage {
        script_masks(stage, &mut masks);
    }
    let mut b = Builder {
        vm,
        cache,
        slots: std::array::from_fn(|_| (FREE, None)),
        allocated: 0,
        test: 0,
        set: 0,
        dirty: 0,
        script_masks: masks,
        debug_ignore_visibility,
        out: DrawList { stage: (size.0 as f32, size.1 as f32), ..DrawList::default() },
        ramps: HashMap::new(),
        world: None,
        projection,
    };
    if let Some(stage) = stage {
        b.node(&stage, &Affine::default(), &Colour::default());
    }
    b.out
}

/// Library indices of a movie drawn above the first one are shifted by
/// this much, so both movies' bitmaps share one image map.
pub const LAYER_LIBRARY_OFFSET: usize = 1000;

/// Append `top` (a movie on a higher layer, e.g. `global.iggy` at layer 3
/// over the pause movie at 2) to `base`.
pub fn stack(base: &mut DrawList, mut top: DrawList) {
    // Each player's render starts with a cleared stencil (GDraw's render
    // begin, `FUN_1402e5b80`).
    if base.batches.iter().any(|b| b.stencil_set != 0) {
        push_clear(base, 0xff);
    }
    let first = base.vertices.len() as u32;
    let ramps = base.ramps.len();
    for b in &mut top.batches {
        b.first += first;
        match &mut b.texture {
            Texture::Bitmap(c) => c.library += LAYER_LIBRARY_OFFSET,
            Texture::Ramp(i) => *i += ramps,
            Texture::White => {}
        }
    }
    base.vertices.append(&mut top.vertices);
    base.batches.append(&mut top.batches);
    base.ramps.append(&mut top.ramps);
}

#[cfg(test)]
mod shadow_tests {
    #[test]
    fn small_blurs_are_one_copy_and_large_ones_rings() {
        assert_eq!(super::shadow_copies([0.0, 0.0], 1.0, 255), vec![((0.0, 0.0), 255)]);
        let copies = super::shadow_copies([4.0, 2.0], 1.0, 255);
        // b = 4: 6 + 4 + 2 copies; ring alphas 255/21, 255/10, 255/2.6.
        assert_eq!(copies.len(), 12);
        assert_eq!((copies[0].1, copies[6].1, copies[10].1), (12, 25, 98));
        // r = 3: the first copy of the first ring is straight down (sin 0, cos 0).
        assert_eq!(copies[0].0, (0.0, 3.0));
    }
}
