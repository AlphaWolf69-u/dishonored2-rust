//! Host ownership of one movie's runtime allocations. Iggy 1410298b0 tears
//! down its VM (including 1410629a0/141086460), then frees its allocator.
//! Rust reference counts alone cannot release AS3's cyclic object graph.
use super::object::{Kind, Obj, ObjectData};
use std::{cell::RefCell, rc::{Rc, Weak}};
use std::cell::Cell;
mod tracing;

#[derive(Default)]
pub(super) struct Heap {
    objects: RefCell<Vec<Weak<RefCell<ObjectData>>>>,
    allocations: Cell<usize>,
    threshold: Cell<usize>,
    forced: Cell<bool>,
}

impl Heap {
    pub fn allocate(&self, data: ObjectData) -> Obj {
        let object = Obj::new(data);
        let mut objects = self.objects.borrow_mut();
        if objects.len() == objects.capacity() {
            // Weak registrations must not accumulate dead allocations during
            // a long-running movie, or keep their control blocks indefinitely.
            objects.retain(|object| object.strong_count() != 0);
        }
        objects.push(object.downgrade());
        self.allocations.set(self.allocations.get().saturating_add(1));
        object
    }

    /// Host collector for the roots/trace/sweep behavior in 1410642f0,
    /// 141064430, 141063ed0 and 141063530. Run at movie safepoints or the
    /// original System.gc entry (1410b0040 -> 141062a90).
    /// Growth-relative scheduling replaces Iggy's allocation-budget machinery;
    /// it adds no collector work to ordinary gameplay while no movie is ticking.
    pub fn collect(&self, force: bool) {
        if force { self.forced.set(true); }
        if !self.forced.get() && self.allocations.get() < self.threshold.get() { return; }
        if std::env::var("DH2_DEBUG_RETAIN_UI_GARBAGE").as_deref() == Ok("1") { return; }
        let started = std::time::Instant::now();
        let Some((visited, collected)) = tracing::collect(&self.objects.borrow()) else { return };
        self.forced.set(false);
        let mut objects = self.objects.borrow_mut();
        objects.retain(|object| object.strong_count() != 0);
        self.allocations.set(0);
        self.threshold.set(objects.len().max(1));
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY UI collect: visited={visited} collected={collected} remaining={} collect_ms={:.3}",
                objects.len(), started.elapsed().as_secs_f64() * 1000.0);
        }
    }

    pub fn release(&self, enabled: bool) {
        let started = std::time::Instant::now();
        let weak = std::mem::take(&mut *self.objects.borrow_mut());
        let live: Vec<_> = weak.iter().filter_map(Weak::upgrade).collect();
        if enabled {
            // Keep the allocations alive together while breaking their edges:
            // this avoids recursive destruction of a deeply nested display tree.
            for object in &live {
                let mut data = object.borrow_mut();
                if let Some(class) = &data.class { clear_class(class); }
                if let Kind::Class(class) = &data.kind { clear_class(class); }
                *data = ObjectData::plain(None);
            }
        }
        let count = live.len();
        drop(live);
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY UI runtime release: cleanup={enabled} tracked={} live={count} remaining={} teardown_ms={:.3}",
                weak.len(), weak.iter().filter(|object| object.strong_count() != 0).count(),
                started.elapsed().as_secs_f64() * 1000.0);
        }
    }
}

fn clear_class(class: &Rc<super::object::Class>) {
    class.object.borrow_mut().take();
    class.prototype.borrow_mut().take();
    if !class.scope.borrow().is_empty() { *class.scope.borrow_mut() = Rc::new([]); }
}
