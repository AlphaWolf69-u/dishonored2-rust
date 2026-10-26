use super::*;

#[test]
#[ignore = "requires installed executable; independent execution of all350 constructors"]
fn installed_component_definitions_match_executed_constructors() {
    let declarations =
        ClassDeclarations::load(&crate::assets::installation::game_base().unwrap()).unwrap();
    let fixture: Vec<_> = include_str!("../../../tests/fixtures/component_definitions_digest.txt")
        .split_whitespace()
        .map(|s| u64::from_str_radix(s, 16).unwrap())
        .collect();
    assert_eq!(declarations.component_definitions.len() as u64, fixture[0]);
    let mut canonical = Vec::new();
    let mut records = 0;
    for definition in &declarations.component_definitions {
        for list in &definition.ordering {
            canonical.extend((list.len() as u32).to_le_bytes());
            records += list.len();
            for record in list {
                canonical.extend(
                    declarations.classes[record.class]
                        .class_info_rva
                        .to_le_bytes(),
                );
                canonical.extend(record.argument.to_le_bytes());
                canonical.extend((record.reason.len() as u32).to_le_bytes());
                canonical.extend(record.reason.as_bytes());
            }
        }
        for list in std::iter::once(&definition.initialization).chain(&definition.updates) {
            canonical.extend((list.len() as u32).to_le_bytes());
            records += list.len();
            for record in list {
                canonical.extend(
                    declarations.classes[record.class]
                        .class_info_rva
                        .to_le_bytes(),
                );
                canonical.push(u8::from(record.exact));
                canonical.extend((record.reason.len() as u32).to_le_bytes());
                canonical.extend(record.reason.as_bytes());
            }
        }
        for scalar in definition.stage_values {
            canonical.extend(scalar.to_le_bytes());
        }
    }
    assert_eq!(records as u64, fixture[1]);
    let hash = canonical.iter().fold(0xcbf29ce484222325u64, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    assert_eq!(hash, fixture[2]);
}

#[test]
#[ignore = "requires the installed Dishonored 2 executable"]
fn installed_declarations_match_independent_disassembly_audit() {
    let base = crate::assets::installation::game_base().unwrap();
    let declarations = ClassDeclarations::load(&base).unwrap();
    assert_eq!(declarations.classes.len(), 1904);
    assert_eq!(declarations.startup_function_count, 26015);
    assert_eq!(declarations.component_pools.len(), 350);
    let mut pool_bytes = Vec::new();
    for pool in &declarations.component_pools {
        for value in [
            pool.pool_index,
            declarations.classes[pool.class].class_info_rva,
            pool.initial_count_rva,
            pool.vtable_rva,
            pool.constructor_rva,
            0x38 + 8 * pool.pool_index,
        ] {
            pool_bytes.extend(value.to_le_bytes());
        }
        for text in [&pool.initial_count_cvar, &pool.initial_count_reset] {
            pool_bytes.extend((text.len() as u32).to_le_bytes());
            pool_bytes.extend(text.as_bytes());
        }
    }
    assert_eq!(
        blake3::hash(&pool_bytes).to_hex().as_str(),
        "2658128c8542c2ec6b764098f5ede062f599395bbedc25de61e3fd88c63d1d68"
    );
    // Golden data came from the independent PE/Capstone/Ghidra-reference audit,
    // not this reader. Only its digest is retained, not the game's declarations.
    let mut canonical = Vec::new();
    for row in &declarations.classes {
        for bytes in [&row.name, &row.base_name] {
            canonical.extend((bytes.len() as u32).to_le_bytes());
            canonical.extend(bytes);
        }
        canonical.extend(row.instance_size.to_le_bytes());
        canonical.push(row.flags);
        canonical.extend(row.initializer_rva.to_le_bytes());
        canonical.extend(row.class_info_rva.to_le_bytes());
        for key in row.dispatch_rvas {
            canonical.extend(key.to_le_bytes());
        }
        canonical.extend(row.callback_method_rva.to_le_bytes());
    }
    assert_eq!(
        blake3::hash(&canonical).to_hex().as_str(),
        "b25a4da34cc24ce635f37a46dd11afcd7cc7b604672748b677ea5826e18a862b"
    );
    let registry = declarations.registry().unwrap();
    assert_eq!(registry.hierarchy().by_id().len(), 1904);
    assert_eq!(registry.hierarchy().type_bits(), 11);
    for (index, row) in declarations.classes.iter().enumerate() {
        let meta = registry.metadata(index).unwrap();
        assert_eq!(registry.name_text(meta.name).unwrap(), row.name);
        let node = registry.hierarchy().node(index).unwrap();
        if row.name == b"idClass" {
            assert!(node.base.is_none());
        } else {
            let parent = node.base.unwrap();
            assert_eq!(
                registry
                    .name_text(registry.metadata(parent).unwrap().name)
                    .unwrap(),
                row.base_name
            );
            let parent = registry.hierarchy().node(parent).unwrap();
            assert!(parent.type_id <= node.type_id && node.last_type_id <= parent.last_type_id);
        }
    }
}

#[test]
#[ignore = "requires installed executable; independent full constructor audit"]
fn installed_component_type_inputs_match_executed_constructor() {
    let base = crate::assets::installation::game_base().unwrap();
    let declarations = ClassDeclarations::load(&base).unwrap();
    let mut words = include_str!("../../../tests/fixtures/component_type_declarations_digest.txt")
        .split_whitespace();
    let count = usize::from_str_radix(words.next().unwrap(), 16).unwrap();
    let expected = u64::from_str_radix(words.next().unwrap(), 16).unwrap();
    assert_eq!(declarations.component_types.len(), count);
    let mut canonical = Vec::new();
    for (index, entry) in declarations.component_types.iter().enumerate() {
        assert_eq!(entry.pool_index, index as u32);
        assert_eq!(entry.class, declarations.component_pools[index].class);
        for value in [
            declarations.classes[entry.class].class_info_rva,
            entry.setup_rva,
            entry.update_flags,
            entry.definition_rva,
        ] {
            canonical.extend(value.to_le_bytes());
        }
    }
    // Golden checksum comes from executing the actual constructor with owned
    // allocator storage, independently of this checked PE instruction decoder.
    let actual = canonical.iter().fold(0xcbf29ce484222325u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    assert_eq!(actual, expected);
}
