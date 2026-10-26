use super::*;

#[test]
fn body_properties_match_executed_manager_operations() {
    let names: [Option<&[u8]>; 4] = [
        None,
        Some(b""),
        Some(b"water volume"),
        Some(b"body\xffname"),
    ];
    for (case, line) in include_str!("../../../tests/fixtures/body_properties.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let capacity = words.next().unwrap() as usize;
        let fallback = words.next().unwrap();
        let count = words.next().unwrap();
        let mut storage = BodyStorage::new(capacity).unwrap();
        for operation in 0..count {
            let kind = words.next().unwrap();
            let id = words.next().unwrap();
            let input = words.next().unwrap();
            let expected_return = words.next().unwrap();
            match kind {
                0 => assert_eq!(
                    storage
                        .set_mass(id, f32::from_bits(input))
                        .unwrap()
                        .to_bits(),
                    expected_return,
                    "case {case} operation {operation} mass return, fallback {fallback:x}"
                ),
                1 => storage.set_motion_properties(id, input as u16).unwrap(),
                2 => storage.set_name(id, names[input as usize]).unwrap(),
                _ => panic!("unknown oracle operation"),
            }
            let property_count = words.next().unwrap() as usize;
            assert_eq!(storage.mass_properties.len(), property_count);
            for (index, property) in storage.mass_properties.iter().enumerate() {
                assert_eq!(
                    property.mass.to_bits(),
                    words.next().unwrap(),
                    "case {case} operation {operation} mass {index}"
                );
                assert_eq!(
                    u32::from(property.motion_properties_id),
                    words.next().unwrap(),
                    "case {case} operation {operation} motion properties {index}"
                );
            }
            let name_count = words.next().unwrap() as usize;
            assert_eq!(storage.names.len(), name_count);
            for index in 0..name_count {
                let expected = names[words.next().unwrap() as usize];
                assert_eq!(
                    storage.name(index as u32),
                    expected,
                    "case {case} operation {operation} name {index}"
                );
            }
        }
        assert!(words.next().is_none());
    }
}
