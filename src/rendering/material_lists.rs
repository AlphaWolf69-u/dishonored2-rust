//! World material-list selection after program, visibility and instance admission.
//! World surfaces use 339160's exact-value key; models use 3386a0's intervals.

/// Destinations identified by 33bf30's worldSort context and list diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldDrawList {
    Blend,
    Decal,
    Opaque,
    /// Zero-based index into the caller's dark-vision layer lists.
    DarkVision(u8),
    Water,
    BoatVolume,
    SkyClouds,
    OpaqueWind,
    InteriorVolume,
}

/// The finalized fields read by 339160. Shader bank admission and ordinary
/// visibility remain with the source owner; these fields do not imply either.
#[derive(Clone, Copy, Debug)]
pub struct WorldMaterial {
    pub flags_48: u8,
    pub flags_4a: u8,
    pub flags_4b: u8,
    pub main_blend_state: u32,
    pub stage_sort: [f32; 2],
    /// Material+140, base-bank slot20, independent of ordinary MAIN presence.
    pub darkvision_program_present: bool,
}

/// 339160's switch kernel. The input key is composed by world_draw_list below.
/// A raw key selecting a layer with layer==0 is outside the source contract;
/// return None rather than reproducing its unchecked negative pointer index.
pub fn select_world_draw_list(
    key: u32,
    decal_front: bool,
    darkvision_layer: u8,
) -> Option<WorldDrawList> {
    Some(match key {
        0 => WorldDrawList::Blend,
        1 if !decal_front => WorldDrawList::Blend,
        1 | 3 => WorldDrawList::Decal,
        4..=7 => WorldDrawList::DarkVision(darkvision_layer.checked_sub(1)?),
        8 | 10 => WorldDrawList::Water,
        0x12 => WorldDrawList::BoatVolume,
        0x20 | 0x22 => WorldDrawList::SkyClouds,
        0x42 => WorldDrawList::OpaqueWind,
        0x80 | 0x82 => WorldDrawList::InteriorVolume,
        _ => WorldDrawList::Opaque,
    })
}

/// 339160's material key and dark-vision admission. `darkvision_mask` is the
/// caller's view mask OR the renderer's global mask; neither is invented here.
/// Threshold indices follow 351210: boat0, water5. stageSortAdjust participates
/// in both ordered equality tests; this is not the model interval predicate.
pub fn world_draw_list(
    material: WorldMaterial,
    thresholds: [f32; 10],
    darkvision_mask: u32,
) -> Option<WorldDrawList> {
    let layer = material.flags_4a >> 5;
    let implicit_layer = layer != 0 && !material.darkvision_program_present;
    if implicit_layer && darkvision_mask.wrapping_mul(2) >> layer & 1 == 0 {
        return None;
    }
    let is_decal = material.flags_48 & 0x10 != 0;
    let decal_type = material.flags_4a >> 2 & 3;
    let decal_front = is_decal && matches!(decal_type, 0 | 3);
    let sort = material.stage_sort[1] + material.stage_sort[0];
    let key = u32::from(is_decal)
        + u32::from(material.flags_48 & 0x80)
        + u32::from(material.flags_48 & 0x40)
        + (u32::from(material.flags_4b & 3 != 0) << 5)
        + (u32::from(sort == thresholds[0]) << 4)
        + (u32::from(sort == thresholds[5]) << 3)
        + (u32::from(implicit_layer) << 2)
        + (u32::from(material.main_blend_state & 1 == 0) << 1);
    select_world_draw_list(key, decal_front, layer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_list_switch_matches_executed_selector() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/world_draw_lists.txt").lines() {
            let row: Vec<_> = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect();
            assert_eq!(row.len(), 4);
            let actual = match select_world_draw_list(row[0], row[1] != 0, row[2] as u8).unwrap() {
                WorldDrawList::Blend => 10,
                WorldDrawList::Decal => 12,
                WorldDrawList::Opaque => 14,
                WorldDrawList::DarkVision(layer) => 0x100 + u32::from(layer),
                WorldDrawList::Water => 18,
                WorldDrawList::BoatVolume => 20,
                WorldDrawList::SkyClouds => 22,
                WorldDrawList::OpaqueWind => 24,
                WorldDrawList::InteriorVolume => 26,
            };
            assert_eq!(actual, row[3], "case {count}: {row:08x?}");
            count += 1;
        }
        assert_eq!(count, 2048);
        assert_eq!(select_world_draw_list(4, false, 0), None);
    }

    #[test]
    fn world_material_keys_keep_exact_sorts_and_layer_admission() {
        let thresholds = [80., 100., 200., 250., 400., 300., 800., 1050., 1200., 1e30];
        let mut material = WorldMaterial {
            flags_48: 0,
            flags_4a: 0,
            flags_4b: 0,
            main_blend_state: 0,
            stage_sort: [80., 0.],
            darkvision_program_present: false,
        };
        assert_eq!(
            world_draw_list(material, thresholds, 0),
            Some(WorldDrawList::BoatVolume)
        );
        material.stage_sort[1] = 0.5;
        assert_eq!(
            world_draw_list(material, thresholds, 0),
            Some(WorldDrawList::Opaque)
        );
        assert_eq!(
            crate::material::draw_bucket(0, 0, 0, false, material.stage_sort, thresholds),
            0x26
        );
        material.stage_sort = [299., 1.];
        assert_eq!(
            world_draw_list(material, thresholds, 0),
            Some(WorldDrawList::Water)
        );
        material.stage_sort = [0., 0.];
        material.flags_4a = 1 << 5;
        assert_eq!(world_draw_list(material, thresholds, 0), None);
        assert_eq!(
            world_draw_list(material, thresholds, 1),
            Some(WorldDrawList::DarkVision(0))
        );
        material.darkvision_program_present = true;
        assert_eq!(
            world_draw_list(material, thresholds, 0),
            Some(WorldDrawList::Opaque)
        );
        material.flags_4a = 0;
        material.flags_48 = 0x80;
        assert_eq!(
            world_draw_list(material, thresholds, 0),
            Some(WorldDrawList::InteriorVolume)
        );
        material.flags_48 = 0x40;
        assert_eq!(
            world_draw_list(material, thresholds, 0),
            Some(WorldDrawList::OpaqueWind)
        );
        material.flags_48 = 0;
        material.flags_4b = 3;
        assert_eq!(
            world_draw_list(material, thresholds, 0),
            Some(WorldDrawList::SkyClouds)
        );
        material.flags_4b = 0;
        material.flags_48 = 0x10;
        material.main_blend_state = 1;
        assert_eq!(
            world_draw_list(material, thresholds, 0),
            Some(WorldDrawList::Decal)
        );
        material.flags_4a = 1 << 2;
        assert_eq!(
            world_draw_list(material, thresholds, 0),
            Some(WorldDrawList::Blend)
        );
    }
}
