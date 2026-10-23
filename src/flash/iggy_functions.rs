//! `IggyFunctions`: Iggy's own AS3 natives (`chunk_000684` ~5248).
use super::avm2::builtins::{argument as arg, define, Spec};
use super::avm2::value::Value;
use super::avm2::Vm;
use super::display::display;
use super::player;

/// `FUN_140034ee0` via the translation callback: the string-table entry for
/// a non-empty key (the key itself when the table has none).
pub fn translate_text(vm: &Vm, text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let shared = player::state(vm);
    let p = shared.borrow();
    let table = p.strings.as_ref()?;
    Some(table.lookup(text).to_string())
}

/// The HTML/text substitution hook (`FUN_1400358c0` -> `FUN_140034e40`
/// with `'@'`): text that starts with `@` becomes the entry for the rest.
pub fn substitute(vm: &Vm, text: &str) -> String {
    match text.strip_prefix('@') {
        Some(key) => translate_text(vm, key).unwrap_or_else(|| text.to_string()),
        None => text.to_string(),
    }
}

pub fn install(vm: &mut Vm) {
    let mut s = Spec::new("", "IggyFunctions", None);
    s.statics = vec![
        // `FUN_1410af650`: the translated string, or the argument when the
        // callback declines.
        ("translate", |vm, _, a| {
            let v = arg(a, 0);
            let text = vm.to_string(&v)?;
            Ok(match translate_text(vm, &text) {
                Some(t) => t.into(),
                None => v,
            })
        }),
        // `FUN_141015000`: DO byte1 bits 4..6 = flags & 7.
        ("setHittestProperties", |vm, _, a| {
            let flags = vm.to_int(&arg(a, 1))? & 7;
            if let Some(o) = arg(a, 0).as_object() {
                if let Some(mut d) = display(o) {
                    d.hittest = flags as u8;
                }
            }
            Ok(Value::Undefined)
        }),
        ("getHittestProperties", |_, _, a| {
            let flags = arg(a, 0).as_object().and_then(|o| super::display::display_ref(o).map(|d| d.hittest)).unwrap_or(0);
            Ok((flags as i32).into())
        }),
        // `FUN_141014f60`: 0 clears the custom depth; otherwise it is kept as f32.
        ("setDepth", |vm, _, a| {
            let depth = vm.to_number(&arg(a, 1))? as f32;
            if let Some(o) = arg(a, 0).as_object() {
                if let Some(mut d) = display(o) {
                    d.custom_depth = (depth != 0.0).then_some(depth);
                }
            }
            Ok(Value::Undefined)
        }),
        ("getDepth", |_, _, a| {
            let depth = arg(a, 0).as_object().and_then(|o| super::display::display_ref(o).and_then(|d| d.custom_depth)).unwrap_or(0.0);
            Ok((depth as f64).into())
        }),
        // `FUN_141015070`: DO 0xbb&0x40 = !enable.
        ("setObjectAntialiasingEnable", |_, _, a| {
            let enable = arg(a, 1).to_boolean();
            if let Some(o) = arg(a, 0).as_object() {
                if let Some(mut d) = display(o) {
                    d.antialiasing = enable;
                }
            }
            Ok(Value::Undefined)
        }),
        // `FUN_141014ef0`: like getObjectsUnderPoint, skipping flag 4 objects.
        ("iggyGetObjectsUnderPoint", |vm, _, a| {
            let Some(container) = arg(a, 0).as_object().cloned() else { return Ok(Value::Null) };
            let p = arg(a, 1);
            let x = super::avm2::builtins::number_property(vm, &p, "x")?;
            let y = super::avm2::builtins::number_property(vm, &p, "y")?;
            let hits = super::hit::objects_under_point_masked(vm, &container, x, y, 4);
            Ok(Value::Object(vm.array(hits.into_iter().map(Value::Object).collect())))
        }),
        // `FUN_1410150f0` ("O!s?dd"): bind a named game texture to a Bitmap.
        ("setTextureForBitmap", |vm, _, a| {
            let name = match arg(a, 1) {
                Value::Null | Value::Undefined => None,
                v => Some(vm.to_string(&v)?.to_string()),
            };
            if let Some(o) = arg(a, 0).as_object() {
                if let Some(mut d) = display(o) {
                    d.game_texture = name;
                }
            }
            Ok(Value::Undefined)
        }),
    ];
    define(vm, s);
}
