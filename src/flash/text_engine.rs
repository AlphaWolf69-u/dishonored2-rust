//! Iggy's TextField engine (swf_draw_text.c / swf_as3_textfield.c): the
//! text object, run formats, font resolution, the HTML parser, line layout,
//! relayout and the glyph draw pass. Offsets refer to the 0x2c8-byte text
//! object (`FUN_141023c20`) and the 0x58-byte format block; the research is
//! `artifacts/ui-pause/notes-text-engine.md`.
use crate::assets::iggy::font::Font;
use std::rc::Rc;

/// `DAT_142422ef8 == 1`: `<p>` never opens a newline, `</p>` at depth 0 queues one.
const PARAGRAPH_MODE: bool = true;

/// The 0x58-byte format block.
#[derive(Clone, Debug, Default)]
pub struct Format {
    /// +0x00: resolved font (`FUN_14102cf90`).
    pub font: Option<Rc<Font>>,
    /// +0x08: font name.
    pub name: Option<String>,
    pub size: f32,
    pub block_indent: f32,
    pub indent: f32,
    pub leading: f32,
    pub left_margin: f32,
    pub right_margin: f32,
    pub letter_spacing: f32,
    /// 0xRRGGBB.
    pub colour: u32,
    /// +0x30/+0x32: up to 16 stops in pixels.
    pub tab_stops: Vec<u16>,
    /// 0 left, 1 right, 2 center, 3 justify.
    pub align: u8,
    pub bold: bool,
    pub bullet: bool,
    pub italic: bool,
    pub kerning: bool,
    pub underline: bool,
}

/// A run node (0x70): its format and UTF-16 characters.
#[derive(Clone, Debug)]
pub struct Run {
    pub format: Format,
    pub chars: Vec<u16>,
}

/// Fonts a player can resolve, in registry order (this movie's fonts and
/// every loaded library's, `FUN_14102af10`/`FUN_1410287c0`), and the alias
/// pairs installed by `FUN_14102b6e0` (`IggyFontSetIndirect`).
#[derive(Default)]
pub struct Fonts {
    pub fonts: Vec<Rc<Font>>,
    /// (source name, target name); flags -1 match any style.
    pub aliases: Vec<(String, String)>,
}

impl Fonts {
    /// `FUN_14102cf90(player, name, italic, bold)`.
    pub fn resolve(&self, name: &str, italic: bool, bold: bool) -> Option<Rc<Font>> {
        let name = self.aliases.iter().find(|(s, _)| s == name).map_or(name, |(_, t)| t.as_str());
        let name = if name == "_sans" { "Arial" } else { name };
        // Embedded fonts (provider+0x18 == 1) get no installed-font bonus.
        let score = |f: &Font| {
            if f.codes.is_empty() {
                0u32
            } else {
                f.codes.len() as u32 + 0x300_0000 * (bold == f.bold) as u32 + 0x200_0000 * (italic == f.italic) as u32
            }
        };
        let mut best: Option<&Rc<Font>> = None;
        for f in self.fonts.iter().filter(|f| f.name == name) {
            if best.is_none_or(|b| score(f) > score(b)) {
                best = Some(f);
            }
        }
        best.cloned()
    }

    /// `FUN_140031140`: aliases for the UI library language id (Chinese 9,
    /// Japanese 10); other languages install none.
    pub fn language_aliases(language: u32) -> Vec<(String, String)> {
        let pairs: &[(&str, &str)] = match language {
            9 => &[
                ("dis2_dearjoe5 Regular", "dis2_nexa_slab_obl Regular"),
                ("dis2_justlefthand Regular", "dis2_nexa_slab_obl Regular"),
                ("dis2_newage_got Regular", "dis2_nexa_slab_obl Regular"),
                ("dis2_newage_got_sc Regular", "dis2_nexa_slab_obl Regular"),
                ("dis2_bolyar_ornate Regular", "dis2_nexa_slab_obl Regular"),
            ],
            10 => &[
                ("skyrimhd_gyokai_w5 Regular", "dis2_gyokai_w5 Regular"),
                ("skyrimhd_heigos_w5 Regular", "skyrimhd_heigos_w5_1 Regular"),
                ("skyrimhd_heigos_w5_0 Regular", "skyrimhd_heigos_w5_1 Regular"),
            ],
            _ => &[],
        };
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }
}

/// Font metrics in em (`FUN_14102ba20`): ascent, descent, leading, tab
/// unit (first-path width of 'g', else ' ', else 1.0), union ymax.
fn metrics(font: &Font) -> [f32; 5] {
    let (ascent, descent, leading) = if font.has_layout {
        (font.ascent, font.descent, font.leading)
    } else {
        (-font.union_ymin, font.union_ymax, 0.0)
    };
    let unit = [b'g' as u16, b' ' as u16]
        .iter()
        .map(|&c| glyph_index(font, c))
        .find(|&g| g >= 1)
        .and_then(|g| font.glyphs.get(g as usize)?.layers.first().map(|l| l.bounds[2] - l.bounds[0]))
        .unwrap_or(1.0);
    [ascent, descent, leading, unit, font.union_ymax]
}

/// `FUN_14102c060` -> `FUN_14102b970`: 0xA0 maps to space; a miss is the
/// last glyph. -1 for a font without glyphs.
fn glyph_index(font: &Font, c: u16) -> i32 {
    match font.glyph(c) {
        Some(g) => g as i32,
        None => font.codes.len() as i32 - 1,
    }
}

/// `LAB_14102bb10`: first-path bounds and advance, em.
#[derive(Clone, Copy, Default)]
struct GlyphMetrics {
    xmin: f32,
    xmax: f32,
    ymax: f32,
    advance: f32,
    empty: bool,
}

fn glyph_metrics(font: &Font, g: i32) -> GlyphMetrics {
    let Some(shape) = font.glyphs.get(g as usize) else { return GlyphMetrics { empty: true, ..Default::default() } };
    let bounds = shape.layers.first().map_or([0.0; 4], |l| l.bounds);
    let advance = font.advances.get(g as usize).copied().unwrap_or(bounds[2]);
    GlyphMetrics { xmin: bounds[0], xmax: bounds[2], ymax: bounds[3], advance, empty: shape.layers.is_empty() }
}

/// `FUN_141031af0`.
pub fn is_whitespace(c: u16) -> bool {
    matches!(c, 0x20 | 0x09..=0x0d | 0x85 | 0x1680 | 0x180e | 0x2000..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000)
}

fn is_cjk(c: u16) -> bool {
    c.wrapping_sub(0x2e80) <= 0xd17e
}

/// Kinsoku mode 2 (`DAT_143ec08d4`), `FUN_14102d510`.
fn cannot_begin(c: u16) -> bool {
    matches!(c, 0x21 | 0x2c | 0x2e | 0x29 | 0x3a | 0x3b | 0x3f | 0x5d | 0x203c | 0xff5d)
        || matches!(
            c,
            0x3001 | 0x3002 | 0x3005 | 0x3009 | 0x300b | 0x300d | 0x300f | 0x301c | 0x30fb | 0x30fd | 0x3015 | 0x3011 | 0x3019
                | 0x3017 | 0x301f | 0x303b | 0x3041 | 0x3043 | 0x3045 | 0x3047 | 0x3049 | 0x3063 | 0x3083 | 0x3085 | 0x3087
                | 0x308e | 0x3095 | 0x3096 | 0x30a1 | 0x30a3 | 0x30a5 | 0x30a7 | 0x30a9 | 0x30c3 | 0x30e3 | 0x30e5 | 0x30e7
                | 0x30ee | 0x30f5 | 0x30f6 | 0x31f0..=0x31ff
        )
}

/// `FUN_14102d440`.
fn cannot_end(c: u16) -> bool {
    matches!(c, 0x28 | 0x5b | 0x203c | 0xff5b | 0x3008 | 0x300a | 0x300c | 0x300e | 0x3010 | 0x3014 | 0x3016 | 0x3018 | 0x301d)
}

/// Text-object bytes 0xdc..=0xe6, read and written by the TextField field
/// accessors (`0x1410b3970`/`0x1410b39b0`) at these offsets.
pub const ALWAYS_SHOW_SELECTION: u32 = 0xdc;
pub const BACKGROUND: u32 = 0xdd;
pub const BORDER: u32 = 0xde;
pub const CONDENSE_WHITE: u32 = 0xdf;
pub const PASSWORD: u32 = 0xe0;
pub const EMBED_FONTS: u32 = 0xe1;
pub const MOUSE_WHEEL: u32 = 0xe2;
pub const MULTILINE: u32 = 0xe3;
pub const SELECTABLE: u32 = 0xe4;
pub const RICH_CLIPBOARD: u32 = 0xe5;
pub const WORD_WRAP: u32 = 0xe6;

/// The text object (`DO+0x98`).
#[derive(Clone, Debug)]
pub struct TextObject {
    /// +0x08 default format.
    pub default: Format,
    /// +0x60 run list (text runs only; DH2's movies have no `<img>`).
    pub runs: Vec<Run>,
    /// +0x68: (top, bottom) per line.
    pub lines: Vec<(f32, f32)>,
    pub background_colour: u32,
    pub border_colour: u32,
    /// +0x7c declared width, +0x80 autosize anchor, +0x84 base x (never written).
    pub declared_width: f32,
    pub anchor: f32,
    pub base_x: f32,
    /// +0x88/+0x8c record bounds origin, +0x90/+0x94 autosize offsets.
    pub origin: [f32; 2],
    pub offset: [f32; 2],
    /// +0x98/+0x9c box size.
    pub width: f32,
    pub height: f32,
    /// +0xa0/+0xa4 raw text size.
    pub text_width: f32,
    pub text_height: f32,
    /// +0xa8 scrollH, +0xac scrollV-1, +0xb0 maxScrollV-1, +0xb4 numLines.
    pub scroll_h: i32,
    pub scroll_v: i32,
    pub max_scroll_v: i32,
    pub num_lines: i32,
    pub max_chars: i32,
    /// +0xd0 caret index.
    pub caret: i32,
    /// Bytes 0xdc..=0xe6 (see the offset constants).
    pub flags: [bool; 11],
    /// +0x100 = !multiline.
    pub single_line: bool,
    /// 0 NONE, 1 LEFT, 2 RIGHT, 3 CENTER.
    pub auto_size: u8,
    pub editable: bool,
    pub from_record: bool,
}

impl Default for TextObject {
    /// An AS-created field (`FUN_141023c20` without a record): 100×100,
    /// size 12, selectable.
    fn default() -> Self {
        TextObject {
            default: Format { size: 12.0, ..Format::default() },
            runs: Vec::new(),
            lines: Vec::new(),
            background_colour: 0xffffff,
            border_colour: 0,
            declared_width: 100.0,
            anchor: 0.0,
            base_x: 0.0,
            origin: [0.0; 2],
            offset: [0.0; 2],
            width: 100.0,
            height: 100.0,
            text_width: 0.0,
            text_height: 0.0,
            scroll_h: 0,
            scroll_v: 0,
            max_scroll_v: 0,
            num_lines: 0,
            max_chars: 0,
            caret: 0,
            flags: {
                let mut f = [false; 11];
                f[(SELECTABLE - 0xdc) as usize] = true;
                f
            },
            single_line: true,
            auto_size: 0,
            editable: false,
            from_record: false,
        }
    }
}

/// A glyph to draw, in text-object local space.
pub struct PlacedGlyph {
    pub font: Rc<Font>,
    pub glyph: usize,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub colour: u32,
}

/// The per-line result of `measure_line` (`FUN_141031f30`).
#[derive(Clone, Copy, Default)]
struct LineMeasure {
    width: f32,
    top: f32,
    descent: f32,
    breaks: i32,
    count: usize,
    glyphs: usize,
    leading: f32,
    left_offset: f32,
}

/// Iterator edges and mode (`&local_188` / `&local_2a8`).
#[derive(Clone, Copy)]
struct Edges {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    force_left: bool,
    skip: i32,
    scroll_h: i32,
}

/// A laid-out line: its characters, pen start, baseline and justify step.
#[derive(Clone, Copy)]
struct Line {
    start: usize,
    count: usize,
    x: f32,
    baseline: f32,
    justify: f32,
    visible: usize,
    top: f32,
    descent: f32,
    leading: f32,
    skipped: bool,
}

/// Flat characters with their run index.
struct Chars<'a> {
    chars: Vec<(u16, usize)>,
    runs: &'a [Run],
    run_starts: Vec<usize>,
}

impl<'a> Chars<'a> {
    fn new(runs: &'a [Run]) -> Self {
        let mut chars = Vec::new();
        let mut run_starts = Vec::new();
        for (r, run) in runs.iter().enumerate() {
            run_starts.push(chars.len());
            chars.extend(run.chars.iter().map(|&c| (c, r)));
        }
        Chars { chars, runs, run_starts }
    }
    fn format(&self, i: usize) -> &Format {
        &self.runs[self.chars[i].1].format
    }
}

impl TextObject {
    /// A field byte at `offset` (0xdc..=0xe6).
    pub fn flag(&self, offset: u32) -> Option<bool> {
        self.flags.get(offset.checked_sub(0xdc)? as usize).copied()
    }

    pub fn set_flag(&mut self, offset: u32, v: bool) {
        if let Some(slot) = offset.checked_sub(0xdc).and_then(|i| self.flags.get_mut(i as usize)) {
            *slot = v;
        }
        if offset == MULTILINE {
            self.single_line = !v;
        }
    }

    pub fn word_wrap(&self) -> bool {
        self.flags[(WORD_WRAP - 0xdc) as usize]
    }

    fn password(&self) -> bool {
        self.flags[(PASSWORD - 0xdc) as usize]
    }

    /// `FUN_14102c990` into +0xb0 (the height setter).
    pub fn update_max_scroll(&mut self) {
        self.max_scroll_v = self.max_scroll();
    }

    fn chars_at(&self, chars: &Chars, i: usize) -> u16 {
        if self.password() { b'*' as u16 } else { chars.chars[i].0 }
    }

    /// `FUN_14102d830`: the next tab stop after `x`.
    fn tab(&self, format: &Format, x: f32) -> f32 {
        if !format.tab_stops.is_empty() {
            return format.tab_stops.iter().map(|&t| t as f32).find(|&t| t > x).unwrap_or(x);
        }
        let unit = format.font.as_ref().map_or(8.0, |f| metrics(f)[3]);
        let step = unit * format.size * 4.0;
        ((x / step).floor() + 1.0) * step
    }

    /// `FUN_141032780`.
    fn word_fits(&self, chars: &Chars, start: usize, first: bool, avail: f32) -> bool {
        let mut w = 0.0f32;
        let mut prev: Option<u16> = None;
        for i in start..chars.chars.len() {
            let c = self.chars_at(chars, i);
            if let Some(p) = prev {
                if !cannot_end(p) && !cannot_begin(c) && is_cjk(c) {
                    return true;
                }
            }
            if is_whitespace(c) {
                return true;
            }
            let format = chars.format(i);
            if let Some(font) = &format.font {
                let g = glyph_index(font, c);
                if g >= 0 {
                    let m = glyph_metrics(font, g);
                    if !m.empty {
                        if first && i == start {
                            w -= format.size * m.xmin;
                        }
                        if format.size * m.xmax + w > avail {
                            return false;
                        }
                        w += format.size * m.advance;
                    }
                }
            }
            if !cannot_end(c) && (is_cjk(c) || c == b'-' as u16) {
                match chars.chars.get(i + 1).map(|_| self.chars_at(chars, i + 1)) {
                    None | Some(0) => return true,
                    Some(next) if !cannot_begin(next) => return true,
                    _ => {}
                }
            }
            prev = Some(c);
        }
        true
    }

    /// `FUN_141031f30`. Returns the measure and the updated char-break carry.
    fn measure_line(&self, chars: &Chars, start: usize, e: &Edges, carry: bool, max_right: &mut f32, min_left: &mut f32) -> (LineMeasure, bool) {
        let mut x = e.left - e.scroll_h as f32;
        let mut right_max = x;
        // The ink bottom is only read by clip mode 1, which no caller uses.
        let (mut top, mut desc) = (10000.0f32, -10000.0f32);
        let (mut count, mut glyphs, mut breaks) = (0usize, 0usize, 0i32);
        let mut last_was_break = false;
        let mut first = true;
        let mut overflow = false;
        let mut can_break_before = !carry;
        let mut char_break = carry;
        let mut left_off = 0.0f32;
        let mut leading = 0.0f32;
        let mut ended = true;
        let mut i = start;
        while i < chars.chars.len() {
            let c = self.chars_at(chars, i);
            let format = chars.format(i);
            let size = format.size;
            let cjk = is_cjk(c);
            if cjk {
                can_break_before = true;
            }
            if c == b'\n' as u16 || c == b'\r' as u16 {
                count += 1;
                breaks = 0;
                last_was_break = false;
                match &format.font {
                    Some(f) => {
                        let m = metrics(f);
                        top = top.min(-size * m[0]);
                        desc = desc.max(size * m[1]);
                    }
                    None => top = top.max(6.0),
                }
                ended = false;
                break;
            }
            let Some(font) = &format.font else {
                count += 1;
                i += 1;
                continue;
            };
            let fm = metrics(font);
            let (glyph, right_ink, nx, gm) = if c == b'\t' as u16 {
                let nx = self.tab(format, x);
                (false, nx, nx, GlyphMetrics::default())
            } else {
                let g = glyph_index(font, c);
                if g < 0 {
                    i += 1;
                    continue;
                }
                let m = glyph_metrics(font, g);
                (true, x + size * m.xmax, x + size * m.advance + format.letter_spacing, m)
            };
            let (glyph_top, glyph_desc) = (-size * fm[0], size * fm[1]);
            if !self.word_wrap() {
                if x > e.right {
                    overflow = true;
                }
            } else if !char_break {
                if can_break_before && !self.word_fits(chars, i, first, e.right - x) {
                    if !first {
                        ended = false;
                        break;
                    }
                    if !self.word_fits(chars, i, true, e.right - e.left) {
                        char_break = true;
                    }
                }
            } else if glyph && x + size * gm.xmax > e.right {
                ended = false;
                break;
            }
            if is_whitespace(c) || cjk || c == b'-' as u16 {
                breaks += 1;
                last_was_break = true;
                can_break_before = true;
                char_break = false;
            } else {
                can_break_before = false;
                last_was_break = false;
            }
            top = top.min(glyph_top);
            desc = desc.max(glyph_desc);
            if !overflow && first {
                left_off = if glyph { (size * gm.xmin).min(0.0) } else { 0.0 };
            }
            right_max = right_max.max(right_ink);
            leading = format.leading;
            count += 1;
            glyphs += 1;
            x = nx;
            first = false;
            i += 1;
        }
        if ended {
            right_max = right_max.max(x);
        }
        *max_right = max_right.max(right_max);
        *min_left = min_left.min(left_off);
        let measure = LineMeasure {
            width: right_max - e.left,
            top,
            descent: desc,
            breaks: if ended { 0 } else { breaks - last_was_break as i32 },
            count,
            glyphs,
            leading,
            left_offset: left_off,
        };
        (measure, char_break)
    }

    /// Line starts (`FUN_141033cf0`) over the whole text with the given
    /// edges; also returns the measure pass's max right / min left offset.
    fn layout(&self, chars: &Chars, e: &Edges) -> (Vec<Line>, f32, f32) {
        let mut lines = Vec::new();
        let (mut max_right, mut min_left) = (-1e20f32, 1e20f32);
        let mut start = 0usize;
        let mut carry = false;
        let mut skip = e.skip;
        let mut next_top = e.top;
        let mut first_line = true;
        let (mut cur_leading, mut last_measured_leading) = (0.0f32, 0.0f32);
        while start < chars.chars.len() {
            let prev_leading = cur_leading;
            cur_leading = last_measured_leading;
            let (m, c) = self.measure_line(chars, start, e, carry, &mut max_right, &mut min_left);
            carry = c;
            if m.count == 0 {
                break;
            }
            last_measured_leading = m.leading;
            if chars.run_starts.contains(&start) {
                cur_leading = chars.format(start).leading;
            }
            let mut line = Line {
                start,
                count: m.count,
                x: 0.0,
                baseline: 0.0,
                justify: 0.0,
                visible: m.glyphs,
                top: m.top,
                descent: m.descent,
                leading: m.leading,
                skipped: false,
            };
            if skip > 0 {
                skip -= 1;
                line.visible = 0;
                line.skipped = true;
            } else {
                let y = next_top + if first_line { 0.0 } else { prev_leading };
                first_line = false;
                line.baseline = y - m.top;
                next_top = line.baseline + m.descent;
                // Clip mode 0: stop below the box.
                if e.bottom < line.baseline {
                    break;
                }
            }
            let align = if e.force_left { 0 } else { chars.format(start).align };
            line.x = match align {
                1 => e.right - m.width - m.left_offset,
                2 => (e.right + e.left) * 0.5 - m.width * 0.5 - m.left_offset,
                3 => {
                    if m.breaks > 0 {
                        line.justify = ((e.right - e.left) - m.width) / m.breaks as f32;
                    }
                    e.left - m.left_offset
                }
                _ => e.left - m.left_offset,
            } - e.scroll_h as f32;
            lines.push(line);
            start += m.count;
        }
        (lines, max_right, min_left)
    }

    fn measure_edges(&self) -> Edges {
        let right = if self.word_wrap() {
            if self.declared_width > 6.0 { self.declared_width - 6.0 } else { 94.0 }
        } else {
            4000.0
        };
        Edges { left: 0.0, top: 0.0, right, bottom: 1e20, force_left: true, skip: 0, scroll_h: 0 }
    }

    /// Draw/query edges: a 3 px horizontal and 2 px vertical gutter.
    fn draw_edges(&self) -> Edges {
        let x0 = self.origin[0] + self.offset[0];
        let y0 = self.origin[1] + self.offset[1];
        Edges {
            left: x0 + 3.0,
            top: y0 + 2.0,
            right: x0 + self.width - 3.0,
            bottom: y0 + self.height - 2.0,
            force_left: false,
            skip: self.scroll_v,
            scroll_h: self.scroll_h,
        }
    }

    /// `FUN_141030f60`. Returns the new display `tx` when RIGHT/CENTER
    /// autosize moves the field.
    pub fn relayout(&mut self) -> Option<f32> {
        let chars = Chars::new(&self.runs);
        let e = self.measure_edges();
        let (lines, max_right, min_left) = self.layout(&chars, &e);
        let (mut text_w, mut text_h, mut min_ink) = (0.0f32, 0.0f32, 1e20f32);
        self.lines.clear();
        self.num_lines = 0;
        for line in &lines {
            self.lines.push((line.baseline + line.top, line.baseline + line.descent));
            let mut x = line.x;
            for i in line.start..line.start + line.count {
                let c = self.chars_at(&chars, i);
                let format = chars.format(i);
                if let Some(font) = &format.font {
                    if c == b'\t' as u16 {
                        x = self.tab(format, x);
                    } else if c != b'\n' as u16 && c != b'\r' as u16 {
                        let g = glyph_index(font, c);
                        if g >= 0 {
                            let m = glyph_metrics(font, g);
                            text_h = text_h.max(line.baseline + format.size * metrics(font)[4]);
                            text_w = text_w.max(x + format.size * m.xmax);
                            min_ink = min_ink.min(x + format.size * m.xmin);
                            x += format.size * m.advance + format.letter_spacing;
                        }
                    }
                }
                if is_whitespace(c) && line.justify != 0.0 {
                    x += line.justify;
                }
            }
            self.num_lines += 1;
        }
        let all_newline = !chars.chars.is_empty() && chars.chars.iter().all(|&(c, _)| c == b'\n' as u16) && chars.chars.len() == 1;
        if self.editable || all_newline {
            if let Some(font) = &self.default.font {
                let m = metrics(font);
                text_h += self.default.size * (m[0] + m[1]);
                if !self.lines.is_empty() {
                    text_h += self.default.leading;
                }
            }
        }
        let _ = text_w - min_ink;
        // `DAT_142422efc == 1`: the line extents override the ink width.
        text_w = if lines.is_empty() { 0.0 } else { max_right - min_left };
        self.text_width = text_w;
        self.text_height = text_h;
        let mut tx = None;
        if self.auto_size == 0 {
            self.offset[1] = 0.0;
            self.offset[0] = self.base_x;
            self.width = self.declared_width;
        } else if !self.word_wrap() {
            let w = text_w + 6.0;
            self.width = w;
            match self.auto_size {
                1 => self.offset[0] = self.base_x,
                2 => tx = Some(self.anchor - w),
                3 => tx = Some(self.anchor - w * 0.5),
                _ => {}
            }
            self.height = text_h + 4.0;
        } else {
            self.width = self.declared_width;
            self.height = text_h + 4.0;
        }
        self.max_scroll_v = if self.auto_size == 0 { self.max_scroll() } else { 0 };
        let over = (text_w - (self.width - 6.0)) as i32;
        if over > 0 && self.scroll_h > over {
            self.scroll_h = over;
        }
        self.scroll_h = self.scroll_h.max(0);
        self.scroll_v = self.scroll_v.min(self.max_scroll_v).max(0);
        tx
    }

    /// `FUN_14102c990`: maxScrollV - 1.
    fn max_scroll(&self) -> i32 {
        let n = self.lines.len();
        if n == 0 {
            return 0;
        }
        let last = self.lines[n - 1].1;
        for i in (0..n).rev() {
            if last - self.lines[i].0 > self.height {
                return if i == n - 1 { (n - 1) as i32 } else { (i + 1) as i32 };
            }
        }
        0
    }

    /// The draw pass (`FUN_14102dcd0`): visible glyphs in local space.
    pub fn glyphs(&self) -> Vec<PlacedGlyph> {
        let chars = Chars::new(&self.runs);
        let e = self.draw_edges();
        let (lines, _, _) = self.layout(&chars, &e);
        let mut out = Vec::new();
        for line in lines.iter().filter(|l| !l.skipped) {
            let mut x = line.x;
            let mut visible = line.visible as i64;
            for i in line.start..line.start + line.count {
                let c = self.chars_at(&chars, i);
                let format = chars.format(i);
                let Some(font) = &format.font else { continue };
                if c == b'\t' as u16 {
                    x = self.tab(format, x);
                } else if c != b'\n' as u16 && c != b'\r' as u16 {
                    let g = glyph_index(font, c);
                    if g < 0 {
                        continue;
                    }
                    let m = glyph_metrics(font, g);
                    visible -= 1;
                    let shown = visible >= 0 && !(format.size * m.xmax + x + 10.0 < e.left);
                    if shown && !m.empty {
                        out.push(PlacedGlyph { font: font.clone(), glyph: g as usize, x, y: line.baseline, size: format.size, colour: format.colour });
                    }
                    x += format.size * m.advance + format.letter_spacing;
                }
                if is_whitespace(c) && line.justify != 0.0 {
                    x += line.justify;
                }
            }
        }
        out
    }

    /// `getLineMetrics(i)` (`FUN_1410b0cc0` -> `FUN_141030a40`): ascent,
    /// descent, height, leading, width (box - 6), x (left edge).
    pub fn line_metrics(&self, index: usize) -> Option<[f32; 6]> {
        let chars = Chars::new(&self.runs);
        let e = Edges { skip: 0, ..self.draw_edges() };
        let (lines, _, _) = self.layout(&chars, &e);
        let line = lines.get(index)?;
        let (ascent, descent) = (-line.top, line.descent);
        Some([ascent, descent, descent + ascent + line.leading, line.leading, e.right - e.left, e.left])
    }

    /// Text and HTML getters: the concatenated run characters.
    pub fn text(&self) -> String {
        let chars: Vec<u16> = self.runs.iter().flat_map(|r| r.chars.iter().copied()).collect();
        String::from_utf16_lossy(&chars)
    }

    pub fn length(&self) -> usize {
        self.runs.iter().map(|r| r.chars.len()).sum()
    }

    fn resolved(&self, fonts: &Fonts, mut format: Format) -> Format {
        format.font = format.name.as_deref().and_then(|n| fonts.resolve(n, format.italic, format.bold));
        format
    }

    /// `FUN_1410307c0(text, s, append = 0)`; `s` already substituted.
    pub fn set_text(&mut self, fonts: &Fonts, s: &str) -> Option<f32> {
        let chars: Vec<u16> = s.encode_utf16().collect();
        if self.runs.len() == 1 && self.runs[0].chars == chars {
            return None;
        }
        self.runs.clear();
        if !chars.is_empty() {
            let format = self.resolved(fonts, self.default.clone());
            self.runs.push(Run { format, chars });
        }
        self.caret = 0;
        self.relayout()
    }

    /// `appendText` (`FUN_1410307c0`, append = 1): no substitution.
    pub fn append_text(&mut self, fonts: &Fonts, s: &str) -> Option<f32> {
        let chars: Vec<u16> = s.encode_utf16().collect();
        if !chars.is_empty() {
            let format = self.resolved(fonts, self.default.clone());
            self.runs.push(Run { format, chars });
        }
        self.caret = self.length() as i32;
        self.relayout()
    }

    /// `FUN_1410303d0`; `src` already substituted.
    pub fn set_html(&mut self, fonts: &Fonts, src: &str) -> Option<f32> {
        let mut p = Html {
            src: src.encode_utf16().collect(),
            cur: 0,
            pending: 0,
            closed: true,
            depth: 0,
            format: self.resolved(fonts, self.default.clone()),
            runs: Vec::new(),
            fonts,
        };
        while p.cur < p.src.len() {
            if p.src[p.cur] == b'<' as u16 {
                let mut j = p.cur + 1;
                while j < p.src.len() && is_whitespace(p.src[j]) {
                    j += 1;
                }
                // An unmatched closing tag ends the whole text.
                if p.src.get(j) == Some(&(b'/' as u16)) {
                    break;
                }
                p.cur += 1;
                p.parse_tag();
            } else {
                if PARAGRAPH_MODE {
                    p.closed = false;
                }
                p.emit_text();
                p.closed = false;
            }
        }
        self.runs = p.runs;
        self.caret = 0;
        self.relayout()
    }

    /// `setTextFormat` (`FUN_1410b2080`) over [begin, end).
    pub fn set_format(&mut self, fonts: &Fonts, apply: &dyn Fn(&mut Format), begin: i32, end: i32) -> Option<f32> {
        let len = self.length() as i32;
        let (begin, end) = match (begin, end) {
            (b, _) if b < 0 => (0, len),
            (b, e) if e < 0 => (b, b + 1),
            (b, e) => (b, e),
        };
        let end = end.min(len);
        let begin = begin.min(end).max(0);
        // Split runs straddling the bounds (`FUN_1410b3c30`).
        let mut runs = Vec::new();
        let mut pos = 0i32;
        for run in std::mem::take(&mut self.runs) {
            let n = run.chars.len() as i32;
            let mut cuts = vec![0];
            for cut in [begin - pos, end - pos] {
                if cut > 0 && cut < n && !cuts.contains(&cut) {
                    cuts.push(cut);
                }
            }
            cuts.push(n);
            cuts.sort();
            for w in cuts.windows(2) {
                let (a, b) = (w[0] as usize, w[1] as usize);
                let mut piece = Run { format: run.format.clone(), chars: run.chars[a..b].to_vec() };
                let (s, e) = (pos + a as i32, pos + b as i32);
                if s >= begin && e <= end && s < e {
                    apply(&mut piece.format);
                }
                runs.push(piece);
            }
            pos += n;
        }
        // `FUN_1410318d0`: re-resolve every run's font.
        for run in &mut runs {
            run.format.font = run.format.name.as_deref().and_then(|n| fonts.resolve(n, run.format.italic, run.format.bold));
        }
        self.runs = runs;
        self.relayout()
    }
}

/// HTML parser state (`local_e8`).
struct Html<'a> {
    src: Vec<u16>,
    cur: usize,
    pending: usize,
    closed: bool,
    depth: i32,
    format: Format,
    runs: Vec<Run>,
    fonts: &'a Fonts,
}

impl Html<'_> {
    fn peek(&self) -> Option<u16> {
        self.src.get(self.cur).copied()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(is_whitespace) {
            self.cur += 1;
        }
    }

    fn word(&mut self, stop: &dyn Fn(u16) -> bool, max: usize) -> String {
        let start = self.cur;
        while let Some(c) = self.peek() {
            if stop(c) {
                break;
            }
            self.cur += 1;
        }
        let end = self.cur.min(start + max);
        String::from_utf16_lossy(&self.src[start..end])
    }

    fn resolve(&mut self) {
        self.format.font = self.format.name.as_deref().and_then(|n| self.fonts.resolve(n, self.format.italic, self.format.bold));
    }

    /// `FUN_14102d2d0`: pending newlines as a newline-only run.
    fn flush_newlines(&mut self) {
        if self.pending > 0 && self.format.name.is_some() {
            self.runs.push(Run { format: self.format.clone(), chars: vec![b'\n' as u16; self.pending] });
        }
        self.pending = 0;
    }

    /// `FUN_141033b40` -> `FUN_14102c750`.
    fn emit_text(&mut self) {
        let mut text = Vec::new();
        while let Some(c) = self.peek() {
            if c == b'<' as u16 {
                break;
            }
            self.cur += 1;
            if c == b'&' as u16 {
                if let Some(decoded) = self.entity() {
                    text.push(decoded);
                    continue;
                }
            }
            text.push(c);
        }
        if (!text.is_empty() || self.pending > 0) && self.format.name.is_some() {
            let mut chars = vec![b'\n' as u16; self.pending];
            chars.extend(text);
            self.runs.push(Run { format: self.format.clone(), chars });
        }
        self.pending = 0;
    }

    /// `FUN_141032a90`, after the `&`: the decoded character, or `None` to
    /// keep the `&` literal.
    fn entity(&mut self) -> Option<u16> {
        let rest = &self.src[self.cur..];
        let semi = rest.iter().take(13).position(|&c| c == b';' as u16)?;
        let name = String::from_utf16_lossy(&rest[..semi]);
        let value = match name.as_str() {
            "lt" => b'<' as u16,
            "gt" => b'>' as u16,
            "amp" => b'&' as u16,
            "quot" => b'"' as u16,
            "apos" => b'\'' as u16,
            "nbsp" => 0xa0,
            n if n.starts_with(['x', 'X']) => 0, // strtol starts at the 'x'
            n if n.starts_with(|c: char| c.is_ascii_digit()) => {
                let digits: String = n.chars().take_while(char::is_ascii_digit).collect();
                digits.parse::<u32>().unwrap_or(0) as u16
            }
            _ => return None,
        };
        self.cur += semi + 1;
        Some(value)
    }

    /// `FUN_141032c30`, after the `<`.
    fn parse_tag(&mut self) {
        let saved = self.format.clone();
        self.skip_ws();
        let name = self.word(&|c| is_whitespace(c) || c == b'>' as u16, 31);
        if !matches!(name.as_str(), "p" | "br" | "sbr") {
            self.flush_newlines();
        }
        match name.as_str() {
            "p" => self.depth += 1,
            "b" => {
                self.format.bold = true;
                self.resolve();
            }
            "i" => {
                self.format.italic = true;
                self.resolve();
            }
            "u" => {
                self.format.underline = true;
                self.resolve();
            }
            "br" | "sbr" => {
                self.pending += 1;
                self.closed = false;
            }
            _ => {}
        }
        // Attributes.
        loop {
            self.skip_ws();
            match self.peek() {
                None => {
                    self.format = saved;
                    return;
                }
                Some(c) if c == b'>' as u16 => {
                    self.cur += 1;
                    break;
                }
                Some(c) if c == b'/' as u16 => {
                    self.cur += 1;
                    continue;
                }
                _ => {}
            }
            let attr = self.word(&|c| is_whitespace(c) || c == b'/' as u16 || c == b'=' as u16 || c == b'>' as u16, 31);
            let mut value = String::new();
            if self.peek() == Some(b'=' as u16) {
                self.cur += 1;
                match self.peek() {
                    Some(q) if q == b'"' as u16 || q == b'\'' as u16 => {
                        self.cur += 1;
                        value = self.word(&|c| c == q, 127);
                        if self.peek().is_some() {
                            self.cur += 1;
                        }
                    }
                    _ => value = self.word(&|c| is_whitespace(c) || c == b'>' as u16 || c == b'/' as u16, 127),
                }
            } else if attr.is_empty() {
                self.cur += 1;
            }
            self.attribute(&name, &attr, &value);
        }
        if !matches!(name.as_str(), "br" | "sbr" | "img") {
            // Children until `</`.
            loop {
                match self.peek() {
                    None => {
                        self.format = saved;
                        return;
                    }
                    Some(c) if c == b'<' as u16 => {
                        let mut j = self.cur + 1;
                        while j < self.src.len() && is_whitespace(self.src[j]) {
                            j += 1;
                        }
                        if self.src.get(j) == Some(&(b'/' as u16)) {
                            self.cur = j + 1;
                            break;
                        }
                        self.cur += 1;
                        self.parse_tag();
                    }
                    _ => self.emit_text(),
                }
            }
            self.skip_ws();
            let close = self.word(&|c| is_whitespace(c) || c == b'>' as u16, 31);
            if close == name {
                if name == "p" {
                    self.depth -= 1;
                    if self.depth == 0 {
                        self.pending += 1;
                        self.closed = true;
                    }
                }
            } else if self.format.name.is_some() {
                self.runs.push(Run { format: self.format.clone(), chars: "BAD HTML".encode_utf16().collect() });
            }
            while let Some(c) = self.peek() {
                self.cur += 1;
                if c == b'>' as u16 {
                    break;
                }
            }
            self.flush_newlines();
        }
        self.format = saved;
    }

    fn attribute(&mut self, tag: &str, attr: &str, value: &str) {
        let float = |v: &str| float_prefix(v).unwrap_or(0.0);
        match (tag, attr) {
            ("font", "face") => {
                self.format.name = Some(value.to_string());
                self.resolve();
            }
            ("font", "color") => self.format.colour = strtol(value.trim_start_matches('#'), 16) as u32,
            ("font", "size") => {
                let n = strtol(value.trim_start_matches(['+', '-']), 10) as f32;
                self.format.size = match value.as_bytes().first() {
                    Some(b'+') => self.format.size + n,
                    Some(b'-') => self.format.size - n,
                    _ => n,
                };
            }
            ("font", "letterSpacing") => self.format.letter_spacing = float(value),
            ("p", "align") => match value {
                "left" => self.format.align = 0,
                "right" => self.format.align = 1,
                "center" => self.format.align = 2,
                "justify" => self.format.align = 3,
                _ => {}
            },
            ("textformat", "blockindent") => self.format.block_indent = float(value),
            ("textformat", "indent") => self.format.indent = float(value),
            ("textformat", "leading") => self.format.leading = float(value),
            ("textformat", "leftmargin") => self.format.left_margin = float(value),
            ("textformat", "rightmargin") => self.format.right_margin = float(value),
            ("textformat", "tabstops") => {
                self.format.tab_stops = value
                    .split(',')
                    .filter_map(|t| {
                        let t = t.trim_start_matches(|c: char| !c.is_ascii_digit());
                        let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
                        let rest = &t[digits.len()..];
                        (!digits.is_empty() && !rest.chars().any(|c| c.is_ascii_digit())).then(|| digits.parse().ok()).flatten()
                    })
                    .take(16)
                    .collect();
            }
            _ => {}
        }
    }
}

/// C `strtol` over the leading digits (no sign handling needed here).
fn strtol(s: &str, radix: u32) -> i64 {
    let digits: String = s.chars().take_while(|c| c.is_digit(radix)).collect();
    i64::from_str_radix(&digits, radix).unwrap_or(0)
}

/// `strtod` prefix.
fn float_prefix(s: &str) -> Option<f32> {
    let s = s.trim_start();
    (1..=s.len()).rev().filter_map(|end| s.get(..end)).find_map(|p| p.parse::<f32>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_runs_follow_the_parser() {
        let fonts = Fonts::default();
        let mut t = TextObject { default: Format { name: Some("X".into()), size: 12.0, ..Format::default() }, ..TextObject::default() };
        t.set_html(&fonts, "<p align=\"left\"><font size=\"40\" color=\"#d1d7d6\" letterSpacing=\"0.00\" kerning=\"1\">La</font></p>");
        assert_eq!(t.text(), "La\n");
        assert_eq!(t.runs.len(), 2);
        assert_eq!(t.runs[0].format.size, 40.0);
        assert_eq!(t.runs[0].format.colour, 0xd1d7d6);
        // Entities and an unmatched closing tag.
        t.set_html(&fonts, "a&amp;b&65;c&#65;</p>ignored");
        assert_eq!(t.text(), "a&bAc&#65;");
        t.set_html(&fonts, "<b>x</i>y");
        assert_eq!(t.text(), "xBAD HTMLy");
    }
}
