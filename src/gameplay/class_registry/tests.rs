use super::*;

fn label(index: u32) -> Vec<u8> {
    match index {
        u32::MAX => Vec::new(),
        0 => b"idClass".to_vec(),
        _ => format!("T{index}").into_bytes(),
    }
}
fn snapshot(classes: &ClassRegistry) -> Vec<u64> {
    let hierarchy = classes.hierarchy();
    let mut out = vec![
        classes.metadata.len() as u64,
        u64::from(hierarchy.is_initialized()),
        u64::from(hierarchy.type_bits() as u32),
    ];
    out.extend(
        classes
            .names
            .test_key_location(classes.root_name)
            .map(u64::from),
    );
    for list in [
        hierarchy.registration_order(),
        hierarchy.by_name(),
        hierarchy.by_id(),
    ] {
        out.push(list.len() as u64);
        out.extend(list.iter().map(|&v| v as u64));
    }
    for (index, meta) in classes.metadata.iter().enumerate() {
        let node = hierarchy.node(index).unwrap();
        out.extend(
            classes
                .names
                .test_key_location(Some(meta.name))
                .map(u64::from),
        );
        out.extend(
            classes
                .names
                .test_key_location(Some(meta.base_name))
                .map(u64::from),
        );
        out.extend([
            u64::from(node.base.map_or(u32::MAX, |v| v as u32)),
            u64::from(meta.instance_size),
            u64::from(meta.flags),
        ]);
        out.extend(meta.dispatch_keys);
        out.extend(meta.callback_words);
        out.extend([
            u64::from(node.type_id as u32),
            u64::from(node.last_type_id as u32),
            u64::from(node.initialized),
        ]);
    }
    out
}

#[test]
fn registrations_match_composed_name_pool_class_constructor_and_hierarchy() {
    let mut classes = ClassRegistry::default();
    let mut scenarios = 0;
    for line in include_str!("../../../tests/fixtures/class_registry.txt").lines() {
        let mut parts = line.split_whitespace();
        let operation = parts.next().unwrap();
        if operation == "reset" {
            classes = ClassRegistry::default();
            scenarios += 1;
            continue;
        }
        let mut words = parts.map(|v| u64::from_str_radix(v, 16).unwrap());
        let mut actual = Vec::new();
        match operation {
            "register" => {
                let name = label(words.next().unwrap() as u32);
                let base_name = label(words.next().unwrap() as u32);
                let size = words.next().unwrap() as u32;
                let flags = words.next().unwrap() as u8;
                let dispatch_keys = std::array::from_fn(|_| words.next().unwrap());
                let callback_words = std::array::from_fn(|_| words.next().unwrap());
                let index = classes
                    .register(ClassDefinition {
                        name: &name,
                        base_name: &base_name,
                        instance_size: size,
                        flags,
                        dispatch_keys,
                        callback_words,
                    })
                    .unwrap();
                assert_eq!(
                    classes.name_text(classes.metadata(index).unwrap().name),
                    Some(name.as_slice())
                );
            }
            "intern" => {
                let name = label(words.next().unwrap() as u32);
                let key = classes.intern_name(Some(&name)).unwrap();
                actual.extend(classes.names.test_key_location(Some(key)).map(u64::from));
            }
            "initialize" => {
                classes.initialize().unwrap();
            }
            _ => unreachable!(),
        }
        actual.extend(snapshot(&classes));
        assert_eq!(
            actual,
            words.collect::<Vec<_>>(),
            "scenario {scenarios}, operation {operation}"
        );
    }
    assert_eq!(scenarios, 64);
}
