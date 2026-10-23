//! `flash.text`: TextField, TextFormat and Font natives over Iggy's text
//! engine (`text_engine`).
use super::avm2::builtins::{argument as arg, define, native_class, Spec};
use super::avm2::name::Name;
use super::avm2::object::{Kind, NativeClass, NativeFn, Obj};
use super::avm2::value::Value;
use super::avm2::{Result, Vm};
use super::display::{alloc, display, DisplayKind};
use super::text_engine::{Fonts, Format, TextObject};
use super::{player, Native};

pub fn text_object(o: &Obj) -> Option<std::cell::RefMut<'_, TextObject>> {
    std::cell::RefMut::filter_map(o.host_mut::<Native>()?, |n| n.text.as_mut()).ok()
}

/// Local bounds of the text box: origin plus autosize offset, box size.
pub fn field_bounds(o: &Obj) -> Option<[f64; 4]> {
    let t = text_object(o)?;
    let (x, y) = ((t.origin[0] + t.offset[0]) as f64, (t.origin[1] + t.offset[1]) as f64);
    Some([x, y, x + t.width as f64, y + t.height as f64])
}

/// The player's font registry and the language aliases.
pub fn fonts(vm: &Vm) -> Fonts {
    let shared = player::state(vm);
    let p = shared.borrow();
    Fonts { fonts: p.fonts.clone(), aliases: p.font_aliases.clone() }
}

/// Run `f` on the text object; a returned `tx` moves the field (RIGHT and
/// CENTER autosize write the display matrix).
fn edit(vm: &mut Vm, this: &Value, f: impl FnOnce(&Fonts, &mut TextObject) -> Option<f32>) -> Result<()> {
    let o = this.as_object().cloned().ok_or_else(|| vm.type_error(1009, "null"))?;
    let fonts = fonts(vm);
    let tx = match text_object(&o) {
        Some(mut t) => f(&fonts, &mut t),
        None => return Err(vm.type_error(1034, "receiver is not a TextField.")),
    };
    if let (Some(tx), Some(mut d)) = (tx, display(&o)) {
        let mut m = d.matrix;
        m.tx = tx as f64;
        d.set_matrix(m);
    }
    Ok(())
}

fn read<R>(vm: &mut Vm, this: &Value, f: impl FnOnce(&TextObject) -> R) -> Result<R> {
    match this.as_object().and_then(text_object) {
        Some(t) => Ok(f(&t)),
        None => Err(vm.type_error(1034, "receiver is not a TextField.")),
    }
}

const AUTO_SIZE: [&str; 4] = ["none", "left", "right", "center"];
const ALIGN: [&str; 4] = ["left", "right", "center", "justify"];

/// A TextFormat object from a format; `None` fields are null
/// (`getTextFormat` across differing runs).
fn format_object(vm: &mut Vm, f: &PartialFormat) -> Result<Value> {
    let class = native_class(vm, "flash.text", "TextFormat");
    let o = vm.construct_class(&class, &[])?;
    let num = |v: Option<f32>| v.map_or(Value::Null, |v| (v as f64).into());
    let flag = |v: Option<bool>| v.map_or(Value::Null, Value::from);
    for (k, v) in [
        // The font getter returns "Times New Roman" for a null name.
        ("font", f.name.clone().map_or(Value::Null, |n| Value::from(n.unwrap_or_else(|| "Times New Roman".into()).as_str()))),
        ("size", num(f.size)),
        ("color", f.colour.map_or(Value::Null, Value::from)),
        ("align", f.align.map_or(Value::Null, |a| Value::from(ALIGN.get(a as usize).copied().unwrap_or("left")))),
        ("blockIndent", num(f.block_indent)),
        ("indent", num(f.indent)),
        ("leading", num(f.leading)),
        ("leftMargin", num(f.left_margin)),
        ("rightMargin", num(f.right_margin)),
        ("letterSpacing", num(f.letter_spacing)),
        ("bold", flag(f.bold)),
        ("bullet", flag(f.bullet)),
        ("italic", flag(f.italic)),
        ("kerning", flag(f.kerning)),
        ("underline", flag(f.underline)),
    ] {
        vm.set_property(&o, &Name::public(k), v)?;
    }
    Ok(o)
}

/// Format fields with null as `None` (TextFormat "null" encoding).
#[derive(Clone, Default)]
struct PartialFormat {
    name: Option<Option<String>>,
    size: Option<f32>,
    block_indent: Option<f32>,
    indent: Option<f32>,
    leading: Option<f32>,
    left_margin: Option<f32>,
    right_margin: Option<f32>,
    letter_spacing: Option<f32>,
    colour: Option<u32>,
    align: Option<u8>,
    bold: Option<bool>,
    bullet: Option<bool>,
    italic: Option<bool>,
    kerning: Option<bool>,
    underline: Option<bool>,
}

impl PartialFormat {
    fn of(f: &Format) -> Self {
        PartialFormat {
            name: Some(f.name.clone()),
            size: Some(f.size),
            block_indent: Some(f.block_indent),
            indent: Some(f.indent),
            leading: Some(f.leading),
            left_margin: Some(f.left_margin),
            right_margin: Some(f.right_margin),
            letter_spacing: Some(f.letter_spacing),
            colour: Some(f.colour),
            align: Some(f.align),
            bold: Some(f.bold),
            bullet: Some(f.bullet),
            italic: Some(f.italic),
            kerning: Some(f.kerning),
            underline: Some(f.underline),
        }
    }

    /// Later runs turn differing fields to null (`FUN_1410b1150`).
    fn merge(&mut self, f: &Format) {
        fn keep<T: PartialEq>(slot: &mut Option<T>, v: T) {
            if slot.as_ref() != Some(&v) {
                *slot = None;
            }
        }
        keep(&mut self.name, f.name.clone());
        keep(&mut self.size, f.size);
        keep(&mut self.block_indent, f.block_indent);
        keep(&mut self.indent, f.indent);
        keep(&mut self.leading, f.leading);
        keep(&mut self.left_margin, f.left_margin);
        keep(&mut self.right_margin, f.right_margin);
        keep(&mut self.letter_spacing, f.letter_spacing);
        keep(&mut self.colour, f.colour);
        keep(&mut self.align, f.align);
        keep(&mut self.bold, f.bold);
        keep(&mut self.bullet, f.bullet);
        keep(&mut self.italic, f.italic);
        keep(&mut self.kerning, f.kerning);
        keep(&mut self.underline, f.underline);
    }

    /// `FUN_1410b3b00`: copy every non-null field (tab stops excluded).
    fn apply(&self, f: &mut Format) {
        if let Some(n) = &self.name {
            f.name = n.clone();
        }
        let set = |slot: &mut f32, v: Option<f32>| {
            if let Some(v) = v {
                *slot = v;
            }
        };
        set(&mut f.size, self.size);
        set(&mut f.block_indent, self.block_indent);
        set(&mut f.indent, self.indent);
        set(&mut f.leading, self.leading);
        set(&mut f.left_margin, self.left_margin);
        set(&mut f.right_margin, self.right_margin);
        set(&mut f.letter_spacing, self.letter_spacing);
        if let Some(c) = self.colour {
            f.colour = c;
        }
        if let Some(a) = self.align {
            f.align = a;
        }
        for (slot, v) in [(&mut f.bold, self.bold), (&mut f.bullet, self.bullet), (&mut f.italic, self.italic), (&mut f.kerning, self.kerning), (&mut f.underline, self.underline)] {
            if let Some(v) = v {
                *slot = v;
            }
        }
    }
}

/// Read a TextFormat object's non-null fields.
fn read_format(vm: &mut Vm, o: &Value) -> Result<PartialFormat> {
    let mut p = PartialFormat::default();
    let get = |vm: &mut Vm, k: &str| vm.get_property(o, &Name::public(k));
    let v = get(vm, "font")?;
    if !v.is_nullish() {
        p.name = Some(Some(vm.to_string(&v)?.to_string()));
    }
    let num = |vm: &mut Vm, k: &str| -> Result<Option<f32>> {
        let v = get(vm, k)?;
        Ok(if v.is_nullish() { None } else { Some(vm.to_number(&v)? as f32) })
    };
    p.size = num(vm, "size")?;
    p.block_indent = num(vm, "blockIndent")?;
    p.indent = num(vm, "indent")?;
    p.leading = num(vm, "leading")?;
    p.left_margin = num(vm, "leftMargin")?;
    p.right_margin = num(vm, "rightMargin")?;
    p.letter_spacing = num(vm, "letterSpacing")?;
    let v = get(vm, "color")?;
    if !v.is_nullish() {
        p.colour = Some(vm.to_uint(&v)?);
    }
    let v = get(vm, "align")?;
    if !v.is_nullish() {
        let s = vm.to_string(&v)?;
        p.align = ALIGN.iter().position(|a| *a == &*s).map(|i| i as u8);
    }
    let flag = |vm: &mut Vm, k: &str| -> Result<Option<bool>> {
        let v = get(vm, k)?;
        Ok((!v.is_nullish()).then(|| v.to_boolean()))
    };
    p.bold = flag(vm, "bold")?;
    p.bullet = flag(vm, "bullet")?;
    p.italic = flag(vm, "italic")?;
    p.kerning = flag(vm, "kerning")?;
    p.underline = flag(vm, "underline")?;
    Ok(p)
}

/// `FUN_1410b1dd0` / `FUN_1410b17d0`: the box size scaled by the length of
/// the matrix row (m0, m1) or (m2, m3), in f32.
fn scaled_extent(o: &Obj, horizontal: bool) -> f64 {
    let m = super::display::display_ref(o).map(|d| d.matrix).unwrap_or_default();
    let (p, q, size) = {
        let (w, h) = text_object(o).map(|t| (t.width, t.height)).unwrap_or_default();
        if horizontal { (m.a as f32, m.c as f32, w) } else { (m.b as f32, m.d as f32, h) }
    };
    ((p * p + q * q).sqrt() * size) as f64
}

/// Range arguments of `getTextFormat`/`setTextFormat`: -1 when absent.
fn range(vm: &mut Vm, a: &[Value], first: usize) -> Result<(i32, i32)> {
    let index = |vm: &mut Vm, i: usize| -> Result<i32> {
        match a.get(i) {
            Some(v) if !v.is_nullish() => vm.to_int(v),
            _ => Ok(-1),
        }
    };
    Ok((index(vm, first)?, index(vm, first + 1)?))
}

fn getters() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("width", |vm, this, _| {
            let o = this.as_object().cloned().ok_or_else(|| vm.type_error(1009, "null"))?;
            Ok(scaled_extent(&o, true).into())
        }),
        ("height", |vm, this, _| {
            let o = this.as_object().cloned().ok_or_else(|| vm.type_error(1009, "null"))?;
            Ok(scaled_extent(&o, false).into())
        }),
        ("text", |vm, this, _| read(vm, this, |t| Value::from(t.text()))),
        ("htmlText", |vm, this, _| read(vm, this, |t| Value::from(t.text()))),
        ("length", |vm, this, _| read(vm, this, |t| Value::from(t.length() as u32))),
        ("textColor", |vm, this, _| read(vm, this, |t| Value::from(t.default.colour))),
        ("autoSize", |vm, this, _| read(vm, this, |t| Value::from(AUTO_SIZE[t.auto_size as usize & 3]))),
        ("maxChars", |vm, this, _| read(vm, this, |t| Value::from(t.max_chars))),
        ("type", |vm, this, _| read(vm, this, |t| Value::from(if t.editable { "input" } else { "dynamic" }))),
        ("backgroundColor", |vm, this, _| read(vm, this, |t| Value::from(t.background_colour))),
        ("borderColor", |vm, this, _| read(vm, this, |t| Value::from(t.border_colour))),
        ("defaultTextFormat", |vm, this, _| {
            let f = read(vm, this, |t| PartialFormat::of(&t.default))?;
            format_object(vm, &f)
        }),
        // `textWidth` = raw + 1, `textHeight` = ceil(raw) + 2.
        ("textWidth", |vm, this, _| read(vm, this, |t| Value::from((t.text_width + 1.0) as f64))),
        ("textHeight", |vm, this, _| read(vm, this, |t| Value::from((t.text_height.ceil() + 2.0) as f64))),
        ("numLines", |vm, this, _| read(vm, this, |t| Value::from(t.num_lines))),
        ("scrollV", |vm, this, _| read(vm, this, |t| Value::from(t.scroll_v + 1))),
        ("maxScrollV", |vm, this, _| read(vm, this, |t| Value::from(t.max_scroll_v + 1))),
        ("scrollH", |vm, this, _| read(vm, this, |t| Value::from(t.scroll_h))),
        ("maxScrollH", |vm, this, _| read(vm, this, |t| Value::from(((t.text_width - (t.width - 6.0 - 3.0)) as i32).max(0)))),
        ("caretIndex", |vm, this, _| read(vm, this, |t| Value::from(t.caret))),
    ]
}

fn setters() -> Vec<(&'static str, NativeFn)> {
    vec![
        // `FUN_1410b2b50` -> `FUN_1410317a0`: the value is not divided by the
        // matrix scale.
        ("width", |vm, this, a| {
            let w = vm.to_number(&arg(a, 0))? as f32;
            edit(vm, this, |_, t| {
                let moved = match (t.word_wrap(), t.auto_size) {
                    (false, 2) => Some(t.anchor - t.width),
                    (false, 3) => Some(t.anchor - t.width * 0.5),
                    _ => None,
                };
                t.declared_width = w;
                t.relayout().or(moved)
            })?;
            Ok(Value::Undefined)
        }),
        // `FUN_1410b2550` -> `FUN_141031780`: only without autosize.
        ("height", |vm, this, a| {
            let h = vm.to_number(&arg(a, 0))? as f32;
            edit(vm, this, |_, t| {
                if t.auto_size == 0 {
                    t.height = h;
                    t.update_max_scroll();
                }
                None
            })?;
            Ok(Value::Undefined)
        }),
        ("text", |vm, this, a| {
            let s = vm.to_string(&arg(a, 0))?.to_string();
            let s = super::iggy_functions::substitute(vm, &s);
            edit(vm, this, |fonts, t| t.set_text(fonts, &s))?;
            Ok(Value::Undefined)
        }),
        ("htmlText", |vm, this, a| {
            let s = vm.to_string(&arg(a, 0))?.to_string();
            let s = super::iggy_functions::substitute(vm, &s);
            edit(vm, this, |fonts, t| t.set_html(fonts, &s))?;
            Ok(Value::Undefined)
        }),
        // Writes the default colour and every run's, without relayout.
        ("textColor", |vm, this, a| {
            let c = vm.to_uint(&arg(a, 0))?;
            edit(vm, this, |_, t| {
                t.default.colour = c;
                t.runs.iter_mut().for_each(|r| r.format.colour = c);
                None
            })?;
            Ok(Value::Undefined)
        }),
        // `FUN_1410b22a0`: index of the atom; the anchor from the current tx.
        ("autoSize", |vm, this, a| {
            let s = vm.to_string(&arg(a, 0))?.to_string();
            let o = this.as_object().cloned().ok_or_else(|| vm.type_error(1009, "null"))?;
            let tx = super::display::display_ref(&o).map_or(0.0, |d| d.matrix.tx) as f32;
            edit(vm, this, |_, t| {
                t.auto_size = AUTO_SIZE.iter().position(|v| *v == s).unwrap_or(0) as u8;
                t.anchor = match t.auto_size {
                    1 => tx,
                    2 => tx + t.width,
                    3 => tx + t.width * 0.5,
                    _ => t.anchor,
                };
                t.relayout()
            })?;
            Ok(Value::Undefined)
        }),
        // `0x1410b2bb0`: the field byte, then relayout.
        ("wordWrap", |vm, this, a| {
            let v = arg(a, 0).to_boolean();
            edit(vm, this, |_, t| {
                t.set_flag(super::text_engine::WORD_WRAP, v);
                t.relayout()
            })?;
            Ok(Value::Undefined)
        }),
        // `0x1410b2680`: +0xe3 and +0x100 = !v, then relayout.
        ("multiline", |vm, this, a| {
            let v = arg(a, 0).to_boolean();
            edit(vm, this, |_, t| {
                t.set_flag(super::text_engine::MULTILINE, v);
                t.relayout()
            })?;
            Ok(Value::Undefined)
        }),
        ("maxChars", |vm, this, a| {
            let v = vm.to_int(&arg(a, 0))?;
            edit(vm, this, |_, t| {
                t.max_chars = v;
                None
            })?;
            Ok(Value::Undefined)
        }),
        ("type", |vm, this, a| {
            let v = vm.to_string(&arg(a, 0))?;
            edit(vm, this, |_, t| {
                t.editable = &*v == "input";
                None
            })?;
            Ok(Value::Undefined)
        }),
        ("backgroundColor", |vm, this, a| {
            let v = vm.to_uint(&arg(a, 0))?;
            edit(vm, this, |_, t| {
                t.background_colour = v;
                None
            })?;
            Ok(Value::Undefined)
        }),
        ("borderColor", |vm, this, a| {
            let v = vm.to_uint(&arg(a, 0))?;
            edit(vm, this, |_, t| {
                t.border_colour = v;
                None
            })?;
            Ok(Value::Undefined)
        }),
        // +0xac = clamp(v - 1, 0, maxScrollV - 1), no relayout.
        ("scrollV", |vm, this, a| {
            let v = vm.to_int(&arg(a, 0))?;
            edit(vm, this, |_, t| {
                t.scroll_v = (v - 1).clamp(0, t.max_scroll_v.max(0));
                None
            })?;
            Ok(Value::Undefined)
        }),
        // Copied into the default format only: no relayout or font resolve.
        ("defaultTextFormat", |vm, this, a| {
            let p = read_format(vm, &arg(a, 0))?;
            edit(vm, this, |_, t| {
                p.apply(&mut t.default);
                None
            })?;
            Ok(Value::Undefined)
        }),
    ]
}

fn methods() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("appendText", |vm, this, a| {
            let s = vm.to_string(&arg(a, 0))?.to_string();
            edit(vm, this, |fonts, t| t.append_text(fonts, &s))?;
            Ok(Value::Undefined)
        }),
        // `FUN_1410b1150`.
        ("getTextFormat", |vm, this, a| {
            let (begin, end) = range(vm, a, 0)?;
            let p = read(vm, this, |t| {
                let len = t.length() as i32;
                let (b, e) = match (begin, end) {
                    (b, _) if b < 0 => (0, len),
                    (b, e) if e < 0 => (b, b + 1),
                    (b, e) => (b, e.min(len)),
                };
                let mut out: Option<PartialFormat> = None;
                let mut pos = 0i32;
                for run in &t.runs {
                    let n = run.chars.len() as i32;
                    if pos < e && pos + n > b {
                        match &mut out {
                            Some(p) => p.merge(&run.format),
                            None => out = Some(PartialFormat::of(&run.format)),
                        }
                    }
                    pos += n;
                }
                out.unwrap_or_default()
            })?;
            format_object(vm, &p)
        }),
        // `FUN_1410b2080`.
        ("setTextFormat", |vm, this, a| {
            let p = read_format(vm, &arg(a, 0))?;
            let (begin, end) = range(vm, a, 1)?;
            edit(vm, this, |fonts, t| t.set_format(fonts, &|f| p.apply(f), begin, end))?;
            Ok(Value::Undefined)
        }),
        ("setSelection", |_, _, _| Ok(Value::Undefined)),
        // `FUN_1410b0cc0`.
        ("getLineMetrics", |vm, this, a| {
            let index = vm.to_int(&arg(a, 0))?;
            let m = read(vm, this, |t| if index >= 0 { t.line_metrics(index as usize) } else { None })?;
            let Some([ascent, descent, height, leading, width, x]) = m else {
                return Err(vm.range_error(2006, "The supplied index is out of bounds."));
            };
            let class = native_class(vm, "flash.text", "TextLineMetrics");
            let o = Value::Object(vm.allocate(&class)?);
            for (k, v) in [("x", x), ("width", width), ("height", height), ("ascent", ascent), ("descent", descent), ("leading", leading)] {
                vm.set_property(&o, &Name::public(k), (v as f64).into())?;
            }
            Ok(o)
        }),
    ]
}

pub fn install(vm: &mut Vm) {
    let interactive = native_class(vm, "flash.display", "InteractiveObject");
    let mut s = Spec::new("flash.text", "TextField", Some(interactive));
    s.native = NativeClass {
        allocate: Some(|vm, c| {
            let mut k = alloc(DisplayKind::Text)(vm, c)?;
            if let Kind::Host(h) = &mut k {
                if let Some(n) = h.downcast_mut::<Native>() {
                    n.text = Some(TextObject::default());
                }
            }
            Ok(k)
        }),
        call: None,
        init: None,
    };
    s.getters = getters();
    s.setters = setters();
    s.methods = methods();
    define(vm, s);

    let object = vm.b().object.clone();
    let mut s = Spec::new("flash.text", "TextFormat", Some(object.clone()));
    let fields = ["font", "size", "color", "bold", "italic", "underline", "url", "target", "align", "leftMargin", "rightMargin", "indent", "leading"];
    s.vars = fields
        .iter()
        .map(|f| (*f, Value::Null))
        .chain(["letterSpacing", "kerning", "blockIndent", "bullet", "tabStops"].iter().map(|f| (*f, Value::Null)))
        .collect();
    s.native = NativeClass {
        allocate: None,
        call: None,
        init: Some(|vm, this, a| {
            const ORDER: [&str; 13] = ["font", "size", "color", "bold", "italic", "underline", "url", "target", "align", "leftMargin", "rightMargin", "indent", "leading"];
            for (i, f) in ORDER.iter().enumerate() {
                if let Some(v) = a.get(i) {
                    vm.set_property(this, &Name::public(f), v.clone())?;
                }
            }
            Ok(Value::Undefined)
        }),
    };
    define(vm, s);

    let mut s = Spec::new("flash.text", "TextLineMetrics", Some(object.clone()));
    s.vars = vec![("x", 0.into()), ("width", 0.into()), ("height", 0.into()), ("ascent", 0.into()), ("descent", 0.into()), ("leading", 0.into())];
    define(vm, s);

    let mut s = Spec::new("flash.text", "Font", Some(object.clone()));
    s.getters = vec![("fontName", |vm, this, _| {
        let name = font_name_of_class(vm, this);
        Ok(name.map(Value::from).unwrap_or(Value::Null))
    }), ("fontStyle", |_, _, _| Ok("regular".into())), ("fontType", |_, _, _| Ok("embedded".into()))];
    s.methods = vec![("hasGlyphs", |_, _, _| Ok(true.into()))];
    s.statics = vec![("enumerateFonts", |vm, _, _| Ok(Value::Object(vm.array(Vec::new()))))];
    define(vm, s);

    for (name, constants) in [
        ("TextFieldAutoSize", &[("NONE", "none"), ("LEFT", "left"), ("RIGHT", "right"), ("CENTER", "center")][..]),
        ("TextFormatAlign", &[("LEFT", "left"), ("RIGHT", "right"), ("CENTER", "center"), ("JUSTIFY", "justify")][..]),
        ("TextFieldType", &[("DYNAMIC", "dynamic"), ("INPUT", "input")][..]),
        ("AntiAliasType", &[("NORMAL", "normal"), ("ADVANCED", "advanced")][..]),
    ] {
        let mut s = Spec::new("flash.text", name, Some(object.clone()));
        s.constants = constants.iter().map(|(k, v)| (*k, Value::from(*v))).collect();
        define(vm, s);
    }
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
