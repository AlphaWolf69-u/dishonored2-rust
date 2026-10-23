//! The bytecode interpreter.
use super::name::{Name, Namespace, PoolName};
use super::object::{Kind, Obj, ObjectData, Scope, Slot, Traits};
use super::unit::Unit;
use super::value::{self, Value};
use super::{Error, Result, Vm};
use std::rc::Rc;
use swf::avm2::types::{MethodFlags, Op};

/// A resolved operand name. `key` keeps a runtime name value so Dictionary
/// lookups can use object identity.
struct Operand {
    name: Name,
    key: Option<Value>,
}

struct Frame<'a> {
    unit: &'a Rc<Unit>,
    stack: Vec<Value>,
    locals: Vec<Value>,
    scopes: Vec<Scope>,
    base: usize,
}

impl Frame<'_> {
    fn pop(&mut self) -> Result<Value> {
        self.stack.pop().ok_or_else(|| Error::Internal("operand stack underflow".into()))
    }

    fn pop_n(&mut self, n: u32) -> Result<Vec<Value>> {
        let n = n as usize;
        if self.stack.len() < n {
            return Err(Error::Internal("operand stack underflow".into()));
        }
        Ok(self.stack.split_off(self.stack.len() - n))
    }

    fn push(&mut self, v: impl Into<Value>) {
        self.stack.push(v.into());
    }

    fn peek(&self) -> Result<&Value> {
        self.stack.last().ok_or_else(|| Error::Internal("operand stack underflow".into()))
    }
}

fn resolve(vm: &mut Vm, frame: &mut Frame, index: u32) -> Result<Operand> {
    let pool = frame.unit.pools.name(index);
    Ok(match pool {
        PoolName::QName { name, attribute } => Operand {
            name: Name { name: Some(name.name.clone()), set: Rc::new([name.ns]), attribute },
            key: None,
        },
        PoolName::Multiname { name, set, attribute } => Operand { name: Name { name: Some(name), set, attribute }, key: None },
        PoolName::RtQName { name, attribute } => {
            let ns = runtime_namespace(frame.pop()?)?;
            Operand { name: Name { name: Some(name), set: Rc::new([ns]), attribute }, key: None }
        }
        PoolName::RtQNameL { attribute } => {
            let key = frame.pop()?;
            let ns = runtime_namespace(frame.pop()?)?;
            let local = vm.to_string(&key)?;
            Operand { name: Name { name: Some(local), set: Rc::new([ns]), attribute }, key: Some(key) }
        }
        PoolName::MultinameL { set, attribute } => {
            let key = frame.pop()?;
            let local = match &key {
                Value::String(s) => s.clone(),
                Value::Object(_) => "".into(),
                other => vm.to_string(other)?,
            };
            Operand { name: Name { name: Some(local), set, attribute }, key: Some(key) }
        }
        PoolName::TypeName { .. } | PoolName::Any => {
            Operand { name: Name { name: None, set: Rc::new([Namespace::public()]), attribute: false }, key: None }
        }
    })
}

fn runtime_namespace(v: Value) -> Result<Namespace> {
    match &v {
        Value::Object(o) => match &o.borrow().kind {
            Kind::Namespace(ns) => Ok(ns.clone()),
            _ => Err(Error::Internal("runtime namespace is not a Namespace".into())),
        },
        _ => Err(Error::Internal("runtime namespace is not a Namespace".into())),
    }
}

/// Dictionary access with a runtime key keeps object identity.
fn dictionary_target(target: &Value, op: &Operand) -> Option<(Obj, Value)> {
    let key = op.key.as_ref()?;
    let o = target.as_object()?;
    matches!(o.borrow().kind, Kind::Dictionary(_)).then(|| (o.clone(), key.clone()))
}

fn get(vm: &mut Vm, target: &Value, op: &Operand) -> Result<Value> {
    if let Some((o, key)) = dictionary_target(target, op) {
        return Ok(vm.dictionary_get(&o, &key).unwrap_or_default());
    }
    vm.get_property(target, &op.name)
}

fn set(vm: &mut Vm, target: &Value, op: &Operand, value: Value, init: bool) -> Result<()> {
    if let Some((o, key)) = dictionary_target(target, op) {
        vm.set_dictionary(&o, key, value);
        return Ok(());
    }
    if init {
        vm.init_property(target, &op.name, value)
    } else {
        vm.set_property(target, &op.name, value)
    }
}

/// The class whose method is executing: the innermost class object on the
/// captured scope chain (`newclass` appends it).
fn home_class(frame: &Frame) -> Option<Rc<super::object::Class>> {
    frame.scopes[..frame.base].iter().rev().find_map(|s| s.value.as_object().and_then(Obj::as_class))
}

/// ECMA-262 11.8.5; `None` is the undefined result (NaN operands).
fn less_than(vm: &mut Vm, a: &Value, b: &Value) -> Result<Option<bool>> {
    let pa = vm.to_primitive(a, false)?;
    let pb = vm.to_primitive(b, false)?;
    if let (Value::String(x), Value::String(y)) = (&pa, &pb) {
        return Ok(Some(x.encode_utf16().lt(y.encode_utf16())));
    }
    let (x, y) = (pa.primitive_to_number(), pb.primitive_to_number());
    if x.is_nan() || y.is_nan() {
        return Ok(None);
    }
    Ok(Some(x < y))
}

/// ECMA-262 11.9.3 abstract equality.
pub fn loose_equals(vm: &mut Vm, a: &Value, b: &Value) -> Result<bool> {
    use Value::*;
    Ok(match (a, b) {
        (Undefined | Null, Undefined | Null) => true,
        (Undefined | Null, _) | (_, Undefined | Null) => false,
        (Number(x), Number(y)) => x == y,
        (String(x), String(y)) => x == y,
        (Bool(x), Bool(y)) => x == y,
        (Object(x), Object(y)) => {
            if x == y {
                true
            } else {
                let px = primitive_of(x);
                let py = primitive_of(y);
                match (px, py) {
                    (Some(p), Some(q)) => loose_equals(vm, &p, &q)?,
                    _ => super::xml::equals(vm, x, y)?,
                }
            }
        }
        (Number(x), String(_)) => *x == b.primitive_to_number(),
        (String(_), Number(y)) => a.primitive_to_number() == *y,
        (Bool(_), _) => loose_equals(vm, &Number(a.primitive_to_number()), b)?,
        (_, Bool(_)) => loose_equals(vm, a, &Number(b.primitive_to_number()))?,
        (Object(_), _) => {
            let p = vm.to_primitive(a, false)?;
            loose_equals(vm, &p, b)?
        }
        (_, Object(_)) => {
            let p = vm.to_primitive(b, false)?;
            loose_equals(vm, a, &p)?
        }
    })
}

fn primitive_of(o: &Obj) -> Option<Value> {
    match &o.borrow().kind {
        Kind::Primitive(p) => Some(p.clone()),
        _ => None,
    }
}

pub fn strict_equals(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (Undefined, Undefined) | (Null, Null) => true,
        (Number(x), Number(y)) => x == y,
        (String(x), String(y)) => x == y,
        (Bool(x), Bool(y)) => x == y,
        (Object(x), Object(y)) => x == y,
        _ => false,
    }
}

fn add(vm: &mut Vm, a: &Value, b: &Value) -> Result<Value> {
    if let (Value::Number(x), Value::Number(y)) = (a, b) {
        return Ok(Value::Number(x + y));
    }
    if let (Some(x), Some(y)) = (a.as_object(), b.as_object()) {
        if let Some(v) = super::xml::concat(vm, x, y)? {
            return Ok(v);
        }
    }
    let pa = vm.to_primitive(a, false)?;
    let pb = vm.to_primitive(b, false)?;
    if matches!(pa, Value::String(_)) || matches!(pb, Value::String(_)) {
        let mut s = String::from(&*pa.primitive_to_string());
        s.push_str(&pb.primitive_to_string());
        return Ok(Value::from(s));
    }
    Ok(Value::Number(pa.primitive_to_number() + pb.primitive_to_number()))
}

/// Property enumeration order: indexed elements, dictionary keys, then
/// enumerable dynamic properties.
fn enum_count(o: &Obj) -> usize {
    let d = o.borrow();
    let indexed = match &d.kind {
        Kind::Array(items) => items.len(),
        Kind::Vector(v) => v.items.len(),
        Kind::Dictionary(e) => e.len(),
        Kind::XmlList(items) => items.len(),
        _ => 0,
    };
    indexed + d.dynamic.len()
}

fn enum_entry(o: &Obj, i: usize) -> Option<(Value, Value)> {
    let d = o.borrow();
    let indexed = match &d.kind {
        Kind::Array(items) => {
            if i < items.len() {
                return Some((Value::from(i as u32), items[i].clone()));
            }
            items.len()
        }
        Kind::Vector(v) => {
            if i < v.items.len() {
                return Some((Value::from(i as u32), v.items[i].clone()));
            }
            v.items.len()
        }
        Kind::XmlList(items) => {
            if i < items.len() {
                return Some((Value::from(i as u32), items[i].clone()));
            }
            items.len()
        }
        Kind::Dictionary(e) => {
            if i < e.len() {
                return Some(e[i].clone());
            }
            e.len()
        }
        _ => 0,
    };
    let (key, enumerable) = d.dynamic.key_at(i - indexed)?;
    enumerable.then(|| (Value::String(key.clone()), d.dynamic.get(key).cloned().unwrap_or_default()))
}

/// `hasnext2`: advance `(object, index)` to the next enumerable property.
fn has_next(vm: &Vm, object: &mut Value, index: &mut u32) {
    loop {
        let Value::Object(o) = object.clone() else {
            *index = 0;
            return;
        };
        let count = enum_count(&o);
        let mut i = *index as usize;
        while i < count {
            if enum_entry(&o, i).is_some() {
                *index = i as u32 + 1;
                return;
            }
            i += 1;
        }
        // Continue with the prototype chain's dynamic properties.
        let proto = o.borrow().proto.clone();
        let _ = vm;
        match proto {
            Some(p) if enum_count(&p) > 0 => {
                *object = Value::Object(p);
                *index = 0;
            }
            _ => {
                *object = Value::Null;
                *index = 0;
                return;
            }
        }
    }
}

fn catch_scope(handler: &super::unit::Handler, unit: &Rc<Unit>) -> Obj {
    let mut traits = Traits::default();
    traits.slots.push(None);
    if let Some(q) = unit.pools.qname(handler.variable) {
        traits.slots.push(Some(Slot { name: q.clone(), type_name: 0, unit: None, default: Value::Undefined, constant: false }));
        traits.bindings.insert(q, super::object::Binding::Slot(1));
    }
    let traits = Rc::new(traits);
    Obj::new(ObjectData { slots: traits.initial_slots(), traits: Some(traits), ..ObjectData::plain(None) })
}

pub fn run(
    vm: &mut Vm,
    unit: &Rc<Unit>,
    index: u32,
    mut this: Value,
    args: &[Value],
    outer: Rc<[Scope]>,
    callee: Option<Obj>,
) -> Result<Value> {
    let info = &unit.abc.methods[index as usize];
    let body = unit.body(index)?;
    let declared = info.params.len();
    let flags = info.flags;
    let required = declared - info.params.iter().filter(|p| p.default_value.is_some()).count();
    let rest = flags.contains(MethodFlags::NEED_REST) || flags.contains(MethodFlags::NEED_ARGUMENTS);
    if args.len() < required || (args.len() > declared && !rest && !flags.contains(MethodFlags::IGNORE_REST)) {
        return Err(vm.argument_error(
            1063,
            format!("Argument count mismatch on {}. Expected {}, got {}.", info.name.0, declared, args.len()),
        ));
    }
    if this.is_nullish() {
        if let Some(global) = outer.first() {
            this = global.value.clone();
        }
    }
    let mut locals = vec![Value::Undefined; body.locals.max(declared + 2)];
    locals[0] = this;
    for (i, param) in info.params.iter().enumerate() {
        let v = match args.get(i) {
            Some(v) => v.clone(),
            None => param.default_value.as_ref().map(|d| unit.constant(d)).unwrap_or_default(),
        };
        locals[i + 1] = if param.kind.0 == 0 {
            v
        } else {
            match vm.resolve_type(unit, param.kind.0)? {
                Some(class) => vm.coerce(&v, &class)?,
                None => v,
            }
        };
    }
    if flags.contains(MethodFlags::NEED_REST) {
        locals[declared + 1] = Value::Object(vm.array(args.get(declared..).unwrap_or_default().to_vec()));
    } else if flags.contains(MethodFlags::NEED_ARGUMENTS) {
        let arguments = vm.array(args.to_vec());
        if let Some(c) = callee {
            arguments.borrow_mut().dynamic.set("callee".into(), Value::Object(c));
            arguments.borrow_mut().dynamic.set_enumerable("callee", false);
        }
        locals[declared + 1] = Value::Object(arguments);
    }
    let base = outer.len();
    let mut frame = Frame { unit, stack: Vec::new(), locals, scopes: outer.to_vec(), base };
    let mut pc = 0usize;
    let result = loop {
        if pc >= body.ops.len() {
            break Ok(Value::Undefined);
        }
        match step(vm, &mut frame, &body, &mut pc) {
            Ok(Some(v)) => break Ok(v),
            Ok(None) => {}
            Err(Error::Throw(v)) => {
                let at = pc;
                let mut handled = false;
                for h in &body.handlers {
                    if at < h.from || at >= h.to {
                        continue;
                    }
                    let matches = h.type_name == 0
                        || match vm.resolve_type(unit, h.type_name)? {
                            Some(class) => vm.is_type(&v, &class),
                            None => false,
                        };
                    if matches {
                        frame.stack.clear();
                        frame.scopes.truncate(frame.base);
                        frame.stack.push(v.clone());
                        pc = h.target;
                        handled = true;
                        break;
                    }
                }
                if !handled {
                    break Err(Error::Throw(v));
                }
            }
            Err(e) => break Err(e),
        }
    };
    let value = result?;
    if info.return_type.0 != 0 {
        if let Some(class) = vm.resolve_type(unit, info.return_type.0)? {
            return vm.coerce(&value, &class);
        }
    }
    Ok(value)
}

fn branch(pc: &mut usize, body: &super::unit::Body, taken: bool) {
    if taken {
        *pc = body.targets[*pc][0];
    } else {
        *pc += 1;
    }
}

fn int_pool(unit: &Unit, i: u32) -> f64 {
    unit.abc.constant_pool.ints.get(i as usize - 1).copied().unwrap_or(0) as f64
}

fn step(vm: &mut Vm, f: &mut Frame, body: &super::unit::Body, pc: &mut usize) -> Result<Option<Value>> {
    let op = &body.ops[*pc];
    let unit = f.unit;
    match op {
        Op::Nop | Op::Label | Op::Bkpt | Op::BkptLine { .. } | Op::Debug { .. } | Op::DebugFile { .. } | Op::DebugLine { .. } | Op::Timestamp => {}
        Op::Throw => {
            let v = f.pop()?;
            return Err(Error::Throw(v));
        }
        Op::GetSuper { index } => {
            let op = resolve(vm, f, index.0)?;
            let target = f.pop()?;
            let class = home_class(f).and_then(|c| c.super_class.clone()).ok_or_else(|| Error::Internal("getsuper without base".into()))?;
            let v = super_get(vm, &class, &target, &op.name)?;
            f.push(v);
        }
        Op::SetSuper { index } => {
            let value = f.pop()?;
            let op = resolve(vm, f, index.0)?;
            let target = f.pop()?;
            let class = home_class(f).and_then(|c| c.super_class.clone()).ok_or_else(|| Error::Internal("setsuper without base".into()))?;
            super_set(vm, &class, &target, &op.name, value)?;
        }
        Op::Dxns { .. } | Op::DxnsLate => {
            if matches!(op, Op::DxnsLate) {
                f.pop()?;
            }
        }
        Op::Kill { index } => f.locals[*index as usize] = Value::Undefined,
        Op::IfNlt { .. } | Op::IfNle { .. } | Op::IfNgt { .. } | Op::IfNge { .. } | Op::IfLt { .. } | Op::IfLe { .. } | Op::IfGt { .. } | Op::IfGe { .. } => {
            let b = f.pop()?;
            let a = f.pop()?;
            let r = match op {
                Op::IfLt { .. } | Op::IfNlt { .. } => less_than(vm, &a, &b)?,
                Op::IfGt { .. } | Op::IfNgt { .. } => less_than(vm, &b, &a)?,
                Op::IfLe { .. } | Op::IfNle { .. } => less_than(vm, &b, &a)?.map(|r| !r),
                _ => less_than(vm, &a, &b)?.map(|r| !r),
            };
            let truth = r == Some(true);
            let taken = match op {
                Op::IfLt { .. } | Op::IfGt { .. } | Op::IfLe { .. } | Op::IfGe { .. } => truth,
                _ => !truth,
            };
            branch(pc, body, taken);
            return Ok(None);
        }
        Op::IfTrue { .. } => {
            let v = f.pop()?.to_boolean();
            branch(pc, body, v);
            return Ok(None);
        }
        Op::IfFalse { .. } => {
            let v = f.pop()?.to_boolean();
            branch(pc, body, !v);
            return Ok(None);
        }
        Op::IfEq { .. } | Op::IfNe { .. } => {
            let b = f.pop()?;
            let a = f.pop()?;
            let eq = loose_equals(vm, &a, &b)?;
            branch(pc, body, eq == matches!(op, Op::IfEq { .. }));
            return Ok(None);
        }
        Op::IfStrictEq { .. } | Op::IfStrictNe { .. } => {
            let b = f.pop()?;
            let a = f.pop()?;
            let eq = strict_equals(&a, &b);
            branch(pc, body, eq == matches!(op, Op::IfStrictEq { .. }));
            return Ok(None);
        }
        Op::Jump { .. } => {
            branch(pc, body, true);
            return Ok(None);
        }
        Op::LookupSwitch(_) => {
            let i = f.pop()?.primitive_to_number();
            let targets = &body.targets[*pc];
            let case = if i.fract() == 0.0 && i >= 0.0 && (i as usize) < targets.len() - 1 { i as usize + 1 } else { 0 };
            *pc = targets[case];
            return Ok(None);
        }
        Op::PushWith => {
            let v = f.pop()?;
            if v.is_nullish() {
                return Err(vm.type_error(1009, "Cannot access a property or method of a null object reference."));
            }
            f.scopes.push(Scope { value: v, with: true });
        }
        Op::PopScope => {
            if f.scopes.len() > f.base {
                f.scopes.pop();
            }
        }
        Op::NextName | Op::NextValue => {
            let i = vm.to_uint(&f.pop()?)? as usize;
            let o = f.pop()?;
            let v = match o.as_object() {
                Some(o) if i > 0 => match enum_entry(o, i - 1) {
                    Some((k, v)) => {
                        if matches!(op, Op::NextName) {
                            // Array indices enumerate as strings.
                            match k {
                                Value::Number(n) if !matches!(o.borrow().kind, Kind::Dictionary(_)) => Value::from(value::number_to_string(n)),
                                k => k,
                            }
                        } else {
                            v
                        }
                    }
                    None => Value::Undefined,
                },
                _ => Value::Undefined,
            };
            f.push(v);
        }
        Op::HasNext => {
            let mut i = vm.to_uint(&f.pop()?)?;
            let mut o = f.pop()?;
            has_next(vm, &mut o, &mut i);
            // `hasnext` cannot advance the object; it reports exhaustion of `o`.
            f.push(Value::from(i));
        }
        Op::PushNull => f.push(Value::Null),
        Op::PushUndefined => f.push(Value::Undefined),
        Op::PushByte { value } => f.push(Value::from(*value as i8 as i32)),
        Op::PushShort { value } => f.push(Value::from(*value as i32)),
        Op::PushTrue => f.push(true),
        Op::PushFalse => f.push(false),
        Op::PushNaN => f.push(f64::NAN),
        Op::Pop => {
            f.pop()?;
        }
        Op::Dup => {
            let v = f.peek()?.clone();
            f.push(v);
        }
        Op::Swap => {
            let b = f.pop()?;
            let a = f.pop()?;
            f.push(b);
            f.push(a);
        }
        Op::PushString { value } => f.push(Value::String(unit.pools.string(value.0))),
        Op::PushInt { value } => f.push(int_pool(unit, value.0)),
        Op::PushUint { value } => {
            let v = unit.abc.constant_pool.uints.get(value.0 as usize - 1).copied().unwrap_or(0);
            f.push(Value::from(v));
        }
        Op::PushDouble { value } => {
            let v = unit.abc.constant_pool.doubles.get(value.0 as usize - 1).copied().unwrap_or(f64::NAN);
            f.push(v);
        }
        Op::PushScope => {
            let v = f.pop()?;
            if v.is_nullish() {
                return Err(vm.type_error(1009, "Cannot access a property or method of a null object reference."));
            }
            f.scopes.push(Scope { value: v, with: false });
        }
        Op::PushNamespace { value } => {
            let ns = unit.pools.namespace(value.0);
            let o = builtins_namespace(vm, ns);
            f.push(o);
        }
        Op::HasNext2 { object_register, index_register } => {
            let mut o = f.locals[*object_register as usize].clone();
            let mut i = vm.to_uint(&f.locals[*index_register as usize])?;
            has_next(vm, &mut o, &mut i);
            f.locals[*object_register as usize] = o;
            f.locals[*index_register as usize] = Value::from(i);
            f.push(i != 0);
        }
        Op::Li8 | Op::Li16 | Op::Li32 | Op::Lf32 | Op::Lf64 | Op::Si8 | Op::Si16 | Op::Si32 | Op::Sf32 | Op::Sf64 => {
            return Err(Error::Internal("domain memory opcodes are not used by Iggy UI movies".into()));
        }
        Op::NewFunction { index } => {
            let scope: Rc<[Scope]> = f.scopes.clone().into();
            let func = vm.function(unit.method(index.0), scope, None);
            f.push(func);
        }
        Op::Call { num_args } => {
            let args = f.pop_n(*num_args)?;
            let this = f.pop()?;
            let func = f.pop()?;
            let r = vm.call(&func, this, &args)?;
            f.push(r);
        }
        Op::Construct { num_args } => {
            let args = f.pop_n(*num_args)?;
            let ctor = f.pop()?;
            let r = vm.construct(&ctor, &args)?;
            f.push(r);
        }
        Op::CallMethod { .. } => return Err(Error::Internal("callmethod (dispatch id calls) is not emitted for these movies".into())),
        Op::CallStatic { index, num_args } => {
            let args = f.pop_n(*num_args)?;
            let this = f.pop()?;
            let scope: Rc<[Scope]> = f.scopes[..f.base].to_vec().into();
            let r = vm.call_method(&unit.method(index.0), this, &args, scope, None)?;
            f.push(r);
        }
        Op::CallSuper { index, num_args } | Op::CallSuperVoid { index, num_args } => {
            let args = f.pop_n(*num_args)?;
            let operand = resolve(vm, f, index.0)?;
            let target = f.pop()?;
            let class = home_class(f).and_then(|c| c.super_class.clone()).ok_or_else(|| Error::Internal("callsuper without base".into()))?;
            let local = operand.name.local().to_string();
            let method = class.instance.find(&local, &operand.name.set).map(|(_, b)| b.clone());
            let r = match method {
                Some(super::object::Binding::Method(m)) => {
                    let scope = class.scope.borrow().clone();
                    vm.call_method(&m, target, &args, scope, None)?
                }
                _ => {
                    let func = super_get(vm, &class, &target, &operand.name)?;
                    vm.call(&func, target, &args)?
                }
            };
            if matches!(op, Op::CallSuper { .. }) {
                f.push(r);
            }
        }
        Op::CallProperty { index, num_args } | Op::CallPropVoid { index, num_args } | Op::CallPropLex { index, num_args } => {
            let args = f.pop_n(*num_args)?;
            let operand = resolve(vm, f, index.0)?;
            let target = f.pop()?;
            let r = if let Some((o, key)) = dictionary_target(&target, &operand) {
                let func = vm.dictionary_get(&o, &key).unwrap_or_default();
                vm.call(&func, target.clone(), &args)?
            } else if matches!(op, Op::CallPropLex { .. }) {
                let func = vm.get_property(&target, &operand.name)?;
                vm.call(&func, Value::Null, &args)?
            } else {
                vm.call_property(&target, &operand.name, &args)?
            };
            if !matches!(op, Op::CallPropVoid { .. }) {
                f.push(r);
            }
        }
        Op::ReturnVoid => return Ok(Some(Value::Undefined)),
        Op::ReturnValue => return Ok(Some(f.pop()?)),
        Op::ConstructSuper { num_args } => {
            let args = f.pop_n(*num_args)?;
            let this = f.pop()?;
            if let Some(base) = home_class(f).and_then(|c| c.super_class.clone()) {
                vm.run_constructor(&base, &this, &args)?;
            }
        }
        Op::ConstructProp { index, num_args } => {
            let args = f.pop_n(*num_args)?;
            let operand = resolve(vm, f, index.0)?;
            let target = f.pop()?;
            let ctor = get(vm, &target, &operand)?;
            let r = vm.construct(&ctor, &args)?;
            f.push(r);
        }
        Op::NewActivation => {
            let traits = body.activation.clone();
            let o = Obj::new(ObjectData { slots: traits.initial_slots(), traits: Some(traits), ..ObjectData::plain(None) });
            f.push(o);
        }
        Op::NewObject { num_args } => {
            let pairs = f.pop_n(num_args * 2)?;
            let o = vm.plain_object();
            for pair in pairs.chunks_exact(2) {
                let key = vm.to_string(&pair[0])?;
                o.borrow_mut().dynamic.set(key, pair[1].clone());
            }
            f.push(o);
        }
        Op::NewArray { num_args } => {
            let items = f.pop_n(*num_args)?;
            f.push(vm.array(items));
        }
        Op::NewClass { index } => {
            let base = f.pop()?;
            let scope: Rc<[Scope]> = f.scopes.clone().into();
            let class = vm.new_class(unit, index.0, base, scope)?;
            f.push(class);
        }
        Op::GetDescendants { index } => {
            let operand = resolve(vm, f, index.0)?;
            let target = f.pop()?;
            let r = super::xml::descendants(vm, &target, &operand.name)?;
            f.push(r);
        }
        Op::NewCatch { index } => {
            let h = &body.handlers[index.0 as usize];
            f.push(catch_scope(h, unit));
        }
        Op::FindPropStrict { index } | Op::FindProperty { index } | Op::FindDef { index } => {
            let operand = resolve(vm, f, index.0)?;
            let r = match op {
                Op::FindDef { .. } => vm.find_property(&[], &operand.name, true)?,
                _ => vm.find_property(&f.scopes, &operand.name, matches!(op, Op::FindPropStrict { .. }))?,
            };
            f.push(r);
        }
        Op::GetLex { index } => {
            let operand = resolve(vm, f, index.0)?;
            let holder = vm.find_property(&f.scopes, &operand.name, true)?;
            let v = vm.get_property(&holder, &operand.name)?;
            f.push(v);
        }
        Op::SetProperty { index } | Op::InitProperty { index } => {
            let value = f.pop()?;
            let operand = resolve(vm, f, index.0)?;
            let target = f.pop()?;
            set(vm, &target, &operand, value, matches!(op, Op::InitProperty { .. }))?;
        }
        Op::GetLocal { index } => {
            let v = f.locals[*index as usize].clone();
            f.push(v);
        }
        Op::SetLocal { index } => {
            let v = f.pop()?;
            let i = *index as usize;
            if f.locals.len() <= i {
                f.locals.resize(i + 1, Value::Undefined);
            }
            f.locals[i] = v;
        }
        Op::GetGlobalScope => {
            let v = f.scopes.first().map(|s| s.value.clone()).unwrap_or_default();
            f.push(v);
        }
        Op::GetScopeObject { index } => {
            let v = f.scopes.get(f.base + *index as usize).map(|s| s.value.clone()).ok_or_else(|| Error::Internal("scope index".into()))?;
            f.push(v);
        }
        Op::GetOuterScope { index } => {
            let v = f.scopes.get(*index as usize).map(|s| s.value.clone()).ok_or_else(|| Error::Internal("outer scope index".into()))?;
            f.push(v);
        }
        Op::GetProperty { index } => {
            let operand = resolve(vm, f, index.0)?;
            let target = f.pop()?;
            let v = get(vm, &target, &operand)?;
            f.push(v);
        }
        Op::DeleteProperty { index } => {
            let operand = resolve(vm, f, index.0)?;
            let target = f.pop()?;
            let r = match dictionary_target(&target, &operand) {
                Some((o, key)) => vm.delete_dictionary(&o, &key),
                None => vm.delete_property(&target, &operand.name)?,
            };
            f.push(r);
        }
        Op::GetSlot { index } => {
            let o = f.pop()?;
            let Value::Object(o) = o else {
                return Err(vm.type_error(1009, "Cannot access a property or method of a null object reference."));
            };
            let v = o.borrow().slots.get(*index as usize).cloned().unwrap_or_default();
            f.push(v);
        }
        Op::SetSlot { index } => {
            let value = f.pop()?;
            let o = f.pop()?;
            let Value::Object(o) = o else {
                return Err(vm.type_error(1009, "Cannot access a property or method of a null object reference."));
            };
            set_slot(vm, &o, *index as usize, value)?;
        }
        Op::GetGlobalSlot { index } => {
            let g = f.scopes.first().map(|s| s.value.clone()).unwrap_or_default();
            let v = g.as_object().and_then(|o| o.borrow().slots.get(*index as usize).cloned()).unwrap_or_default();
            f.push(v);
        }
        Op::SetGlobalSlot { index } => {
            let value = f.pop()?;
            if let Some(g) = f.scopes.first().and_then(|s| s.value.as_object().cloned()) {
                set_slot(vm, &g, *index as usize, value)?;
            }
        }
        Op::ConvertS => {
            let v = f.pop()?;
            let s = vm.to_string(&v)?;
            f.push(Value::String(s));
        }
        Op::EscXElem | Op::EscXAttr => {
            let v = f.pop()?;
            let s = vm.to_string(&v)?;
            f.push(Value::from(super::xml::escape(&s, matches!(op, Op::EscXAttr))));
        }
        Op::ConvertI | Op::CoerceI => {
            let v = f.pop()?;
            let n = vm.to_int(&v)?;
            f.push(n);
        }
        Op::ConvertU | Op::CoerceU => {
            let v = f.pop()?;
            let n = vm.to_uint(&v)?;
            f.push(n);
        }
        Op::ConvertD | Op::CoerceD => {
            let v = f.pop()?;
            let n = vm.to_number(&v)?;
            f.push(n);
        }
        Op::ConvertB | Op::CoerceB => {
            let v = f.pop()?.to_boolean();
            f.push(v);
        }
        Op::ConvertO => {
            let v = f.pop()?;
            if v.is_nullish() {
                return Err(vm.type_error(1009, "Cannot access a property or method of a null object reference."));
            }
            f.push(v);
        }
        Op::CheckFilter => {
            let v = f.peek()?;
            if !matches!(v, Value::Object(o) if o.is_xml() || matches!(o.borrow().kind, Kind::XmlList(_))) {
                return Err(vm.type_error(1123, "Filter operator not supported on this type."));
            }
        }
        Op::Coerce { index } => {
            let v = f.pop()?;
            let r = match vm.resolve_type(unit, index.0)? {
                Some(class) => vm.coerce(&v, &class)?,
                None => v,
            };
            f.push(r);
        }
        Op::CoerceA => {}
        Op::CoerceS => {
            let v = f.pop()?;
            let r = vm.coerce_string(&v)?;
            f.push(r);
        }
        Op::CoerceO => {
            let v = f.pop()?;
            f.push(if matches!(v, Value::Undefined) { Value::Null } else { v });
        }
        Op::AsType { type_name } => {
            let v = f.pop()?;
            let r = match vm.resolve_type(unit, type_name.0)? {
                Some(class) if vm.is_type(&v, &class) => v,
                Some(_) => Value::Null,
                None => v,
            };
            f.push(r);
        }
        Op::AsTypeLate => {
            let c = f.pop()?;
            let v = f.pop()?;
            let class = c.as_object().and_then(Obj::as_class);
            let r = match class {
                Some(class) if vm.is_type(&v, &class) => v,
                Some(_) => Value::Null,
                None => return Err(vm.type_error(1041, "The right-hand side of operator must be a class.")),
            };
            f.push(r);
        }
        Op::IsType { index } => {
            let v = f.pop()?;
            let r = match vm.resolve_type(unit, index.0)? {
                Some(class) => vm.is_type(&v, &class),
                None => true,
            };
            f.push(r);
        }
        Op::IsTypeLate => {
            let c = f.pop()?;
            let v = f.pop()?;
            let Some(class) = c.as_object().and_then(Obj::as_class) else {
                return Err(vm.type_error(1041, "The right-hand side of operator must be a class."));
            };
            let r = vm.is_type(&v, &class);
            f.push(r);
        }
        Op::InstanceOf => {
            let ctor = f.pop()?;
            let v = f.pop()?;
            let proto = match &ctor {
                Value::Object(c) => match c.as_class() {
                    Some(class) => Some(class.prototype()),
                    None => vm.get_property(&ctor, &Name::public("prototype"))?.as_object().cloned(),
                },
                _ => return Err(vm.type_error(1040, "The right-hand side of instanceof must be a class or function.")),
            };
            let mut p = v.as_object().and_then(|o| o.borrow().proto.clone());
            let mut r = false;
            while let Some(o) = p {
                if Some(&o) == proto.as_ref() {
                    r = true;
                    break;
                }
                p = o.borrow().proto.clone();
            }
            f.push(r);
        }
        Op::TypeOf => {
            let v = f.pop()?;
            f.push(v.type_of());
        }
        Op::In => {
            let o = f.pop()?;
            let k = f.pop()?;
            let r = match o.as_object() {
                Some(d) if matches!(d.borrow().kind, Kind::Dictionary(_)) => {
                    matches!(&d.borrow().kind, Kind::Dictionary(e) if e.iter().any(|(x, _)| super::dictionary_key_eq(x, &k)))
                }
                _ => {
                    let name = vm.to_string(&k)?;
                    vm.has_property(&o, &Name::public(&name))?
                }
            };
            f.push(r);
        }
        Op::IncLocal { index } | Op::IncLocalI { index } | Op::DecLocal { index } | Op::DecLocalI { index } => {
            let i = *index as usize;
            let n = vm.to_number(&f.locals[i].clone())?;
            f.locals[i] = match op {
                Op::IncLocal { .. } => Value::from(n + 1.0),
                Op::DecLocal { .. } => Value::from(n - 1.0),
                Op::IncLocalI { .. } => Value::from(value::to_int32(n).wrapping_add(1)),
                _ => Value::from(value::to_int32(n).wrapping_sub(1)),
            };
        }
        Op::Increment | Op::Decrement | Op::IncrementI | Op::DecrementI | Op::Negate | Op::NegateI | Op::BitNot => {
            let v = f.pop()?;
            let n = vm.to_number(&v)?;
            f.push(match op {
                Op::Increment => Value::from(n + 1.0),
                Op::Decrement => Value::from(n - 1.0),
                Op::IncrementI => Value::from(value::to_int32(n).wrapping_add(1)),
                Op::DecrementI => Value::from(value::to_int32(n).wrapping_sub(1)),
                Op::Negate => Value::from(-n),
                Op::NegateI => Value::from(value::to_int32(n).wrapping_neg()),
                _ => Value::from(!value::to_int32(n)),
            });
        }
        Op::Not => {
            let v = f.pop()?.to_boolean();
            f.push(!v);
        }
        Op::Add => {
            let b = f.pop()?;
            let a = f.pop()?;
            let r = add(vm, &a, &b)?;
            f.push(r);
        }
        Op::Subtract | Op::Multiply | Op::Divide | Op::Modulo | Op::AddI | Op::SubtractI | Op::MultiplyI => {
            let b = f.pop()?;
            let a = f.pop()?;
            let x = vm.to_number(&a)?;
            let y = vm.to_number(&b)?;
            f.push(match op {
                Op::Subtract => Value::from(x - y),
                Op::Multiply => Value::from(x * y),
                Op::Divide => Value::from(x / y),
                Op::Modulo => Value::from(x % y),
                Op::AddI => Value::from(value::to_int32(x).wrapping_add(value::to_int32(y))),
                Op::SubtractI => Value::from(value::to_int32(x).wrapping_sub(value::to_int32(y))),
                _ => Value::from(value::to_int32(x).wrapping_mul(value::to_int32(y))),
            });
        }
        Op::LShift | Op::RShift | Op::URShift | Op::BitAnd | Op::BitOr | Op::BitXor => {
            let b = f.pop()?;
            let a = f.pop()?;
            let x = vm.to_number(&a)?;
            let y = vm.to_number(&b)?;
            let (xi, yu) = (value::to_int32(x), value::to_uint32(y));
            f.push(match op {
                Op::LShift => Value::from(xi.wrapping_shl(yu & 31)),
                Op::RShift => Value::from(xi.wrapping_shr(yu & 31)),
                Op::URShift => Value::from(value::to_uint32(x).wrapping_shr(yu & 31)),
                Op::BitAnd => Value::from(xi & value::to_int32(y)),
                Op::BitOr => Value::from(xi | value::to_int32(y)),
                _ => Value::from(xi ^ value::to_int32(y)),
            });
        }
        Op::Equals => {
            let b = f.pop()?;
            let a = f.pop()?;
            let r = loose_equals(vm, &a, &b)?;
            f.push(r);
        }
        Op::StrictEquals => {
            let b = f.pop()?;
            let a = f.pop()?;
            f.push(strict_equals(&a, &b));
        }
        Op::LessThan | Op::LessEquals | Op::GreaterThan | Op::GreaterEquals => {
            let b = f.pop()?;
            let a = f.pop()?;
            let r = match op {
                Op::LessThan => less_than(vm, &a, &b)? == Some(true),
                Op::GreaterThan => less_than(vm, &b, &a)? == Some(true),
                Op::LessEquals => less_than(vm, &b, &a)? == Some(false),
                _ => less_than(vm, &a, &b)? == Some(false),
            };
            f.push(r);
        }
        Op::ApplyType { num_types } => {
            let params = f.pop_n(*num_types)?;
            let base = f.pop()?;
            let is_vector = base.as_object().and_then(Obj::as_class).is_some_and(|c| Rc::ptr_eq(&c, &vm.b().vector));
            if !is_vector || params.len() != 1 {
                return Err(vm.type_error(1127, "Type application attempted on a non-parameterized type."));
            }
            let element = params[0].as_object().and_then(Obj::as_class);
            let class = vm.vector_class(element)?;
            f.push(class.object());
        }
        Op::Sxi1 | Op::Sxi8 | Op::Sxi16 => {
            let v = f.pop()?;
            let n = vm.to_int(&v)?;
            f.push(match op {
                Op::Sxi1 => -(n & 1),
                Op::Sxi8 => n as i8 as i32,
                _ => n as i16 as i32,
            });
        }
    }
    *pc += 1;
    Ok(None)
}

fn set_slot(vm: &mut Vm, o: &Obj, index: usize, value: Value) -> Result<()> {
    let slot = o.borrow().traits.as_ref().and_then(|t| t.slots.get(index).cloned().flatten());
    let value = match &slot {
        Some(s) => vm.coerce_slot(s, value)?,
        None => value,
    };
    let mut d = o.borrow_mut();
    if d.slots.len() <= index {
        d.slots.resize(index + 1, Value::Undefined);
    }
    d.slots[index] = value;
    Ok(())
}

/// `super.x` reads the base class's binding with the original receiver.
fn super_get(vm: &mut Vm, class: &Rc<super::object::Class>, target: &Value, name: &Name) -> Result<Value> {
    let local = name.local().to_string();
    match class.instance.find(&local, &name.set).map(|(q, b)| (q.clone(), b.clone())) {
        Some((_, super::object::Binding::Accessor { get: Some(g), .. })) => {
            let scope = class.scope.borrow().clone();
            vm.call_method(&g, target.clone(), &[], scope, None)
        }
        Some((_, super::object::Binding::Method(m))) => {
            let scope = class.scope.borrow().clone();
            Ok(Value::Object(vm.function(m, scope, Some(target.clone()))))
        }
        _ => vm.get_property(target, name),
    }
}

fn super_set(vm: &mut Vm, class: &Rc<super::object::Class>, target: &Value, name: &Name, value: Value) -> Result<()> {
    let local = name.local().to_string();
    match class.instance.find(&local, &name.set).map(|(_, b)| b.clone()) {
        Some(super::object::Binding::Accessor { set: Some(s), .. }) => {
            let scope = class.scope.borrow().clone();
            vm.call_method(&s, target.clone(), &[value], scope, None).map(|_| ())
        }
        _ => vm.set_property(target, name, value),
    }
}

fn builtins_namespace(vm: &mut Vm, ns: Namespace) -> Value {
    super::builtins::namespace_object(vm, ns)
}
