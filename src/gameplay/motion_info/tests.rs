use super::*;
#[test]
fn compound_mass_matches_executed_virtual_dispatch() {
    for (case, line) in include_str!("../../../tests/fixtures/bounds_mass.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let config = [0, words.next().unwrap(), words.next().unwrap()];
        let bounds =
            std::array::from_fn(|_| std::array::from_fn(|_| f32::from_bits(words.next().unwrap())));
        for _ in 0..16 {
            words.next().unwrap();
        }
        let expected: [u32; 16] = std::array::from_fn(|_| words.next().unwrap());
        let actual = calculate_compound_mass(config, bounds).unwrap();
        let actual: Vec<_> = [actual.volume, actual.mass]
            .into_iter()
            .chain(actual.center)
            .chain(actual.inertia)
            .chain(actual.orientation)
            .map(f32::to_bits)
            .collect();
        let expected: Vec<_> = expected[..2]
            .iter()
            .chain(&expected[4..])
            .copied()
            .collect();
        assert_eq!(actual, expected, "case {case}");
        assert!(words.next().is_none());
    }
}

fn snapshot(info: MotionInfo) -> Vec<u32> {
    [
        u32::from(info.properties_id) | (u32::from(info.enable_deactivation) << 16),
        info.inverse_mass.to_bits(),
        info.limits[0].to_bits(),
        info.limits[1].to_bits(),
    ]
    .into_iter()
    .chain(info.inverse_inertia.map(f32::to_bits))
    .chain(info.pose.center.map(f32::to_bits))
    .chain(info.pose.orientation.map(f32::to_bits))
    .chain(info.linear_velocity.map(f32::to_bits))
    .chain(info.angular_velocity.map(f32::to_bits))
    .collect()
}

#[test]
fn motion_info_matches_executed_constructor_and_body_preparation() {
    for (case, line) in include_str!("../../../tests/fixtures/motion_info.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let cached_kind = words.next().unwrap();
        let present = words.next().unwrap() != 0;
        let mass = f32::from_bits(words.next().unwrap());
        let motion_type = words.next().unwrap() as u8;
        let motion_properties_id = words.next().unwrap() as u16;
        let compressed: Vec<u8> = words.by_ref().take(8).flat_map(u32::to_le_bytes).collect();
        let frame: [f32; 14] = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
        let calculated = PrincipalMassProperties {
            volume: frame[0],
            mass: frame[1],
            center: frame[2..6].try_into().unwrap(),
            inertia: frame[6..10].try_into().unwrap(),
            orientation: frame[10..14].try_into().unwrap(),
        };
        let distribution = MassDistribution {
            center_of_mass_and_volume: std::array::from_fn(|_| {
                f32::from_bits(words.next().unwrap())
            }),
            major_axis_space: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
            inertia_tensor: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
        };
        let body = BodyInfo {
            mass,
            motion_type,
            motion_properties_id,
            mass_distribution: usize::from(present),
            position: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
            orientation: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
            linear_velocity: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
            angular_velocity: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
            ..BodyInfo::default()
        };
        let before: Vec<_> = words.by_ref().take(24).collect();
        assert_eq!(
            snapshot(MotionInfo::default()),
            before,
            "case {case} constructor"
        );
        let actual = MotionInfo::from_body(
            &body,
            if cached_kind == 1 {
                Some(crate::assets::physworld::mass::decode_principal(&compressed).unwrap())
            } else {
                None
            },
            present.then_some(distribution),
            |_| Ok(calculated),
        )
        .unwrap();
        assert_eq!(
            snapshot(actual),
            words.by_ref().take(24).collect::<Vec<_>>(),
            "case {case} body preparation"
        );
        assert!(words.next().is_none());
    }
}
