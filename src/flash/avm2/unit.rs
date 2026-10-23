//! One loaded ABC block: resolved pools, decoded bodies and its classes.
use super::name::{Pools, QName};
use super::object::{Binding, Class, Method, MethodKind, Slot, Traits};
use super::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use swf::avm2::read::Reader;
use swf::avm2::types::{AbcFile, DefaultValue, Op, Trait, TraitKind};
use swf::extensions::ReadSwfExt;

#[derive(Debug)]
pub struct Handler {
    pub from: usize,
    pub to: usize,
    pub target: usize,
    pub type_name: u32,
    pub variable: u32,
}

/// Branch target of an op whose offset does not land on an op boundary.
/// Compilers leave such jumps in unreachable code; the verifier only
/// follows reachable paths, so the error is raised if one executes.
pub const INVALID_TARGET: usize = usize::MAX;

/// A decoded method body. Branch targets are op indices.
pub struct Body {
    pub ops: Vec<Op>,
    /// Op index targeted by each branch op (relative jumps resolved).
    pub targets: Vec<Vec<usize>>,
    pub handlers: Vec<Handler>,
    pub locals: usize,
    pub activation: Rc<Traits>,
}

pub struct Unit {
    pub id: u32,
    pub abc: AbcFile,
    pub pools: Pools,
    body_of: Vec<Option<usize>>,
    bodies: RefCell<HashMap<usize, Rc<Body>>>,
    methods: RefCell<HashMap<u32, Rc<Method>>>,
    pub classes: RefCell<Vec<Option<Rc<Class>>>>,
    labels: RefCell<Option<HashMap<u32, Rc<str>>>>,
    /// The VM's Namespace class, for namespace-valued constants.
    namespace: Option<Rc<Class>>,
}

impl std::fmt::Debug for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Unit({})", self.id)
    }
}

impl Unit {
    pub fn new(id: u32, abc: AbcFile) -> Rc<Self> {
        Self::with_namespace_class(id, abc, None)
    }

    pub fn with_namespace_class(id: u32, abc: AbcFile, namespace: Option<Rc<Class>>) -> Rc<Self> {
        let pools = Pools::new(id, &abc);
        let mut body_of = vec![None; abc.methods.len()];
        for (i, body) in abc.method_bodies.iter().enumerate() {
            if let Some(slot) = body_of.get_mut(body.method.0 as usize) {
                *slot = Some(i);
            }
        }
        let classes = RefCell::new(vec![None; abc.classes.len()]);
        Rc::new(Unit {
            id,
            abc,
            pools,
            body_of,
            bodies: RefCell::default(),
            methods: RefCell::default(),
            classes,
            labels: RefCell::default(),
            namespace,
        })
    }

    /// `Class.member` for diagnostics (built on first use).
    pub fn method_label(&self, index: u32) -> Rc<str> {
        let mut labels = self.labels.borrow_mut();
        let map = labels.get_or_insert_with(|| {
            let mut map = HashMap::new();
            let name = |i: u32| self.pools.qname(i).map(|q| q.name.to_string()).unwrap_or_default();
            let add = |map: &mut HashMap<u32, Rc<str>>, owner: &str, traits: &[Trait]| {
                for t in traits {
                    if let TraitKind::Method { method, .. } | TraitKind::Getter { method, .. } | TraitKind::Setter { method, .. } = &t.kind {
                        map.insert(method.0, format!("{owner}.{}", name(t.name.0)).into());
                    }
                }
            };
            for (i, inst) in self.abc.instances.iter().enumerate() {
                let owner = name(inst.name.0);
                add(&mut map, &owner, &inst.traits);
                add(&mut map, &format!("{owner}$"), &self.abc.classes[i].traits);
                map.insert(inst.init_method.0, format!("{owner}()").into());
                map.insert(self.abc.classes[i].init_method.0, format!("{owner}$cinit").into());
            }
            map
        });
        map.get(&index).cloned().unwrap_or_else(|| format!("method#{index}").into())
    }

    pub fn method(self: &Rc<Self>, index: u32) -> Rc<Method> {
        if let Some(m) = self.methods.borrow().get(&index) {
            return m.clone();
        }
        let name = self
            .abc
            .methods
            .get(index as usize)
            .map(|m| self.pools.string(m.name.0))
            .unwrap_or_default();
        let method = Rc::new(Method { name, kind: MethodKind::Bytecode { unit: self.clone(), index }, owner: RefCell::default() });
        self.methods.borrow_mut().insert(index, method.clone());
        method
    }

    pub fn body(self: &Rc<Self>, method: u32) -> Result<Rc<Body>, String> {
        let index = self
            .body_of
            .get(method as usize)
            .copied()
            .flatten()
            .ok_or_else(|| format!("method {method} has no body"))?;
        if let Some(b) = self.bodies.borrow().get(&index) {
            return Ok(b.clone());
        }
        let source = &self.abc.method_bodies[index];
        let code = &source.code;
        let mut reader = Reader::new(code);
        let mut ops = Vec::new();
        let mut starts = Vec::new();
        let mut ends = Vec::new();
        while reader.pos(code) < code.len() {
            starts.push(reader.pos(code));
            ops.push(reader.read_op().map_err(|e| format!("method {method}: {e}"))?);
            ends.push(reader.pos(code));
        }
        let index_at = |byte: i64| -> Result<usize, String> {
            starts
                .binary_search(&(byte as usize))
                .map_err(|_| format!("method {method}: branch into the middle of an op at {byte}"))
        };
        let mut targets = Vec::with_capacity(ops.len());
        for (i, op) in ops.iter().enumerate() {
            let after = ends[i] as i64;
            targets.push(match op {
                Op::Jump { offset }
                | Op::IfEq { offset }
                | Op::IfFalse { offset }
                | Op::IfGe { offset }
                | Op::IfGt { offset }
                | Op::IfLe { offset }
                | Op::IfLt { offset }
                | Op::IfNe { offset }
                | Op::IfNge { offset }
                | Op::IfNgt { offset }
                | Op::IfNle { offset }
                | Op::IfNlt { offset }
                | Op::IfStrictEq { offset }
                | Op::IfStrictNe { offset }
                | Op::IfTrue { offset } => {
                    let target = after + *offset as i64;
                    // A jump to the end of the code is a fallthrough return.
                    if target as usize == code.len() {
                        vec![ops.len()]
                    } else {
                        vec![index_at(target).unwrap_or(INVALID_TARGET)]
                    }
                }
                // Lookupswitch offsets are relative to the op itself.
                Op::LookupSwitch(table) => {
                    let base = starts[i] as i64;
                    let mut t = vec![index_at(base + table.default_offset as i64).unwrap_or(INVALID_TARGET)];
                    for c in table.case_offsets.iter() {
                        t.push(index_at(base + *c as i64).unwrap_or(INVALID_TARGET));
                    }
                    t
                }
                _ => Vec::new(),
            });
        }
        let to_index = |byte: u32| -> usize {
            match starts.binary_search(&(byte as usize)) {
                Ok(i) => i,
                Err(i) => i,
            }
        };
        let handlers = source
            .exceptions
            .iter()
            .map(|e| {
                Ok(Handler {
                    from: to_index(e.from_offset),
                    to: to_index(e.to_offset),
                    target: index_at(e.target_offset as i64)?,
                    type_name: e.type_name.0,
                    variable: e.variable_name.0,
                })
            })
            .collect::<Result<_, String>>()?;
        let activation = Rc::new(build_traits(self, &source.traits, None)?);
        let body = Rc::new(Body {
            ops,
            targets,
            handlers,
            locals: source.num_locals as usize,
            activation,
        });
        self.bodies.borrow_mut().insert(index, body.clone());
        Ok(body)
    }

    pub fn default_value(&self, value: &Option<DefaultValue>, type_name: u32) -> Value {
        match value {
            Some(v) => self.constant(v),
            None => self.type_default(type_name),
        }
    }

    pub fn constant(&self, v: &DefaultValue) -> Value {
        let pool = &self.abc.constant_pool;
        match v {
            DefaultValue::Int(i) => Value::Number(pool.ints.get(i.0 as usize - 1).copied().unwrap_or(0) as f64),
            DefaultValue::Uint(i) => Value::Number(pool.uints.get(i.0 as usize - 1).copied().unwrap_or(0) as f64),
            DefaultValue::Double(i) => Value::Number(pool.doubles.get(i.0 as usize - 1).copied().unwrap_or(f64::NAN)),
            DefaultValue::String(i) => Value::String(self.pools.string(i.0)),
            DefaultValue::True => Value::Bool(true),
            DefaultValue::False => Value::Bool(false),
            DefaultValue::Null => Value::Null,
            DefaultValue::Undefined => Value::Undefined,
            DefaultValue::Namespace(i)
            | DefaultValue::Package(i)
            | DefaultValue::PackageInternal(i)
            | DefaultValue::Protected(i)
            | DefaultValue::Explicit(i)
            | DefaultValue::StaticProtected(i)
            | DefaultValue::Private(i) => {
                let ns = self.pools.namespace(i.0);
                match &self.namespace {
                    Some(class) => Value::Object(super::object::Obj::new(super::object::ObjectData {
                        class: Some(class.clone()),
                        traits: Some(class.instance.clone()),
                        kind: super::object::Kind::Namespace(ns),
                        ..super::object::ObjectData::plain(Some(class.prototype()))
                    })),
                    None => Value::Undefined,
                }
            }
        }
    }

    /// The AVM2 default for an uninitialized slot of the declared type.
    pub fn type_default(&self, type_name: u32) -> Value {
        if type_name == 0 {
            return Value::Undefined;
        }
        match self.pools.qname(type_name) {
            Some(q) if q.ns.is_public() => match &*q.name {
                "int" | "uint" => Value::Number(0.0),
                "Number" => Value::Number(f64::NAN),
                "Boolean" => Value::Bool(false),
                _ => Value::Null,
            },
            _ => Value::Null,
        }
    }
}

/// Lay out `traits` after `base` (instance traits of a superclass). Explicit
/// slot ids of a derived class follow the base class's slots, as in Tamarin.
pub fn build_traits(unit: &Rc<Unit>, traits: &[Trait], base: Option<&Traits>) -> Result<Traits, String> {
    let mut out = Traits::default();
    let offset = base.map_or(0, |b| b.slots.len().saturating_sub(1));
    if let Some(b) = base {
        out.bindings = b.bindings.clone();
        out.slots = b.slots.clone();
    }
    if out.slots.is_empty() {
        out.slots.push(None);
    }
    for t in traits {
        let name = unit
            .pools
            .qname(t.name.0)
            .ok_or_else(|| format!("trait name {} is not a QName", t.name.0))?;
        let slot = |out: &mut Traits, id: u32, slot: Slot| -> u32 {
            let id = if id == 0 { out.slots.len() as u32 } else { id + offset as u32 };
            if out.slots.len() <= id as usize {
                out.slots.resize(id as usize + 1, None);
            }
            out.slots[id as usize] = Some(slot);
            id
        };
        match &t.kind {
            TraitKind::Slot { slot_id, type_name, value } | TraitKind::Const { slot_id, type_name, value } => {
                let constant = matches!(t.kind, TraitKind::Const { .. });
                let s = Slot {
                    name: name.clone(),
                    type_name: type_name.0,
                    unit: Some(unit.clone()),
                    default: unit.default_value(value, type_name.0),
                    constant,
                };
                let id = slot(&mut out, *slot_id, s);
                out.bindings.insert(name, Binding::Slot(id));
            }
            TraitKind::Class { slot_id, .. } | TraitKind::Function { slot_id, .. } => {
                let s = Slot {
                    name: name.clone(),
                    type_name: 0,
                    unit: Some(unit.clone()),
                    default: Value::Undefined,
                    constant: true,
                };
                let id = slot(&mut out, *slot_id, s);
                out.bindings.insert(name, Binding::Slot(id));
            }
            TraitKind::Method { method, .. } => {
                out.bindings.insert(name, Binding::Method(unit.method(method.0)));
            }
            TraitKind::Getter { method, .. } => {
                let get = Some(unit.method(method.0));
                let entry = out.bindings.entry(name).or_insert(Binding::Accessor { get: None, set: None });
                match entry {
                    Binding::Accessor { get: g, .. } => *g = get,
                    other => *other = Binding::Accessor { get, set: None },
                }
            }
            TraitKind::Setter { method, .. } => {
                let set = Some(unit.method(method.0));
                let entry = out.bindings.entry(name).or_insert(Binding::Accessor { get: None, set: None });
                match entry {
                    Binding::Accessor { set: s, .. } => *s = set,
                    other => *other = Binding::Accessor { get: None, set },
                }
            }
        }
        alias_protected(&mut out, &unit.pools.qname(t.name.0).unwrap());
    }
    Ok(out)
}

/// A protected override is declared in the derived class's own protected
/// namespace; it replaces the inherited binding of the same local name in
/// the base classes' protected namespaces (Tamarin matches protected names
/// across the hierarchy).
fn alias_protected(out: &mut Traits, name: &QName) {
    if !matches!(name.ns, crate::flash::avm2::name::Namespace::Protected(_)) {
        return;
    }
    let Some(binding) = out.bindings.get(name).cloned() else { return };
    let inherited: Vec<QName> = out
        .bindings
        .keys()
        .filter(|k| k.name == name.name && *k != name && matches!(k.ns, crate::flash::avm2::name::Namespace::Protected(_)))
        .cloned()
        .collect();
    for k in inherited {
        let merged = match (&binding, out.bindings.get(&k)) {
            // A derived getter or setter alone keeps the inherited partner.
            (Binding::Accessor { get, set }, Some(Binding::Accessor { get: g0, set: s0 })) => {
                Binding::Accessor { get: get.clone().or_else(|| g0.clone()), set: set.clone().or_else(|| s0.clone()) }
            }
            _ => binding.clone(),
        };
        if matches!(merged, Binding::Accessor { .. }) {
            out.bindings.insert(name.clone(), merged.clone());
        }
        out.bindings.insert(k, merged);
    }
}

/// Function traits (`TraitKind::Function`) that must be closed over when an
/// object with these traits is created.
pub fn function_traits(unit: &Rc<Unit>, traits: &[Trait]) -> Vec<(QName, u32)> {
    traits
        .iter()
        .filter_map(|t| match &t.kind {
            TraitKind::Function { function, .. } => Some((unit.pools.qname(t.name.0)?, function.0)),
            _ => None,
        })
        .collect()
}
