//! An AVM2 (ActionScript 3) virtual machine for Iggy movies.
//!
//! Opcode and conversion semantics follow the AVM2 Overview and ECMA-262
//! rules that Iggy's verifier and interpreter implement; Iggy-specific
//! behavior is cited where the executable defines it.
pub mod builtins;
mod interpreter;
pub mod name;
pub mod object;
pub mod unit;
pub mod value;
pub mod xml;

use name::{Name, Namespace, PoolName, QName};
use object::{Binding, Class, Function, Kind, Method, MethodKind, Obj, ObjectData, Scope, Traits};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use unit::Unit;
use value::Value;

/// Script and native failures. Thrown AS3 values propagate as `Throw`.
#[derive(Clone, Debug)]
pub enum Error {
    Throw(Value),
    Internal(String),
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Internal(s)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Iggy's verifier limits (`Verification error: ...` strings near the
/// interpreter); the rewrite enforces the call depth used by Flash players.
const MAX_CALL_DEPTH: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ScriptState {
    Pending,
    Running,
    Done,
}

pub struct Script {
    unit: Rc<Unit>,
    index: usize,
    global: Obj,
    state: Cell<ScriptState>,
}

/// Core classes the interpreter needs directly.
pub struct Builtins {
    pub object: Rc<Class>,
    pub function: Rc<Class>,
    pub class: Rc<Class>,
    pub array: Rc<Class>,
    pub string: Rc<Class>,
    pub number: Rc<Class>,
    pub int: Rc<Class>,
    pub uint: Rc<Class>,
    pub boolean: Rc<Class>,
    pub namespace: Rc<Class>,
    pub vector: Rc<Class>,
    pub xml: Rc<Class>,
    pub xml_list: Rc<Class>,
    pub error: Rc<Class>,
    pub type_error: Rc<Class>,
    pub reference_error: Rc<Class>,
    pub range_error: Rc<Class>,
    pub argument_error: Rc<Class>,
}

pub struct Vm {
    pub units: Vec<Rc<Unit>>,
    scripts: Vec<Rc<Script>>,
    /// Script-defined names in load order of their defining scripts.
    definitions: HashMap<QName, Rc<Script>>,
    /// Natively provided definitions (top-level builtins and `flash.*`).
    pub natives: HashMap<QName, Value>,
    pub builtins: Option<Builtins>,
    /// Applied `Vector.<T>` classes by element type address (0 = `*`).
    vectors: HashMap<usize, Rc<Class>>,
    depth: usize,
    /// `trace` output.
    pub trace: Vec<String>,
    /// Host state for native classes.
    pub host: Box<dyn std::any::Any>,
    pub started: std::time::Instant,
    pub random: u64,
}

impl Vm {
    pub fn new() -> Self {
        let mut vm = Vm {
            units: Vec::new(),
            scripts: Vec::new(),
            definitions: HashMap::new(),
            natives: HashMap::new(),
            builtins: None,
            vectors: HashMap::new(),
            depth: 0,
            trace: Vec::new(),
            host: Box::new(()),
            started: std::time::Instant::now(),
            random: 0x9e37_79b9_7f4a_7c15,
        };
        builtins::install(&mut vm);
        vm
    }

    pub fn b(&self) -> &Builtins {
        self.builtins.as_ref().expect("builtins installed")
    }

    // ---- errors ----

    fn error_of(&mut self, class: Rc<Class>, id: u32, message: String) -> Error {
        let text = format!("Error #{id}: {message}");
        match self.construct_class(&class, &[Value::from(text.clone()), Value::from(id)]) {
            Ok(v) => Error::Throw(v),
            Err(_) => Error::Internal(text),
        }
    }

    pub fn type_error(&mut self, id: u32, message: impl Into<String>) -> Error {
        let class = self.b().type_error.clone();
        self.error_of(class, id, message.into())
    }

    pub fn reference_error(&mut self, id: u32, message: impl Into<String>) -> Error {
        let class = self.b().reference_error.clone();
        self.error_of(class, id, message.into())
    }

    pub fn range_error(&mut self, id: u32, message: impl Into<String>) -> Error {
        let class = self.b().range_error.clone();
        self.error_of(class, id, message.into())
    }

    pub fn argument_error(&mut self, id: u32, message: impl Into<String>) -> Error {
        let class = self.b().argument_error.clone();
        self.error_of(class, id, message.into())
    }

    // ---- loading ----

    /// Load an ABC block. Script traits become definitions; scripts run on
    /// first reference, and the last script is the entry point.
    pub fn load_abc(&mut self, abc: &[u8]) -> Result<Rc<Unit>> {
        let parsed = swf::avm2::read::Reader::new(abc)
            .read()
            .map_err(|e| Error::Internal(format!("ABC: {e}")))?;
        let unit = Unit::new(self.units.len() as u32, parsed);
        self.units.push(unit.clone());
        for (index, script) in unit.abc.scripts.iter().enumerate() {
            let traits = unit::build_traits(&unit, &script.traits, None)?;
            let traits = Rc::new(traits);
            let global = Obj::new(ObjectData {
                slots: traits.initial_slots(),
                traits: Some(traits.clone()),
                proto: Some(self.b().object.prototype()),
                ..ObjectData::plain(None)
            });
            let script_ref = Rc::new(Script { unit: unit.clone(), index, global, state: Cell::new(ScriptState::Pending) });
            for name in traits.bindings.keys() {
                self.definitions.entry(name.clone()).or_insert_with(|| script_ref.clone());
            }
            self.scripts.push(script_ref);
        }
        Ok(unit)
    }

    fn ensure_script(&mut self, script: &Rc<Script>) -> Result<()> {
        if script.state.get() != ScriptState::Pending {
            return Ok(());
        }
        script.state.set(ScriptState::Running);
        let scope: Rc<[Scope]> = Rc::new([Scope { value: Value::Object(script.global.clone()), with: false }]);
        let functions = unit::function_traits(&script.unit, &script.unit.abc.scripts[script.index].traits);
        for (name, method) in functions {
            let f = self.function(script.unit.method(method), scope.clone(), None);
            self.init_property(&Value::Object(script.global.clone()), &Name::qname(&name), f.into())?;
        }
        let init = script.unit.abc.scripts[script.index].init_method.0;
        let method = script.unit.method(init);
        let result = self.call_method(&method, Value::Object(script.global.clone()), &[], Rc::new([]), None);
        script.state.set(ScriptState::Done);
        result.map(|_| ())
    }

    /// Run every loaded script that has not run (Iggy runs a movie's
    /// entry script when the player is created).
    pub fn run_entry_script(&mut self, unit: &Rc<Unit>) -> Result<()> {
        let last = self.scripts.iter().filter(|s| Rc::ptr_eq(&s.unit, unit)).last().cloned();
        match last {
            Some(script) => self.ensure_script(&script),
            None => Ok(()),
        }
    }

    /// Resolve a global definition (class, function, namespace, variable).
    pub fn get_definition(&mut self, q: &QName) -> Result<Option<Value>> {
        if let Some(v) = self.natives.get(q) {
            return Ok(Some(v.clone()));
        }
        let Some(script) = self.definitions.get(q).cloned() else {
            return Ok(None);
        };
        self.ensure_script(&script)?;
        self.get_property(&Value::Object(script.global.clone()), &Name::qname(q)).map(Some)
    }

    /// The global object (script or native holder) that defines `name`.
    fn find_definition(&mut self, name: &Name) -> Result<Option<Value>> {
        let Some(local) = &name.name else {
            return Ok(None);
        };
        for ns in name.set.iter() {
            let q = QName { ns: ns.clone(), name: local.clone() };
            if self.natives.contains_key(&q) {
                return Ok(Some(Value::Object(self.native_holder(&q))));
            }
            if let Some(script) = self.definitions.get(&q).cloned() {
                self.ensure_script(&script)?;
                return Ok(Some(Value::Object(script.global.clone())));
            }
        }
        Ok(None)
    }

    /// Natives live in a single holder object exposing them as properties.
    fn native_holder(&mut self, _q: &QName) -> Obj {
        builtins::native_globals(self)
    }

    /// The class for a type multiname, if defined (used for coercion).
    pub fn resolve_type(&mut self, unit: &Rc<Unit>, index: u32) -> Result<Option<Rc<Class>>> {
        match unit.pools.name(index) {
            PoolName::Any => Ok(None),
            PoolName::QName { name, .. } => Ok(self
                .get_definition(&name)?
                .and_then(|v| v.as_object().and_then(Obj::as_class))),
            PoolName::TypeName { base, params } => {
                let base = self.resolve_type(unit, base)?;
                let param = match params.first() {
                    Some(&p) => self.resolve_type(unit, p)?,
                    None => None,
                };
                match base {
                    Some(b) if Rc::ptr_eq(&b, &self.b().vector) => Ok(Some(self.vector_class(param)?)),
                    _ => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }

    // ---- object creation ----

    pub fn plain_object(&self) -> Obj {
        let object = self.b().object.clone();
        Obj::new(ObjectData { class: Some(object.clone()), ..ObjectData::plain(Some(object.prototype())) })
    }

    pub fn array(&self, items: Vec<Value>) -> Obj {
        let array = self.b().array.clone();
        Obj::new(ObjectData {
            class: Some(array.clone()),
            traits: Some(array.instance.clone()),
            kind: Kind::Array(items),
            ..ObjectData::plain(Some(array.prototype()))
        })
    }

    pub fn function(&self, method: Rc<Method>, scope: Rc<[Scope]>, this: Option<Value>) -> Obj {
        let function = self.b().function.clone();
        let proto = self.plain_object();
        let f = Obj::new(ObjectData {
            class: Some(function.clone()),
            traits: Some(function.instance.clone()),
            kind: Kind::Function(Function { method, scope, this }),
            ..ObjectData::plain(Some(function.prototype()))
        });
        // Every function closure gets a fresh prototype object.
        proto.borrow_mut().dynamic.set("constructor".into(), Value::Object(f.clone()));
        proto.borrow_mut().dynamic.set_enumerable("constructor", false);
        f.borrow_mut().dynamic.set("prototype".into(), Value::Object(proto));
        f.borrow_mut().dynamic.set_enumerable("prototype", false);
        f
    }

    pub fn native_function(&self, name: &str, f: object::NativeFn) -> Obj {
        self.function(Method::native(name, f), Rc::new([]), None)
    }

    /// Create the class object and prototype for a newly defined class.
    pub fn finish_class(&mut self, class: &Rc<Class>, scope: Rc<[Scope]>) -> Obj {
        let class_class = self.builtins.as_ref().map(|b| b.class.clone());
        let proto_parent = class.super_class.as_ref().map(|s| s.prototype());
        let prototype = Obj::new(ObjectData {
            class: self.builtins.as_ref().map(|b| b.object.clone()),
            ..ObjectData::plain(proto_parent)
        });
        let object = Obj::new(ObjectData {
            class: class_class,
            slots: class.statics.initial_slots(),
            traits: Some(class.statics.clone()),
            kind: Kind::Class(class.clone()),
            ..ObjectData::plain(None)
        });
        prototype.borrow_mut().dynamic.set("constructor".into(), Value::Object(object.clone()));
        prototype.borrow_mut().dynamic.set_enumerable("constructor", false);
        *class.object.borrow_mut() = Some(object.clone());
        *class.prototype.borrow_mut() = Some(prototype);
        *class.scope.borrow_mut() = scope;
        object
    }

    /// `Vector.<element>`.
    pub fn vector_class(&mut self, element: Option<Rc<Class>>) -> Result<Rc<Class>> {
        let key = element.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize);
        if let Some(c) = self.vectors.get(&key) {
            return Ok(c.clone());
        }
        let base = self.b().vector.clone();
        let name = format!("Vector.<{}>", element.as_ref().map_or("*".into(), |e| e.name.qualified()));
        let class = Rc::new(Class {
            name: QName::new(Namespace::package("__AS3__.vec"), &name),
            super_class: base.super_class.clone(),
            interfaces: Vec::new(),
            instance: base.instance.clone(),
            statics: base.statics.clone(),
            instance_init: None,
            sealed: false,
            interface: false,
            native: base.native,
            object: Default::default(),
            prototype: Default::default(),
            scope: Default::default(),
            applied: Some(element),
        });
        self.finish_class(&class, Rc::new([]));
        *class.prototype.borrow_mut() = Some(base.prototype());
        self.vectors.insert(key, class.clone());
        Ok(class)
    }

    // ---- conversions ----

    pub fn to_primitive(&mut self, v: &Value, hint_string: bool) -> Result<Value> {
        let Value::Object(o) = v else {
            return Ok(v.clone());
        };
        if let Kind::Primitive(p) = &o.borrow().kind {
            return Ok(p.clone());
        }
        // Date objects default to the string hint; none are used by Iggy UIs.
        let order = if hint_string { ["toString", "valueOf"] } else { ["valueOf", "toString"] };
        for method in order {
            let f = self.get_property(v, &Name::public(method))?;
            if matches!(&f, Value::Object(f) if f.is_function()) {
                let r = self.call(&f, v.clone(), &[])?;
                if !matches!(r, Value::Object(_)) {
                    return Ok(r);
                }
            }
        }
        Err(self.type_error(1050, "Cannot convert object to primitive."))
    }

    pub fn to_number(&mut self, v: &Value) -> Result<f64> {
        match v {
            Value::Object(_) => Ok(self.to_primitive(v, false)?.primitive_to_number()),
            _ => Ok(v.primitive_to_number()),
        }
    }

    pub fn to_string(&mut self, v: &Value) -> Result<Rc<str>> {
        match v {
            Value::Object(_) => Ok(self.to_primitive(v, true)?.primitive_to_string()),
            _ => Ok(v.primitive_to_string()),
        }
    }

    pub fn to_int(&mut self, v: &Value) -> Result<i32> {
        Ok(value::to_int32(self.to_number(v)?))
    }

    pub fn to_uint(&mut self, v: &Value) -> Result<u32> {
        Ok(value::to_uint32(self.to_number(v)?))
    }

    /// `coerce_s`: null and undefined stay null.
    pub fn coerce_string(&mut self, v: &Value) -> Result<Value> {
        Ok(match v {
            Value::Undefined | Value::Null => Value::Null,
            Value::String(_) => v.clone(),
            _ => Value::String(self.to_string(v)?),
        })
    }

    pub fn class_of(&self, v: &Value) -> Option<Rc<Class>> {
        let b = self.b();
        match v {
            Value::Undefined | Value::Null => None,
            Value::Bool(_) => Some(b.boolean.clone()),
            Value::Number(_) if v.is_int() => Some(b.int.clone()),
            Value::Number(_) => Some(b.number.clone()),
            Value::String(_) => Some(b.string.clone()),
            Value::Object(o) => o.class(),
        }
    }

    /// AVM2 `istype`.
    pub fn is_type(&self, v: &Value, class: &Rc<Class>) -> bool {
        let b = self.b();
        if Rc::ptr_eq(class, &b.object) {
            return !v.is_nullish();
        }
        match v {
            Value::Undefined | Value::Null => false,
            Value::Number(_) if Rc::ptr_eq(class, &b.int) => v.is_int(),
            Value::Number(_) if Rc::ptr_eq(class, &b.uint) => v.is_uint(),
            Value::Number(_) => Rc::ptr_eq(class, &b.number),
            Value::Bool(_) => Rc::ptr_eq(class, &b.boolean),
            Value::String(_) => Rc::ptr_eq(class, &b.string),
            Value::Object(o) => {
                if let Kind::Primitive(p) = &o.borrow().kind {
                    if self.is_type(p, class) {
                        return true;
                    }
                }
                match o.class() {
                    Some(c) => c.is_subclass_of(class) || self.vector_matches(&c, class),
                    None => false,
                }
            }
        }
    }

    /// `Vector.<T>` instances are also `Vector.<*>`-typed objects only when
    /// the classes match exactly; the generic `Vector` base matches all.
    fn vector_matches(&self, c: &Rc<Class>, class: &Rc<Class>) -> bool {
        c.applied.is_some() && Rc::ptr_eq(class, &self.b().vector)
    }

    /// AVM2 `coerce` to a class.
    pub fn coerce(&mut self, v: &Value, class: &Rc<Class>) -> Result<Value> {
        let b = self.b();
        if Rc::ptr_eq(class, &b.int) {
            return Ok(Value::from(self.to_int(v)?));
        }
        if Rc::ptr_eq(class, &b.uint) {
            return Ok(Value::from(self.to_uint(v)?));
        }
        if Rc::ptr_eq(class, &b.number) {
            return Ok(Value::from(self.to_number(v)?));
        }
        if Rc::ptr_eq(class, &b.boolean) {
            return Ok(Value::from(v.to_boolean()));
        }
        if Rc::ptr_eq(class, &b.string) {
            return self.coerce_string(v);
        }
        if Rc::ptr_eq(class, &b.object) {
            return Ok(if matches!(v, Value::Undefined) { Value::Null } else { v.clone() });
        }
        if v.is_nullish() {
            return Ok(Value::Null);
        }
        if self.is_type(v, class) {
            return Ok(v.clone());
        }
        let from = self.class_of(v).map_or("?".to_string(), |c| c.name.qualified());
        Err(self.type_error(
            1034,
            format!("Type Coercion failed: cannot convert {} to {}.", from, class.name.qualified()),
        ))
    }

    /// Coerce a value stored into a typed slot.
    fn coerce_slot(&mut self, slot: &object::Slot, v: Value) -> Result<Value> {
        let Some(unit) = &slot.unit else {
            return Ok(v);
        };
        if slot.type_name == 0 {
            return Ok(v);
        }
        if let Some(q) = unit.pools.qname(slot.type_name) {
            if q.ns.is_public() {
                match &*q.name {
                    "int" => return Ok(Value::from(self.to_int(&v)?)),
                    "uint" => return Ok(Value::from(self.to_uint(&v)?)),
                    "Number" => return Ok(Value::from(self.to_number(&v)?)),
                    "Boolean" => return Ok(Value::from(v.to_boolean())),
                    "String" => return self.coerce_string(&v),
                    "Object" => return Ok(if matches!(v, Value::Undefined) { Value::Null } else { v }),
                    _ => {}
                }
            }
        }
        let unit = unit.clone();
        match self.resolve_type(&unit, slot.type_name)? {
            Some(class) => self.coerce(&v, &class),
            None => Ok(v),
        }
    }

    // ---- property access ----

    /// The traits, class and object used for property lookup on `v`.
    fn lookup_target(&self, v: &Value) -> Option<(Option<Obj>, Option<Rc<Traits>>, Option<Obj>)> {
        let b = self.b();
        let class = match v {
            Value::Undefined | Value::Null => return None,
            Value::Object(o) => {
                let d = o.borrow();
                return Some((Some(o.clone()), d.traits.clone(), d.proto.clone()));
            }
            Value::Bool(_) => &b.boolean,
            Value::Number(_) => &b.number,
            Value::String(_) => &b.string,
        };
        Some((None, Some(class.instance.clone()), Some(class.prototype())))
    }

    fn null_error(&mut self, v: &Value, name: &Name) -> Error {
        if matches!(v, Value::Undefined) {
            self.type_error(1010, format!("A term is undefined and has no properties. ({})", name.local()))
        } else {
            self.type_error(1009, format!("Cannot access a property or method of a null object reference. ({})", name.local()))
        }
    }

    /// Array-style index for a public numeric name.
    fn index_of(name: &Name) -> Option<usize> {
        let local = name.name.as_ref()?;
        if !name.includes_public() {
            return None;
        }
        let b = local.as_bytes();
        if b.is_empty() || (b.len() > 1 && b[0] == b'0') || !b.iter().all(u8::is_ascii_digit) {
            return None;
        }
        local.parse::<u32>().ok().filter(|&i| i != u32::MAX).map(|i| i as usize)
    }

    pub fn get_property(&mut self, target: &Value, name: &Name) -> Result<Value> {
        let Some((obj, traits, proto)) = self.lookup_target(target) else {
            return Err(self.null_error(target, name));
        };
        let local = name.name.clone().unwrap_or_else(|| "*".into());
        if let Some(traits) = &traits {
            if let Some((q, binding)) = traits.find(&local, &name.set) {
                let (q, binding) = (q.clone(), binding.clone());
                return self.read_binding(target, obj.as_ref(), &q, &binding, name);
            }
        }
        if let Some(o) = &obj {
            if let Some(v) = self.get_kind_property(o, name)? {
                return Ok(v);
            }
            if name.includes_public() {
                if let Some(v) = o.borrow().dynamic.get(&local) {
                    return Ok(v.clone());
                }
            }
        }
        if name.includes_public() {
            let mut p = proto;
            while let Some(o) = p {
                if let Some(v) = o.borrow().dynamic.get(&local) {
                    return Ok(v.clone());
                }
                p = o.borrow().proto.clone();
            }
        }
        // Sealed instances throw for unknown properties (#1069).
        let sealed = match target {
            Value::Object(o) => o.class().is_some_and(|c| c.sealed) && !matches!(o.borrow().kind, Kind::Class(_) | Kind::Function(_)),
            _ => true,
        };
        if sealed && !name.attribute {
            let class = self.class_of(target).map_or("?".to_string(), |c| c.name.qualified());
            return Err(self.reference_error(
                1069,
                format!("Property {} not found on {} and there is no default value.", local, class),
            ));
        }
        Ok(Value::Undefined)
    }

    fn read_binding(&mut self, target: &Value, obj: Option<&Obj>, q: &QName, binding: &Binding, name: &Name) -> Result<Value> {
        match binding {
            Binding::Slot(i) => Ok(match obj {
                Some(o) => o.borrow().slots.get(*i as usize).cloned().unwrap_or_default(),
                None => Value::Undefined,
            }),
            Binding::Method(m) => {
                if let Some(o) = obj {
                    if let Some(c) = o.borrow().closures.get(q) {
                        return Ok(Value::Object(c.clone()));
                    }
                }
                let scope = self.method_scope(target);
                let f = self.function(m.clone(), scope, Some(target.clone()));
                if let Some(o) = obj {
                    o.borrow_mut().closures.insert(q.clone(), f.clone());
                }
                Ok(Value::Object(f))
            }
            Binding::Accessor { get: Some(g), .. } => {
                let g = g.clone();
                let scope = self.method_scope(target);
                self.call_method(&g, target.clone(), &[], scope, None)
            }
            Binding::Accessor { get: None, .. } => Err(self.reference_error(
                1077,
                format!("Illegal read of write-only property {}.", name.local()),
            )),
        }
    }

    /// The scope chain a bound method of `target` runs in: its class scope.
    fn method_scope(&self, target: &Value) -> Rc<[Scope]> {
        match target {
            Value::Object(o) => {
                let d = o.borrow();
                match &d.kind {
                    Kind::Class(c) => c.scope.borrow().clone(),
                    _ => d.class.as_ref().map(|c| c.scope.borrow().clone()).unwrap_or_else(|| Rc::new([])),
                }
            }
            _ => Rc::new([]),
        }
    }

    /// Scope for a method declared by `class` (methods close over their
    /// declaring class's scope plus the class object).
    pub fn class_method_scope(&self, class: &Rc<Class>) -> Rc<[Scope]> {
        class.scope.borrow().clone()
    }

    fn get_kind_property(&mut self, o: &Obj, name: &Name) -> Result<Option<Value>> {
        let d = o.borrow();
        match &d.kind {
            Kind::Array(items) => Ok(Self::index_of(name).map(|i| items.get(i).cloned().unwrap_or_default())),
            Kind::Vector(v) => match Self::index_of(name) {
                Some(i) if i < v.items.len() => Ok(Some(v.items[i].clone())),
                Some(i) => {
                    let len = v.items.len();
                    drop(d);
                    Err(self.range_error(1125, format!("The index {i} is out of range {len}.")))
                }
                None => Ok(None),
            },
            Kind::Dictionary(entries) => {
                let key = name.name.clone().map(Value::String).unwrap_or_default();
                Ok(entries.iter().find(|(k, _)| dictionary_key_eq(k, &key)).map(|(_, v)| v.clone()))
            }
            Kind::Xml(_) | Kind::XmlList(_) => {
                drop(d);
                xml::get_property(self, o, name).map(Some)
            }
            _ => Ok(None),
        }
    }

    /// Dictionary access with an arbitrary key value (`d[obj]`).
    pub fn dictionary_get(&self, o: &Obj, key: &Value) -> Option<Value> {
        match &o.borrow().kind {
            Kind::Dictionary(entries) => {
                Some(entries.iter().find(|(k, _)| dictionary_key_eq(k, key)).map(|(_, v)| v.clone()).unwrap_or_default())
            }
            _ => None,
        }
    }

    pub fn set_property(&mut self, target: &Value, name: &Name, value: Value) -> Result<()> {
        self.write_property(target, name, value, false)
    }

    pub fn init_property(&mut self, target: &Value, name: &Name, value: Value) -> Result<()> {
        self.write_property(target, name, value, true)
    }

    fn write_property(&mut self, target: &Value, name: &Name, value: Value, init: bool) -> Result<()> {
        let Value::Object(o) = target else {
            if target.is_nullish() {
                return Err(self.null_error(target, name));
            }
            // Writes to primitives are ignored for dynamic names, errors for fixed.
            return Ok(());
        };
        let local = name.name.clone().unwrap_or_else(|| "*".into());
        let traits = o.borrow().traits.clone();
        if let Some(traits) = &traits {
            if let Some((_, binding)) = traits.find(&local, &name.set) {
                match binding.clone() {
                    Binding::Slot(i) => {
                        let slot = traits.slots.get(i as usize).cloned().flatten();
                        if let Some(slot) = &slot {
                            if slot.constant && !init {
                                return Err(self.reference_error(
                                    1074,
                                    format!("Illegal write to read-only property {}.", local),
                                ));
                            }
                        }
                        let value = match &slot {
                            Some(s) => self.coerce_slot(s, value)?,
                            None => value,
                        };
                        let mut d = o.borrow_mut();
                        if d.slots.len() <= i as usize {
                            d.slots.resize(i as usize + 1, Value::Undefined);
                        }
                        d.slots[i as usize] = value;
                        return Ok(());
                    }
                    Binding::Accessor { set: Some(s), .. } => {
                        let scope = self.method_scope(target);
                        self.call_method(&s, target.clone(), &[value], scope, None)?;
                        return Ok(());
                    }
                    Binding::Accessor { set: None, .. } => {
                        return Err(self.reference_error(1074, format!("Illegal write to read-only property {}.", local)));
                    }
                    Binding::Method(_) => {
                        return Err(self.reference_error(1037, format!("Cannot assign to a method {}.", local)));
                    }
                }
            }
        }
        {
            let mut d = o.borrow_mut();
            match &mut d.kind {
                Kind::Array(items) => {
                    if let Some(i) = Self::index_of(name) {
                        if items.len() <= i {
                            items.resize(i + 1, Value::Undefined);
                        }
                        items[i] = value;
                        return Ok(());
                    }
                }
                Kind::Dictionary(entries) => {
                    let key = Value::String(local.clone());
                    dictionary_set(entries, key, value);
                    return Ok(());
                }
                _ => {}
            }
        }
        let vector = matches!(o.borrow().kind, Kind::Vector(_));
        if vector {
            if let Some(i) = Self::index_of(name) {
                return builtins::vector_set(self, o, i, value);
            }
        }
        if matches!(o.borrow().kind, Kind::Xml(_) | Kind::XmlList(_)) {
            return xml::set_property(self, o, name, value);
        }
        let dynamic = o.class().is_none_or(|c| !c.sealed) || matches!(o.borrow().kind, Kind::Class(_) | Kind::Function(_));
        if !dynamic || !name.includes_public() {
            let class = o.class().map_or("?".to_string(), |c| c.name.qualified());
            return Err(self.reference_error(
                1056,
                format!("Cannot create property {} on {}.", local, class),
            ));
        }
        o.borrow_mut().dynamic.set(local, value);
        Ok(())
    }

    pub fn set_dictionary(&mut self, o: &Obj, key: Value, value: Value) -> bool {
        match &mut o.borrow_mut().kind {
            Kind::Dictionary(entries) => {
                dictionary_set(entries, key, value);
                true
            }
            _ => false,
        }
    }

    pub fn delete_property(&mut self, target: &Value, name: &Name) -> Result<bool> {
        let Value::Object(o) = target else {
            return Ok(true);
        };
        let local = name.name.clone().unwrap_or_else(|| "*".into());
        if let Some(traits) = o.borrow().traits.clone() {
            if traits.find(&local, &name.set).is_some() {
                return Ok(false);
            }
        }
        let mut d = o.borrow_mut();
        match &mut d.kind {
            Kind::Array(items) => {
                if let Some(i) = Self::index_of(name) {
                    if i < items.len() {
                        items[i] = Value::Undefined;
                    }
                    return Ok(true);
                }
            }
            Kind::Dictionary(entries) => {
                let key = Value::String(local.clone());
                entries.retain(|(k, _)| !dictionary_key_eq(k, &key));
                return Ok(true);
            }
            _ => {}
        }
        if name.includes_public() {
            d.dynamic.delete(&local);
        }
        Ok(true)
    }

    pub fn delete_dictionary(&mut self, o: &Obj, key: &Value) -> bool {
        match &mut o.borrow_mut().kind {
            Kind::Dictionary(entries) => {
                entries.retain(|(k, _)| !dictionary_key_eq(k, key));
                true
            }
            _ => false,
        }
    }

    /// The `in` operator and `hasproperty`.
    pub fn has_property(&mut self, target: &Value, name: &Name) -> Result<bool> {
        let Some((obj, traits, proto)) = self.lookup_target(target) else {
            return Err(self.null_error(target, name));
        };
        let local = name.name.clone().unwrap_or_else(|| "*".into());
        if traits.as_ref().is_some_and(|t| t.find(&local, &name.set).is_some()) {
            return Ok(true);
        }
        if let Some(o) = &obj {
            let d = o.borrow();
            match &d.kind {
                Kind::Array(items) => {
                    if let Some(i) = Self::index_of(name) {
                        return Ok(i < items.len());
                    }
                }
                Kind::Vector(v) => {
                    if let Some(i) = Self::index_of(name) {
                        return Ok(i < v.items.len());
                    }
                }
                Kind::Dictionary(entries) => {
                    let key = Value::String(local.clone());
                    return Ok(entries.iter().any(|(k, _)| dictionary_key_eq(k, &key)));
                }
                _ => {}
            }
            if name.includes_public() && d.dynamic.contains(&local) {
                return Ok(true);
            }
        }
        if name.includes_public() {
            let mut p = proto;
            while let Some(o) = p {
                if o.borrow().dynamic.contains(&local) {
                    return Ok(true);
                }
                p = o.borrow().proto.clone();
            }
        }
        Ok(false)
    }

    /// Whether `target` has `name` as a fixed or own property (scope search).
    fn has_own_or_trait(&self, target: &Value, name: &Name) -> bool {
        let Value::Object(o) = target else {
            return false;
        };
        let d = o.borrow();
        let local = name.name.as_deref().unwrap_or("*");
        if d.traits.as_ref().is_some_and(|t| t.find(local, &name.set).is_some()) {
            return true;
        }
        name.includes_public() && d.dynamic.contains(local)
    }

    /// `findproperty` / `findpropstrict` over a scope stack.
    pub fn find_property(&mut self, scopes: &[Scope], name: &Name, strict: bool) -> Result<Value> {
        for scope in scopes.iter().rev() {
            if scope.with {
                if self.has_property(&scope.value, name)? {
                    return Ok(scope.value.clone());
                }
            } else if self.has_own_or_trait(&scope.value, name) {
                return Ok(scope.value.clone());
            }
        }
        if let Some(g) = self.find_definition(name)? {
            return Ok(g);
        }
        if strict {
            return Err(self.reference_error(1065, format!("Variable {} is not defined.", name.local())));
        }
        Ok(scopes.first().map(|s| s.value.clone()).unwrap_or_default())
    }

    pub fn call_property(&mut self, target: &Value, name: &Name, args: &[Value]) -> Result<Value> {
        if let Some((obj, Some(traits), _)) = self.lookup_target(target) {
            let local = name.name.clone().unwrap_or_else(|| "*".into());
            if let Some((_, Binding::Method(m))) = traits.find(&local, &name.set) {
                let m = m.clone();
                let scope = self.method_scope(target);
                let _ = obj;
                return self.call_method(&m, target.clone(), args, scope, None);
            }
        }
        let f = self.get_property(target, name)?;
        match &f {
            Value::Object(fo) if fo.is_function() || fo.as_class().is_some() => {
                // Calling a property passes the object as receiver, except
                // for primitive receivers of class calls (`String(x)`).
                self.call(&f, target.clone(), args)
            }
            _ => Err(self.type_error(1006, format!("{} is not a function.", name.local()))),
        }
    }

    /// Call a function or class value.
    pub fn call(&mut self, f: &Value, this: Value, args: &[Value]) -> Result<Value> {
        let Value::Object(o) = f else {
            return Err(self.type_error(1006, "value is not a function."));
        };
        let kind = {
            let d = o.borrow();
            match &d.kind {
                Kind::Function(func) => Some(func.clone()),
                _ => None,
            }
        };
        if let Some(func) = kind {
            let this = func.this.clone().unwrap_or(this);
            // Unbound functions called with null receive the global object.
            return self.call_method(&func.method, this, args, func.scope.clone(), Some(o.clone()));
        }
        if let Some(class) = o.as_class() {
            return self.call_class(&class, args);
        }
        Err(self.type_error(1006, "value is not a function."))
    }

    /// Calling a class: conversion for builtins, cast for other classes.
    fn call_class(&mut self, class: &Rc<Class>, args: &[Value]) -> Result<Value> {
        if let Some(call) = class.native.and_then(|n| n.call) {
            return call(self, &Value::Object(class.object()), args);
        }
        let v = args.first().cloned().unwrap_or_default();
        if v.is_nullish() {
            return Ok(Value::Null);
        }
        if self.is_type(&v, class) {
            return Ok(v);
        }
        self.coerce(&v, class)
    }

    pub fn construct(&mut self, ctor: &Value, args: &[Value]) -> Result<Value> {
        let Value::Object(o) = ctor else {
            return Err(self.type_error(1007, "Instantiation attempted on a non-constructor."));
        };
        if let Some(class) = o.as_class() {
            return self.construct_class(&class, args);
        }
        if o.is_function() {
            // A plain function used as a constructor.
            let proto = match self.get_property(ctor, &Name::public("prototype"))? {
                Value::Object(p) => Some(p),
                _ => Some(self.b().object.prototype()),
            };
            let object = Obj::new(ObjectData { class: Some(self.b().object.clone()), ..ObjectData::plain(proto) });
            let r = self.call(ctor, Value::Object(object.clone()), args)?;
            return Ok(match r {
                Value::Object(_) => r,
                _ => Value::Object(object),
            });
        }
        Err(self.type_error(1007, "Instantiation attempted on a non-constructor."))
    }

    pub fn construct_class(&mut self, class: &Rc<Class>, args: &[Value]) -> Result<Value> {
        if class.interface {
            return Err(self.type_error(1001, format!("{} is an interface.", class.name.qualified())));
        }
        let kind = match class.allocator() {
            Some((_, alloc)) => alloc(self, class)?,
            None => Kind::Plain,
        };
        let object = Obj::new(ObjectData {
            class: Some(class.clone()),
            traits: Some(class.instance.clone()),
            proto: Some(class.prototype()),
            slots: class.instance.initial_slots(),
            dynamic: Default::default(),
            closures: HashMap::new(),
            kind,
        });
        let this = Value::Object(object.clone());
        self.run_constructor(class, &this, args)?;
        Ok(this)
    }

    /// Run `class`'s instance initializer on `this` (`construct`/`constructsuper`).
    pub fn run_constructor(&mut self, class: &Rc<Class>, this: &Value, args: &[Value]) -> Result<()> {
        if let Some(init) = &class.instance_init {
            let scope = class.scope.borrow().clone();
            self.call_method(init, this.clone(), args, scope, None)?;
            return Ok(());
        }
        if let Some(init) = class.native.and_then(|n| n.init) {
            init(self, this, args)?;
            return Ok(());
        }
        match &class.super_class {
            Some(s) => self.run_constructor(&s.clone(), this, args),
            None => Ok(()),
        }
    }

    /// Execute a method with a receiver, arguments and captured scope.
    pub fn call_method(
        &mut self,
        method: &Rc<Method>,
        this: Value,
        args: &[Value],
        scope: Rc<[Scope]>,
        callee: Option<Obj>,
    ) -> Result<Value> {
        if self.depth >= MAX_CALL_DEPTH {
            return Err(Error::Internal("Error #1023: Stack overflow occurred.".into()));
        }
        self.depth += 1;
        let result = match &method.kind {
            MethodKind::Native(f) => f(self, &this, args),
            MethodKind::Bytecode { unit, index } => {
                let (unit, index) = (unit.clone(), *index);
                interpreter::run(self, &unit, index, this, args, scope, callee)
            }
        };
        self.depth -= 1;
        result
    }

    /// Resolve a class by qualified name (`getDefinitionByName`).
    pub fn class_by_name(&mut self, qualified: &str) -> Result<Option<Value>> {
        let (ns, name) = match qualified.rsplit_once("::").or_else(|| qualified.rsplit_once('.')) {
            Some((ns, name)) => (ns, name),
            None => ("", qualified),
        };
        self.get_definition(&QName::new(Namespace::package(ns), name))
    }

    /// Create a class from an ABC class definition (`newclass`).
    pub fn new_class(&mut self, unit: &Rc<Unit>, index: u32, base: Value, scope: Rc<[Scope]>) -> Result<Obj> {
        let instance = &unit.abc.instances[index as usize];
        let name = unit.pools.qname(instance.name.0).ok_or_else(|| Error::Internal("class name".into()))?;
        let super_class = match &base {
            Value::Object(o) => Some(o.as_class().ok_or_else(|| Error::Internal("base is not a class".into()))?),
            _ => None,
        };
        let mut interfaces = Vec::new();
        for i in &instance.interfaces {
            let iface = match unit.pools.name(i.0) {
                PoolName::Multiname { name, set, .. } => {
                    let n = Name { name: Some(name), set, attribute: false };
                    let holder = self.find_property(&scope, &n, true)?;
                    self.get_property(&holder, &n)?
                }
                PoolName::QName { name, .. } => self.get_definition(&name)?.unwrap_or_default(),
                _ => Value::Undefined,
            };
            if let Some(c) = iface.as_object().and_then(Obj::as_class) {
                interfaces.push(c);
            }
        }
        let mut traits = unit::build_traits(unit, &instance.traits, super_class.as_ref().map(|s| &*s.instance))?;
        // Interface method names alias the public implementation (Tamarin
        // binds interface namespaces to the class's public methods).
        let mut all = Vec::new();
        fn collect(c: &Rc<Class>, out: &mut Vec<Rc<Class>>) {
            out.push(c.clone());
            for i in &c.interfaces {
                collect(i, out);
            }
        }
        for i in &interfaces {
            collect(i, &mut all);
        }
        for iface in &all {
            for q in iface.instance.bindings.keys() {
                let public = QName { ns: Namespace::public(), name: q.name.clone() };
                if let Some(b) = traits.bindings.get(&public).cloned() {
                    traits.bindings.entry(q.clone()).or_insert(b);
                }
            }
        }
        let class_def = &unit.abc.classes[index as usize];
        let statics = unit::build_traits(unit, &class_def.traits, None)?;
        let class = Rc::new(Class {
            name,
            super_class: super_class.clone(),
            interfaces,
            instance: Rc::new(traits),
            statics: Rc::new(statics),
            instance_init: Some(unit.method(instance.init_method.0)),
            sealed: instance.is_sealed,
            interface: instance.is_interface,
            native: None,
            object: Default::default(),
            prototype: Default::default(),
            scope: Default::default(),
            applied: None,
        });
        // Methods close over the defining scope plus the class object.
        let object = self.finish_class(&class, Rc::new([]));
        let mut chain: Vec<Scope> = scope.to_vec();
        chain.push(Scope { value: Value::Object(object.clone()), with: false });
        let chain: Rc<[Scope]> = chain.into();
        *class.scope.borrow_mut() = chain.clone();
        unit.classes.borrow_mut()[index as usize] = Some(class.clone());
        for (q, method) in unit::function_traits(unit, &class_def.traits) {
            let f = self.function(unit.method(method), chain.clone(), None);
            self.init_property(&Value::Object(object.clone()), &Name::qname(&q), f.into())?;
        }
        let cinit = unit.method(class_def.init_method.0);
        self.call_method(&cinit, Value::Object(object.clone()), &[], chain, None)?;
        Ok(object)
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

/// Dictionary key identity: objects by reference, primitives by value
/// (with numeric strings and numbers kept distinct, as Flash does).
pub fn dictionary_key_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => x == y || (x.is_nan() && y.is_nan()),
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Null, Value::Null) | (Value::Undefined, Value::Undefined) => true,
        // A number key and its string spelling address the same slot.
        (Value::Number(n), Value::String(s)) | (Value::String(s), Value::Number(n)) => {
            *value::number_to_string(*n) == **s
        }
        _ => false,
    }
}

fn dictionary_set(entries: &mut Vec<(Value, Value)>, key: Value, value: Value) {
    match entries.iter_mut().find(|(k, _)| dictionary_key_eq(k, &key)) {
        Some(e) => e.1 = value,
        None => entries.push((key, value)),
    }
}

#[cfg(test)]
mod tests;
