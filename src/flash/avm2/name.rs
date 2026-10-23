//! Namespaces and names, resolved from an ABC constant pool.
use std::rc::Rc;
use swf::avm2::types::{AbcFile, Index, Multiname as AbcMultiname, Namespace as AbcNamespace};

/// AVM2 namespace identity: private namespaces are distinct per constant-pool
/// entry; every other kind compares by kind and URI.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Namespace {
    /// `Namespace` and `PackageNamespace` entries; `""` is the public namespace.
    Public(Rc<str>),
    Internal(Rc<str>),
    Protected(Rc<str>),
    StaticProtected(Rc<str>),
    Explicit(Rc<str>),
    Private { unit: u32, index: u32 },
}

impl Namespace {
    pub fn public() -> Self {
        Namespace::Public("".into())
    }

    pub fn package(uri: &str) -> Self {
        Namespace::Public(uri.into())
    }

    pub fn is_public(&self) -> bool {
        matches!(self, Namespace::Public(uri) if uri.is_empty())
    }

    pub fn uri(&self) -> &str {
        match self {
            Namespace::Public(u)
            | Namespace::Internal(u)
            | Namespace::Protected(u)
            | Namespace::StaticProtected(u)
            | Namespace::Explicit(u) => u,
            Namespace::Private { .. } => "",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct QName {
    pub ns: Namespace,
    pub name: Rc<str>,
}

impl QName {
    pub fn new(ns: Namespace, name: &str) -> Self {
        QName { ns, name: name.into() }
    }

    pub fn public(name: &str) -> Self {
        QName::new(Namespace::public(), name)
    }

    /// `package::Name` / `Name`, as `getQualifiedClassName` prints it.
    pub fn qualified(&self) -> String {
        match self.ns.uri() {
            "" => self.name.to_string(),
            uri => format!("{uri}::{}", self.name),
        }
    }
}

/// A constant-pool multiname before runtime components are popped.
#[derive(Clone, Debug)]
pub enum PoolName {
    QName { name: QName, attribute: bool },
    /// Compile-time name, namespace set.
    Multiname { name: Rc<str>, set: Rc<[Namespace]>, attribute: bool },
    /// Runtime namespace, compile-time name.
    RtQName { name: Rc<str>, attribute: bool },
    /// Runtime namespace and name.
    RtQNameL { attribute: bool },
    /// Runtime name, namespace set.
    MultinameL { set: Rc<[Namespace]>, attribute: bool },
    /// `Vector.<T>` style application.
    TypeName { base: u32, params: Vec<u32> },
    /// Index zero: the any name.
    Any,
}

impl PoolName {
    pub fn runtime_namespace(&self) -> bool {
        matches!(self, PoolName::RtQName { .. } | PoolName::RtQNameL { .. })
    }

    pub fn runtime_name(&self) -> bool {
        matches!(self, PoolName::RtQNameL { .. } | PoolName::MultinameL { .. })
    }
}

/// A fully resolved lookup name: a local name searched in a namespace set.
#[derive(Clone, Debug)]
pub struct Name {
    pub name: Option<Rc<str>>,
    pub set: Rc<[Namespace]>,
    pub attribute: bool,
}

impl Name {
    pub fn qname(q: &QName) -> Self {
        Name { name: Some(q.name.clone()), set: Rc::new([q.ns.clone()]), attribute: false }
    }

    pub fn public(name: &str) -> Self {
        Name { name: Some(name.into()), set: Rc::new([Namespace::public()]), attribute: false }
    }

    pub fn includes_public(&self) -> bool {
        self.set.iter().any(Namespace::is_public)
    }

    pub fn local(&self) -> &str {
        self.name.as_deref().unwrap_or("*")
    }
}

/// Resolved pools for one ABC unit.
#[derive(Debug)]
pub struct Pools {
    pub strings: Vec<Rc<str>>,
    pub namespaces: Vec<Namespace>,
    pub sets: Vec<Rc<[Namespace]>>,
    pub names: Vec<PoolName>,
}

impl Pools {
    pub fn new(unit: u32, abc: &AbcFile) -> Self {
        let pool = &abc.constant_pool;
        let strings: Vec<Rc<str>> =
            pool.strings.iter().map(|s| String::from_utf8_lossy(s).as_ref().into()).collect();
        let string = |i: &Index<String>| -> Rc<str> {
            match i.0 {
                0 => "".into(),
                i => strings[i as usize - 1].clone(),
            }
        };
        let namespaces: Vec<Namespace> = pool
            .namespaces
            .iter()
            .enumerate()
            .map(|(i, ns)| match ns {
                AbcNamespace::Namespace(s) | AbcNamespace::Package(s) => Namespace::Public(string(s)),
                AbcNamespace::PackageInternal(s) => Namespace::Internal(string(s)),
                AbcNamespace::Protected(s) => Namespace::Protected(string(s)),
                AbcNamespace::StaticProtected(s) => Namespace::StaticProtected(string(s)),
                AbcNamespace::Explicit(s) => Namespace::Explicit(string(s)),
                AbcNamespace::Private(_) => Namespace::Private { unit, index: i as u32 + 1 },
            })
            .collect();
        let namespace = |i: &Index<AbcNamespace>| -> Namespace {
            match i.0 {
                0 => Namespace::public(),
                i => namespaces[i as usize - 1].clone(),
            }
        };
        let sets: Vec<Rc<[Namespace]>> = pool
            .namespace_sets
            .iter()
            .map(|set| set.iter().map(&namespace).collect())
            .collect();
        let set = |i: u32| -> Rc<[Namespace]> { sets[i as usize - 1].clone() };
        let names = pool
            .multinames
            .iter()
            .map(|m| match m {
                AbcMultiname::QName { namespace: ns, name } => PoolName::QName {
                    name: QName { ns: namespace(ns), name: string(name) },
                    attribute: false,
                },
                AbcMultiname::QNameA { namespace: ns, name } => PoolName::QName {
                    name: QName { ns: namespace(ns), name: string(name) },
                    attribute: true,
                },
                AbcMultiname::RTQName { name } => PoolName::RtQName { name: string(name), attribute: false },
                AbcMultiname::RTQNameA { name } => PoolName::RtQName { name: string(name), attribute: true },
                AbcMultiname::RTQNameL => PoolName::RtQNameL { attribute: false },
                AbcMultiname::RTQNameLA => PoolName::RtQNameL { attribute: true },
                AbcMultiname::Multiname { namespace_set, name } => PoolName::Multiname {
                    name: string(name),
                    set: set(namespace_set.0),
                    attribute: false,
                },
                AbcMultiname::MultinameA { namespace_set, name } => PoolName::Multiname {
                    name: string(name),
                    set: set(namespace_set.0),
                    attribute: true,
                },
                AbcMultiname::MultinameL { namespace_set } => {
                    PoolName::MultinameL { set: set(namespace_set.0), attribute: false }
                }
                AbcMultiname::MultinameLA { namespace_set } => {
                    PoolName::MultinameL { set: set(namespace_set.0), attribute: true }
                }
                AbcMultiname::TypeName { base_type, parameters } => PoolName::TypeName {
                    base: base_type.0,
                    params: parameters.iter().map(|p| p.0).collect(),
                },
            })
            .collect();
        Pools { strings, namespaces, sets, names }
    }

    pub fn string(&self, index: u32) -> Rc<str> {
        match index {
            0 => "".into(),
            i => self.strings[i as usize - 1].clone(),
        }
    }

    pub fn name(&self, index: u32) -> PoolName {
        match index {
            0 => PoolName::Any,
            i => self.names[i as usize - 1].clone(),
        }
    }

    /// A trait or class name, which the ABC format requires to be a QName.
    pub fn qname(&self, index: u32) -> Option<QName> {
        match self.name(index) {
            PoolName::QName { name, .. } => Some(name),
            _ => None,
        }
    }

    pub fn namespace(&self, index: u32) -> Namespace {
        match index {
            0 => Namespace::public(),
            i => self.namespaces[i as usize - 1].clone(),
        }
    }
}
