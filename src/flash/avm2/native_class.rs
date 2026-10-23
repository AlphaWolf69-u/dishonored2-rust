//! Native classes built from Iggy's registration table.
//!
//! Every native class takes its members, namespaces, constants and trivial
//! handlers from `crate::flash::native_table`. A Rust `Spec` supplies the
//! implementations of non-trivial natives; a native Iggy implements that
//! has no Rust implementation yet becomes `MethodKind::Unimplemented`, and
//! Spec members the table does not declare are reported.
use super::builtins::Spec;
use super::name::{Namespace, QName};
use super::object::{Binding, Method, MethodKind, NativeFn, Slot, Traits};
use super::value::Value;
use super::{Result, Vm};
use crate::flash::native_decl::{ClassDecl, Cond, Const, Imp, Kind, Member, Ns};
use std::cell::RefCell;
use std::rc::Rc;

pub const AS3: &str = "http://adobe.com/AS3/2006/builtin";

pub fn namespace(ns: Ns) -> Namespace {
    match ns {
        Ns::Public => Namespace::public(),
        Ns::As3 => Namespace::package(AS3),
        Ns::Uri(u) => Namespace::package(u),
    }
}

pub fn constant(c: Const) -> Value {
    match c {
        Const::Int(i) => Value::Number(i as f64),
        Const::Number(n) => Value::Number(n),
        Const::Str(s) => Value::from(s),
        Const::Bool(b) => Value::Bool(b),
        Const::Null => Value::Null,
        Const::Undefined => Value::Undefined,
    }
}

/// Whether a member is registered in this player. 3D members require a
/// SWF version above 9 (every installed UI movie uses Matrix3D); gated
/// classes are present in full (non-light) VMs.
fn registered(cond: Cond) -> bool {
    matches!(cond, Cond::Always | Cond::SwfVersionAbove9 | Cond::Unknown)
}

fn throw_illegal_operation(vm: &mut Vm, _: &Value, _: &[Value]) -> Result<Value> {
    Err(vm.illegal_operation())
}

fn throw_type(vm: &mut Vm, _: &Value, _: &[Value]) -> Result<Value> {
    Err(vm.type_error(0, ""))
}

fn noop(_: &mut Vm, _: &Value, _: &[Value]) -> Result<Value> {
    Ok(Value::Undefined)
}

fn return_this(_: &mut Vm, this: &Value, _: &[Value]) -> Result<Value> {
    Ok(this.clone())
}

fn trivial(imp: Imp) -> Option<NativeFn> {
    Some(match imp {
        Imp::ThrowIllegalOperation => throw_illegal_operation,
        Imp::ThrowType => throw_type,
        Imp::Noop => noop,
        Imp::ReturnThis => return_this,
        Imp::Return(Const::Bool(false)) => |_, _, _| Ok(Value::Bool(false)),
        Imp::Return(Const::Bool(true)) => |_, _, _| Ok(Value::Bool(true)),
        Imp::Return(Const::Null) => |_, _, _| Ok(Value::Null),
        Imp::Return(Const::Undefined) => |_, _, _| Ok(Value::Undefined),
        Imp::Return(Const::Int(0)) => |_, _, _| Ok(Value::Number(0.0)),
        Imp::Return(Const::Int(1)) => |_, _, _| Ok(Value::Number(1.0)),
        Imp::Return(Const::Int(1048576)) => |_, _, _| Ok(Value::Number(1048576.0)),
        _ => return None,
    })
}

/// The method for a member: Iggy's trivial handler, the Rust port of its
/// native, or an explicit unimplemented marker.
fn method(member: &Member, port: Option<NativeFn>, report: &mut Vec<String>, owner: &str) -> Rc<Method> {
    if let Some(f) = trivial(member.imp) {
        if port.is_some() {
            report.push(format!("{owner}.{}: Iggy uses a trivial handler {:?}; the Rust port is ignored", member.name, member.imp));
        }
        return Method::native(member.name, f);
    }
    match (member.imp, port) {
        (_, Some(f)) => Method::native(member.name, f),
        (Imp::Native(address), None) => Rc::new(Method {
            name: member.name.into(),
            kind: MethodKind::Unimplemented(address),
            owner: RefCell::default(),
        }),
        (Imp::Interface, None) => Method::native(member.name, throw_type),
        (imp, None) => {
            report.push(format!("{owner}.{}: no implementation for {imp:?}", member.name));
            Method::native(member.name, throw_illegal_operation)
        }
    }
}

fn accessor(traits: &mut Traits, q: QName, get: Option<Rc<Method>>, set: Option<Rc<Method>>) {
    let entry = traits.bindings.entry(q).or_insert(Binding::Accessor { get: None, set: None });
    match entry {
        Binding::Accessor { get: g, set: s } => {
            if get.is_some() {
                *g = get;
            }
            if set.is_some() {
                *s = set;
            }
        }
        other => *other = Binding::Accessor { get, set },
    }
}

fn slot(traits: &mut Traits, q: QName, value: Value, constant: bool) {
    let id = traits.slots.len() as u32;
    traits.slots.push(Some(Slot { name: q.clone(), type_name: 0, unit: None, default: value, constant }));
    traits.bindings.insert(q, Binding::Slot(id));
}

fn field(member: &Member, set: bool) -> Rc<Method> {
    Rc::new(Method {
        name: member.name.into(),
        kind: MethodKind::Field { offset: member.offset, ty: member.ty, set },
        owner: RefCell::default(),
    })
}

/// Traits and prototype functions of a table class.
pub struct Built {
    pub instance: Traits,
    pub statics: Traits,
    pub proto: Vec<(&'static str, Rc<Method>)>,
}

fn lookup(list: &[(&'static str, NativeFn)], name: &str) -> Option<NativeFn> {
    list.iter().find(|(n, _)| *n == name).map(|(_, f)| *f)
}

/// Build traits for `decl` on top of the base class's instance traits and
/// the Class instance traits (class objects are Class instances).
pub fn build(decl: &ClassDecl, spec: &Spec, base: Option<&Traits>, class_instance: Option<&Traits>, report: &mut Vec<String>) -> Built {
    let owner = if decl.package.is_empty() { decl.name.to_string() } else { format!("{}.{}", decl.package, decl.name) };
    let mut instance = Traits::default();
    if let Some(b) = base {
        instance.bindings = b.bindings.clone();
        instance.slots = b.slots.clone();
    }
    if instance.slots.is_empty() {
        instance.slots.push(None);
    }
    let mut statics = Traits::default();
    if let Some(c) = class_instance {
        statics.bindings = c.bindings.clone();
    }
    statics.slots.push(None);
    let mut proto = Vec::new();
    let mut used: Vec<(&'static str, &'static str)> = Vec::new();
    // An interface declares its methods in the `package:Name` namespace.
    let interface_ns = decl.interface.then(|| {
        let uri = if decl.package.is_empty() { decl.name.to_string() } else { format!("{}:{}", decl.package, decl.name) };
        Namespace::Public(uri.into())
    });
    for m in decl.members.iter().filter(|m| registered(m.cond)) {
        let ns = match (&interface_ns, m.ns) {
            (Some(i), Ns::Public) => i.clone(),
            _ => namespace(m.ns),
        };
        let q = QName::new(ns, m.name);
        match m.kind {
            Kind::Method => {
                let port = lookup(&spec.methods, m.name).or_else(|| lookup(&spec.proto, m.name));
                used.push(("method", m.name));
                instance.bindings.insert(q, Binding::Method(method(m, port, report, &owner)));
            }
            Kind::ProtoMethod => {
                let port = lookup(&spec.proto, m.name).or_else(|| lookup(&spec.methods, m.name));
                used.push(("method", m.name));
                proto.push((m.name, method(m, port, report, &owner)));
            }
            Kind::Getter | Kind::FieldGet | Kind::Setter | Kind::FieldSet => {
                let get = matches!(m.kind, Kind::Getter | Kind::FieldGet);
                let port = if get { lookup(&spec.getters, m.name) } else { lookup(&spec.setters, m.name) };
                used.push((if get { "getter" } else { "setter" }, m.name));
                if port.is_none() {
                    // Interim slot storage for a property the Rust side keeps as a var.
                    if let Some((_, v)) = spec.vars.iter().find(|(n, _)| *n == m.name) {
                        used.push(("var", m.name));
                        if !matches!(instance.bindings.get(&q), Some(Binding::Slot(_))) {
                            slot(&mut instance, q, v.clone(), false);
                        }
                        continue;
                    }
                }
                let method = match (m.kind, port) {
                    (Kind::FieldGet | Kind::FieldSet, None) => field(m, !get),
                    _ => method(m, port, report, &owner),
                };
                if get {
                    accessor(&mut instance, q, Some(method), None);
                } else {
                    accessor(&mut instance, q, None, Some(method));
                }
            }
            Kind::Var => {
                used.push(("var", m.name));
                let v = spec.vars.iter().find(|(n, _)| *n == m.name).map_or_else(|| constant(m.value), |(_, v)| v.clone());
                slot(&mut instance, q, v, false);
            }
            Kind::StaticMethod => {
                used.push(("static", m.name));
                let port = lookup(&spec.statics, m.name);
                statics.bindings.insert(q, Binding::Method(method(m, port, report, &owner)));
            }
            Kind::StaticGetter => {
                used.push(("static_getter", m.name));
                let port = lookup(&spec.static_getters, m.name);
                accessor(&mut statics, q, Some(method(m, port, report, &owner)), None);
            }
            Kind::StaticSetter => {
                used.push(("static_setter", m.name));
                let port = lookup(&spec.static_setters, m.name);
                accessor(&mut statics, q, None, Some(method(m, port, report, &owner)));
            }
            Kind::StaticConst => {
                used.push(("constant", m.name));
                slot(&mut statics, q, constant(m.value), true);
            }
            Kind::StaticVar => {
                used.push(("constant", m.name));
                slot(&mut statics, q, constant(m.value), false);
            }
            Kind::GlobalFunction | Kind::GlobalConst => {}
        }
    }
    // Rust members the table does not declare would be invented API.
    let groups: [(&str, Vec<&str>); 7] = [
        ("method", spec.methods.iter().chain(&spec.proto).map(|(n, _)| *n).collect()),
        ("getter", spec.getters.iter().map(|(n, _)| *n).collect()),
        ("setter", spec.setters.iter().map(|(n, _)| *n).collect()),
        ("static", spec.statics.iter().map(|(n, _)| *n).collect()),
        ("static_getter", spec.static_getters.iter().map(|(n, _)| *n).collect()),
        ("static_setter", spec.static_setters.iter().map(|(n, _)| *n).collect()),
        ("constant", spec.constants.iter().map(|(n, _)| *n).collect()),
    ];
    for (group, names) in groups {
        for n in names {
            if !used.iter().any(|(g, u)| *g == group && *u == n) {
                report.push(format!("{owner}: Rust {group} {n} is not an Iggy member"));
            }
        }
    }
    for (n, _) in &spec.vars {
        if !used.iter().any(|(g, u)| (*g == "var" || *g == "getter") && u == n) {
            report.push(format!("{owner}: Rust var {n} is not an Iggy member"));
        }
    }
    Built { instance, statics, proto }
}
