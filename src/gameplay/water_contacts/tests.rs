use super::*;

fn handle(slot: u32) -> ComponentHandle {
    ComponentHandle {
        class_id: 23,
        slot: slot as u16,
    }
}

#[test]
fn membership_updates_match_executed_component_reordering() {
    for (case, row) in include_str!("../../../tests/fixtures/water_contact_order.txt")
        .lines()
        .enumerate()
    {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let count = words.next().unwrap() as usize;
        let components = words.by_ref().take(count).map(handle).collect();
        let changed = handle(words.next().unwrap());
        let states: [WaterContactState; 9] = std::array::from_fn(|_| WaterContactState {
            flags: words.next().unwrap(),
            plane: Some([0., 0., 1., f32::from_bits(words.next().unwrap())]),
        });
        let expected_result = words.next().unwrap() != 0;
        let expected: Vec<_> = words.map(handle).collect();
        let mut contacts = WaterContacts {
            components,
            density: None,
        };
        let result = contacts
            .component_changed(changed, |h| Ok(states[h.slot as usize]))
            .unwrap();
        assert_eq!(result, expected_result, "case {case}");
        assert_eq!(contacts.components, expected, "case {case}");
    }
}

struct DensityWorld {
    density: f32,
    after: f32,
    change: bool,
    mass: Option<f32>,
    properties: u32,
    volume: [f32; 4],
    events: std::cell::RefCell<Vec<u32>>,
}
impl WaterDensityWorld for DensityWorld {
    fn prepare_body(&mut self, body: u32) -> Result<(), String> {
        assert_eq!(body, 1);
        self.events.borrow_mut().push(1);
        if self.change {
            self.density = self.after;
        }
        Ok(())
    }
    fn density_override(&self, _: u32) -> Result<f32, String> {
        Ok(self.density)
    }
    fn body_mass(&self, _: u32) -> Result<f32, String> {
        Ok(self.mass.unwrap_or(-1.))
    }
    fn shape_volume(&self, _: u32) -> Result<Option<[f32; 4]>, String> {
        if self.properties != 0 {
            self.events.borrow_mut().push(10);
        }
        Ok((self.properties == 2).then_some(self.volume))
    }
    fn body_label(&self, _: u32) -> Result<String, String> {
        self.events.borrow_mut().extend([2, 3]);
        Ok("fixture body".into())
    }
}

#[test]
fn density_updates_match_executed_callback_math_and_failure_paths() {
    for (case, row) in include_str!("../../../tests/fixtures/water_contact_density.txt")
        .lines()
        .enumerate()
    {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let density = f32::from_bits(words.next().unwrap());
        let after = f32::from_bits(words.next().unwrap());
        let change = words.next().unwrap() != 0;
        let present = words.next().unwrap() != 0;
        let mass = f32::from_bits(words.next().unwrap());
        let properties = words.next().unwrap();
        let volume = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
        let prior = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
        let failed = words.next().unwrap() != 0;
        let expected: [u32; 4] = std::array::from_fn(|_| words.next().unwrap());
        let count = words.next().unwrap() as usize;
        let events: Vec<_> = words.collect();
        assert_eq!(events.len(), count);
        let mut world = DensityWorld {
            density,
            after,
            change,
            mass: present.then_some(mass),
            properties,
            volume,
            events: Default::default(),
        };
        let mut contacts = WaterContacts {
            density: Some(prior),
            components: Vec::new(),
        };
        let result = contacts.update_density(1, &mut world);
        assert_eq!(result.is_err(), failed, "case {case}");
        if result.is_err() {
            world.events.borrow_mut().push(4);
        }
        assert_eq!(
            contacts.density.unwrap().map(f32::to_bits),
            expected,
            "case {case}"
        );
        assert_eq!(*world.events.borrow(), events, "case {case}");
    }
}

#[test]
fn entry_and_exit_match_executed_membership_transitions() {
    for (case, row) in include_str!("../../../tests/fixtures/water_contact_membership.txt")
        .lines()
        .enumerate()
    {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let count = words.next().unwrap() as usize;
        let components = words.by_ref().take(count).map(handle).collect();
        let changed = handle(words.next().unwrap());
        let adding = words.next().unwrap() != 0;
        let states: [WaterContactState; 9] = std::array::from_fn(|_| WaterContactState {
            flags: words.next().unwrap(),
            plane: Some([0., 0., 1., f32::from_bits(words.next().unwrap())]),
        });
        let expected_result = words.next().unwrap();
        let expected_count = words.next().unwrap() as usize;
        let expected: Vec<_> = words.map(handle).collect();
        assert_eq!(expected.len(), expected_count);
        let mut contacts = WaterContacts {
            components,
            density: None,
        };
        let state = |h: ComponentHandle| Ok(states[h.slot as usize]);
        let result = if adding {
            contacts.enter(changed, state)
        } else {
            contacts.leave(changed, state)
        }
        .unwrap();
        assert_eq!(result as u32, expected_result, "case {case}");
        assert_eq!(contacts.components, expected, "case {case}");
    }
}

struct ContactWorld {
    events: Vec<u32>,
}
impl WaterDensityWorld for ContactWorld {
    fn prepare_body(&mut self, body: u32) -> Result<(), String> {
        self.events.extend([0, body]);
        Ok(())
    }
    fn density_override(&self, _: u32) -> Result<f32, String> {
        Ok(1000.)
    }
    fn body_mass(&self, _: u32) -> Result<f32, String> {
        panic!("positive override bypasses mass")
    }
    fn shape_volume(&self, _: u32) -> Result<Option<[f32; 4]>, String> {
        panic!("positive override bypasses shape volume")
    }
    fn body_label(&self, _: u32) -> Result<String, String> {
        panic!("valid density needs no diagnostic")
    }
}
impl WaterContactWorld for ContactWorld {
    fn water_state(&self, h: ComponentHandle) -> Result<WaterContactState, String> {
        Ok(WaterContactState {
            flags: [8, 4, 8, 4][h.slot as usize],
            plane: Some([0., 0., 1., [-2., 3., -4., 5.][h.slot as usize]]),
        })
    }
    fn deactivate_body(&mut self, body: u32) -> Result<(), String> {
        self.events.extend([2, body]);
        Ok(())
    }
}

#[test]
fn manager_matches_executed_bucket_order_membership_and_body_callbacks() {
    let mut manager = WaterContactManager::with_bucket_count(4).unwrap();
    let mut world = ContactWorld { events: Vec::new() };
    for (case, row) in include_str!("../../../tests/fixtures/water_contact_manager.txt")
        .lines()
        .enumerate()
    {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let operation = words.next().unwrap();
        let body = words.next().unwrap();
        let water = handle(words.next().unwrap());
        let event_count = words.next().unwrap() as usize;
        let expected_events: Vec<_> = words.by_ref().take(event_count).collect();
        let expected_count = words.next().unwrap() as usize;
        let expected: Vec<_> = words.collect();
        world.events.clear();
        if operation == 3 {
            manager.remove_bodies(&[body, body ^ 0x03000000, body]);
        } else if operation == 2 {
            manager.notify_component(water, &mut world).unwrap();
        } else {
            manager
                .contact(body, water, operation != 0, &mut world)
                .unwrap();
        }
        assert_eq!(world.events, expected_events, "events at case {case}");
        assert_eq!(manager.count, expected_count, "count at case {case}");
        let mut actual = Vec::new();
        for bucket in &manager.buckets {
            for entry in bucket {
                actual.extend([entry.id, entry.contacts.components.len() as u32]);
                actual.extend(entry.contacts.components.iter().map(|h| u32::from(h.slot)));
            }
        }
        assert_eq!(actual, expected, "table at case {case}");
    }
}

#[test]
fn stopped_component_order_and_deactivation_match_executed_manager() {
    for (case, row) in include_str!("../../../tests/fixtures/water_contact_stop.txt")
        .lines()
        .enumerate()
    {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let count = words.next().unwrap() as usize;
        let components = words.by_ref().take(count).map(handle).collect();
        let stopped = handle(words.next().unwrap());
        let states: [WaterContactState; 9] = std::array::from_fn(|_| WaterContactState {
            flags: words.next().unwrap(),
            plane: None,
        });
        let expected_result = words.next().unwrap() != 0;
        let expected: Vec<_> = words.map(handle).collect();
        let mut contacts = WaterContacts {
            components,
            density: None,
        };
        let result = contacts
            .component_stopped(stopped, |h| Ok(states[h.slot as usize]))
            .unwrap();
        assert_eq!(result, expected_result, "case {case}");
        assert_eq!(contacts.components, expected, "case {case}");
    }
}

#[test]
fn contact_table_defaults_match_executed_owner_constructor() {
    let words: Vec<_> = include_str!("../../../tests/fixtures/water_contact_constructor.txt")
        .split_whitespace()
        .map(|v| u32::from_str_radix(v, 16).unwrap())
        .collect();
    let manager = WaterContactManager::new().unwrap();
    assert_eq!(manager.buckets.len() as u32, words[0]);
    assert_eq!(manager.count as u32, words[1]);
    assert_eq!(manager.mask, words[2]);
    assert!(manager.buckets.iter().all(Vec::is_empty));
}
