//! Text metrics for TextField queries.
//!
//! `approximate_measure` lays text out with the embedded font's advances,
//! kerning and em metrics. Iggy's relayout (`FUN_141030f60`, HTML parsing in
//! `FUN_1410303d0`) is not traced yet, so line breaking, letter spacing and
//! autosize rounding may differ from the game until it is.
use super::avm2::object::Obj;
use super::avm2::value::Value;
use super::avm2::Vm;
use super::player;
use super::text::text_state;
use crate::assets::iggy::font::Font;
use std::rc::Rc;

/// Gutter Flash and Iggy add around TextField text (2 px each side).
const GUTTER: f64 = 2.0;

/// Find an embedded font by name across loaded libraries.
pub fn find_font(vm: &Vm, name: &str) -> Option<Rc<Font>> {
    let shared = player::state(vm);
    let p = shared.borrow();
    p.fonts.iter().find(|f| f.name == name).cloned()
}

/// `Font.fontName` of a library font class instance (SymbolClass binding).
pub fn font_name_of_class(vm: &Vm, this: &Value) -> Option<String> {
    let class = this.as_object()?.class()?;
    let qualified = class.name.qualified().replace("::", ".");
    let shared = player::state(vm);
    let p = shared.borrow();
    let c = p.character_for_class(&qualified)?;
    p.file(c.library).font(c.id as usize).ok().flatten().map(|f| f.name)
}

/// (text width, text height, line count) in pixels.
pub fn approximate_measure(vm: &Vm, o: &Obj) -> (f64, f64, usize) {
    let Some(t) = text_state(o).map(|t| t.clone()) else { return (0.0, 0.0, 0) };
    let Some(font) = find_font(vm, &t.format.font) else { return (0.0, 0.0, 0) };
    let size = t.format.size;
    let max_width = (t.rect[2] - t.rect[0]) - 2.0 * GUTTER - t.format.left_margin - t.format.right_margin;
    let mut lines: Vec<f64> = vec![0.0];
    let mut prev: Option<u16> = None;
    for c in t.text.encode_utf16() {
        if c == b'\r' as u16 || c == b'\n' as u16 {
            lines.push(0.0);
            prev = None;
            continue;
        }
        let advance = font.glyph(c).map_or(0.0, |g| font.advances[g] as f64) * size
            + prev.map_or(0.0, |p| if t.format.kerning { font.kerning(p, c) as f64 * size } else { 0.0 })
            + t.format.letter_spacing;
        let line = lines.last_mut().unwrap();
        if t.word_wrap && *line + advance > max_width && *line > 0.0 {
            lines.push(advance);
        } else {
            *line += advance;
        }
        prev = Some(c);
    }
    let line_height = (font.ascent + font.descent) as f64 * size + t.format.leading;
    let width = lines.iter().cloned().fold(0.0, f64::max);
    let height = line_height * lines.len() as f64 - t.format.leading;
    (width, height, lines.len())
}
