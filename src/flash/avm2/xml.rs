//! A read-only E4X subset: the trees `describeType` returns and the child,
//! attribute and descendant access the installed movies apply to them.
use super::builtins::{define, Spec};
use super::name::Name;
use super::object::{Binding, Kind, NativeClass, Obj, ObjectData};
use super::value::Value;
use super::{Result, Vm};
use std::rc::Rc;

#[derive(Debug, Default)]
pub struct XmlNode {
    pub name: Rc<str>,
    pub attributes: Vec<(Rc<str>, Rc<str>)>,
    pub children: Vec<Rc<XmlNode>>,
    /// Text nodes have an empty name and a value.
    pub text: Option<Rc<str>>,
}

impl XmlNode {
    pub fn element(name: &str, attributes: Vec<(&str, String)>) -> Self {
        XmlNode {
            name: name.into(),
            attributes: attributes.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
            ..Default::default()
        }
    }

    fn to_xml_string(&self, out: &mut String, depth: usize) {
        if let Some(t) = &self.text {
            out.push_str(&escape(t, false));
            return;
        }
        out.push_str(&"  ".repeat(depth));
        out.push('<');
        out.push_str(&self.name);
        for (k, v) in &self.attributes {
            out.push_str(&format!(" {k}=\"{}\"", escape(v, true)));
        }
        if self.children.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        for c in &self.children {
            out.push('\n');
            c.to_xml_string(out, depth + 1);
        }
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
        out.push_str(&format!("</{}>", self.name));
    }

    fn string_value(&self) -> String {
        match &self.text {
            Some(t) => t.to_string(),
            None if self.children.iter().all(|c| c.text.is_some()) => self.children.iter().map(|c| c.string_value()).collect(),
            None => {
                let mut s = String::new();
                self.to_xml_string(&mut s, 0);
                s
            }
        }
    }
}

pub fn escape(s: &str, attribute: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' if !attribute => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' if attribute => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn install(vm: &mut Vm) {
    let object = vm.b().object.clone();
    let mut s = Spec::new("", "XML", Some(object.clone()));
    s.native = NativeClass { allocate: Some(|_, _| Ok(Kind::Xml(Rc::new(XmlNode::default())))), call: None, init: None };
    let xml = define(vm, s);
    let mut s = Spec::new("", "XMLList", Some(object));
    s.native = NativeClass { allocate: Some(|_, _| Ok(Kind::XmlList(Vec::new()))), call: None, init: None };
    let xml_list = define(vm, s);
    let b = vm.builtins.as_mut().unwrap();
    b.xml = xml;
    b.xml_list = xml_list;
}

fn node(v: &Value) -> Option<Rc<XmlNode>> {
    match &v.as_object()?.borrow().kind {
        Kind::Xml(n) => Some(n.clone()),
        _ => None,
    }
}

fn items(v: &Value) -> Vec<Value> {
    match v.as_object().map(|o| o.borrow()) {
        Some(d) => match &d.kind {
            Kind::XmlList(items) => items.clone(),
            Kind::Xml(_) => vec![v.clone()],
            _ => Vec::new(),
        },
        None => Vec::new(),
    }
}

fn string_value(vm: &mut Vm, v: &Value) -> String {
    match v.as_object().map(|o| o.borrow()) {
        Some(d) => match &d.kind {
            Kind::Xml(n) => n.string_value(),
            Kind::XmlList(items) => {
                let items = items.clone();
                drop(d);
                items.iter().map(|i| string_value(vm, i)).collect()
            }
            _ => String::new(),
        },
        None => match v {
            Value::String(s) => s.to_string(),
            _ => String::new(),
        },
    }
}

pub fn xml_value(vm: &Vm, n: Rc<XmlNode>) -> Value {
    let class = vm.b().xml.clone();
    Value::Object(Obj::new(ObjectData {
        class: Some(class.clone()),
        traits: Some(class.instance.clone()),
        kind: Kind::Xml(n),
        ..ObjectData::plain(Some(class.prototype()))
    }))
}

fn list(vm: &Vm, items: Vec<Value>) -> Value {
    let class = vm.b().xml_list.clone();
    Value::Object(Obj::new(ObjectData {
        class: Some(class.clone()),
        traits: Some(class.instance.clone()),
        kind: Kind::XmlList(items),
        ..ObjectData::plain(Some(class.prototype()))
    }))
}

/// `x.child`, `x.@attr`, `list[i]`.
pub fn get_property(vm: &mut Vm, o: &Obj, name: &Name) -> Result<Value> {
    let local = name.local().to_string();
    let target = Value::Object(o.clone());
    if let Ok(i) = local.parse::<usize>() {
        if name.includes_public() {
            return Ok(items(&target).get(i).cloned().unwrap_or_default());
        }
    }
    let mut out = Vec::new();
    for item in items(&target) {
        let Some(n) = node(&item) else { continue };
        if name.attribute {
            for (k, v) in &n.attributes {
                if local == "*" || **k == *local {
                    out.push(Value::String(v.clone()));
                }
            }
        } else {
            for c in &n.children {
                if c.text.is_none() && (local == "*" || *c.name == *local) {
                    out.push(xml_value(vm, c.clone()));
                }
            }
        }
    }
    Ok(list(vm, out))
}

pub fn set_property(vm: &mut Vm, _o: &Obj, name: &Name, _value: Value) -> Result<()> {
    Err(vm.type_error(1089, format!("Assignment to XML property {} is not supported by this read-only XML.", name.local())))
}

/// `x..name`.
pub fn descendants(vm: &mut Vm, target: &Value, name: &Name) -> Result<Value> {
    fn walk(vm: &Vm, n: &Rc<XmlNode>, local: &str, out: &mut Vec<Value>) {
        for c in &n.children {
            if c.text.is_none() && (local == "*" || *c.name == *local) {
                out.push(xml_value(vm, c.clone()));
            }
            walk(vm, c, local, out);
        }
    }
    let mut out = Vec::new();
    for item in items(target) {
        if let Some(n) = node(&item) {
            walk(vm, &n, name.local(), &mut out);
        }
    }
    Ok(list(vm, out))
}

pub fn equals(vm: &mut Vm, a: &Obj, b: &Obj) -> Result<bool> {
    let xml = |o: &Obj| matches!(o.borrow().kind, Kind::Xml(_) | Kind::XmlList(_));
    if xml(a) || xml(b) {
        let x = string_value(vm, &Value::Object(a.clone()));
        let y = string_value(vm, &Value::Object(b.clone()));
        return Ok(x == y);
    }
    Ok(false)
}

pub fn concat(_vm: &mut Vm, _a: &Obj, _b: &Obj) -> Result<Option<Value>> {
    Ok(None)
}

/// `flash.utils.describeType`: the documented `<type>` description of a
/// class or instance (names, bases, interfaces, variables, accessors and
/// methods).
pub fn describe_type(vm: &mut Vm, _: &Value, args: &[Value]) -> Result<Value> {
    let v = args.first().cloned().unwrap_or_default();
    let (class, is_static) = match &v {
        Value::Object(o) => match o.as_class() {
            Some(c) => (Some(c), true),
            None => (o.class(), false),
        },
        other => (vm.class_of(other), false),
    };
    let Some(class) = class else {
        return Ok(xml_value(vm, Rc::new(XmlNode::element("type", vec![("name", "null".into())]))));
    };
    let mut root = XmlNode::element(
        "type",
        vec![
            ("name", class.name.qualified()),
            ("base", if is_static { "Class".into() } else { class.super_class.as_ref().map_or("Object".into(), |s| s.name.qualified()) }),
            ("isDynamic", (is_static || !class.sealed).to_string()),
            ("isFinal", is_static.to_string()),
            ("isStatic", is_static.to_string()),
        ],
    );
    let traits = if is_static { class.statics.clone() } else { class.instance.clone() };
    let mut s = class.super_class.clone();
    while let Some(c) = s {
        root.children.push(Rc::new(XmlNode::element("extendsClass", vec![("type", c.name.qualified())])));
        s = c.super_class.clone();
    }
    for i in &class.interfaces {
        root.children.push(Rc::new(XmlNode::element("implementsInterface", vec![("type", i.name.qualified())])));
    }
    let mut members: Vec<_> = traits.bindings.iter().filter(|(q, _)| q.ns.is_public()).collect();
    members.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    for (q, binding) in members {
        let element = match binding {
            Binding::Slot(i) => {
                let constant = traits.slots.get(*i as usize).cloned().flatten().is_some_and(|s| s.constant);
                XmlNode::element(if constant { "constant" } else { "variable" }, vec![("name", q.name.to_string())])
            }
            Binding::Method(_) => XmlNode::element("method", vec![("name", q.name.to_string())]),
            Binding::Accessor { get, set } => {
                let access = match (get.is_some(), set.is_some()) {
                    (true, true) => "readwrite",
                    (true, false) => "readonly",
                    _ => "writeonly",
                };
                XmlNode::element("accessor", vec![("name", q.name.to_string()), ("access", access.into())])
            }
        };
        root.children.push(Rc::new(element));
    }
    Ok(xml_value(vm, Rc::new(root)))
}
