//! AS3 top-level classes and functions (ECMA-262 3rd edition library plus
//! the AS3 additions). Methods live on prototypes as non-enumerable
//! properties and as `AS3`-namespace traits, matching playerglobal.
use super::name::{Name, Namespace, QName};
use super::object::{Binding, Class, Kind, Method, NativeClass, NativeFn, Obj, ObjectData, Traits, Vector};
use super::value::{self, Value};
use super::{Builtins, Error, Result, Vm};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const AS3: &str = "http://adobe.com/AS3/2006/builtin";

/// Declarative description of a native class.
pub struct Spec {
    pub package: &'static str,
    pub name: &'static str,
    pub base: Option<Rc<Class>>,
    pub sealed: bool,
    /// Interfaces declare their methods in the `package:Name` namespace.
    pub interface: bool,
    pub implements: Vec<Rc<Class>>,
    pub native: NativeClass,
    /// Instance methods placed on the prototype and as AS3 traits.
    pub proto: Vec<(&'static str, NativeFn)>,
    /// Instance accessors (public traits).
    pub getters: Vec<(&'static str, NativeFn)>,
    pub setters: Vec<(&'static str, NativeFn)>,
    /// Instance methods as public traits (flash.* API style).
    pub methods: Vec<(&'static str, NativeFn)>,
    /// Public `var` slots with initial values.
    pub vars: Vec<(&'static str, Value)>,
    /// Static methods, accessors and constants.
    pub statics: Vec<(&'static str, NativeFn)>,
    pub static_getters: Vec<(&'static str, NativeFn)>,
    pub constants: Vec<(&'static str, Value)>,
}

impl Spec {
    pub fn new(package: &'static str, name: &'static str, base: Option<Rc<Class>>) -> Self {
        Spec {
            package,
            name,
            base,
            sealed: true,
            interface: false,
            implements: Vec::new(),
            native: NativeClass { allocate: None, call: None, init: None },
            proto: Vec::new(),
            getters: Vec::new(),
            setters: Vec::new(),
            methods: Vec::new(),
            vars: Vec::new(),
            statics: Vec::new(),
            static_getters: Vec::new(),
            constants: Vec::new(),
        }
    }
}

fn accessor(traits: &mut Traits, q: QName, get: Option<Rc<Method>>, set: Option<Rc<Method>>) {
    let entry = traits.bindings.entry(q).or_insert(Binding::Accessor { get: None, set: None });
    if let Binding::Accessor { get: g, set: s } = entry {
        if get.is_some() {
            *g = get;
        }
        if set.is_some() {
            *s = set;
        }
    }
}

/// Define a native class and register it as a global definition.
pub fn define(vm: &mut Vm, spec: Spec) -> Rc<Class> {
    let mut instance = Traits::default();
    if let Some(b) = &spec.base {
        instance.bindings = b.instance.bindings.clone();
        instance.slots = b.instance.slots.clone();
    }
    if instance.slots.is_empty() {
        instance.slots.push(None);
    }
    let as3 = Namespace::package(AS3);
    for (n, f) in &spec.proto {
        instance.bindings.insert(QName::new(as3.clone(), n), Binding::Method(Method::native(n, *f)));
    }
    let member_ns = if spec.interface {
        let uri = if spec.package.is_empty() { spec.name.to_string() } else { format!("{}:{}", spec.package, spec.name) };
        Namespace::Public(uri.into())
    } else {
        Namespace::public()
    };
    for (n, f) in &spec.methods {
        instance.bindings.insert(QName::new(member_ns.clone(), n), Binding::Method(Method::native(n, *f)));
    }
    // Interface names alias the public implementation, as for ABC classes.
    for iface in &spec.implements {
        for q in iface.instance.bindings.keys() {
            if let Some(b) = instance.bindings.get(&QName::public(&q.name)).cloned() {
                instance.bindings.entry(q.clone()).or_insert(b);
            }
        }
    }
    for (n, f) in &spec.getters {
        accessor(&mut instance, QName::public(n), Some(Method::native(n, *f)), None);
    }
    for (n, v) in &spec.vars {
        let id = instance.slots.len() as u32;
        instance.slots.push(Some(super::object::Slot {
            name: QName::public(n),
            type_name: 0,
            unit: None,
            default: v.clone(),
            constant: false,
        }));
        instance.bindings.insert(QName::public(n), Binding::Slot(id));
    }
    for (n, f) in &spec.setters {
        accessor(&mut instance, QName::public(n), None, Some(Method::native(n, *f)));
    }
    // Class objects are Class instances: statics extend its traits.
    let mut statics = Traits::default();
    if let Some(b) = &vm.builtins {
        statics.bindings = b.class.instance.bindings.clone();
    }
    statics.slots.push(None);
    for (n, f) in &spec.statics {
        statics.bindings.insert(QName::public(n), Binding::Method(Method::native(n, *f)));
    }
    for (n, f) in &spec.static_getters {
        accessor(&mut statics, QName::public(n), Some(Method::native(n, *f)), None);
    }
    for (n, v) in &spec.constants {
        let id = statics.slots.len() as u32;
        statics.slots.push(Some(super::object::Slot {
            name: QName::public(n),
            type_name: 0,
            unit: None,
            default: v.clone(),
            constant: true,
        }));
        statics.bindings.insert(QName::public(n), Binding::Slot(id));
    }
    let name = QName::new(Namespace::package(spec.package), spec.name);
    let class = Rc::new(Class {
        name: name.clone(),
        super_class: spec.base.clone(),
        interfaces: spec.implements.clone(),
        instance: Rc::new(instance),
        statics: Rc::new(statics),
        instance_init: None,
        sealed: spec.sealed,
        interface: spec.interface,
        native: Some(spec.native),
        object: RefCell::default(),
        prototype: RefCell::default(),
        scope: RefCell::default(),
        applied: None,
    });
    let object = vm.finish_class(&class, Rc::new([]));
    for (n, f) in &spec.proto {
        let func = vm.native_function(n, *f);
        let proto = class.prototype();
        let mut p = proto.borrow_mut();
        p.dynamic.set((*n).into(), Value::Object(func));
        p.dynamic.set_enumerable(n, false);
    }
    vm.natives.insert(name, Value::Object(object));
    class
}

/// Bootstrap a class before `Class`/`Object` exist (their objects are fixed up).
fn bootstrap(name: &str, base: Option<Rc<Class>>, sealed: bool, native: NativeClass, mut instance: Traits) -> Rc<Class> {
    instance.slots.push(None);
    if let Some(b) = &base {
        for (q, binding) in &b.instance.bindings {
            instance.bindings.entry(q.clone()).or_insert_with(|| binding.clone());
        }
    }
    let mut statics = Traits::default();
    statics.slots.push(None);
    Rc::new(Class {
        name: QName::public(name),
        super_class: base,
        interfaces: Vec::new(),
        instance: Rc::new(instance),
        statics: Rc::new(statics),
        instance_init: None,
        sealed,
        interface: false,
        native: Some(native),
        object: RefCell::default(),
        prototype: RefCell::default(),
        scope: RefCell::default(),
        applied: None,
    })
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or_default()
}

fn none() -> NativeClass {
    NativeClass { allocate: None, call: None, init: None }
}

pub fn install(vm: &mut Vm) {
    // Object, Function and Class reference each other; build them first.
    let object = bootstrap("Object", None, false, NativeClass { allocate: None, call: Some(object_call), init: None }, object_traits());
    let class = bootstrap("Class", Some(object.clone()), false, none(), Traits::default());
    let function = bootstrap("Function", Some(object.clone()), false, none(), function_traits());
    for c in [&object, &class, &function] {
        let proto_parent = c.super_class.as_ref().map(|s| s.prototype());
        let proto = Obj::new(ObjectData::plain(proto_parent));
        *c.prototype.borrow_mut() = Some(proto);
    }
    for c in [&object, &class, &function] {
        let o = Obj::new(ObjectData {
            class: Some(class.clone()),
            traits: Some(c.statics.clone()),
            kind: Kind::Class(c.clone()),
            ..ObjectData::plain(Some(class.prototype()))
        });
        c.prototype().borrow_mut().class = Some(object.clone());
        c.prototype().borrow_mut().dynamic.set("constructor".into(), Value::Object(o.clone()));
        c.prototype().borrow_mut().dynamic.set_enumerable("constructor", false);
        *c.object.borrow_mut() = Some(o.clone());
        vm.natives.insert(c.name.clone(), Value::Object(o));
    }
    // Placeholder builtins so later definitions can create functions/arrays.
    let placeholder = object.clone();
    vm.builtins = Some(Builtins {
        object: object.clone(),
        function: function.clone(),
        class: class.clone(),
        array: placeholder.clone(),
        string: placeholder.clone(),
        number: placeholder.clone(),
        int: placeholder.clone(),
        uint: placeholder.clone(),
        boolean: placeholder.clone(),
        namespace: placeholder.clone(),
        vector: placeholder.clone(),
        xml: placeholder.clone(),
        xml_list: placeholder.clone(),
        error: placeholder.clone(),
        type_error: placeholder.clone(),
        reference_error: placeholder.clone(),
        range_error: placeholder.clone(),
        argument_error: placeholder,
    });
    install_object_methods(vm, &object, &function);

    let mut s = Spec::new("", "Array", Some(object.clone()));
    s.sealed = false;
    s.native = NativeClass { allocate: Some(|_, _| Ok(Kind::Array(Vec::new()))), call: Some(array_call), init: Some(array_init) };
    s.proto = array_methods();
    s.getters = vec![("length", array_length)];
    s.setters = vec![("length", array_set_length)];
    s.constants = vec![
        ("CASEINSENSITIVE", 1.into()),
        ("DESCENDING", 2.into()),
        ("UNIQUESORT", 4.into()),
        ("RETURNINDEXEDARRAY", 8.into()),
        ("NUMERIC", 16.into()),
    ];
    let array = define(vm, s);
    vm.builtins.as_mut().unwrap().array = array;

    let mut s = Spec::new("", "String", Some(object.clone()));
    s.native = NativeClass { allocate: None, call: Some(string_call), init: Some(primitive_init::<0>) };
    s.proto = string_methods();
    s.getters = vec![("length", string_length)];
    s.statics = vec![("fromCharCode", string_from_char_code)];
    let string = define(vm, s);
    vm.builtins.as_mut().unwrap().string = string;

    let mut s = Spec::new("", "Number", Some(object.clone()));
    s.native = NativeClass { allocate: None, call: Some(number_call), init: Some(primitive_init::<1>) };
    s.proto = number_methods();
    s.constants = vec![
        ("NaN", f64::NAN.into()),
        ("MAX_VALUE", f64::MAX.into()),
        ("MIN_VALUE", 5e-324f64.into()),
        ("POSITIVE_INFINITY", f64::INFINITY.into()),
        ("NEGATIVE_INFINITY", f64::NEG_INFINITY.into()),
    ];
    let number = define(vm, s);
    vm.builtins.as_mut().unwrap().number = number;

    let mut s = Spec::new("", "int", Some(object.clone()));
    s.native = NativeClass { allocate: None, call: Some(int_call), init: Some(primitive_init::<2>) };
    s.proto = number_methods();
    s.constants = vec![("MAX_VALUE", i32::MAX.into()), ("MIN_VALUE", i32::MIN.into())];
    let int = define(vm, s);
    vm.builtins.as_mut().unwrap().int = int;

    let mut s = Spec::new("", "uint", Some(object.clone()));
    s.native = NativeClass { allocate: None, call: Some(uint_call), init: Some(primitive_init::<3>) };
    s.proto = number_methods();
    s.constants = vec![("MAX_VALUE", u32::MAX.into()), ("MIN_VALUE", 0.into())];
    let uint = define(vm, s);
    vm.builtins.as_mut().unwrap().uint = uint;

    let mut s = Spec::new("", "Boolean", Some(object.clone()));
    s.native = NativeClass { allocate: None, call: Some(boolean_call), init: Some(primitive_init::<4>) };
    s.proto = vec![("toString", boolean_to_string), ("valueOf", boolean_value_of)];
    let boolean = define(vm, s);
    vm.builtins.as_mut().unwrap().boolean = boolean;

    let mut s = Spec::new("", "Namespace", Some(object.clone()));
    s.native = NativeClass { allocate: Some(|_, _| Ok(Kind::Namespace(Namespace::public()))), call: None, init: Some(namespace_init) };
    s.getters = vec![("uri", namespace_uri), ("prefix", |_, _, _| Ok(Value::Undefined))];
    s.proto = vec![("toString", namespace_uri)];
    let namespace = define(vm, s);
    vm.builtins.as_mut().unwrap().namespace = namespace;

    let mut s = Spec::new("", "Error", Some(object.clone()));
    s.sealed = false;
    s.native = NativeClass { allocate: None, call: Some(error_call), init: Some(error_init) };
    s.proto = vec![("toString", error_to_string)];
    s.methods = vec![("getStackTrace", |_, _, _| Ok(Value::Null))];
    s.getters = vec![("errorID", error_id)];
    let error = define(vm, s);
    error.prototype().borrow_mut().dynamic.set("name".into(), "Error".into());
    error.prototype().borrow_mut().dynamic.set("message".into(), "".into());
    vm.builtins.as_mut().unwrap().error = error.clone();
    let mut errors = HashMap::new();
    for name in [
        "TypeError",
        "ReferenceError",
        "RangeError",
        "ArgumentError",
        "VerifyError",
        "SecurityError",
        "SyntaxError",
        "EvalError",
        "URIError",
        "DefinitionError",
        "UninitializedError",
    ] {
        let mut s = Spec::new("", name, Some(error.clone()));
        s.sealed = false;
        s.native = NativeClass { allocate: None, call: Some(error_call), init: Some(error_init) };
        let c = define(vm, s);
        c.prototype().borrow_mut().dynamic.set("name".into(), name.into());
        errors.insert(name, c);
    }
    {
        let b = vm.builtins.as_mut().unwrap();
        b.type_error = errors["TypeError"].clone();
        b.reference_error = errors["ReferenceError"].clone();
        b.range_error = errors["RangeError"].clone();
        b.argument_error = errors["ArgumentError"].clone();
    }

    let mut s = Spec::new("__AS3__.vec", "Vector", Some(object.clone()));
    s.sealed = false;
    s.native = NativeClass { allocate: Some(vector_alloc), call: None, init: Some(vector_init) };
    s.proto = vector_methods();
    s.getters = vec![("length", vector_length), ("fixed", vector_fixed)];
    s.setters = vec![("length", vector_set_length), ("fixed", vector_set_fixed)];
    let vector = define(vm, s);
    vm.builtins.as_mut().unwrap().vector = vector;

    let mut s = Spec::new("flash.utils", "Dictionary", Some(object.clone()));
    s.sealed = false;
    s.native = NativeClass { allocate: Some(|_, _| Ok(Kind::Dictionary(Vec::new()))), call: None, init: Some(|_, _, _| Ok(Value::Undefined)) };
    define(vm, s);

    super::xml::install(vm);

    let mut math = Spec::new("", "Math", Some(object.clone()));
    math.statics = math_functions();
    math.constants = vec![
        ("E", std::f64::consts::E.into()),
        ("LN10", std::f64::consts::LN_10.into()),
        ("LN2", std::f64::consts::LN_2.into()),
        ("LOG10E", std::f64::consts::LOG10_E.into()),
        ("LOG2E", std::f64::consts::LOG2_E.into()),
        ("PI", std::f64::consts::PI.into()),
        ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2.into()),
        ("SQRT2", std::f64::consts::SQRT_2.into()),
    ];
    define(vm, math);

    let globals: [(&str, &str, NativeFn); 11] = [
        ("", "trace", global_trace),
        ("", "isNaN", |vm, _, a| Ok(vm.to_number(&arg(a, 0))?.is_nan().into())),
        ("", "isFinite", |vm, _, a| Ok(vm.to_number(&arg(a, 0))?.is_finite().into())),
        ("", "parseInt", global_parse_int),
        ("", "parseFloat", global_parse_float),
        ("flash.utils", "getQualifiedClassName", get_qualified_class_name),
        ("flash.utils", "getQualifiedSuperclassName", get_qualified_superclass_name),
        ("flash.utils", "getDefinitionByName", get_definition_by_name),
        ("flash.utils", "describeType", super::xml::describe_type),
        ("flash.utils", "getTimer", |vm, _, _| Ok(crate::flash::host_timer(vm).into())),
        ("", "escape", global_escape),
    ];
    for (package, name, f) in globals {
        let func = vm.native_function(name, f);
        vm.natives.insert(QName::new(Namespace::package(package), name), Value::Object(func));
    }
    vm.natives.insert(QName::public("NaN"), f64::NAN.into());
    vm.natives.insert(QName::public("Infinity"), f64::INFINITY.into());
    vm.natives.insert(QName::public("undefined"), Value::Undefined);
    let as3 = namespace_object(vm, Namespace::package(AS3));
    vm.natives.insert(QName::public("AS3"), as3);
}

/// The holder object through which `findpropstrict` returns natives.
pub fn native_globals(vm: &mut Vm) -> Obj {
    thread_local! {
        static HOLDER: RefCell<Option<Obj>> = const { RefCell::new(None) };
    }
    if let Some(h) = HOLDER.with(|h| h.borrow().clone()) {
        if h.borrow().traits.as_ref().is_some_and(|t| t.bindings.len() == vm.natives.len()) {
            return h;
        }
    }
    let mut traits = Traits::default();
    traits.slots.push(None);
    let mut slots = vec![Value::Undefined];
    for (q, v) in &vm.natives {
        traits.bindings.insert(q.clone(), Binding::Slot(slots.len() as u32));
        traits.slots.push(Some(super::object::Slot { name: q.clone(), type_name: 0, unit: None, default: v.clone(), constant: true }));
        slots.push(v.clone());
    }
    let holder = Obj::new(ObjectData { traits: Some(Rc::new(traits)), slots, ..ObjectData::plain(None) });
    HOLDER.with(|h| *h.borrow_mut() = Some(holder.clone()));
    holder
}

pub fn namespace_object(vm: &mut Vm, ns: Namespace) -> Value {
    let class = vm.b().namespace.clone();
    Value::Object(Obj::new(ObjectData {
        class: Some(class.clone()),
        traits: Some(class.instance.clone()),
        kind: Kind::Namespace(ns),
        ..ObjectData::plain(Some(class.prototype()))
    }))
}

// ---- Object ----

fn object_call(vm: &mut Vm, _: &Value, args: &[Value]) -> Result<Value> {
    let v = arg(args, 0);
    Ok(if v.is_nullish() { Value::Object(vm.plain_object()) } else { v })
}

/// Object's AS3-namespace instance methods (Iggy registers these three
/// with `FUN_14104c470` as well as on `Object.prototype`).
fn object_traits() -> Traits {
    let mut t = Traits::default();
    let as3 = Namespace::package(AS3);
    for (n, f) in object_methods().into_iter().take(3) {
        t.bindings.insert(QName::new(as3.clone(), n), Binding::Method(Method::native(n, f)));
    }
    t
}

fn object_methods() -> [(&'static str, NativeFn); 7] {
    [
        ("hasOwnProperty", |vm, this, a| {
            let name = vm.to_string(&arg(a, 0))?;
            let n = Name::public(&name);
            Ok(match this {
                Value::Object(o) => {
                    let own = o.borrow().dynamic.contains(&name)
                        || o.borrow().traits.as_ref().is_some_and(|t| t.find(&name, &n.set).is_some());
                    own || matches!(&o.borrow().kind, Kind::Array(items) if name.parse::<usize>().is_ok_and(|i| i < items.len()))
                        || matches!(&o.borrow().kind, Kind::Dictionary(_)) && vm.dictionary_get(o, &Value::String(name.clone())).is_some_and(|v| !matches!(v, Value::Undefined))
                }
                _ => vm.has_property(this, &n)?,
            }
            .into())
        }),
        ("isPrototypeOf", |_, this, a| {
            let mut p = arg(a, 0).as_object().and_then(|o| o.borrow().proto.clone());
            while let Some(o) = p {
                if Some(&o) == this.as_object() {
                    return Ok(true.into());
                }
                p = o.borrow().proto.clone();
            }
            Ok(false.into())
        }),
        ("propertyIsEnumerable", |vm, this, a| {
            let name = vm.to_string(&arg(a, 0))?;
            Ok(this.as_object().is_some_and(|o| o.borrow().dynamic.is_enumerable(&name)).into())
        }),
        ("setPropertyIsEnumerable", |vm, this, a| {
            let name = vm.to_string(&arg(a, 0))?;
            if let Some(o) = this.as_object() {
                o.borrow_mut().dynamic.set_enumerable(&name, arg(a, 1).to_boolean());
            }
            Ok(Value::Undefined)
        }),
        ("toString", object_to_string),
        ("toLocaleString", object_to_string),
        ("valueOf", |_, this, _| Ok(this.clone())),
    ]
}

fn install_object_methods(vm: &mut Vm, object: &Rc<Class>, function: &Rc<Class>) {
    let methods = object_methods();
    let proto = object.prototype();
    for (n, f) in methods {
        let func = vm.native_function(n, f);
        proto.borrow_mut().dynamic.set(n.into(), Value::Object(func));
        proto.borrow_mut().dynamic.set_enumerable(n, false);
    }
    let fmethods: [(&str, NativeFn); 3] = [
        ("call", |vm, this, a| {
            let receiver = arg(a, 0);
            vm.call(this, receiver, a.get(1..).unwrap_or_default())
        }),
        ("apply", |vm, this, a| {
            let receiver = arg(a, 0);
            let list = match arg(a, 1) {
                Value::Object(o) => match &o.borrow().kind {
                    Kind::Array(items) => items.clone(),
                    Kind::Vector(v) => v.items.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            vm.call(this, receiver, &list)
        }),
        ("toString", |_, _, _| Ok("function Function() {}".into())),
    ];
    let fproto = function.prototype();
    for (n, f) in fmethods {
        let func = vm.native_function(n, f);
        fproto.borrow_mut().dynamic.set(n.into(), Value::Object(func));
        fproto.borrow_mut().dynamic.set_enumerable(n, false);
    }
}

/// `Function.length`: the declared parameter count.
fn function_traits() -> Traits {
    let mut t = Traits::default();
    accessor(
        &mut t,
        QName::public("length"),
        Some(Method::native("length", |_, this, _| {
            let Some(o) = this.as_object() else { return Ok(0.into()) };
            let d = o.borrow();
            let Kind::Function(f) = &d.kind else { return Ok(0.into()) };
            Ok(match &f.method.kind {
                super::object::MethodKind::Bytecode { unit, index } => (unit.abc.methods[*index as usize].params.len() as u32).into(),
                super::object::MethodKind::Native(_) => 0.into(),
            })
        })),
        None,
    );
    t
}

fn object_to_string(vm: &mut Vm, this: &Value, _: &[Value]) -> Result<Value> {
    Ok(match this {
        Value::Object(o) => match o.as_class() {
            Some(c) => format!("[class {}]", c.name.name).into(),
            None => format!("[object {}]", o.class().map_or_else(|| "Object".into(), |c| c.name.name.to_string())).into(),
        },
        other => Value::String(vm.to_string(other)?),
    })
}

// ---- Array ----

fn items(this: &Value) -> Vec<Value> {
    match this.as_object().map(|o| o.borrow()) {
        Some(d) => match &d.kind {
            Kind::Array(items) => items.clone(),
            Kind::Vector(v) => v.items.clone(),
            _ => Vec::new(),
        },
        None => Vec::new(),
    }
}

fn with_items<R>(this: &Value, f: impl FnOnce(&mut Vec<Value>) -> R) -> Option<R> {
    let o = this.as_object()?;
    let mut d = o.borrow_mut();
    match &mut d.kind {
        Kind::Array(items) => Some(f(items)),
        Kind::Vector(v) => Some(f(&mut v.items)),
        _ => None,
    }
}

fn array_call(vm: &mut Vm, _: &Value, args: &[Value]) -> Result<Value> {
    let o = vm.array(Vec::new());
    array_init(vm, &Value::Object(o.clone()), args)?;
    Ok(Value::Object(o))
}

fn array_init(vm: &mut Vm, this: &Value, args: &[Value]) -> Result<Value> {
    let initial = if let [Value::Number(n)] = args {
        if n.fract() != 0.0 || *n < 0.0 || *n > u32::MAX as f64 {
            return Err(vm.range_error(1005, format!("Array index is not a positive integer ({n}).")));
        }
        vec![Value::Undefined; *n as usize]
    } else {
        args.to_vec()
    };
    with_items(this, |items| *items = initial);
    Ok(Value::Undefined)
}

fn array_length(_: &mut Vm, this: &Value, _: &[Value]) -> Result<Value> {
    Ok(with_items(this, |i| i.len() as u32).unwrap_or(0).into())
}

fn array_set_length(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    let n = vm.to_uint(&arg(a, 0))? as usize;
    with_items(this, |i| i.resize(n, Value::Undefined));
    Ok(Value::Undefined)
}

fn relative_index(n: f64, len: usize) -> usize {
    let n = value::to_integer(n);
    if n < 0.0 { (len as f64 + n).max(0.0) as usize } else { (n as usize).min(len) }
}

fn join(vm: &mut Vm, items: &[Value], sep: &str) -> Result<String> {
    let mut out = String::new();
    for (i, v) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(sep);
        }
        if !v.is_nullish() {
            out.push_str(&vm.to_string(v)?);
        }
    }
    Ok(out)
}

fn callback_each(vm: &mut Vm, this: &Value, a: &[Value], mut visit: impl FnMut(&Value, Value) -> bool) -> Result<()> {
    let f = arg(a, 0);
    let receiver = arg(a, 1);
    for (i, v) in items(this).into_iter().enumerate() {
        let r = vm.call(&f, receiver.clone(), &[v.clone(), Value::from(i as u32), this.clone()])?;
        if !visit(&v, r) {
            break;
        }
    }
    Ok(())
}

fn array_methods() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("push", |vm, this, a| {
            if let Some(Err(e)) = this.as_object().map(|o| check_vector_push(vm, o, a)) {
                return Err(e);
            }
            Ok(with_items(this, |i| {
                i.extend_from_slice(a);
                i.len() as u32
            })
            .unwrap_or(0)
            .into())
        }),
        ("pop", |_, this, _| Ok(with_items(this, |i| i.pop()).flatten().unwrap_or_default())),
        ("shift", |_, this, _| Ok(with_items(this, |i| (!i.is_empty()).then(|| i.remove(0))).flatten().unwrap_or_default())),
        ("unshift", |_, this, a| {
            Ok(with_items(this, |i| {
                i.splice(0..0, a.iter().cloned());
                i.len() as u32
            })
            .unwrap_or(0)
            .into())
        }),
        ("slice", |vm, this, a| {
            let all = items(this);
            let start = relative_index(vm.to_number(&arg(a, 0))?, all.len());
            let end = match a.get(1) {
                Some(v) => relative_index(vm.to_number(v)?, all.len()),
                None => all.len(),
            };
            let part = all.get(start..end.max(start)).unwrap_or_default().to_vec();
            Ok(like(vm, this, part))
        }),
        ("splice", |vm, this, a| {
            let len = items(this).len();
            if a.is_empty() {
                return Ok(like(vm, this, Vec::new()));
            }
            let start = relative_index(vm.to_number(&arg(a, 0))?, len);
            let count = match a.get(1) {
                Some(v) => (value::to_integer(vm.to_number(v)?).max(0.0) as usize).min(len - start),
                None => len - start,
            };
            let removed = with_items(this, |i| i.splice(start..start + count, a.iter().skip(2).cloned()).collect()).unwrap_or_default();
            Ok(like(vm, this, removed))
        }),
        ("concat", |vm, this, a| {
            let mut out = items(this);
            for v in a {
                match v.as_object().map(|o| matches!(o.borrow().kind, Kind::Array(_) | Kind::Vector(_))) {
                    Some(true) => out.extend(items(v)),
                    _ => out.push(v.clone()),
                }
            }
            Ok(like(vm, this, out))
        }),
        ("join", |vm, this, a| {
            let sep = match a.first() {
                None | Some(Value::Undefined) => ",".into(),
                Some(v) => vm.to_string(v)?,
            };
            Ok(join(vm, &items(this), &sep)?.into())
        }),
        ("toString", |vm, this, _| Ok(join(vm, &items(this), ",")?.into())),
        ("reverse", |_, this, _| {
            with_items(this, |i| i.reverse());
            Ok(this.clone())
        }),
        ("indexOf", |vm, this, a| {
            let all = items(this);
            let from = match a.get(1) {
                Some(v) => relative_index(vm.to_number(v)?, all.len()),
                None => 0,
            };
            let needle = arg(a, 0);
            Ok(all.iter().enumerate().skip(from).find(|(_, v)| super::interpreter::strict_equals(v, &needle)).map_or(-1, |(i, _)| i as i32).into())
        }),
        ("lastIndexOf", |vm, this, a| {
            let all = items(this);
            let from = match a.get(1) {
                Some(v) => relative_index(vm.to_number(v)?, all.len()).min(all.len().saturating_sub(1)),
                None => all.len().saturating_sub(1),
            };
            let needle = arg(a, 0);
            Ok((0..=from).rev().find(|&i| all.get(i).is_some_and(|v| super::interpreter::strict_equals(v, &needle))).map_or(-1, |i| i as i32).into())
        }),
        ("forEach", |vm, this, a| {
            callback_each(vm, this, a, |_, _| true)?;
            Ok(Value::Undefined)
        }),
        ("every", |vm, this, a| {
            let mut all = true;
            callback_each(vm, this, a, |_, r| {
                all = r.to_boolean();
                all
            })?;
            Ok(all.into())
        }),
        ("some", |vm, this, a| {
            let mut any = false;
            callback_each(vm, this, a, |_, r| {
                any = r.to_boolean();
                !any
            })?;
            Ok(any.into())
        }),
        ("filter", |vm, this, a| {
            let mut out = Vec::new();
            callback_each(vm, this, a, |v, r| {
                if r.to_boolean() {
                    out.push(v.clone());
                }
                true
            })?;
            Ok(like(vm, this, out))
        }),
        ("map", |vm, this, a| {
            let mut out = Vec::new();
            callback_each(vm, this, a, |_, r| {
                out.push(r);
                true
            })?;
            Ok(like(vm, this, out))
        }),
        ("sort", array_sort),
        ("sortOn", array_sort_on),
    ]
}

/// A result of the same container kind as `this` (Array or same Vector type).
fn like(vm: &mut Vm, this: &Value, items: Vec<Value>) -> Value {
    if let Some(o) = this.as_object() {
        let class = o.class();
        let d = o.borrow();
        if let Kind::Vector(v) = &d.kind {
            let element = v.element.clone();
            drop(d);
            let class = class.unwrap_or_else(|| vm.b().vector.clone());
            return Value::Object(Obj::new(ObjectData {
                class: Some(class.clone()),
                traits: Some(class.instance.clone()),
                kind: Kind::Vector(Vector { element, fixed: false, items }),
                ..ObjectData::plain(Some(class.prototype()))
            }));
        }
    }
    Value::Object(vm.array(items))
}

const CASEINSENSITIVE: u32 = 1;
const DESCENDING: u32 = 2;
const UNIQUESORT: u32 = 4;
const RETURNINDEXEDARRAY: u32 = 8;
const NUMERIC: u32 = 16;

fn compare_default(vm: &mut Vm, a: &Value, b: &Value, options: u32) -> Result<std::cmp::Ordering> {
    use std::cmp::Ordering::*;
    // Undefined sorts last, as in ECMA-262 15.4.4.11.
    match (matches!(a, Value::Undefined), matches!(b, Value::Undefined)) {
        (true, true) => return Ok(Equal),
        (true, false) => return Ok(Greater),
        (false, true) => return Ok(Less),
        _ => {}
    }
    Ok(if options & NUMERIC != 0 {
        let x = vm.to_number(a)?;
        let y = vm.to_number(b)?;
        x.partial_cmp(&y).unwrap_or(Equal)
    } else {
        let mut x = vm.to_string(a)?.to_string();
        let mut y = vm.to_string(b)?.to_string();
        if options & CASEINSENSITIVE != 0 {
            x = x.to_lowercase();
            y = y.to_lowercase();
        }
        x.encode_utf16().cmp(y.encode_utf16())
    })
}

/// Sort with a fallible comparator (insertion into a merge sort).
fn sort_by(vm: &mut Vm, values: &mut [(usize, Value)], cmp: &mut dyn FnMut(&mut Vm, &Value, &Value) -> Result<std::cmp::Ordering>) -> Result<()> {
    let mut error = None;
    values.sort_by(|a, b| {
        if error.is_some() {
            return std::cmp::Ordering::Equal;
        }
        match cmp(vm, &a.1, &b.1) {
            Ok(o) => o,
            Err(e) => {
                error = Some(e);
                std::cmp::Ordering::Equal
            }
        }
    });
    match error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn finish_sort(vm: &mut Vm, this: &Value, mut sorted: Vec<(usize, Value)>, options: u32, unique_conflict: bool) -> Result<Value> {
    if options & UNIQUESORT != 0 && unique_conflict {
        return Ok(0.into());
    }
    if options & DESCENDING != 0 {
        sorted.reverse();
    }
    if options & RETURNINDEXEDARRAY != 0 {
        return Ok(Value::Object(vm.array(sorted.into_iter().map(|(i, _)| Value::from(i as u32)).collect())));
    }
    with_items(this, |items| *items = sorted.into_iter().map(|(_, v)| v).collect());
    Ok(this.clone())
}

fn array_sort(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    let (compare, options) = match a {
        [f @ Value::Object(o), rest @ ..] if o.is_function() => (Some(f.clone()), match rest.first() {
            Some(v) => vm.to_uint(v)?,
            None => 0,
        }),
        [v, ..] => (None, vm.to_uint(v)?),
        [] => (None, 0),
    };
    let mut values: Vec<(usize, Value)> = items(this).into_iter().enumerate().collect();
    let mut conflict = false;
    sort_by(vm, &mut values, &mut |vm, x, y| {
        let o = match &compare {
            Some(f) => {
                let r = vm.call(f, Value::Null, &[x.clone(), y.clone()])?;
                let n = vm.to_number(&r)?;
                n.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)
            }
            None => compare_default(vm, x, y, options)?,
        };
        conflict |= o == std::cmp::Ordering::Equal;
        Ok(o)
    })?;
    finish_sort(vm, this, values, options, conflict)
}

fn array_sort_on(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    let fields: Vec<Value> = match arg(a, 0) {
        Value::Object(o) if matches!(o.borrow().kind, Kind::Array(_)) => items(&Value::Object(o.clone())),
        v => vec![v],
    };
    let mut names = Vec::new();
    for f in &fields {
        names.push(vm.to_string(f)?);
    }
    let options: Vec<u32> = match arg(a, 1) {
        Value::Object(o) if matches!(o.borrow().kind, Kind::Array(_)) => {
            let mut out = Vec::new();
            for v in items(&Value::Object(o.clone())) {
                out.push(vm.to_uint(&v)?);
            }
            out
        }
        Value::Undefined => vec![0],
        v => vec![vm.to_uint(&v)?],
    };
    let mut values: Vec<(usize, Value)> = items(this).into_iter().enumerate().collect();
    let mut conflict = false;
    let first = options.first().copied().unwrap_or(0);
    sort_by(vm, &mut values, &mut |vm, x, y| {
        for (i, name) in names.iter().enumerate() {
            let opt = options.get(i).copied().unwrap_or(first);
            let fx = vm.get_property(x, &Name::public(name)).unwrap_or_default();
            let fy = vm.get_property(y, &Name::public(name)).unwrap_or_default();
            let mut o = compare_default(vm, &fx, &fy, opt)?;
            if opt & DESCENDING != 0 && i > 0 {
                o = o.reverse();
            }
            if o != std::cmp::Ordering::Equal {
                return Ok(o);
            }
        }
        conflict = true;
        Ok(std::cmp::Ordering::Equal)
    })?;
    finish_sort(vm, this, values, first, conflict)
}

// ---- Vector ----

fn vector_alloc(_: &mut Vm, class: &Rc<Class>) -> Result<Kind> {
    Ok(Kind::Vector(Vector { element: class.applied.clone().flatten(), fixed: false, items: Vec::new() }))
}

fn vector_init(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    let len = match a.first() {
        Some(v) => vm.to_uint(v)? as usize,
        None => 0,
    };
    let fixed = arg(a, 1).to_boolean();
    let element = this.as_object().and_then(|o| match &o.borrow().kind {
        Kind::Vector(v) => v.element.clone(),
        _ => None,
    });
    let fill = match &element {
        Some(e) => element_default(vm, e),
        None => Value::Undefined,
    };
    if let Some(o) = this.as_object() {
        if let Kind::Vector(v) = &mut o.borrow_mut().kind {
            v.items = vec![fill; len];
            v.fixed = fixed;
        }
    }
    Ok(Value::Undefined)
}

fn element_default(vm: &Vm, class: &Rc<Class>) -> Value {
    let b = vm.b();
    if Rc::ptr_eq(class, &b.int) || Rc::ptr_eq(class, &b.uint) {
        0.into()
    } else if Rc::ptr_eq(class, &b.number) {
        f64::NAN.into()
    } else if Rc::ptr_eq(class, &b.boolean) {
        false.into()
    } else {
        Value::Null
    }
}

fn check_vector_push(vm: &mut Vm, o: &Obj, a: &[Value]) -> Result<()> {
    let fixed = matches!(&o.borrow().kind, Kind::Vector(v) if v.fixed);
    if fixed {
        return Err(vm.range_error(1126, "Cannot change the length of a fixed Vector."));
    }
    Ok(()).and_then(|_| {
        let _ = a;
        Ok(())
    })
}

/// `v[i] = x`: coerce to the element type; `i == length` appends.
pub fn vector_set(vm: &mut Vm, o: &Obj, i: usize, value: Value) -> Result<()> {
    let (element, len, fixed) = match &o.borrow().kind {
        Kind::Vector(v) => (v.element.clone(), v.items.len(), v.fixed),
        _ => return Ok(()),
    };
    if i > len || (i == len && fixed) {
        return Err(vm.range_error(1125, format!("The index {i} is out of range {len}.")));
    }
    let value = match &element {
        Some(e) => vm.coerce(&value, e)?,
        None => value,
    };
    if let Kind::Vector(v) = &mut o.borrow_mut().kind {
        if i == v.items.len() {
            v.items.push(value);
        } else {
            v.items[i] = value;
        }
    }
    Ok(())
}

fn vector_length(_: &mut Vm, this: &Value, _: &[Value]) -> Result<Value> {
    Ok(with_items(this, |i| i.len() as u32).unwrap_or(0).into())
}

fn vector_set_length(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    let n = vm.to_uint(&arg(a, 0))? as usize;
    let Some(o) = this.as_object() else { return Ok(Value::Undefined) };
    let (element, fixed) = match &o.borrow().kind {
        Kind::Vector(v) => (v.element.clone(), v.fixed),
        _ => return Ok(Value::Undefined),
    };
    if fixed {
        return Err(vm.range_error(1126, "Cannot change the length of a fixed Vector."));
    }
    let fill = element.map_or(Value::Undefined, |e| element_default(vm, &e));
    with_items(this, |i| i.resize(n, fill));
    Ok(Value::Undefined)
}

fn vector_fixed(_: &mut Vm, this: &Value, _: &[Value]) -> Result<Value> {
    Ok(this.as_object().is_some_and(|o| matches!(&o.borrow().kind, Kind::Vector(v) if v.fixed)).into())
}

fn vector_set_fixed(_: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    if let Some(o) = this.as_object() {
        if let Kind::Vector(v) = &mut o.borrow_mut().kind {
            v.fixed = arg(a, 0).to_boolean();
        }
    }
    Ok(Value::Undefined)
}

fn vector_methods() -> Vec<(&'static str, NativeFn)> {
    // Vector shares Array's generic algorithms over its item list.
    array_methods().into_iter().filter(|(n, _)| *n != "sortOn").collect()
}

// ---- String ----

fn this_string(vm: &mut Vm, this: &Value) -> Result<Rc<str>> {
    vm.to_string(this)
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn from_utf16(u: &[u16]) -> Value {
    Value::from(String::from_utf16_lossy(u))
}

fn string_call(vm: &mut Vm, _: &Value, a: &[Value]) -> Result<Value> {
    Ok(match a.first() {
        Some(v) => Value::String(vm.to_string(v)?),
        None => "".into(),
    })
}

fn number_call(vm: &mut Vm, _: &Value, a: &[Value]) -> Result<Value> {
    Ok(match a.first() {
        Some(v) => vm.to_number(v)?.into(),
        None => 0.into(),
    })
}

fn int_call(vm: &mut Vm, _: &Value, a: &[Value]) -> Result<Value> {
    Ok(vm.to_int(&arg(a, 0))?.into())
}

fn uint_call(vm: &mut Vm, _: &Value, a: &[Value]) -> Result<Value> {
    Ok(vm.to_uint(&arg(a, 0))?.into())
}

fn boolean_call(_: &mut Vm, _: &Value, a: &[Value]) -> Result<Value> {
    Ok(arg(a, 0).to_boolean().into())
}

/// `new String(x)` etc. box their primitive.
fn primitive_init<const K: u8>(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    let v = match K {
        0 => string_call(vm, this, a)?,
        1 => number_call(vm, this, a)?,
        2 => int_call(vm, this, a)?,
        3 => uint_call(vm, this, a)?,
        _ => boolean_call(vm, this, a)?,
    };
    if let Some(o) = this.as_object() {
        o.borrow_mut().kind = Kind::Primitive(v);
    }
    Ok(Value::Undefined)
}

fn string_length(vm: &mut Vm, this: &Value, _: &[Value]) -> Result<Value> {
    Ok((this_string(vm, this)?.encode_utf16().count() as u32).into())
}

fn string_from_char_code(vm: &mut Vm, _: &Value, a: &[Value]) -> Result<Value> {
    let mut units = Vec::new();
    for v in a {
        units.push(vm.to_uint(v)? as u16);
    }
    Ok(from_utf16(&units))
}

fn string_methods() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("charAt", |vm, this, a| {
            let s = utf16(&this_string(vm, this)?);
            let i = value::to_integer(vm.to_number(&arg(a, 0))?);
            Ok(if i >= 0.0 && (i as usize) < s.len() { from_utf16(&s[i as usize..i as usize + 1]) } else { "".into() })
        }),
        ("charCodeAt", |vm, this, a| {
            let s = utf16(&this_string(vm, this)?);
            let i = value::to_integer(vm.to_number(&arg(a, 0))?);
            Ok(if i >= 0.0 && (i as usize) < s.len() { (s[i as usize] as u32).into() } else { f64::NAN.into() })
        }),
        ("indexOf", |vm, this, a| {
            let s = utf16(&this_string(vm, this)?);
            let needle = utf16(&vm.to_string(&arg(a, 0))?);
            let from = (value::to_integer(vm.to_number(&arg(a, 1))?).max(0.0) as usize).min(s.len());
            Ok(find_utf16(&s, &needle, from).map_or(-1, |i| i as i32).into())
        }),
        ("lastIndexOf", |vm, this, a| {
            let s = utf16(&this_string(vm, this)?);
            let needle = utf16(&vm.to_string(&arg(a, 0))?);
            let from = match a.get(1) {
                Some(v) => {
                    let n = vm.to_number(v)?;
                    if n.is_nan() { s.len() } else { value::to_integer(n).max(0.0) as usize }
                }
                None => s.len(),
            };
            let last = (0..=from.min(s.len().saturating_sub(needle.len()))).rev().find(|&i| s[i..].starts_with(&needle));
            Ok(last.map_or(-1, |i| i as i32).into())
        }),
        ("substr", |vm, this, a| {
            let s = utf16(&this_string(vm, this)?);
            let start = relative_index(vm.to_number(&arg(a, 0))?, s.len());
            let len = match a.get(1) {
                Some(Value::Undefined) | None => s.len(),
                Some(v) => value::to_integer(vm.to_number(v)?).max(0.0) as usize,
            };
            Ok(from_utf16(&s[start..(start + len).min(s.len())]))
        }),
        ("substring", |vm, this, a| {
            let s = utf16(&this_string(vm, this)?);
            let clamp = |n: f64| (if n.is_nan() { 0.0 } else { n }).clamp(0.0, s.len() as f64) as usize;
            let x = clamp(vm.to_number(&arg(a, 0))?);
            let y = match a.get(1) {
                Some(Value::Undefined) | None => s.len(),
                Some(v) => clamp(vm.to_number(v)?),
            };
            Ok(from_utf16(&s[x.min(y)..x.max(y)]))
        }),
        ("slice", |vm, this, a| {
            let s = utf16(&this_string(vm, this)?);
            let start = relative_index(vm.to_number(&arg(a, 0))?, s.len());
            let end = match a.get(1) {
                Some(Value::Undefined) | None => s.len(),
                Some(v) => relative_index(vm.to_number(v)?, s.len()),
            };
            Ok(from_utf16(&s[start..end.max(start)]))
        }),
        ("split", |vm, this, a| {
            let s = this_string(vm, this)?;
            let limit = match a.get(1) {
                Some(Value::Undefined) | None => usize::MAX,
                Some(v) => vm.to_uint(v)? as usize,
            };
            let parts: Vec<Value> = match a.first() {
                None | Some(Value::Undefined) => vec![Value::String(s.clone())],
                Some(sep) => {
                    let sep = vm.to_string(sep)?;
                    if sep.is_empty() {
                        s.encode_utf16().map(|u| from_utf16(&[u])).collect()
                    } else {
                        s.split(&*sep).map(Value::from).collect()
                    }
                }
            };
            Ok(Value::Object(vm.array(parts.into_iter().take(limit).collect())))
        }),
        ("toLowerCase", |vm, this, _| Ok(this_string(vm, this)?.to_lowercase().into())),
        ("toUpperCase", |vm, this, _| Ok(this_string(vm, this)?.to_uppercase().into())),
        ("toLocaleLowerCase", |vm, this, _| Ok(this_string(vm, this)?.to_lowercase().into())),
        ("toLocaleUpperCase", |vm, this, _| Ok(this_string(vm, this)?.to_uppercase().into())),
        ("concat", |vm, this, a| {
            let mut s = this_string(vm, this)?.to_string();
            for v in a {
                s.push_str(&vm.to_string(v)?);
            }
            Ok(s.into())
        }),
        ("replace", |vm, this, a| {
            // Only string patterns occur in the installed UI code paths.
            let s = this_string(vm, this)?;
            let pattern = vm.to_string(&arg(a, 0))?;
            let Some(at) = s.find(&*pattern) else { return Ok(Value::String(s)) };
            let replacement = match arg(a, 1) {
                f @ Value::Object(_) => {
                    let r = vm.call(&f, Value::Null, &[Value::String(pattern.clone()), Value::from(at as u32), Value::String(s.clone())])?;
                    vm.to_string(&r)?
                }
                v => vm.to_string(&v)?,
            };
            Ok(format!("{}{}{}", &s[..at], replacement, &s[at + pattern.len()..]).into())
        }),
        ("localeCompare", |vm, this, a| {
            let s = this_string(vm, this)?;
            let t = vm.to_string(&arg(a, 0))?;
            Ok((s.cmp(&t) as i32).into())
        }),
        ("toString", |vm, this, _| Ok(Value::String(this_string(vm, this)?))),
        ("valueOf", |vm, this, _| Ok(Value::String(this_string(vm, this)?))),
    ]
}

fn find_utf16(s: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(s.len()));
    }
    (from..=s.len().checked_sub(needle.len())?).find(|&i| s[i..i + needle.len()] == *needle)
}

// ---- Number / Boolean ----

fn this_number(vm: &mut Vm, this: &Value) -> Result<f64> {
    vm.to_number(this)
}

fn number_methods() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("toString", |vm, this, a| {
            let n = this_number(vm, this)?;
            let radix = match a.first() {
                Some(Value::Undefined) | None => 10,
                Some(v) => vm.to_uint(v)?,
            };
            if !(2..=36).contains(&radix) {
                return Err(vm.range_error(1003, format!("The radix argument must be between 2 and 36; got {radix}.")));
            }
            Ok(value::number_to_radix_string(n, radix).into())
        }),
        ("toFixed", |vm, this, a| {
            let n = this_number(vm, this)?;
            let digits = vm.to_uint(&arg(a, 0))?;
            if digits > 20 {
                return Err(vm.range_error(1002, "Number.toFixed has a range of 0 to 20."));
            }
            if !n.is_finite() || n.abs() >= 1e21 {
                return Ok(value::number_to_string(n).into());
            }
            Ok(format!("{:.*}", digits as usize, n).into())
        }),
        ("toPrecision", |vm, this, a| {
            let n = this_number(vm, this)?;
            if matches!(arg(a, 0), Value::Undefined) {
                return Ok(value::number_to_string(n).into());
            }
            let p = vm.to_uint(&arg(a, 0))?.clamp(1, 21) as usize;
            if !n.is_finite() {
                return Ok(value::number_to_string(n).into());
            }
            let e = if n == 0.0 { 0 } else { n.abs().log10().floor() as i32 };
            Ok(if e < -6 || e >= p as i32 {
                format!("{:.*e}", p - 1, n).replace('e', "e+").replace("e+-", "e-")
            } else {
                format!("{:.*}", (p as i32 - 1 - e).max(0) as usize, n)
            }
            .into())
        }),
        ("valueOf", |vm, this, _| Ok(this_number(vm, this)?.into())),
    ]
}

fn boolean_to_string(_: &mut Vm, this: &Value, _: &[Value]) -> Result<Value> {
    Ok(match this {
        Value::Object(o) => match &o.borrow().kind {
            Kind::Primitive(p) => p.primitive_to_string(),
            _ => "false".into(),
        },
        v => v.primitive_to_string(),
    }
    .into())
}

fn boolean_value_of(_: &mut Vm, this: &Value, _: &[Value]) -> Result<Value> {
    Ok(match this {
        Value::Object(o) => match &o.borrow().kind {
            Kind::Primitive(p) => p.to_boolean(),
            _ => false,
        },
        v => v.to_boolean(),
    }
    .into())
}

// ---- Namespace ----

fn namespace_init(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    let uri = match a.last() {
        Some(v) => vm.to_string(v)?,
        None => "".into(),
    };
    if let Some(o) = this.as_object() {
        o.borrow_mut().kind = Kind::Namespace(Namespace::Public(uri));
    }
    Ok(Value::Undefined)
}

fn namespace_uri(_: &mut Vm, this: &Value, _: &[Value]) -> Result<Value> {
    Ok(match this.as_object().map(|o| o.borrow()) {
        Some(d) => match &d.kind {
            Kind::Namespace(ns) => ns.uri().into(),
            _ => "".into(),
        },
        None => "".into(),
    })
}

// ---- Error ----

fn error_call(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    let class = this.as_object().and_then(Obj::as_class).unwrap_or_else(|| vm.b().error.clone());
    vm.construct_class(&class, a)
}

fn error_init(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    let message = match a.first() {
        Some(Value::Undefined) | None => "".into(),
        Some(v) => vm.to_string(v)?,
    };
    let id = match a.get(1) {
        Some(v) => vm.to_int(v)?,
        None => 0,
    };
    if let Some(o) = this.as_object() {
        let mut d = o.borrow_mut();
        d.dynamic.set("message".into(), Value::String(message));
        d.dynamic.set_enumerable("message", false);
        d.dynamic.set("$errorID".into(), id.into());
        d.dynamic.set_enumerable("$errorID", false);
    }
    Ok(Value::Undefined)
}

fn error_id(_: &mut Vm, this: &Value, _: &[Value]) -> Result<Value> {
    Ok(this.as_object().and_then(|o| o.borrow().dynamic.get("$errorID").cloned()).unwrap_or(0.into()))
}

fn error_to_string(vm: &mut Vm, this: &Value, _: &[Value]) -> Result<Value> {
    let name = vm.get_property(this, &Name::public("name"))?;
    let message = vm.get_property(this, &Name::public("message"))?;
    let name = vm.to_string(&name)?;
    let message = vm.to_string(&message)?;
    Ok(if message.is_empty() { Value::String(name) } else { format!("{name}: {message}").into() })
}

/// Text of a thrown value for diagnostics.
pub fn describe_error(vm: &mut Vm, e: &Error) -> String {
    let mut text = match e {
        Error::Internal(s) => s.clone(),
        Error::Throw(v) => vm.to_string(v).map(|s| s.to_string()).unwrap_or_else(|_| format!("{v:?}")),
    };
    if let Some(stack) = vm.failure_stack.take() {
        for frame in stack.iter().rev() {
            text.push_str("\n    at ");
            text.push_str(frame);
        }
    }
    text
}

// ---- Math ----

fn math_functions() -> Vec<(&'static str, NativeFn)> {
    macro_rules! unary {
        ($name:literal, $f:expr) => {
            ($name, |vm: &mut Vm, _: &Value, a: &[Value]| -> Result<Value> {
                let x = vm.to_number(&arg(a, 0))?;
                let f: fn(f64) -> f64 = $f;
                Ok(f(x).into())
            })
        };
    }
    vec![
        unary!("abs", f64::abs),
        unary!("acos", f64::acos),
        unary!("asin", f64::asin),
        unary!("atan", f64::atan),
        unary!("ceil", f64::ceil),
        unary!("cos", f64::cos),
        unary!("exp", f64::exp),
        unary!("floor", f64::floor),
        unary!("log", f64::ln),
        // ECMA-262 Math.round: floor(x + 0.5), keeping -0 for [-0.5, -0].
        unary!("round", |x| if x.is_finite() && x < 0.0 && x >= -0.5 { -0.0 } else { (x + 0.5).floor() }),
        unary!("sin", f64::sin),
        unary!("sqrt", f64::sqrt),
        unary!("tan", f64::tan),
        ("atan2", |vm, _, a| {
            let y = vm.to_number(&arg(a, 0))?;
            let x = vm.to_number(&arg(a, 1))?;
            Ok(y.atan2(x).into())
        }),
        ("pow", |vm, _, a| {
            let x = vm.to_number(&arg(a, 0))?;
            let y = vm.to_number(&arg(a, 1))?;
            Ok(x.powf(y).into())
        }),
        ("max", |vm, _, a| {
            let mut r = f64::NEG_INFINITY;
            for v in a {
                let n = vm.to_number(v)?;
                if n.is_nan() {
                    return Ok(f64::NAN.into());
                }
                r = r.max(n);
            }
            Ok(r.into())
        }),
        ("min", |vm, _, a| {
            let mut r = f64::INFINITY;
            for v in a {
                let n = vm.to_number(v)?;
                if n.is_nan() {
                    return Ok(f64::NAN.into());
                }
                r = r.min(n);
            }
            Ok(r.into())
        }),
        ("random", |vm, _, _| Ok(crate::flash::host_random(vm).into())),
    ]
}

// ---- globals ----

fn global_trace(vm: &mut Vm, _: &Value, a: &[Value]) -> Result<Value> {
    let mut parts = Vec::new();
    for v in a {
        parts.push(vm.to_string(v)?.to_string());
    }
    vm.trace.push(parts.join(" "));
    Ok(Value::Undefined)
}

fn global_parse_int(vm: &mut Vm, _: &Value, a: &[Value]) -> Result<Value> {
    let s = vm.to_string(&arg(a, 0))?;
    let mut radix = match a.get(1) {
        Some(v) => vm.to_int(v)?,
        None => 0,
    };
    let t = s.trim_start();
    let (negative, mut t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    if (radix == 0 || radix == 16) && (t.starts_with("0x") || t.starts_with("0X")) {
        t = &t[2..];
        radix = 16;
    }
    if radix == 0 {
        radix = 10;
    }
    if !(2..=36).contains(&radix) {
        return Ok(f64::NAN.into());
    }
    let digits: String = t.chars().take_while(|c| c.to_digit(radix as u32).is_some()).collect();
    if digits.is_empty() {
        return Ok(f64::NAN.into());
    }
    let n = digits.chars().fold(0.0, |acc, c| acc * radix as f64 + c.to_digit(radix as u32).unwrap() as f64);
    Ok((if negative { -n } else { n }).into())
}

fn global_parse_float(vm: &mut Vm, _: &Value, a: &[Value]) -> Result<Value> {
    let s = vm.to_string(&arg(a, 0))?;
    let t = s.trim_start();
    let mut end = 0;
    let bytes = t.as_bytes();
    let mut seen_digit = false;
    let mut seen_dot = false;
    let mut seen_e = false;
    while end < bytes.len() {
        let c = bytes[end];
        let ok = match c {
            b'0'..=b'9' => {
                seen_digit = true;
                true
            }
            b'+' | b'-' => end == 0 || matches!(bytes[end - 1], b'e' | b'E'),
            b'.' if !seen_dot && !seen_e => {
                seen_dot = true;
                true
            }
            b'e' | b'E' if seen_digit && !seen_e => {
                seen_e = true;
                true
            }
            _ => false,
        };
        if !ok {
            break;
        }
        end += 1;
    }
    let mut candidate = &t[..end];
    while !candidate.is_empty() {
        if let Ok(v) = candidate.parse::<f64>() {
            return Ok(v.into());
        }
        candidate = &candidate[..candidate.len() - 1];
    }
    if t.starts_with("Infinity") || t.starts_with("+Infinity") {
        return Ok(f64::INFINITY.into());
    }
    if t.starts_with("-Infinity") {
        return Ok(f64::NEG_INFINITY.into());
    }
    Ok(f64::NAN.into())
}

fn global_escape(vm: &mut Vm, _: &Value, a: &[Value]) -> Result<Value> {
    let s = vm.to_string(&arg(a, 0))?;
    let mut out = String::new();
    for u in s.encode_utf16() {
        match u {
            0x30..=0x39 | 0x41..=0x5a | 0x61..=0x7a | 0x40 | 0x2a | 0x5f | 0x2b | 0x2d | 0x2e | 0x2f => out.push(u as u8 as char),
            0..=0xff => out.push_str(&format!("%{u:02X}")),
            _ => out.push_str(&format!("%u{u:04X}")),
        }
    }
    Ok(out.into())
}

fn get_qualified_class_name(vm: &mut Vm, _: &Value, a: &[Value]) -> Result<Value> {
    let v = arg(a, 0);
    let class = match &v {
        Value::Object(o) => o.as_class().or_else(|| o.class()),
        other => vm.class_of(other),
    };
    Ok(match class {
        Some(c) => c.name.qualified().into(),
        None => if matches!(v, Value::Null) { "null" } else { "void" }.into(),
    })
}

fn get_qualified_superclass_name(vm: &mut Vm, _: &Value, a: &[Value]) -> Result<Value> {
    let v = arg(a, 0);
    let class = match &v {
        Value::Object(o) => o.as_class().or_else(|| o.class()),
        other => vm.class_of(other),
    };
    Ok(match class.and_then(|c| c.super_class.clone()) {
        Some(c) => c.name.qualified().into(),
        None => Value::Null,
    })
}

fn get_definition_by_name(vm: &mut Vm, _: &Value, a: &[Value]) -> Result<Value> {
    let name = vm.to_string(&arg(a, 0))?;
    match vm.class_by_name(&name)? {
        Some(v) => Ok(v),
        None => Err(vm.reference_error(1065, format!("Variable {name} is not defined."))),
    }
}

/// Read a public fixed or dynamic property as a number.
pub fn number_property(vm: &mut Vm, this: &Value, name: &str) -> Result<f64> {
    let v = vm.get_property(this, &Name::public(name))?;
    vm.to_number(&v)
}

pub fn set_number_property(vm: &mut Vm, this: &Value, name: &str, value: f64) -> Result<()> {
    vm.set_property(this, &Name::public(name), value.into())
}

/// Look up a native class definition by package and name.
pub fn native_class(vm: &Vm, package: &str, name: &str) -> Rc<Class> {
    vm.natives
        .get(&QName::new(Namespace::package(package), name))
        .and_then(|v| v.as_object().and_then(Obj::as_class))
        .unwrap_or_else(|| panic!("native class {package}.{name} is not installed"))
}

pub fn argument(args: &[Value], i: usize) -> Value {
    arg(args, i)
}
