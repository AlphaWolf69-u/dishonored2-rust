use super::*;

fn handle(index: usize) -> ComponentHandle {
    ComponentHandle {
        class_id: 17,
        slot: index as u16,
    }
}
struct World {
    owners: Vec<ComponentOwner>,
    ranks: Vec<i32>,
    mode: u32,
    number: usize,
    added: bool,
    events: Vec<u32>,
}
impl World {
    fn record(&mut self, tag: u32, h: ComponentHandle, r: &ComponentRegistry) {
        self.events.extend([
            tag,
            u32::from(h.slot),
            self.owners[h.slot as usize].flags,
            r.processed_initialization_count as u32,
            r.processed_start_count as u32,
        ]);
    }
    fn append_new(&mut self, registry: &mut ComponentRegistry) -> Result<(), String> {
        if !self.added {
            self.added = true;
            let h = handle(self.number);
            registry.registered.append(h.packed(), false)?;
            registry.maximum_registered = registry
                .maximum_registered
                .max(registry.registered.count() as i32);
            // Both admitted component vtables use0998b0 for virtual38: an
            // actual empty callback, independently confirmed in Ghidra.
            registry.request_component_start(
                &mut self.owners[self.number],
                h.packed() as usize,
                false,
                |_, _| Ok(()),
            )?;
        }
        Ok(())
    }
}
impl ComponentLifecycleWorld for World {
    fn priority(&self, h: ComponentHandle) -> Result<i32, String> {
        Ok(self.ranks[h.slot as usize])
    }
    fn owner(&self, h: ComponentHandle) -> Result<&ComponentOwner, String> {
        Ok(&self.owners[h.slot as usize])
    }
    fn owner_mut(&mut self, h: ComponentHandle) -> Result<&mut ComponentOwner, String> {
        Ok(&mut self.owners[h.slot as usize])
    }
    fn initialize(&mut self, h: ComponentHandle, r: &mut ComponentRegistry) -> Result<(), String> {
        self.record(0x18, h, r);
        if h.slot == 0 && self.mode >= 3 {
            self.owners[0].flags = [5, 9, 0x10000009][(self.mode - 3) as usize];
        } else {
            self.owners[h.slot as usize].mark_initialized();
        }
        if h.slot == 0 && self.mode == 1 {
            self.append_new(r)?;
        }
        Ok(())
    }
    fn start(&mut self, h: ComponentHandle, r: &mut ComponentRegistry) -> Result<(), String> {
        self.record(0x28, h, r);
        let flags = &mut self.owners[h.slot as usize].flags;
        *flags = (*flags & !4) | 8;
        if h.slot == 0 && self.mode == 2 {
            self.append_new(r)?;
        }
        Ok(())
    }
    fn stop_queued(&mut self, h: ComponentHandle, r: &mut ComponentRegistry) -> Result<(), String> {
        self.record(0x40, h, r);
        Ok(())
    }
    fn initialization_failed(
        &mut self,
        h: ComponentHandle,
        r: &mut ComponentRegistry,
    ) -> Result<(), String> {
        self.record(0x78, h, r);
        if self.mode >= 4 {
            self.owners[h.slot as usize].flags = 2;
        }
        Ok(())
    }
}

impl ComponentReleaseWorld for World {
    fn shutdown(&mut self, h: ComponentHandle, r: &mut ComponentRegistry) -> Result<(), String> {
        self.record(0x10, h, r);
        self.owners[h.slot as usize].flags &= !3;
        Ok(())
    }
    fn released(&mut self, h: ComponentHandle, r: &mut ComponentRegistry) -> Result<(), String> {
        self.record(0x78, h, r);
        self.owners[h.slot as usize].flags |= 0x2000_0000;
        Ok(())
    }
}

#[test]
fn release_requests_match_executed_stop_cancel_virtuals_and_queues() {
    let mut cases = 0;
    for row in include_str!("../../../../tests/fixtures/component_release_requests.txt").lines() {
        let mut words = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap());
        let flags = words.next().unwrap();
        let mut registry = ComponentRegistry::new().unwrap();
        registry.processed_initialization_count = words.next().unwrap() as i32;
        registry.processed_start_count = words.next().unwrap() as i32;
        registry.maximum_registered = words.next().unwrap() as i32;
        for list in [
            &mut registry.registered,
            &mut registry.pending_start,
            &mut registry.pending_stop,
            &mut registry.pending_release,
        ] {
            let count = words.next().unwrap();
            for _ in 0..count {
                list.append(handle(words.next().unwrap() as usize).packed(), false)
                    .unwrap();
            }
        }
        let mut world = World {
            owners: (0..8)
                .map(|_| ComponentOwner {
                    flags,
                    ..Default::default()
                })
                .collect(),
            ranks: vec![0; 8],
            mode: 0,
            number: 7,
            added: false,
            events: Vec::new(),
        };
        registry.request_release(handle(0), &mut world).unwrap();
        let mut actual = vec![
            world.owners[0].flags,
            registry.maximum_registered as u32,
            world.events.len() as u32,
        ];
        actual.extend(world.events);
        for list in [
            &registry.registered,
            &registry.pending_start,
            &registry.pending_stop,
            &registry.pending_release,
        ] {
            let (count, capacity) = list.packed_header();
            actual.extend([count, capacity]);
            actual.extend(
                list.values()
                    .iter()
                    .map(|&v| u32::from(ComponentHandle::from_packed(v).unwrap().slot)),
            );
        }
        assert_eq!(actual, words.collect::<Vec<_>>(), "case{cases}");
        cases += 1;
    }
    assert_eq!(cases, 512);
}

#[test]
fn complete_batches_match_executed_sort_callbacks_and_queue_ownership() {
    for (case, row) in include_str!("../../../../tests/fixtures/component_lifecycle.txt")
        .lines()
        .enumerate()
    {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let number = words.next().unwrap() as usize;
        let mode = words.next().unwrap();
        let capacity = words.next().unwrap();
        let ranks = words.by_ref().take(number + 1).map(|v| v as i32).collect();
        let mut registry = ComponentRegistry::new().unwrap();
        registry.registered = PointerList::owned_with_capacity(capacity).unwrap();
        registry.pending_start = PointerList::owned_with_capacity(capacity).unwrap();
        registry.pending_stop = PointerList::owned_with_capacity(capacity).unwrap();
        for index in words.by_ref().take(number) {
            registry
                .registered
                .append(handle(index as usize).packed(), false)
                .unwrap();
        }
        for index in words.by_ref().take(number) {
            registry
                .pending_start
                .append(handle(index as usize).packed(), false)
                .unwrap();
        }
        let mut world = World {
            owners: (0..=number)
                .map(|i| ComponentOwner {
                    flags: if i < number { 5 } else { 1 },
                    ..Default::default()
                })
                .collect(),
            ranks,
            mode,
            number,
            added: false,
            events: Vec::new(),
        };
        registry.initialize_and_start(&mut world).unwrap();
        let mut actual = vec![world.events.len() as u32];
        actual.extend(world.events);
        actual.extend(world.owners.iter().map(|v| v.flags));
        actual.extend([
            registry.maximum_registered as u32,
            registry.queue_reset_count as u32,
            registry.processed_initialization_count as u32,
            registry.processed_start_count as u32,
        ]);
        for list in [
            &registry.registered,
            &registry.pending_start,
            &registry.pending_stop,
        ] {
            let (count, capacity) = list.packed_header();
            actual.extend([count, capacity]);
            actual.extend(
                list.values()
                    .iter()
                    .map(|&v| u32::from(ComponentHandle::from_packed(v).unwrap().slot)),
            );
        }
        assert_eq!(actual, words.collect::<Vec<_>>(), "case {case}");
    }
}
