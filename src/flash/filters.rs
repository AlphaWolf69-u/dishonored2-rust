//! `flash.filters`: filter value classes (rendered by the display backend).
use super::avm2::builtins::{argument as arg, define, native_class, Spec};
use super::avm2::name::Name;
use super::avm2::object::{NativeClass, NativeFn};
use super::avm2::value::Value;
use super::avm2::{Result, Vm};

/// Public fields and their constructor defaults, in constructor order.
const FILTERS: &[(&str, &[(&str, f64)])] = &[
    ("BlurFilter", &[("blurX", 4.0), ("blurY", 4.0), ("quality", 1.0)]),
    (
        "DropShadowFilter",
        &[
            ("distance", 4.0),
            ("angle", 45.0),
            ("color", 0.0),
            ("alpha", 1.0),
            ("blurX", 4.0),
            ("blurY", 4.0),
            ("strength", 1.0),
            ("quality", 1.0),
            ("inner", 0.0),
            ("knockout", 0.0),
            ("hideObject", 0.0),
        ],
    ),
    (
        "GlowFilter",
        &[("color", 16711680.0), ("alpha", 1.0), ("blurX", 6.0), ("blurY", 6.0), ("strength", 2.0), ("quality", 1.0), ("inner", 0.0), ("knockout", 0.0)],
    ),
    (
        "BevelFilter",
        &[
            ("distance", 4.0),
            ("angle", 45.0),
            ("highlightColor", 16777215.0),
            ("highlightAlpha", 1.0),
            ("shadowColor", 0.0),
            ("shadowAlpha", 1.0),
            ("blurX", 4.0),
            ("blurY", 4.0),
            ("strength", 1.0),
            ("quality", 1.0),
            ("knockout", 0.0),
        ],
    ),
];

/// Fields holding arrays (gradient filters, colour matrix).
const ARRAY_FILTERS: &[(&str, &[&str], &[(&str, f64)])] = &[
    ("ColorMatrixFilter", &["matrix"], &[]),
    ("GradientGlowFilter", &["colors", "alphas", "ratios"], &[("distance", 4.0), ("angle", 45.0), ("blurX", 4.0), ("blurY", 4.0), ("strength", 1.0), ("quality", 1.0), ("knockout", 0.0)]),
    ("GradientBevelFilter", &["colors", "alphas", "ratios"], &[("distance", 4.0), ("angle", 45.0), ("blurX", 4.0), ("blurY", 4.0), ("strength", 1.0), ("quality", 1.0), ("knockout", 0.0)]),
    ("ConvolutionFilter", &["matrix"], &[("matrixX", 0.0), ("matrixY", 0.0), ("divisor", 1.0), ("bias", 0.0)]),
];

fn clone(vm: &mut Vm, this: &Value, _: &[Value]) -> Result<Value> {
    let class = this.as_object().and_then(|o| o.class()).unwrap();
    let copy = vm.construct_class(&class, &[])?;
    for (name, _) in class.instance.bindings.iter().filter(|(q, b)| q.ns.is_public() && matches!(b, super::avm2::object::Binding::Slot(_))) {
        let n = Name::public(&name.name);
        let v = vm.get_property(this, &n)?;
        let v = match &v {
            Value::Object(o) if matches!(o.borrow().kind, super::avm2::object::Kind::Array(_)) => {
                vm.call_property(&v, &Name::public("concat"), &[])?
            }
            _ => v,
        };
        vm.set_property(&copy, &n, v)?;
    }
    Ok(copy)
}

pub fn install(vm: &mut Vm) {
    let object = vm.b().object.clone();
    let mut s = Spec::new("flash.filters", "BitmapFilter", Some(object));
    s.methods = vec![("clone", clone as NativeFn)];
    let base = define(vm, s);
    for (name, fields) in FILTERS {
        let mut s = Spec::new("flash.filters", name, Some(base.clone()));
        s.vars = fields
            .iter()
            .map(|(f, d)| (*f, if matches!(*f, "inner" | "knockout" | "hideObject") { Value::from(*d != 0.0) } else { Value::from(*d) }))
            .collect();
        s.native = NativeClass { allocate: None, call: None, init: Some(init_fields) };
        s.methods = vec![("clone", clone as NativeFn)];
        define(vm, s);
    }
    for (name, arrays, fields) in ARRAY_FILTERS {
        let mut s = Spec::new("flash.filters", name, Some(base.clone()));
        let mut vars: Vec<(&'static str, Value)> = arrays.iter().map(|a| (*a, Value::Null)).collect();
        vars.extend(fields.iter().map(|(f, d)| (*f, if *f == "knockout" { Value::from(*d != 0.0) } else { Value::from(*d) })));
        s.vars = vars;
        s.native = NativeClass { allocate: None, call: None, init: Some(init_fields) };
        s.methods = vec![("clone", clone as NativeFn)];
        define(vm, s);
    }
    let mut s = Spec::new("flash.filters", "BitmapFilterQuality", Some(vm.b().object.clone()));
    s.constants = vec![("LOW", 1.into()), ("MEDIUM", 2.into()), ("HIGH", 3.into())];
    define(vm, s);
    let mut s = Spec::new("flash.filters", "BitmapFilterType", Some(vm.b().object.clone()));
    s.constants = vec![("FULL", "full".into()), ("INNER", "inner".into()), ("OUTER", "outer".into())];
    define(vm, s);
}

/// Constructor arguments fill the public fields in declaration order.
fn init_fields(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    let class = this.as_object().and_then(|o| o.class()).unwrap();
    let mut base = Some(class.clone());
    while let Some(c) = &base {
        if c.name.ns.uri() == "flash.filters" {
            break;
        }
        base = c.super_class.clone();
    }
    let name = base.map(|c| c.name.name.to_string()).unwrap_or_default();
    let order: Vec<&str> = FILTERS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, f)| f.iter().map(|(n, _)| *n).collect())
        .or_else(|| {
            ARRAY_FILTERS.iter().find(|(n, _, _)| *n == name).map(|(n, arrays, fields)| match *n {
                // Constructor orders of the array-valued filters.
                "ColorMatrixFilter" => vec!["matrix"],
                "ConvolutionFilter" => vec!["matrixX", "matrixY", "matrix", "divisor", "bias"],
                _ => {
                    let mut o = vec!["distance", "angle"];
                    o.extend(arrays.iter().copied());
                    o.extend(fields.iter().skip(2).map(|(f, _)| *f));
                    o
                }
            })
        })
        .unwrap_or_default();
    for (i, field) in order.iter().enumerate() {
        if let Some(v) = a.get(i) {
            if !matches!(v, Value::Undefined) {
                vm.set_property(this, &Name::public(field), v.clone())?;
            }
        }
    }
    let _ = (arg, native_class);
    Ok(Value::Undefined)
}
