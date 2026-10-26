use super::*;

fn constants() -> Arc<MaterialConstants> {
    Arc::new(MaterialConstants::load(&crate::assets::installation::game_base().unwrap()).unwrap())
}
fn name(key: u32) -> Option<Arc<str>> {
    match key {
        0 => None,
        1 => Some(Arc::from("")),
        2 => Some(Arc::from("mat")),
        3 => Some(Arc::from("Mat")),
        101 => Some(Arc::from("[Preset] Trigger (broad phase)")),
        102 => Some(Arc::from("[Preset] Trigger (narrow phase)")),
        103 => Some(Arc::from("[Preset] Trigger (contact solver)")),
        _ => panic!("unknown fixture name"),
    }
}
fn record(words: &mut impl Iterator<Item = u32>) -> MaterialRecord {
    MaterialRecord {
        name: name(words.next().unwrap()),
        words: std::array::from_fn(|_| words.next().unwrap()),
        tail: std::array::from_fn::<_, 2, _>(|_| words.next().unwrap().to_le_bytes())
            .concat()
            .try_into()
            .unwrap(),
    }
}
fn snapshot(library: &MaterialLibrary<u32, u32>, words: &mut impl Iterator<Item = u32>) {
    assert_eq!(library.records().len() as u32, words.next().unwrap());
    assert_eq!(library.free_head() as u32, words.next().unwrap());
    for actual in library.records() {
        assert_eq!(*actual, record(words));
    }
}

#[test]
#[ignore = "requires packing constants in the installed executable"]
fn installed_material_library_matches_executed_registration_and_reuse() {
    let constants = constants();
    for (case, row) in include_str!("../../../tests/fixtures/material_library.txt")
        .lines()
        .enumerate()
    {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let mut library =
            MaterialLibrary::new(constants.clone(), words.next().unwrap() as usize).unwrap();
        snapshot(&library, &mut words);
        library.added.subscribe(17, 23);
        let steps = words.next().unwrap();
        for step in 0..steps {
            let input = record(&mut words);
            let mut events = Vec::new();
            let result = library
                .add(input, |_, subscriber, callback, id| {
                    assert_eq!((subscriber, callback), (17, 23));
                    events.push(u32::from(id));
                })
                .unwrap();
            assert_eq!(
                u32::from(result),
                words.next().unwrap(),
                "case {case} step {step}"
            );
            snapshot(&library, &mut words);
            assert_eq!(events.len() as u32, words.next().unwrap());
            assert_eq!(
                events,
                words.by_ref().take(events.len()).collect::<Vec<_>>()
            );
        }
        assert!(words.next().is_none());
    }
}

#[test]
#[ignore = "requires packing constants in the installed executable"]
fn installed_material_conversion_and_distance_packing_match_executed_functions() {
    let constants = constants();
    for row in include_str!("../../../tests/fixtures/material_distance_packing.txt").lines() {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        assert_eq!(
            u32::from(constants.pack_distance(f32::from_bits(words.next().unwrap()))),
            words.next().unwrap()
        );
        assert!(words.next().is_none());
    }
    for row in include_str!("../../../tests/fixtures/material_conversion.txt").lines() {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let source_name = name(words.next().unwrap()).unwrap();
        let flags = words.next().unwrap() as u8;
        let values: [f32; 4] = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
        let handle = words.next().unwrap() as usize;
        let mut material = PhysicsMaterial::constructor_default(&source_name);
        material.flags = flags;
        material.density = values[0];
        material.dynamic_friction = values[1];
        material.static_friction = values[2];
        material.restitution = values[3];
        assert_eq!(
            MaterialRecord::from_declaration(&constants, &material, handle).unwrap(),
            record(&mut words)
        );
        assert!(words.next().is_none());
    }
}

#[test]
#[ignore = "requires installed physics material declarations and executable"]
fn installed_material_owner_resolves_trigger_and_water_declarations_once() {
    let catalog =
        crate::assets::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
            .unwrap();
    let mut owner = MaterialOwner::<u32, u32>::new(constants(), 16).unwrap();
    let trigger = owner.declaration(&catalog, "narrow_phase_trigger").unwrap();
    let solver = owner
        .declaration(&catalog, "contact_solver_trigger")
        .unwrap();
    let water = owner
        .declaration(&catalog, "contactsystem/env.water")
        .unwrap();
    assert_eq!(
        owner.declaration(&catalog, "narrow_phase_trigger").unwrap(),
        trigger
    );
    let no_subscribers =
        |_: &mut Signal<u32, u32>, _: u32, _: u32, _: u16| panic!("no observers registered");
    let trigger_id = owner.resolve(trigger, no_subscribers).unwrap();
    let solver_id = owner.resolve(solver, no_subscribers).unwrap();
    let water_id = owner.resolve(water, no_subscribers).unwrap();
    assert_eq!((trigger_id, solver_id, water_id), (4, 5, 6));
    assert_eq!(owner.resolve(water, no_subscribers).unwrap(), water_id);
    assert_ne!(owner.get(water).unwrap().flags & 0x20, 0);
    assert_eq!(
        owner.library.get(trigger_id).unwrap().name.as_deref(),
        Some("narrow_phase_trigger")
    );
}
