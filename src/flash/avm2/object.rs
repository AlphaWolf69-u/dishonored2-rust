//! Objects, traits, methods and classes.
use super::name::{Namespace, QName};
use super::unit::Unit;
use super::value::Value;
use super::{Result, Vm};
use std::any::Any;
use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

/// A garbage-free reference to an AS3 object. Identity is pointer identity.
#[derive(Clone)]
pub struct Obj(Rc<RefCell<ObjectData>>);

impl fmt::Debug for Obj {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.try_borrow() {
            Ok(data) => match (&data.kind, &data.class) {
                (Kind::Class(c), _) => write!(f, "[class {}]", c.name.qualified()),
                (_, Some(c)) => write!(f, "[object {}]", c.name.name),
                _ => write!(f, "[object]"),
            },
            Err(_) => write!(f, "[object (borrowed)]"),
        }
    }
}

impl PartialEq for Obj {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Obj {}

impl std::hash::Hash for Obj {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (Rc::as_ptr(&self.0) as usize).hash(state)
    }
}

impl Obj {
    pub fn new(data: ObjectData) -> Self {
        Obj(Rc::new(RefCell::new(data)))
    }

    pub fn borrow(&self) -> Ref<'_, ObjectData> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, ObjectData> {
        self.0.borrow_mut()
    }

    pub fn is_function(&self) -> bool {
        matches!(self.borrow().kind, Kind::Function(_))
    }

    pub fn is_xml(&self) -> bool {
        matches!(self.borrow().kind, Kind::Xml(_))
    }

    pub fn class(&self) -> Option<Rc<Class>> {
        self.borrow().class.clone()
    }

    /// The class this object represents, if it is a class object.
    pub fn as_class(&self) -> Option<Rc<Class>> {
        match &self.borrow().kind {
            Kind::Class(c) => Some(c.clone()),
            _ => None,
        }
    }

    pub fn host<T: 'static>(&self) -> Option<Ref<'_, T>> {
        Ref::filter_map(self.borrow(), |d| match &d.kind {
            Kind::Host(h) => h.downcast_ref::<T>(),
            _ => None,
        })
        .ok()
    }

    pub fn host_mut<T: 'static>(&self) -> Option<RefMut<'_, T>> {
        RefMut::filter_map(self.borrow_mut(), |d| match &mut d.kind {
            Kind::Host(h) => h.downcast_mut::<T>(),
            _ => None,
        })
        .ok()
    }

    pub fn address(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }
}

/// Ordered public dynamic properties (`for..in` order is insertion order).
#[derive(Default, Debug, Clone)]
pub struct Dynamic {
    entries: Vec<(Rc<str>, Value, bool)>,
    index: HashMap<Rc<str>, usize>,
}

impl Dynamic {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.index.get(name).map(|&i| &self.entries[i].1)
    }

    pub fn set(&mut self, name: Rc<str>, value: Value) {
        match self.index.get(&name) {
            Some(&i) => self.entries[i].1 = value,
            None => {
                self.index.insert(name.clone(), self.entries.len());
                self.entries.push((name, value, true));
            }
        }
    }

    pub fn delete(&mut self, name: &str) -> bool {
        let Some(i) = self.index.remove(name) else {
            return false;
        };
        self.entries.remove(i);
        for (_, slot) in self.index.iter_mut() {
            if *slot > i {
                *slot -= 1;
            }
        }
        true
    }

    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    pub fn set_enumerable(&mut self, name: &str, enumerable: bool) {
        if let Some(&i) = self.index.get(name) {
            self.entries[i].2 = enumerable;
        }
    }

    pub fn is_enumerable(&self, name: &str) -> bool {
        self.index.get(name).is_some_and(|&i| self.entries[i].2)
    }

    pub fn enumerable(&self) -> impl Iterator<Item = (&Rc<str>, &Value)> {
        self.entries.iter().filter(|e| e.2).map(|e| (&e.0, &e.1))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn key_at(&self, i: usize) -> Option<(&Rc<str>, bool)> {
        self.entries.get(i).map(|e| (&e.0, e.2))
    }
}

pub struct ObjectData {
    /// The class used for `is`, `constructor` and sealed-property checks.
    pub class: Option<Rc<Class>>,
    /// Fixed properties: class instance, class static, script or activation.
    pub traits: Option<Rc<Traits>>,
    pub proto: Option<Obj>,
    pub slots: Vec<Value>,
    pub dynamic: Dynamic,
    /// Method closures are cached so `o.f === o.f`, as in the AVM2.
    pub closures: HashMap<QName, Obj>,
    pub kind: Kind,
}

impl ObjectData {
    pub fn plain(proto: Option<Obj>) -> Self {
        ObjectData {
            class: None,
            traits: None,
            proto,
            slots: Vec::new(),
            dynamic: Dynamic::default(),
            closures: HashMap::new(),
            kind: Kind::Plain,
        }
    }
}

pub enum Kind {
    Plain,
    Function(Function),
    /// A class object; its traits are the class's static traits.
    Class(Rc<Class>),
    Array(Vec<Value>),
    Vector(Vector),
    /// `flash.utils.Dictionary`: keys compared by identity for objects and
    /// by value for primitives.
    Dictionary(Vec<(Value, Value)>),
    Namespace(Namespace),
    Xml(Rc<super::xml::XmlNode>),
    XmlList(Vec<Value>),
    /// Boxed primitive (`new Number(1)` etc.).
    Primitive(Value),
    /// Native state for host classes (display objects, events, ...).
    Host(Box<dyn Any>),
}

pub struct Vector {
    pub element: Option<Rc<Class>>,
    pub fixed: bool,
    pub items: Vec<Value>,
}

#[derive(Clone)]
pub struct Function {
    pub method: Rc<Method>,
    pub scope: Rc<[Scope]>,
    /// Method closures carry their receiver.
    pub this: Option<Value>,
}

#[derive(Clone, Debug)]
pub struct Scope {
    pub value: Value,
    pub with: bool,
}

pub type NativeFn = fn(&mut Vm, &Value, &[Value]) -> Result<Value>;

pub enum MethodKind {
    Bytecode { unit: Rc<Unit>, index: u32 },
    Native(NativeFn),
}

pub struct Method {
    pub name: Rc<str>,
    pub kind: MethodKind,
}

impl fmt::Debug for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Method({})", self.name)
    }
}

impl Method {
    pub fn native(name: &str, f: NativeFn) -> Rc<Self> {
        Rc::new(Method { name: name.into(), kind: MethodKind::Native(f) })
    }
}

#[derive(Clone, Debug)]
pub enum Binding {
    Slot(u32),
    Method(Rc<Method>),
    Accessor { get: Option<Rc<Method>>, set: Option<Rc<Method>> },
}

/// A declared slot or constant.
#[derive(Clone, Debug)]
pub struct Slot {
    pub name: QName,
    /// The declared type's multiname index in `unit`, or 0 for `*`.
    pub type_name: u32,
    pub unit: Option<Rc<Unit>>,
    pub default: Value,
    pub constant: bool,
}

/// The fixed properties of an object kind.
#[derive(Default, Debug)]
pub struct Traits {
    pub bindings: HashMap<QName, Binding>,
    /// Slot `i` is ABC slot id `i`; index 0 is unused.
    pub slots: Vec<Option<Slot>>,
}

impl Traits {
    /// The first binding whose namespace is in `set` (AVM2 multiname lookup).
    pub fn find(&self, name: &str, set: &[Namespace]) -> Option<(&QName, &Binding)> {
        set.iter().find_map(|ns| {
            let q = QName { ns: ns.clone(), name: name.into() };
            self.bindings.get_key_value(&q)
        })
    }

    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    pub fn initial_slots(&self) -> Vec<Value> {
        self.slots.iter().map(|s| s.as_ref().map(|s| s.default.clone()).unwrap_or_default()).collect()
    }
}

/// Host behavior for natively implemented classes.
#[derive(Clone, Copy)]
pub struct NativeClass {
    /// Builds the instance kind before the constructor runs.
    pub allocate: Option<fn(&mut Vm, &Rc<Class>) -> Result<Kind>>,
    /// Behavior when the class object is called as a function (`String(x)`).
    pub call: Option<NativeFn>,
    /// Constructor body run by `constructsuper` / `construct`.
    pub init: Option<NativeFn>,
}

pub struct Class {
    pub name: QName,
    pub super_class: Option<Rc<Class>>,
    pub interfaces: Vec<Rc<Class>>,
    pub instance: Rc<Traits>,
    pub statics: Rc<Traits>,
    pub instance_init: Option<Rc<Method>>,
    pub sealed: bool,
    pub interface: bool,
    pub native: Option<NativeClass>,
    /// The closest native ancestor (including self) that allocates kinds.
    pub object: RefCell<Option<Obj>>,
    pub prototype: RefCell<Option<Obj>>,
    /// Scope the methods of this class close over.
    pub scope: RefCell<Rc<[Scope]>>,
    /// Vector element type for applied `Vector.<T>` classes.
    pub applied: Option<Option<Rc<Class>>>,
}

impl fmt::Debug for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Class({})", self.name.qualified())
    }
}

impl Class {
    pub fn object(&self) -> Obj {
        self.object.borrow().clone().expect("class object not initialized")
    }

    pub fn prototype(&self) -> Obj {
        self.prototype.borrow().clone().expect("class prototype not initialized")
    }

    /// Whether `self` is `other` or derives from / implements it.
    pub fn is_subclass_of(self: &Rc<Self>, other: &Rc<Class>) -> bool {
        let mut c = Some(self.clone());
        while let Some(class) = c {
            if Rc::ptr_eq(&class, other) || class.implements(other) {
                return true;
            }
            c = class.super_class.clone();
        }
        false
    }

    fn implements(&self, other: &Rc<Class>) -> bool {
        self.interfaces.iter().any(|i| Rc::ptr_eq(i, other) || i.implements(other))
    }

    /// The nearest class (self or ancestor) with native allocation.
    pub fn allocator(self: &Rc<Self>) -> Option<(Rc<Class>, fn(&mut Vm, &Rc<Class>) -> Result<Kind>)> {
        let mut c = Some(self.clone());
        while let Some(class) = c {
            if let Some(alloc) = class.native.and_then(|n| n.allocate) {
                return Some((class, alloc));
            }
            c = class.super_class.clone();
        }
        None
    }

    pub fn ancestor_named(self: &Rc<Self>, name: &str) -> bool {
        let mut c = Some(self.clone());
        while let Some(class) = c {
            if class.name.qualified() == name {
                return true;
            }
            c = class.super_class.clone();
        }
        false
    }
}
