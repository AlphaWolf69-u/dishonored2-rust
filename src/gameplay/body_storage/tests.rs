use super::*;
#[test]
fn body_creation_matches_executed_complete_initializer() {
    let names: [Option<&[u8]>; 4] = [
        None,
        Some(b""),
        Some(b"water volume"),
        Some(b"body\xffname"),
    ];
    let mut storage = BodyStorage::new(2).unwrap();
    let id = storage.reserve(&mut []).unwrap();
    for (case, line) in include_str!("../../../tests/fixtures/body_creation.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let flags = words.next().unwrap();
        let quality_id = words.next().unwrap() as u8;
        let material_id = words.next().unwrap() as u16;
        let mass = f32::from_bits(words.next().unwrap());
        let motion_properties_id = words.next().unwrap() as u16;
        let dynamic = words.next().unwrap() != 0;
        let name = names[words.next().unwrap() as usize];
        let present = words.next().unwrap() != 0;
        let vectors: [[f32; 4]; 3] =
            std::array::from_fn(|_| std::array::from_fn(|_| f32::from_bits(words.next().unwrap())));
        let distribution = MassDistribution {
            center_of_mass_and_volume: vectors[0],
            major_axis_space: vectors[1],
            inertia_tensor: vectors[2],
        };
        let info = BodyInfo {
            flags,
            quality_id,
            material_id,
            mass,
            motion_properties_id,
            name: usize::from(name.is_some()),
            mass_distribution: usize::from(present),
            ..BodyInfo::default()
        };
        // The native probe starts this record cleared each time, while retaining
        // the manager sidecars. Preserve only the reserved ID in the Rust owner.
        *storage.get_mut(id).unwrap() = CollisionBody {
            id,
            ..CollisionBody::default()
        };
        storage
            .initialize(
                id,
                &info,
                dynamic,
                BodyCreationAssets {
                    shape: ShapeHeader {
                        flags: 0,
                        shape_type: 0,
                        key_bits: 0,
                        dispatch_type: 0,
                        radius: 0.,
                        user_data: 0,
                    },
                    wrapped_shapes: None,
                    name,
                    mass_distribution: present.then_some(distribution),
                },
            )
            .unwrap();
        let expected: [u32; 44] = std::array::from_fn(|_| words.next().unwrap());
        assert_eq!(
            anchor_words(storage.get(id).unwrap()),
            expected,
            "case {case} body"
        );
        assert_eq!(storage.name(id), name, "case {case} name");
        let override_length = words.next().unwrap();
        if override_length != 0 {
            assert_eq!(override_length, 2);
            let property = storage.mass_properties(id).unwrap();
            assert_eq!(
                property.mass.to_bits(),
                words.next().unwrap(),
                "case {case} mass"
            );
            assert_eq!(
                u32::from(property.motion_properties_id),
                words.next().unwrap(),
                "case {case} properties"
            );
        } else {
            assert!(storage.mass_properties(id).is_none());
        }
        let distribution = storage.mass_distribution(id).unwrap();
        for value in [
            distribution.center_of_mass_and_volume,
            distribution.major_axis_space,
            distribution.inertia_tensor,
        ]
        .into_iter()
        .flatten()
        {
            assert_eq!(
                value.to_bits(),
                words.next().unwrap(),
                "case {case} distribution"
            );
        }
        assert!(words.next().is_none());
    }
}

fn anchor_words(body: &CollisionBody) -> [u32; 44] {
    let mut w = [0; 44];
    for (out, value) in w[..16].iter_mut().zip(body.transform.into_iter().flatten()) {
        *out = value.to_bits();
    }
    w[0x40 / 4] = body.motion_id;
    w[0x44 / 4] = body.flags;
    w[0x48 / 4] = u32::from(body.status);
    w[0x50 / 4..0x60 / 4].copy_from_slice(&body.bounds.words);
    w[0x68 / 4] = u32::from(body.lod_flags)
        | (u32::from(body.quality_id) << 8)
        | (u32::from(body.material_id) << 16);
    w[0x6c / 4] = body.filter_info;
    w[0x70 / 4] = body.id;
    w[0x74 / 4] = body.next_attached_body_id;
    w[0x78 / 4] = body.broadphase_id;
    w[0x7c / 4] = u32::from(body.packed_radius) << 16;
    w[0x90 / 4] = body.user_data as u32;
    w[0x94 / 4] = (body.user_data >> 32) as u32;
    w[0x98 / 4] = body.active_list_index;
    w[0x9c / 4] = u32::from(body.packed_average_surface_velocity);
    w[0xa0 / 4..].copy_from_slice(&body.motion_to_body_orientation.map(f32::to_bits));
    w
}
#[test]
fn body_storage_matches_executed_construction_and_reservation() {
    for (case, line) in include_str!("../../../tests/fixtures/body_storage.txt")
        .lines()
        .enumerate()
    {
        let mut w = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let capacity = w.next().unwrap() as usize;
        let generations: Vec<_> = w.by_ref().take(capacity).collect();
        let markers: Vec<_> = w.by_ref().take(capacity).map(|v| v as u8).collect();
        let auxiliary: Vec<_> = w.by_ref().take(capacity).collect();
        let bounds: Vec<_> = (0..capacity)
            .map(|_| PackedBodyBounds {
                words: std::array::from_fn(|_| w.next().unwrap()),
            })
            .collect();
        let property_count = w.next().unwrap() as usize;
        let mut properties: Vec<_> = (0..property_count)
            .map(|_| {
                let size = w.next().unwrap() as usize;
                let count = w.next().unwrap() as usize;
                BodyPropertyBits::from_words(size, w.by_ref().take(count).collect()).unwrap()
            })
            .collect();
        let mut storage = BodyStorage::new(capacity).unwrap();
        let expected: [u32; 44] = std::array::from_fn(|_| w.next().unwrap());
        assert_eq!(
            anchor_words(storage.get(0).unwrap()),
            expected,
            "case {case} anchor"
        );
        for i in 1..capacity {
            storage.bodies[i].id = (storage.bodies[i].id & 0xff_ffff) | (generations[i] << 24);
        }
        storage.allocation_marks = markers;
        storage.change_indices = auxiliary.clone();
        storage.previous_bounds = bounds.clone();
        for _ in 1..capacity {
            let id = storage.reserve(&mut properties).unwrap();
            assert_eq!(id, w.next().unwrap(), "case {case} ID");
            assert_eq!(
                storage.active_count(),
                w.next().unwrap(),
                "case {case} count"
            );
            assert_eq!(
                storage.maximum_index(),
                w.next().unwrap(),
                "case {case} maximum index"
            );
            assert_eq!(
                storage.free_head.map_or(u32::MAX, |i| i as u32),
                w.next().unwrap(),
                "case {case} free head"
            );
            assert_eq!(
                u32::from(storage.allocation_marks()[(id & 0xff_ffff) as usize]),
                w.next().unwrap(),
                "case {case} allocation marker"
            );
            for property in &properties {
                for &word in property.words() {
                    assert_eq!(word, w.next().unwrap(), "case {case} property mask");
                }
            }
            let body = storage.get(id).unwrap();
            let mut expected = [0; 44];
            expected[0x70 / 4] = id;
            assert_eq!(
                anchor_words(body),
                expected,
                "case {case} cleared reserved record"
            );
        }
        assert!(!storage.has_free_slot());
        assert!(storage.reserve(&mut properties).is_err());
        assert_eq!(storage.capacity(), capacity);
        assert_eq!(storage.change_indices(), auxiliary);
        assert_eq!(storage.previous_bounds(), bounds);
        assert!(storage.get(0x00ff_ffff).is_none());
        assert_eq!(w.next(), None);
    }
}
