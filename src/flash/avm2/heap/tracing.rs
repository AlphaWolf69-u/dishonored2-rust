//! Host cycle collection with conservative external roots. Object and captured
//! scope reference counts distinguish VM/host/stack roots from internal edges.
//! Shared scopes are graph nodes: counting their Values once per closure would
//! incorrectly collect a scope still owned by another closure or active frame.
use super::super::{object::{Kind, Obj, ObjectData, Scope}, value::Value};
use std::{cell::RefCell, hash::BuildHasher, rc::{Rc, Weak}};
use hashbrown::HashMap;

struct Graph<S: BuildHasher> {
    objects: HashMap<usize, usize, S>,
    scopes: Vec<Rc<[Scope]>>,
    scope_indices: HashMap<usize, usize, S>,
    edges: Vec<usize>,
    starts: Vec<usize>,
    debug_nested: Option<Vec<Vec<usize>>>,
    incoming: Vec<usize>,
    object_count: usize,
}

impl<S: BuildHasher> Graph<S> {
    fn edge(&mut self, from: usize, to: usize) {
        if let Some(edges) = &mut self.debug_nested { edges[from].push(to); }
        else { self.edges.push(to); }
        self.incoming[to] += 1;
    }

    fn outgoing(&self, index: usize) -> &[usize] {
        match &self.debug_nested {
            Some(edges) => &edges[index],
            None => &self.edges[self.starts[index]..self.starts[index + 1]],
        }
    }

    fn object(&mut self, from: usize, object: &Obj) {
        if let Some(&to) = self.objects.get(&object.address()) { self.edge(from, to); }
    }

    fn value(&mut self, from: usize, value: &Value) {
        if let Some(object) = value.as_object() { self.object(from, object); }
    }

    fn scope(&mut self, from: usize, scope: &Rc<[Scope]>) {
        if scope.is_empty() { return; }
        let address = Rc::as_ptr(scope) as *const Scope as usize;
        let to = if let Some(&index) = self.scope_indices.get(&address) {
            index
        } else {
            let index = self.incoming.len();
            self.scopes.push(scope.clone());
            self.scope_indices.insert(address, index);
            if let Some(edges) = &mut self.debug_nested { edges.push(Vec::new()); }
            self.incoming.push(0);
            index
        };
        self.edge(from, to);
    }

    fn trace(&mut self, from: usize, data: &ObjectData) {
        if let Some(object) = &data.proto { self.object(from, object); }
        for value in &data.slots { self.value(from, value); }
        for value in data.dynamic.values() { self.value(from, value); }
        for object in data.closures.values() { self.object(from, object); }
        match &data.kind {
            Kind::Function(function) => {
                self.scope(from, &function.scope);
                if let Some(value) = &function.this { self.value(from, value); }
            }
            Kind::Array(values) | Kind::XmlList(values) => {
                for value in values { self.value(from, value); }
            }
            Kind::Vector(vector) => {
                for value in &vector.items { self.value(from, value); }
            }
            Kind::Dictionary(values) => {
                for (key, value) in values {
                    if data.weak_dictionary_keys {
                        // 141063ed0 traces weak-table values, but not their
                        // keys. Count the physical Rc without marking its
                        // target; 141064cd0 deletes entries with dead keys.
                        if let Some(&to) = key.as_object().and_then(|key| self.objects.get(&key.address())) {
                            self.incoming[to] += 1;
                        }
                    } else { self.value(from, key); }
                    self.value(from, value);
                }
            }
            Kind::Primitive(value) => self.value(from, value),
            Kind::Host(host) => {
                if let Some(native) = host.downcast_ref::<crate::flash::Native>() {
                    self.native(from, native);
                }
                // References owned by unknown host types remain conservative
                // external roots. They can prevent collection, never cause a
                // referenced object to be reclaimed.
            }
            Kind::Plain | Kind::Class(_) | Kind::Namespace(_) | Kind::Xml(_) => {}
        }
        // Class/unit/trait definitions are independently owned by the loaded
        // VM libraries. Their object/scope/default references remain external
        // roots; collecting an instance must never clear shared class state.
    }

    fn native(&mut self, from: usize, native: &crate::flash::Native) {
        for listener in &native.listeners {
            if let crate::flash::events::ListenerFunction::Strong(value) = &listener.function {
                self.value(from, value);
            }
        }
        if let Some(event) = &native.event {
            self.value(from, &event.target);
            self.value(from, &event.current_target);
            for (_, value) in &event.fields { self.value(from, value); }
        }
        if let Some(display) = &native.display {
            for object in display.parent.iter().chain(&display.children)
                .chain(display.mask.iter()).chain(display.hit_area.iter()).chain(display.graphics.iter()) {
                self.object(from, object);
            }
            for value in &display.filters { self.value(from, value); }
            if let Some(timeline) = &display.timeline {
                for (_, script) in &timeline.scripts { self.value(from, script); }
            }
            if let Some(value) = &display.transform3d { self.value(from, value); }
        }
        if let Some(graphics) = &native.graphics {
            for value in &graphics.bitmaps { self.value(from, value); }
        }
        if let Some(target) = &native.target { self.object(from, target); }
        if let Some(value) = &native.target_value { self.value(from, value); }
        for value in native.fields.values() { self.value(from, value); }
        // Text/font data, bitmap dimensions, timer counters and transforms
        // contain no AVM object references; their Rust owners release normally.
    }
}

pub(super) fn collect(objects: &[Weak<RefCell<ObjectData>>]) -> Option<(usize, usize)> {
    if std::env::var("DH2_DEBUG_UI_GC_NESTED_EDGES").as_deref() == Ok("1") {
        collect_with_hasher::<std::collections::hash_map::RandomState>(objects)
    } else {
        collect_with_hasher::<hashbrown::DefaultHashBuilder>(objects)
    }
}

fn collect_with_hasher<S: BuildHasher + Default>(objects: &[Weak<RefCell<ObjectData>>]) -> Option<(usize, usize)> {
    let live: Vec<_> = objects.iter().filter_map(Weak::upgrade).collect();
    let count = live.len();
    let mut graph: Graph<S> = Graph {
        objects: live.iter().enumerate().map(|(i, object)| (Rc::as_ptr(object) as usize, i)).collect(),
        scopes: Vec::new(), scope_indices: HashMap::default(),
        edges: Vec::new(), starts: Vec::with_capacity(count + 1),
        debug_nested: (std::env::var("DH2_DEBUG_UI_GC_NESTED_EDGES").as_deref() == Ok("1")).then(|| vec![Vec::new(); count]),
        incoming: vec![0; count], object_count: count,
    };
    for (index, object) in live.iter().enumerate() {
        graph.starts.push(graph.edges.len());
        // A reentrant native can hold a RefCell borrow while System.gc runs.
        // Defer that collection to the next safepoint, without changing roots.
        let data = object.try_borrow().ok()?;
        graph.trace(index, &data);
    }
    for index in 0..graph.scopes.len() {
        graph.starts.push(graph.edges.len());
        let scope = graph.scopes[index].clone();
        for entry in scope.iter() { graph.value(count + index, &entry.value); }
    }
    graph.starts.push(graph.edges.len());
    let mut marked = vec![false; graph.incoming.len()];
    let mut pending = Vec::new();
    for index in 0..graph.incoming.len() {
        let strong = if index < graph.object_count {
            Rc::strong_count(&live[index])
        } else {
            Rc::strong_count(&graph.scopes[index - count])
        };
        // One reference belongs to this collector's snapshot. Any remaining
        // reference not explained by traced edges is a genuine external root.
        if strong > graph.incoming[index] + 1 {
            marked[index] = true;
            pending.push(index);
        }
    }
    while let Some(index) = pending.pop() {
        for &to in graph.outgoing(index) {
            if !std::mem::replace(&mut marked[to], true) { pending.push(to); }
        }
    }
    for (index, object) in live.iter().enumerate() {
        let mut data = object.borrow_mut();
        if !marked[index] { *data = ObjectData::plain(None); }
        else if data.weak_dictionary_keys {
            if let Kind::Dictionary(entries) = &mut data.kind {
                entries.retain(|(key, _)| key.as_object().is_none_or(|key|
                    graph.objects.get(&key.address()).is_none_or(|&index| marked[index])));
            }
        }
    }
    drop(graph);
    drop(live);
    let remaining = objects.iter().filter(|object| object.strong_count() != 0).count();
    Some((count, count - remaining))
}
