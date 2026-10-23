//! `flash.text`: TextField, TextFormat and Font. Layout and glyph rendering
//! follow Iggy's text engine (`FUN_141030f60`) in the rendering stage.
use super::avm2::builtins::{argument as arg, define, native_class, Spec};
use super::avm2::name::Name;
use super::avm2::object::{Kind, NativeClass, NativeFn, Obj};
use super::avm2::value::Value;
use super::avm2::{Result, Vm};
use super::display::{alloc, DisplayKind};
use super::Native;

/// The text object of an edit-text instance (`FUN_141023c20`).
#[derive(Clone, Debug)]
pub struct TextState {
    pub text: String,
    /// The last HTML assigned, if any.
    pub html: Option<String>,
    /// xmin, ymin, xmax, ymax in local pixels.
    pub rect: [f64; 4],
    pub word_wrap: bool,
    pub multiline: bool,
    pub selectable: bool,
    pub editable: bool,
    pub embed_fonts: bool,
    pub auto_size: String,
    pub border: bool,
    pub background: bool,
    pub max_chars: i32,
    pub html_mode: bool,
    /// Default format: font, size, colour, align, leading, margins, bold, italic, letterSpacing, kerning.
    pub format: Format,
}

#[derive(Clone, Debug)]
pub struct Format {
    pub font: String,
    pub size: f64,
    pub colour: u32,
    pub align: String,
    pub leading: f64,
    pub left_margin: f64,
    pub right_margin: f64,
    pub bold: bool,
    pub italic: bool,
    pub letter_spacing: f64,
    pub kerning: bool,
}

impl Default for Format {
    fn default() -> Self {
        // FUN_141023c20 defaults: 12.0 size, black, left.
        Format {
            font: "Times New Roman".into(),
            size: 12.0,
            colour: 0,
            align: "left".into(),
            leading: 0.0,
            left_margin: 0.0,
            right_margin: 0.0,
            bold: false,
            italic: false,
            letter_spacing: 0.0,
            kerning: false,
        }
    }
}

impl Default for TextState {
    fn default() -> Self {
        TextState {
            text: String::new(),
            html: None,
            rect: [0.0, 0.0, 100.0, 100.0],
            word_wrap: false,
            multiline: false,
            selectable: true,
            editable: false,
            embed_fonts: false,
            auto_size: "none".into(),
            border: false,
            background: false,
            max_chars: 0,
            html_mode: false,
            format: Format::default(),
        }
    }
}

pub fn text_state(o: &Obj) -> Option<std::cell::RefMut<'_, TextState>> {
    std::cell::RefMut::filter_map(o.host_mut::<Native>()?, |n| n.text.as_mut()).ok()
}

pub fn field_bounds(o: &Obj) -> Option<[f64; 4]> {
    let t = text_state(o)?;
    Some(t.rect)
}

/// Visible text of HTML: tags dropped, entities decoded, paragraphs as `\r`.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                let mut tag = String::new();
                for t in chars.by_ref() {
                    if t == '>' {
                        break;
                    }
                    tag.push(t);
                }
                let name = tag.trim_start_matches('/').split_whitespace().next().unwrap_or("").to_ascii_lowercase();
                if (tag.starts_with('/') && name == "p") || name == "br" {
                    out.push('\r');
                }
            }
            '&' => {
                let mut entity = String::new();
                while let Some(&t) = chars.peek() {
                    chars.next();
                    if t == ';' {
                        break;
                    }
                    entity.push(t);
                }
                out.push_str(match entity.as_str() {
                    "lt" => "<",
                    "gt" => ">",
                    "amp" => "&",
                    "quot" => "\"",
                    "apos" => "'",
                    "nbsp" => "\u{a0}",
                    _ => "",
                });
            }
            c => out.push(c),
        }
    }
    // The final paragraph does not end with a break.
    if out.ends_with('\r') {
        out.pop();
    }
    out
}

fn with<R>(vm: &mut Vm, this: &Value, f: impl FnOnce(&mut TextState) -> R) -> Result<R> {
    match this.as_object().and_then(text_state) {
        Some(mut t) => Ok(f(&mut t)),
        None => Err(vm.type_error(1034, "receiver is not a TextField.")),
    }
}

fn format_object(vm: &mut Vm, f: &Format) -> Result<Value> {
    let class = native_class(vm, "flash.text", "TextFormat");
    let o = vm.construct_class(&class, &[])?;
    for (k, v) in [
        ("font", Value::from(f.font.as_str())),
        ("size", f.size.into()),
        ("color", f.colour.into()),
        ("align", Value::from(f.align.as_str())),
        ("leading", f.leading.into()),
        ("leftMargin", f.left_margin.into()),
        ("rightMargin", f.right_margin.into()),
        ("bold", f.bold.into()),
        ("italic", f.italic.into()),
        ("letterSpacing", f.letter_spacing.into()),
        ("kerning", f.kerning.into()),
    ] {
        vm.set_property(&o, &Name::public(k), v)?;
    }
    Ok(o)
}

/// Apply the non-null fields of a TextFormat object to `f`.
fn apply_format(vm: &mut Vm, f: &mut Format, o: &Value) -> Result<()> {
    let get = |vm: &mut Vm, k: &str| vm.get_property(o, &Name::public(k));
    let v = get(vm, "font")?;
    if !v.is_nullish() {
        f.font = vm.to_string(&v)?.to_string();
    }
    for (k, slot) in [("size", &mut f.size as &mut f64), ("leading", &mut f.leading), ("leftMargin", &mut f.left_margin), ("rightMargin", &mut f.right_margin), ("letterSpacing", &mut f.letter_spacing)] {
        let v = get(vm, k)?;
        if !v.is_nullish() {
            *slot = vm.to_number(&v)?;
        }
    }
    let v = get(vm, "color")?;
    if !v.is_nullish() {
        f.colour = vm.to_uint(&v)?;
    }
    let v = get(vm, "align")?;
    if !v.is_nullish() {
        f.align = vm.to_string(&v)?.to_string();
    }
    for (k, slot) in [("bold", &mut f.bold as &mut bool), ("italic", &mut f.italic), ("kerning", &mut f.kerning)] {
        let v = get(vm, k)?;
        if !v.is_nullish() {
            *slot = v.to_boolean();
        }
    }
    Ok(())
}

/// `FUN_1410b1dd0` / `FUN_1410b17d0`: the text box size scaled by the
/// length of the matrix row (m0, m1) or (m2, m3), in f32.
fn scaled_extent(o: &Obj, horizontal: bool) -> f64 {
    let m = super::display::display_ref(o).map(|d| d.matrix).unwrap_or_default();
    let (p, q, size) = {
        let t = text_state(o).map(|t| t.rect).unwrap_or_default();
        if horizontal { (m.a as f32, m.c as f32, (t[2] - t[0]) as f32) } else { (m.b as f32, m.d as f32, (t[3] - t[1]) as f32) }
    };
    ((p * p + q * q).sqrt() * size) as f64
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
        ("text", |vm, this, _| with(vm, this, |t| Value::from(t.text.replace('\n', "\r")))),
        ("htmlText", |vm, this, _| with(vm, this, |t| Value::from(t.html.clone().unwrap_or_else(|| t.text.clone())))),
        ("length", |vm, this, _| with(vm, this, |t| Value::from(t.text.encode_utf16().count() as u32))),
        ("textColor", |vm, this, _| with(vm, this, |t| Value::from(t.format.colour))),
        ("autoSize", |vm, this, _| with(vm, this, |t| Value::from(t.auto_size.as_str()))),
        ("wordWrap", |vm, this, _| with(vm, this, |t| Value::from(t.word_wrap))),
        ("multiline", |vm, this, _| with(vm, this, |t| Value::from(t.multiline))),
        ("selectable", |vm, this, _| with(vm, this, |t| Value::from(t.selectable))),
        ("embedFonts", |vm, this, _| with(vm, this, |t| Value::from(t.embed_fonts))),
        ("border", |vm, this, _| with(vm, this, |t| Value::from(t.border))),
        ("background", |vm, this, _| with(vm, this, |t| Value::from(t.background))),
        ("maxChars", |vm, this, _| with(vm, this, |t| Value::from(t.max_chars))),
        ("type", |vm, this, _| with(vm, this, |t| Value::from(if t.editable { "input" } else { "dynamic" }))),
        ("defaultTextFormat", |vm, this, _| {
            let f = with(vm, this, |t| t.format.clone())?;
            format_object(vm, &f)
        }),
        ("textWidth", |vm, this, _| {
            let o = this.as_object().cloned().unwrap();
            Ok(super::text_layout::approximate_measure(vm, &o).0.into())
        }),
        ("textHeight", |vm, this, _| {
            let o = this.as_object().cloned().unwrap();
            Ok(super::text_layout::approximate_measure(vm, &o).1.into())
        }),
        ("numLines", |vm, this, _| {
            let o = this.as_object().cloned().unwrap();
            Ok((super::text_layout::approximate_measure(vm, &o).2 as u32).into())
        }),
    ]
}

fn setters() -> Vec<(&'static str, NativeFn)> {
    vec![
        // `FUN_1410b2b50` -> `FUN_1410317a0`: the requested box width, then
        // relayout. The autosize origin adjustment is part of the text-engine
        // port.
        ("width", |vm, this, a| {
            let w = vm.to_number(&arg(a, 0))? as f32 as f64;
            with(vm, this, |t| t.rect[2] = t.rect[0] + w)?;
            Ok(Value::Undefined)
        }),
        // `FUN_1410b2550` -> `FUN_141031780`: only without autosize.
        ("height", |vm, this, a| {
            let h = vm.to_number(&arg(a, 0))? as f32 as f64;
            with(vm, this, |t| {
                if t.auto_size == "none" {
                    t.rect[3] = t.rect[1] + h;
                }
            })?;
            Ok(Value::Undefined)
        }),
        ("text", |vm, this, a| {
            let s = vm.to_string(&arg(a, 0))?.to_string();
            let s = super::iggy_functions::substitute(vm, &s);
            with(vm, this, |t| {
                t.text = s;
                t.html = None;
            })?;
            Ok(Value::Undefined)
        }),
        ("htmlText", |vm, this, a| {
            let s = vm.to_string(&arg(a, 0))?.to_string();
            let s = super::iggy_functions::substitute(vm, &s);
            with(vm, this, |t| {
                t.text = html_to_text(&s);
                t.html = Some(s);
            })?;
            Ok(Value::Undefined)
        }),
        ("textColor", |vm, this, a| {
            let c = vm.to_uint(&arg(a, 0))?;
            with(vm, this, |t| t.format.colour = c)?;
            Ok(Value::Undefined)
        }),
        ("autoSize", |vm, this, a| {
            let s = vm.to_string(&arg(a, 0))?.to_string();
            with(vm, this, |t| t.auto_size = s)?;
            Ok(Value::Undefined)
        }),
        ("wordWrap", |vm, this, a| {
            let v = arg(a, 0).to_boolean();
            with(vm, this, |t| t.word_wrap = v)?;
            Ok(Value::Undefined)
        }),
        ("multiline", |vm, this, a| {
            let v = arg(a, 0).to_boolean();
            with(vm, this, |t| t.multiline = v)?;
            Ok(Value::Undefined)
        }),
        ("selectable", |vm, this, a| {
            let v = arg(a, 0).to_boolean();
            with(vm, this, |t| t.selectable = v)?;
            Ok(Value::Undefined)
        }),
        ("embedFonts", |vm, this, a| {
            let v = arg(a, 0).to_boolean();
            with(vm, this, |t| t.embed_fonts = v)?;
            Ok(Value::Undefined)
        }),
        ("border", |vm, this, a| {
            let v = arg(a, 0).to_boolean();
            with(vm, this, |t| t.border = v)?;
            Ok(Value::Undefined)
        }),
        ("background", |vm, this, a| {
            let v = arg(a, 0).to_boolean();
            with(vm, this, |t| t.background = v)?;
            Ok(Value::Undefined)
        }),
        ("maxChars", |vm, this, a| {
            let v = vm.to_int(&arg(a, 0))?;
            with(vm, this, |t| t.max_chars = v)?;
            Ok(Value::Undefined)
        }),
        ("type", |vm, this, a| {
            let v = vm.to_string(&arg(a, 0))?;
            with(vm, this, |t| t.editable = &*v == "input")?;
            Ok(Value::Undefined)
        }),
        ("defaultTextFormat", |vm, this, a| {
            let mut f = with(vm, this, |t| t.format.clone())?;
            apply_format(vm, &mut f, &arg(a, 0))?;
            with(vm, this, |t| t.format = f)?;
            Ok(Value::Undefined)
        }),
    ]
}

fn methods() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("appendText", |vm, this, a| {
            let s = vm.to_string(&arg(a, 0))?.to_string();
            with(vm, this, |t| {
                t.text.push_str(&s);
                t.html = None;
            })?;
            Ok(Value::Undefined)
        }),
        ("getTextFormat", |vm, this, _| {
            let f = with(vm, this, |t| t.format.clone())?;
            format_object(vm, &f)
        }),
        ("setTextFormat", |vm, this, a| {
            // Whole-range formats only; runs with ranges are applied by the
            // layout stage.
            let mut f = with(vm, this, |t| t.format.clone())?;
            apply_format(vm, &mut f, &arg(a, 0))?;
            with(vm, this, |t| t.format = f)?;
            Ok(Value::Undefined)
        }),
        ("setSelection", |_, _, _| Ok(Value::Undefined)),
        ("getLineMetrics", |vm, this, _| {
            let o = this.as_object().cloned().unwrap();
            let (w, h, _) = super::text_layout::approximate_measure(vm, &o);
            let class = native_class(vm, "flash.text", "TextLineMetrics");
            let m = Value::Object(vm.allocate(&class)?);
            for (k, v) in [("x", 0.0), ("width", w), ("height", h), ("ascent", 0.0), ("descent", 0.0), ("leading", 0.0)] {
                vm.set_property(&m, &Name::public(k), v.into())?;
            }
            Ok(m)
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
                    n.text = Some(TextState::default());
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
        let name = super::text_layout::font_name_of_class(vm, this);
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
