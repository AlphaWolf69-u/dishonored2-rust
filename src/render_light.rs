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

fn write_word(record: &mut [u8], offset: usize, value: u32) {
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

/// FUN_1402f8670's 112-byte lighting-record fields. This numeric leaf leaves
/// position, shadow index and untouched tail words to its caller. The shared
/// `prepare_tlf_light_with_shadows` path below supplies position, admission,
/// index assignment and 144-byte shadow-record publication. Return the minimum
/// shadow count and area-image association; atlas placement happens later.
pub fn fill_tlf_lighting_packet(
    light: &[u8; 0xe8],
    shadow_count: i32,
    packet: &mut [u32; 28],
) -> (u32, Option<u64>) {
    let word = |offset| u32::from_le_bytes(light[offset..offset + 4].try_into().unwrap());
    let value = |offset| f32::from_bits(word(offset));
    let kind = word(0x5c) as i32;
    for (dst, src) in [
        (3, 0x7c),
        (7, 0x80),
        (4, 0x34),
        (5, 0x38),
        (6, 0x3c),
        (12, 0x98),
        (13, 0x9c),
        (14, 0xa4),
        (15, 0xac),
    ] {
        packet[dst] = word(src);
    }
    packet[11] = native_unit_clamp(value(0x88)).to_bits();
    packet[19] = native_unit_clamp(value(0xb0)).to_bits();
    let flags = ((kind as u32) << 4 & 0x70)
        | u32::from(light[0] >> 7)
        | (u32::from(light[1]) << 3 & 8)
        | (u32::from(light[0] >> 4) & 2)
        | (u32::from(light[1]) << 1 & 4);
    packet[25] = (packet[25] & !0x7f) | flags;
    let normalize = || {
        let v = [value(4), value(8), value(12)];
        let length_squared = (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2];
        let squared = if length_squared > f32::MIN_POSITIVE {
            length_squared
        } else {
            f32::MIN_POSITIVE
        };
        let scale = 1.0 / squared.sqrt();
        v.map(|component| (component * scale).to_bits())
    };
    let copy_second_axis = |packet: &mut [u32; 28]| {
        for (dst, src) in [(20, 0x10), (21, 0x14), (22, 0x18)] {
            packet[dst] = word(src);
        }
    };
    match kind {
        0 => {
            packet[16..19].fill(0);
            if light[1] & 2 != 0 {
                packet[23] = word(0xa8);
                packet[8..11].copy_from_slice(&normalize());
                copy_second_axis(packet);
            }
            (6, None)
        }
        1 => {
            packet[16] = (1.0 / f32::from_bits(word(0x8c) ^ 0x8000_0000)).to_bits();
            packet[17] = 1.0_f32.to_bits();
            packet[8..11].copy_from_slice(&normalize());
            (6, None)
        }
        2 => {
            let inner = value(0x8c);
            let limit = inner - f32::from_bits(0x3a83_126f);
            let outer = if limit <= value(0x90) {
                limit
            } else {
                value(0x90)
            };
            for (dst, src) in [(8, 4), (9, 8), (10, 12)] {
                packet[dst] = word(src) ^ 0x8000_0000;
            }
            packet[16] = (1.0 / (outer - inner)).to_bits();
            packet[17] = (inner / (outer - inner)).to_bits() ^ 0x8000_0000;
            packet[18] = ((1.0 - outer * outer).sqrt() / outer).to_bits();
            packet[25] = (packet[25] & !0x80) | (u32::from(shadow_count > 1) << 7);
            packet[23] = word(0xa0);
            copy_second_axis(packet);
            (1, None)
        }
        3 => {
            packet[8..11].copy_from_slice(&normalize());
            packet[13] = word(0xc8) ^ 0x8000_0000;
            let mut x = value(0xcc);
            let mut y = value(0xd0);
            let squared = x * x + y * y;
            if squared.sqrt() > 1.0 {
                let squared = if squared > f32::MIN_POSITIVE {
                    squared
                } else {
                    f32::MIN_POSITIVE
                };
                let scale = 1.0 / squared.sqrt();
                x *= scale;
                y *= scale;
            }
            let q = (1.0 - x * x) - y * y;
            let q = if q >= 0.0 { q } else { 0.0 };
            let scale = 1000.0
                - value(0xc8).powf(f32::from_bits(0x3c23_d70a)) * f32::from_bits(0x4479_ff5c);
            packet[16] = (scale * x).to_bits();
            packet[17] = (scale * y).to_bits();
            packet[18] = (q.sqrt() * scale).to_bits();
            copy_second_axis(packet);
            packet[11] = word(0xc0);
            packet[23] = word(0xc4);
            packet[25] |= 0xff00;
            let image = u64::from_le_bytes(light[0xb8..0xc0].try_into().unwrap());
            (1, (image != 0).then_some(image))
        }
        _ => (0, None),
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

/// FUN_1402f7260, admitted side publication: each 0x100-byte side contributes
/// its two matrices at +0x20 and its low five flags at +4 to a 0x90-byte record.
/// The remaining flag bits and the three tail words belong to the buffer owner.
pub fn fill_tlf_shadow_packet(side: &[u8; 0x100], packet: &mut [u32; 36]) {
    for (word, source) in packet[..32]
        .iter_mut()
        .zip(side[0x20..0xa0].chunks_exact(4))
    {
        *word = u32::from_le_bytes(source.try_into().unwrap());
    }
    let flags = u32::from_le_bytes(side[4..8].try_into().unwrap());
    packet[32] = (packet[32] & !0x1f) | (flags & 0x1f);
}

/// Shared native admission and side publication from FUN_1402f7260. This is
/// the implemented lighting-to-shadow index ABI: lighting word 24 names the
/// first consecutive 144-byte side record, or u32::MAX when rejected. Rejected
/// lights leave shadow records untouched; lighting retains gathered-light order.
/// The caller must construct/cache the 0x100-byte sides and draw their atlas
/// regions before exposing these records to the live material shader.
pub fn prepare_tlf_light_with_shadows(
    light: &[u8; 0xe8],
    sides: &[[u8; 0x100]],
    shadows_disabled: bool,
    used_shadows: &mut u32,
    lighting: &mut [u32; 28],
    shadows: &mut [[u32; 36]],
) -> Result<(u32, Option<u64>), String> {
    if sides.len() > 0x00ff_ffff || shadows.len() < 0x100 || *used_shadows > 0x100 {
        return Err("native TLF shadow side/count buffer is invalid".into());
    }
    let first = *used_shadows as usize;
    let result = prepare_tlf_lighting_packet(
        light,
        sides.len() as u32,
        shadows_disabled,
        used_shadows,
        lighting,
    );
    if lighting[24] != u32::MAX {
        for (side, packet) in sides
            .iter()
            .zip(&mut shadows[first..*used_shadows as usize])
        {
            fill_tlf_shadow_packet(side, packet);
        }
    }
    Ok(result)
}

/// FUN_1402f7260's caller-owned position and shadow-range writes around the
/// numeric writer. Shadow payload construction and area atlas ownership remain
/// external; assigning an index does not construct those payloads.
pub fn prepare_tlf_lighting_packet(
    light: &[u8; 0xe8],
    shadow_header: u32,
    shadows_disabled: bool,
    used_shadows: &mut u32,
    packet: &mut [u32; 28],
) -> (u32, Option<u64>) {
    let available = shadow_header & 0x00ff_ffff;
    let result = fill_tlf_lighting_packet(light, available as i32, packet);
    for (axis, word) in packet[..3].iter_mut().enumerate() {
        let offset = 0x28 + axis * 4;
        *word = u32::from_le_bytes(light[offset..offset + 4].try_into().unwrap());
    }
    let end = used_shadows.wrapping_add(available);
    if light[0] & 2 != 0 && available >= result.0 && end <= 0x100 && !shadows_disabled {
        packet[24] = *used_shadows;
        *used_shadows = end;
    } else {
        packet[24] = u32::MAX;
    }
    result
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
    fn tlf_caller_copies_position_and_admits_only_native_shadow_ranges() {
        // 1402f76bc..1402f76ef: source bit2, minimum count, 256-entry end,
        // and view override; original position bytes at +28/+2c/+30.
        for (flags, header, start, disabled, expected_index, expected_end) in [
            (1, 6, 0, false, u32::MAX, 0),
            (3, 5, 0, false, u32::MAX, 0),
            (3, 6, 250, false, 250, 256),
            (3, 6, 251, false, u32::MAX, 251),
            (3, 6, 0, true, u32::MAX, 0),
            (3, 0xfe00_0006, 0, false, 0, 6),
        ] {
            let mut light = [0; 0xe8];
            light[0] = flags;
            for (axis, word) in [0x8000_0000_u32, 0x7fc0_1234, 0x3f80_0000]
                .into_iter()
                .enumerate()
            {
                write_word(&mut light, 0x28 + axis * 4, word);
            }
            let mut packet = [0xa5a5_1234; 28];
            let mut used = start;
            assert_eq!(
                prepare_tlf_lighting_packet(&light, header, disabled, &mut used, &mut packet),
                (6, None)
            );
            assert_eq!(&packet[..3], &[0x8000_0000, 0x7fc0_1234, 0x3f80_0000]);
            assert_eq!(packet[24], expected_index);
            assert_eq!(used, expected_end);
            assert_eq!(&packet[26..], &[0xa5a5_1234; 2]);
        }
    }

    #[test]
    fn tlf_lighting_records_match_original_machine_code() {
        let mut cases = 0;
        for (case, line) in include_str!("../analysis/original_tlf_lighting_packet_cases.txt")
            .lines()
            .enumerate()
        {
            let words = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 116);
            let light: [u8; 0xe8] = words[2..60]
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect::<Vec<_>>()
                .try_into()
                .unwrap();
            let mut packet: [u32; 28] = words[60..88].try_into().unwrap();
            let (minimum, image) = fill_tlf_lighting_packet(&light, words[0] as i32, &mut packet);
            assert_eq!(minimum, words[1], "minimum shadows case {case}");
            assert_eq!(image, None);
            for (field, (actual, expected)) in packet.iter().zip(&words[88..]).enumerate() {
                assert_eq!(
                    actual,
                    expected,
                    "lighting case {case} raw kind {} field +{:#x}",
                    words[2 + 0x5c / 4] as i32,
                    field * 4
                );
            }
            cases += 1;
        }
        assert_eq!(cases, 1024);
    }

    #[test]
    fn shadow_side_publication_preserves_owner_bits_and_padding() {
        for flags in 0..64 {
            let mut side = [0; 0x100];
            write_word(&mut side, 4, flags | 0xfedc_0000);
            for index in 0..32 {
                write_word(
                    &mut side,
                    0x20 + index * 4,
                    0x8000_0000_u32.wrapping_add(index as u32 * 0x0101_0101),
                );
            }
            let mut packet = [0xa5a5_5678; 36];
            fill_tlf_shadow_packet(&side, &mut packet);
            for index in 0..32 {
                assert_eq!(
                    packet[index],
                    0x8000_0000_u32.wrapping_add(index as u32 * 0x0101_0101)
                );
            }
            assert_eq!(packet[32], (0xa5a5_5678 & !31) | (flags & 31));
            assert_eq!(&packet[33..], &[0xa5a5_5678; 3]);
        }
    }

    #[test]
    fn admitted_shadow_sides_are_published_once_and_rejections_are_unchanged() {
        for (kind, count, first, disabled, expected) in [
            (0, 6, 250, false, Some(256)),
            (1, 6, 250, false, Some(256)),
            (0, 5, 0, false, None),
            (0, 6, 251, false, None),
            (2, 2, 1, false, Some(3)),
            (3, 1, 255, false, Some(256)),
            (2, 1, 0, true, None),
        ] {
            let mut light = [0; 0xe8];
            light[0] = 3;
            write_word(&mut light, 0x5c, kind);
            let sides = (0..count)
                .map(|index| {
                    let mut side = [0; 0x100];
                    write_word(&mut side, 4, index as u32);
                    write_word(&mut side, 0x20, 0x1234_0000 + index as u32);
                    side
                })
                .collect::<Vec<_>>();
            let mut shadows = [[0xdead_beef; 36]; 256];
            let mut lighting = [0xa5a5_1234; 28];
            let mut used = first;
            prepare_tlf_light_with_shadows(
                &light,
                &sides,
                disabled,
                &mut used,
                &mut lighting,
                &mut shadows,
            )
            .unwrap();
            assert_eq!(used, expected.unwrap_or(first));
            assert_eq!(lighting[24], expected.map_or(u32::MAX, |_| first));
            for (index, packet) in shadows.iter().enumerate() {
                if expected.is_some() && (first as usize..used as usize).contains(&index) {
                    assert_eq!(packet[0], 0x1234_0000 + index as u32 - first);
                    assert_eq!(packet[32], (0xdead_beef & !31) | (index as u32 - first));
                    assert_eq!(&packet[33..], &[0xdead_beef; 3]);
                } else {
                    assert_eq!(*packet, [0xdead_beef; 36]);
                }
            }
        }
    }

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
