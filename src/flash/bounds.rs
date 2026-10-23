//! Display-object bounds (`getBounds`, `width`, `height`).
use super::avm2::object::Obj;
use super::avm2::Vm;
use super::display::{children, concatenated, display_ref, DisplayKind};
use super::geom::Affine;
use super::player;

pub const EMPTY: [f64; 4] = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];

fn include(acc: &mut [f64; 4], m: &Affine, r: [f64; 4]) {
    if r[0] > r[2] || r[1] > r[3] {
        return;
    }
    for (x, y) in [(r[0], r[1]), (r[2], r[1]), (r[0], r[3]), (r[2], r[3])] {
        let (px, py) = m.apply(x, y);
        acc[0] = acc[0].min(px);
        acc[1] = acc[1].min(py);
        acc[2] = acc[2].max(px);
        acc[3] = acc[3].max(py);
    }
}

/// Own content bounds in local space (excluding children).
pub fn content(vm: &Vm, o: &Obj) -> [f64; 4] {
    let mut acc = EMPTY;
    let id = Affine::default();
    let (kind, character, graphics) = match display_ref(o) {
        Some(d) => (d.kind, d.character, d.graphics.clone()),
        None => return acc,
    };
    if kind == DisplayKind::Text {
        if let Some(r) = super::text::field_bounds(o) {
            include(&mut acc, &id, r);
        }
    } else if kind == DisplayKind::Bitmap && character.is_none() {
        // A placed bitmap measures its character record (DO +0xa8), as it
        // draws; a script Bitmap measures its BitmapData.
        if let Some((w, h)) = super::bitmap::bitmap_size(o) {
            include(&mut acc, &id, [0.0, 0.0, w, h]);
        }
    } else if let Some(c) = character {
        let file = player::state(vm).borrow().file(c.library);
        if let Ok(Some(b)) = file.character_bounds(c.id as usize) {
            if !matches!(file.character_kind(c.id as usize), Ok(Some(crate::assets::iggy::timeline::kind::SPRITE))) {
                include(&mut acc, &id, b.map(|v| v as f64));
            }
        }
    }
    if let Some(g) = graphics {
        if let Some(r) = super::graphics::bounds(&g) {
            include(&mut acc, &id, r);
        }
    }
    acc
}

fn accumulate(vm: &Vm, o: &Obj, m: &Affine, acc: &mut [f64; 4]) {
    include(acc, m, content(vm, o));
    for c in children(o) {
        let cm = display_ref(&c).map(|d| d.matrix).unwrap_or_default();
        accumulate(vm, &c, &cm.then(m), acc);
    }
}

/// Bounds of `o` and its descendants in `space` (local space when `None`).
/// Empty content reports a zero rectangle at the origin, as Flash does.
pub fn bounds(vm: &Vm, o: &Obj, space: Option<&Obj>) -> [f64; 4] {
    let m = match space {
        None => Affine::default(),
        Some(s) if s == o => Affine::default(),
        Some(s) => concatenated(o).then(&concatenated(s).invert()),
    };
    let mut acc = EMPTY;
    accumulate(vm, o, &m, &mut acc);
    if acc[0] > acc[2] {
        let (x, y) = m.apply(0.0, 0.0);
        return [x, y, x, y];
    }
    acc
}
