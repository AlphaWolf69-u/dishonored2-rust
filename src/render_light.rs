//! Source-backed base-light initialization and energy/range conversion.

#[derive(Clone, Copy)]
pub struct PresetInput {
    pub kind: u32,
    pub preset: u32,
    pub reflector: u32,
    pub area_energy_conservative: bool,
    pub cutoff: f32,
    pub falloff: f32,
    pub cutoff_cosine: f32,
    pub back_reflection: f32,
    pub area_size: [f32; 2],
    pub color: [f32; 3],
}

#[derive(Debug)]
pub struct PresetOutput {
    pub color: [f32; 3],
    pub range: f32,
    pub falloff: f32,
    pub normalized_intensity: f32,
    pub center_offset: [f32; 3],
}

/// Enum literals from FUN_141a33590; absent input uses idLight's constructor.
pub fn preset_index(name: Option<&str>) -> Option<u32> {
    const NAMES: [&str; 14] = [
        "LIGHT_PRESET_CANDLE",
        "LIGHT_PRESET_CANDLE_STICK6",
        "LIGHT_PRESET_CANDLE_STICK12",
        "LIGHT_PRESET_10W",
        "LIGHT_PRESET_40W",
        "LIGHT_PRESET_60W",
        "LIGHT_PRESET_100W",
        "LIGHT_PRESET_150W",
        "LIGHT_PRESET_1000W",
        "LIGHT_PRESET_RATLIGHT",
        "LIGHT_PRESET_FILL_IN",
        "LIGHT_PRESET_REFLECTOR",
        "LIGHT_PRESET_MOON",
        "LIGHT_PRESET_SUN",
    ];
    name.map_or(Some(6), |name| {
        NAMES
            .iter()
            .position(|&value| value == name)
            .map(|index| index as u32)
    })
}

/// FUN_141a38910; retain the original DAYLIHGT spelling. Constructor +0x368=0.
pub fn reflector_index(name: Option<&str>) -> Option<u32> {
    const NAMES: [&str; 6] = [
        "REFLECTOR_CLEAR_DAYLIGHT_SKY",
        "REFLECTOR_OVERCAST_DAYLIGHT_SKY",
        "REFLECTOR_CLEAR_EVENING_SKY",
        "REFLECTOR_CLEAR_NIGHT_SKY",
        "REFLECTOR_CLEAR_DAYLIHGT_SKY_OFF_50PCT_WALL",
        "REFLECTOR_SUN_OFF_50PCT_WALL",
    ];
    name.map_or(Some(0), |name| {
        NAMES
            .iter()
            .position(|&value| value == name)
            .map(|index| index as u32)
    })
}

/// CRT initializer at 0x141363b20, not the zero-filled on-disk BSS value.
pub fn normalization() -> f32 {
    f32::from_bits(0x3da2_f983) * 1750.0
}

fn write_word(record: &mut [u8; 0xe8], offset: usize, value: u32) {
    record[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_float(record: &[u8; 0xe8], offset: usize) -> f32 {
    f32::from_le_bytes(record[offset..offset + 4].try_into().unwrap())
}

/// FUN_140359b90's four stores reduce to two duplicated fields. SSE
/// MAXSS/MINSS select their second operand on equal or unordered inputs.
pub fn distance_fields(cutoff: f32, falloff: f32) -> (f32, f32) {
    let range = if 0.1 > cutoff { 0.1 } else { cutoff };
    let limit = range * 0.99;
    let falloff = if falloff < limit { falloff } else { limit };
    (range, falloff)
}

/// FUN_140232bc0, after projection bounds have been generated. MAXSS selects
/// its second operand for equal/unordered inputs; keep the six-axis order.
pub fn proximity_from_bounds(range: f32, bounds: [[f32; 3]; 2]) -> ([f32; 3], f32) {
    if !(range > 0.0) {
        return ([0.0; 3], 32_768.0);
    }
    let center = std::array::from_fn(|axis| (bounds[1][axis] + bounds[0][axis]) * 0.5);
    let mut radius = 0.0;
    for side in bounds {
        for axis in 0..3 {
            let extent = side[axis] - center[axis];
            radius = if radius > extent { radius } else { extent };
        }
    }
    (center, radius + range)
}

/// FUN_1403594a0 preserves unspecified bytes, including the upper flag bits.
fn initialize_base(record: &mut [u8; 0xe8]) {
    record[0] = 0x13;
    record[1] &= 0xf8;
    for offset in [
        4, 0x14, 0x24, 0x34, 0x38, 0x3c, 0x40, 0x44, 0x48, 0x88, 0x98, 0xa4, 0xa8, 0xc0, 0xc4,
    ] {
        write_word(record, offset, 1.0f32.to_bits());
    }
    for offset in [
        8, 0xc, 0x10, 0x18, 0x1c, 0x20, 0x28, 0x2c, 0x30, 0x4c, 0x50, 0x54, 0x64, 0x6c, 0x94, 0x9c,
        0xa0, 0xac, 0xb0, 0xb8, 0xbc, 0xc8, 0xcc, 0xd0, 0xd4, 0xd8,
    ] {
        write_word(record, offset, 0);
    }
    for (offset, value) in [
        (0x58, u32::MAX),
        (0x5c, 5),
        (0x60, 3),
        (0x70, 0x3f33_3333),
        (0x74, 0x4120_0000),
        (0x78, 0x4170_0000),
        (0x7c, 0x4120_0000),
        (0x80, 0x4170_0000),
        (0x84, 0x3dcc_cccd),
        (0x8c, 0x3f19_999a),
        (0x90, 0x3f00_0000),
        (0xdc, 0x3f00_0000),
        (0xe0, 0x3e4c_cccd),
        (0xe4, 0x4220_0000),
    ] {
        write_word(record, offset, value);
    }
}

fn native_unit_clamp(value: f32) -> f32 {
    // COMISS/JBE preserves NaNs and signed zero, unlike f32::clamp.
    if value < 0.0 {
        0.0
    } else if value > 1.0 {
        1.0
    } else {
        value
    }
}

/// FUN_140001050 zeros endpoints with magnitude <= the installed threshold.
pub fn interpolate(a: f32, b: f32, fraction: f32) -> f32 {
    let flush = |value: f32| {
        if value.abs() <= f32::from_bits(0x2193_92ef) {
            0.0
        } else {
            value
        }
    };
    (1.0 - fraction) * flush(a) + flush(b) * fraction
}

/// FUN_1403595f0, all 14 preset branches and the shared shape/range tail.
pub fn convert(input: PresetInput) -> Option<PresetOutput> {
    if input.preset >= 14 || input.kind > 5 || input.reflector > 5 {
        return None;
    }
    let mut record = [0; 0xe8];
    initialize_base(&mut record);
    write_word(&mut record, 0x78, input.cutoff.to_bits());
    write_word(&mut record, 0x80, input.cutoff.to_bits());
    write_word(&mut record, 0x74, input.falloff.to_bits());
    write_word(&mut record, 0x7c, input.falloff.to_bits());
    let n = normalization();
    let mut energy = match input.preset {
        0 | 12 => 1.0,
        1 => 6.0,
        2 => 12.0,
        3 => n * 10.0 / 100.0,
        4 => n * 40.0 / 100.0,
        5 => n * 60.0 / 100.0,
        6 | 10 => n,
        7 => n * 150.0 / 100.0,
        8 => n * 1000.0 / 100.0,
        9 => 20000.0,
        11 => match input.reflector {
            0 => 11084.0,
            1 => f32::from_bits(0x448a_8ccd),
            2 => 5642.0,
            3 => f32::from_bits(0x3aa4_a05e),
            4 => 1.0,
            5 => f32::from_bits(0x47af_a9dd) * 0.25,
            _ => unreachable!(),
        },
        13 => f32::from_bits(0x47af_a9dd),
        _ => unreachable!(),
    };
    let back = native_unit_clamp(input.back_reflection);
    if input.kind == 2 && back > 0.0 {
        let full_sphere = f32::from_bits(0x4149_0fdb);
        let cone = (1.0 - input.cutoff_cosine) * f32::from_bits(0x40c9_0fdb);
        let blended = interpolate(full_sphere, cone, back);
        let minimum = f32::from_bits(0x3c23_d70a);
        let divisor = if minimum > blended { minimum } else { blended };
        energy *= full_sphere / divisor;
    }
    if input.kind == 3 && input.preset == 13 {
        energy *= f32::from_bits(0x3f63_b542);
    } else {
        if input.kind == 3 && input.area_energy_conservative && input.preset != 11 {
            energy /= (input.area_size[1] * input.area_size[0]) * 4.0;
        }
        if !(11..=13).contains(&input.preset) {
            energy *= 5.0;
        }
    }
    let intensity = energy / n;
    let color = input
        .color
        .map(|value| native_unit_clamp(value) * intensity);
    let radius = (energy * 2.0).sqrt();
    if input.kind != 3 && ![10, 12, 13].contains(&input.preset) && radius < input.cutoff {
        write_word(&mut record, 0x80, radius.to_bits());
        write_word(
            &mut record,
            0x7c,
            ((radius * input.falloff) / input.cutoff).to_bits(),
        );
    }
    Some(PresetOutput {
        color,
        range: read_float(&record, 0x80),
        falloff: read_float(&record, 0x7c),
        normalized_intensity: intensity,
        center_offset: std::array::from_fn(|axis| read_float(&record, 0x4c + axis * 4)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflected_enum_names_and_constructor_defaults() {
        assert_eq!(preset_index(None), Some(6));
        assert_eq!(preset_index(Some("LIGHT_PRESET_REFLECTOR")), Some(11));
        assert_eq!(preset_index(Some("LIGHT_PRESET_SUN")), Some(13));
        assert_eq!(preset_index(Some("light_preset_100w")), None);
        assert_eq!(reflector_index(None), Some(0));
        for line in include_str!("../analysis/original_light_preset_cases.reflectors.txt").lines() {
            let (index, name) = line.split_once(' ').unwrap();
            assert_eq!(reflector_index(Some(name)), Some(index.parse().unwrap()));
        }
        assert_eq!(
            reflector_index(Some("REFLECTOR_CLEAR_DAYLIGHT_SKY_OFF_50PCT_WALL")),
            None
        );
    }

    #[test]
    fn base_constructor_matches_installed_instructions() {
        let mut count = 0;
        for line in include_str!("../analysis/original_light_preset_cases.constructor.txt").lines()
        {
            let (seed, expected) = line.split_once(' ').unwrap();
            let seed: usize = seed.parse().unwrap();
            let mut record = std::array::from_fn(|index| (seed * 29 + index * 17) as u8);
            initialize_base(&mut record);
            let actual = record
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            assert_eq!(actual, expected, "seed {seed}");
            count += 1;
        }
        assert_eq!(count, 32);
    }

    #[test]
    fn distance_fields_match_installed_instructions() {
        let mut count = 0;
        for line in include_str!("../analysis/original_light_preset_cases.distances.txt").lines() {
            let v = line
                .split_whitespace()
                .map(|v| v.parse::<u32>().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(v.len(), 7);
            let (range, falloff) = distance_fields(f32::from_bits(v[0]), f32::from_bits(v[1]));
            assert_eq!(
                [range, range, falloff, falloff].map(f32::to_bits),
                v[2..6],
                "{line}"
            );
            // The original returns its unchanged cutoff argument, unused by the caller.
            assert_eq!(v[0], v[6]);
            count += 1;
        }
        assert_eq!(count, 225);
    }

    #[test]
    fn initial_preset_center_offsets_match_installed_instructions() {
        let mut count = 0;
        for line in include_str!("../analysis/original_light_preset_cases.offsets.txt").lines() {
            let v = line
                .split_whitespace()
                .map(|v| v.parse::<u32>().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(v.len(), 16);
            let output = convert(PresetInput {
                kind: v[0],
                preset: v[1],
                reflector: v[2],
                area_energy_conservative: v[3] != 0,
                cutoff: f32::from_bits(v[4]),
                falloff: f32::from_bits(v[5]),
                cutoff_cosine: f32::from_bits(v[6]),
                back_reflection: f32::from_bits(v[7]),
                area_size: [v[8], v[9]].map(f32::from_bits),
                color: [v[10], v[11], v[12]].map(f32::from_bits),
            })
            .unwrap();
            assert_eq!(output.center_offset.map(f32::to_bits), v[13..16], "{line}");
            count += 1;
        }
        assert_eq!(count, 1920);
    }

    #[test]
    fn proximity_matches_installed_instructions() {
        let mut count = 0;
        for line in include_str!("../analysis/original_light_proximity_cases.txt").lines() {
            let v = line
                .split_whitespace()
                .map(|value| value.parse::<u32>().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(v.len(), 11);
            let (center, radius) = proximity_from_bounds(
                f32::from_bits(v[0]),
                [v[1..4].try_into().unwrap(), v[4..7].try_into().unwrap()]
                    .map(|side: [u32; 3]| side.map(f32::from_bits)),
            );
            assert_eq!(
                [center[0], center[1], center[2], radius].map(f32::to_bits),
                v[7..11],
                "{line}"
            );
            count += 1;
        }
        assert_eq!(count, 483);
    }

    #[test]
    fn interpolation_matches_installed_instructions() {
        let mut count = 0;
        for line in include_str!("../analysis/original_light_preset_cases.lerp.txt").lines() {
            let values = line
                .split_whitespace()
                .map(|word| word.parse::<u32>().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(values.len(), 4);
            assert_eq!(
                interpolate(
                    f32::from_bits(values[0]),
                    f32::from_bits(values[1]),
                    f32::from_bits(values[2])
                )
                .to_bits(),
                values[3],
                "{line}"
            );
            count += 1;
        }
        assert_eq!(count, 320);
    }

    #[test]
    fn presets_match_installed_instructions() {
        let mut count = 0;
        for line in include_str!("../analysis/original_light_preset_cases.txt").lines() {
            let v = line
                .split_whitespace()
                .map(|word| word.parse::<u32>().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(v.len(), 19);
            let output = convert(PresetInput {
                kind: v[0],
                preset: v[1],
                reflector: v[2],
                area_energy_conservative: v[3] != 0,
                cutoff: f32::from_bits(v[4]),
                falloff: f32::from_bits(v[5]),
                cutoff_cosine: f32::from_bits(v[6]),
                back_reflection: f32::from_bits(v[7]),
                area_size: [v[8], v[9]].map(f32::from_bits),
                color: [v[10], v[11], v[12]].map(f32::from_bits),
            })
            .unwrap();
            let actual = [
                output.color[0],
                output.color[1],
                output.color[2],
                output.range,
                output.falloff,
                output.normalized_intensity,
            ]
            .map(f32::to_bits);
            assert_eq!(actual, v[13..19], "{line}");
            count += 1;
        }
        assert_eq!(count, 1920);
        assert_eq!(normalization().to_bits(), 1124811445);
    }
}
