//! Original render-model flag and finite query helpers.
use crate::tome::NoDepthAabbQuery;

/// FUN_140872a50, 140872b1a: the qword store at +0xa8 also zeros
/// maxVisibleRange at +0xac. This is the constructor default, not a cutoff.
pub const INITIAL_MODEL_MAX_VISIBLE_RANGE: f32 = 0.0;

/// FUN_1401095f0: authored axes are stored by basis, not by matrix row.
pub fn affine_rows(position: [f32; 3], basis: [[f32; 3]; 3], scale: [f32; 3]) -> [[f32; 4]; 4] {
    std::array::from_fn(|row| {
        if row == 3 { [0.0, 0.0, 0.0, 1.0] } else {
            [basis[0][row] * scale[0], basis[1][row] * scale[1], basis[2][row] * scale[2], position[row]]
        }
    })
}

/// FUN_140235c80 / FUN_1400fb8c0: scaled local-box center and absolute rotated extents.
pub fn transformed_bounds(local: [[f32; 3]; 2], position: [f32; 3], basis: [[f32; 3]; 3], scale: [f32; 3]) -> [[f32; 3]; 2] {
    let center = std::array::from_fn::<_, 3, _>(|axis| (local[1][axis] + local[0][axis]) * scale[axis] * 0.5);
    let extent = std::array::from_fn::<_, 3, _>(|axis| scale[axis] * local[1][axis] - center[axis]);
    let rotated_extent = std::array::from_fn::<_, 3, _>(|axis| (extent[1] * basis[1][axis]).abs() + (extent[0] * basis[0][axis]).abs() + (extent[2] * basis[2][axis]).abs());
    let world_center = std::array::from_fn::<_, 3, _>(|axis| center[1] * basis[1][axis] + center[0] * basis[0][axis] + center[2] * basis[2][axis] + position[axis]);
    [std::array::from_fn(|axis| world_center[axis] - rotated_extent[axis]), std::array::from_fn(|axis| rotated_extent[axis] + world_center[axis])]
}

/// FUN_140237340: optional range region; nonpositive range uses its constructor region.
pub fn range_region(bounds: [[f32; 3]; 2], range: f32) -> [f32; 4] {
    if !(range > 0.0) { return [0.0, 0.0, 0.0, 32768.0]; }
    let center = std::array::from_fn::<_, 3, _>(|axis| (bounds[1][axis] + bounds[0][axis]) * 0.5);
    let mut extent = 0.0;
    for side in bounds {
        for axis in 0..3 {
            let value = side[axis] - center[axis];
            if extent <= value { extent = value; }
        }
    }
    [center[0], center[1], center[2], extent + range]
}

pub use crate::bmodel::accumulate_resource_bounds;

/// FUN_140268730: a missing static resource uses the observed zero bounds.
pub fn static_resource_bounds(resource: Option<&[[f32; 3]; 2]>) -> &[[f32; 3]; 2] {
    const EMPTY: [[f32; 3]; 2] = [[0.0; 3]; 2];
    resource.unwrap_or(&EMPTY)
}

/// FUN_14025da70: return the owned resource's bounds without modifying them.
pub fn resource_bounds(bounds: &[[f32; 3]; 2]) -> &[[f32; 3]; 2] { bounds }

/// FUN_14031ad30's finite no-depth selection prefix. The owning model
/// supplies prepared flags and bounds; they are not inferred from shape names.
#[derive(Clone, Copy)]
pub struct ModelQueryInput {
    pub frame_flags: u8,
    pub shape_flags: u8,
    pub region: [f32; 4],
    pub bounds: [[f32; 3]; 2],
}

pub fn model_eligible(frame_flags: u8, shape_flags: u8) -> bool {
    frame_flags & 4 != 0 || (shape_flags & 0x10 != 0 && frame_flags & 1 == 0)
}

pub fn in_model_region(camera: [f32; 3], region: [f32; 4]) -> bool {
    let radius = region[3];
    !(radius < radius.abs() || (0..3).any(|axis| radius < (camera[axis] - region[axis]).abs()))
}

pub fn model_visible(
    query: &NoDepthAabbQuery,
    camera: [f32; 3],
    viewport: [u32; 2],
    model: ModelQueryInput,
) -> Result<bool, String> {
    if !model_eligible(model.frame_flags, model.shape_flags) {
        return Ok(false);
    }
    if viewport.contains(&0) || camera.iter().chain(model.region.iter()).any(|value| !value.is_finite()) {
        return Err("unsupported nonfinite model region or empty viewport".to_owned());
    }
    if !in_model_region(camera, model.region) {
        return Ok(false);
    }
    let (visible, area) = query.visibility_and_area(model.bounds)?;
    Ok(visible && (model.frame_flags & 0x40 != 0 || !(area < 25.0 / (viewport[0] as f32 * viewport[1] as f32))))
}

/// FUN_140399620: the base render-model virtual callback has no side effects.
pub fn base_callback() -> u8 { 0 }

/// FUN_140234ba0: an overlapping mask does not add its other bits.
pub fn request_flags(flags: &mut u32, mask: u32) {
    if *flags & mask == 0 {
        *flags |= mask | 1;
    }
}

/// FUN_140236550: the dirty bit is set before the supplied mask is cleared.
pub fn clear_flags(flags: &mut u32, mask: u32) {
    if *flags & mask != 0 {
        *flags = (*flags | 1) & !mask;
    }
}

/// FUN_14031d0f0: preserve the native multiplication order and NaN propagation.
pub fn distance_fade(start: f32, end: f32, value: f32) -> f32 {
    let t = ((value - start) / (end - start)).clamp(0.0, 1.0);
    (3.0 - t * 2.0) * (t * t)
}

/// FUN_1403013e0: its shared padding input is DAT_1432ef64c, not a fixed margin.
pub fn square_patch_bounds(
    patch: [f32; 3],
    height_center: f32,
    height_extent: f32,
    spacing: f32,
) -> [[f32; 3]; 2] {
    let padding = spacing * 0.25;
    [
        [patch[0] - padding, patch[1] - padding, height_center - height_extent],
        [(patch[2] + patch[0]) + padding, (patch[2] + patch[1]) + padding, height_extent + height_center],
    ]
}

pub fn square_patch_visible(
    query: &NoDepthAabbQuery,
    patch: [f32; 3],
    height_center: f32,
    height_extent: f32,
    spacing: f32,
) -> Result<bool, String> {
    query.visible(square_patch_bounds(patch, height_center, height_extent, spacing))
}

/// FUN_1402370e0's surface inputs; property handle identity is not guessed.
#[derive(Clone, Copy)]
pub struct SurfaceFlags {
    pub byte_48: u8,
    pub byte_4b: u8,
    pub byte_4c: u8,
    pub parameter_14229d460: f32,
}

/// FUN_140342640's global-default branch; later matching owner keys win.
/// Registry selection is FUN_140310dd0; [u32; 4] copies match FUN_1403104a0.
pub fn parameter_float(
    name: &str,
    id: u32,
    flags: u32,
    defaults: &[[u32; 4]],
    overrides: &[(u16, [u32; 4])],
) -> Result<f32, String> {
    if flags & 0xf != 0 && flags & 0x30 != 0x30 {
        return Err(format!("idParmBlock::GetFloat: {name} is not a vector parm"));
    }
    let mut value = *defaults.get(id as usize).ok_or("parameter registry index out of bounds")?;
    for &(key, replacement) in overrides {
        if u32::from(key) == id { value = replacement; }
    }
    Ok(f32::from_bits(value[0]))
}

/// FUN_140342360: stable paired-slot removal; capacity and model dirty state are unchanged.
pub fn remove_parameter(overrides: &mut Vec<(u16, [u32; 4])>, id: u32) {
    overrides.retain(|&(key, _)| u32::from(key) != id);
}

/// An owned, preallocated idParmBlock. Entries follow the ID count, which
/// need not equal the packed value count. Allocation/growth policies are
/// deliberately not substituted with a generic Vec growth rule.
#[derive(Debug)]
pub struct ParameterBlock {
    pub entries: Vec<(u16, [u32; 4])>,
    pub value_count_word: u32,
    pub id_count_word: u32,
    pub value_capacity_word: u32,
    pub id_capacity_word: u32,
}

impl ParameterBlock {
    fn validate_preallocated(&self) -> Result<(), String> {
        if self.entries.len() != (self.id_count_word & 0xffffff) as usize {
            return Err("parameter ID count does not match initialized slots".into());
        }
        if self.entries.len() > (self.value_capacity_word >> 8) as usize
            || self.entries.len() > (self.id_capacity_word >> 8) as usize {
            return Err("native parameter-array growth is not implemented".into());
        }
        Ok(())
    }

    fn set_compacted_counts(&mut self) {
        let count = self.entries.len() as u32;
        self.value_count_word = (self.value_count_word & 0xff000000) | count;
        self.id_count_word = (self.id_count_word & 0xff000000) | count;
    }

    /// FUN_140343670's no-growth branch: the last duplicate wins and retained
    /// slots keep source order. With no duplicates, both count words stay intact.
    pub fn deduplicate(&mut self) -> Result<(), String> {
        self.validate_preallocated()?;
        if self.entries.len() > 512 {
            return Err("native duplicate marker array is limited to 512 entries".into());
        }
        let mut removed = [false; 512];
        for index in (0..self.entries.len()).rev() {
            removed[index] = (index + 1..self.entries.len()).any(|later|
                !removed[later] && self.entries[later].0 == self.entries[index].0);
        }
        if removed[..self.entries.len()].contains(&true) {
            let mut index = 0;
            self.entries.retain(|_| {
                let keep = !removed[index];
                index += 1;
                keep
            });
            self.set_compacted_counts();
        }
        Ok(())
    }

    /// FUN_140344540's no-growth branch repairs both counts even when the
    /// whitelist removes nothing. Its whitelist count is at +800 decimal.
    pub fn filter(&mut self, whitelist: &[u32]) -> Result<(), String> {
        self.validate_preallocated()?;
        self.entries.retain(|&(key, _)| whitelist.contains(&u32::from(key)));
        self.set_compacted_counts();
        Ok(())
    }

    pub fn float(&self, name: &str, id: u32, flags: u32, defaults: &[[u32; 4]]) -> Result<f32, String> {
        self.validate_preallocated()?;
        parameter_float(name, id, flags, defaults, &self.entries)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialParameterGroup {
    Base,
    Effect(i8),
}

/// FUN_1402d4bc0: signed-byte selector; effect blocks live behind the pointer
/// at material+0x90, not inline at that address. Each native block is 0x20 bytes.
pub fn material_parameter_group(material_flags_49: u8, selector: i8) -> MaterialParameterGroup {
    if selector == -1 || material_flags_49 & 0x40 == 0 {
        MaterialParameterGroup::Base
    } else {
        MaterialParameterGroup::Effect(selector)
    }
}

pub struct MaterialParameterBlocks {
    pub base: ParameterBlock,
    pub effects: Vec<ParameterBlock>,
    pub flags_49: u8,
}

impl MaterialParameterBlocks {
    pub fn selected(&self, selector: i8) -> Result<&ParameterBlock, String> {
        match material_parameter_group(self.flags_49, selector) {
            MaterialParameterGroup::Base => Ok(&self.base),
            MaterialParameterGroup::Effect(index) => usize::try_from(index).ok()
                .and_then(|index| self.effects.get(index))
                .ok_or_else(|| "material effect selector outside owned block storage".into()),
        }
    }
}

/// FUN_1402d4be0 after the existing property getter's float return.
pub fn parameter_bit(value: f32) -> bool {
    value > f32::from_bits(0x3a83126f)
}

/// FUN_1402370e0: OR-reduction retains low flags and the other frame bits.
pub fn reduce_surface_flags(previous: [u8; 2], surfaces: &[SurfaceFlags]) -> [u8; 2] {
    let mut result = [previous[0] & 0x0f, previous[1] & 0xfe];
    for surface in surfaces {
        result[0] |= u8::from(surface.byte_4b & 0x0c != 4) << 4;
        result[0] |= (surface.byte_48 & 1) << 5;
        result[0] |= u8::from(parameter_bit(surface.parameter_14229d460)) << 6;
        result[0] |= (surface.byte_4c & 1) << 7;
        result[1] |= (surface.byte_4c >> 6) & 1;
    }
    result
}

/// FUN_14087c1f0's owned graph; missing render-model handles are skipped.
#[derive(Default)]
pub struct VisibilityNode {
    pub model_flags: Option<u32>,
    pub children: Vec<VisibilityNode>,
}

impl VisibilityNode {
    pub fn set_visible(&mut self, visible: bool) {
        if let Some(flags) = &mut self.model_flags {
            if visible {
                request_flags(flags, 2);
            } else {
                clear_flags(flags, 2);
            }
        }
        for child in &mut self.children {
            child.set_visible(visible);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_range_matches_original_qword_store() {
        let cases = include_str!("../analysis/original_model_info_default_cases.txt");
        let mut checked = 0;
        for line in cases.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            assert_eq!(fields.len(), 3);
            assert_eq!(u32::from_str_radix(fields[1], 16).unwrap(), 0x3fff0010);
            assert_eq!(u32::from_str_radix(fields[2], 16).unwrap(), INITIAL_MODEL_MAX_VISIBLE_RANGE.to_bits());
            checked += 1;
        }
        assert_eq!(checked, 4);
    }

    #[test]
    fn parameter_float_matches_original_global_defaults_and_overrides() {
        let mut checked = 0;
        for line in include_str!("../analysis/original_parm_float_cases.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let u = |i: usize| fields[i].parse::<u32>().unwrap();
            if fields[0] == "copy" {
                let source = [u(1), u(2), u(3), u(4)];
                let copied = source;
                assert_eq!(copied, source);
            } else {
                let mut defaults = [[0; 4]; 12];
                defaults[u(1) as usize][0] = u(3);
                let overrides: Vec<_> = (0..u(4) as usize).map(|i| (u(5 + i * 2) as u16, [u(6 + i * 2), 0, 0, 0])).collect();
                assert_eq!(parameter_float("emissiveintensity", u(1), u(2), &defaults, &overrides).unwrap().to_bits(), u(fields.len() - 1), "{line}");
            }
            checked += 1;
        }
        assert_eq!(checked, 305);
        assert_eq!(parameter_float("bad", 0, 1, &[[0; 4]], &[]).unwrap_err(), "idParmBlock::GetFloat: bad is not a vector parm");
        assert!(parameter_float("missing", 1, 0, &[[0; 4]], &[]).is_err());
    }

    #[test]
    fn parameter_removal_matches_original_paired_arrays() {
        let mut checked = 0;
        for line in include_str!("../analysis/original_parm_clear_cases.txt").lines() {
            let fields: Vec<u32> = line.split_whitespace().map(|value| value.parse().unwrap()).collect();
            let id = fields[0];
            let count = fields[2] as usize;
            let slot = |base: usize| (fields[base] as u16, std::array::from_fn(|lane| fields[base + 1 + lane]));
            let mut overrides: Vec<_> = (0..count).map(|index| slot(3 + index * 5)).collect();
            let capacity = overrides.capacity();
            remove_parameter(&mut overrides, id);
            assert_eq!(overrides.capacity(), capacity, "{line}");
            let base = 3 + count * 5;
            assert_eq!(fields[base] & 0xffffff, overrides.len() as u32, "{line}");
            assert_eq!(fields[base + 1] & 0xffffff, overrides.len() as u32, "{line}");
            let expected: Vec<_> = (0..fields[base + 2] as usize).map(|index| slot(base + 3 + index * 5)).collect();
            assert_eq!(overrides, expected, "{line}");
            assert_eq!(fields.len(), base + 3 + expected.len() * 5);
            if id < 6 {
                let defaults = [[0x3e800000, 0, 0, 0]; 6];
                assert_eq!(parameter_float("test", id, 0, &defaults, &overrides).unwrap().to_bits(), 0x3e800000);
            }
            checked += 1;
        }
        assert_eq!(checked, 168);
    }

    #[test]
    fn binding_compaction_and_selection_match_native() {
        let mut operations = [0; 3];
        for line in include_str!("../analysis/original_material_binding_cases.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let u = |index: usize| fields[index].parse::<u32>().unwrap();
            match fields[0] {
                "D" | "W" => {
                    let count = u(4) as usize;
                    let whitelist_count = u(5) as usize;
                    let whitelist: Vec<_> = (0..whitelist_count).map(|i| u(6 + i)).collect();
                    let base = 6 + whitelist_count;
                    let slot = |offset: usize| (u(offset) as u16, std::array::from_fn(|lane| u(offset + 1 + lane)));
                    let mut block = ParameterBlock {
                        entries: (0..count).map(|i| slot(base + 5 * i)).collect(),
                        value_count_word: u(1), id_count_word: u(2),
                        value_capacity_word: (u(3) << 8) | 0x13,
                        id_capacity_word: (u(3) << 8) | 0x3f,
                    };
                    let capacity = block.entries.capacity();
                    if fields[0] == "D" { block.deduplicate().unwrap(); }
                    else { block.filter(&whitelist).unwrap(); }
                    let expected = base + count * 5;
                    assert_eq!(block.value_count_word, u(expected), "{line}");
                    assert_eq!(block.id_count_word, u(expected + 1), "{line}");
                    let slots: Vec<_> = (0..u(expected + 2) as usize)
                        .map(|i| slot(expected + 3 + i * 5)).collect();
                    assert_eq!(block.entries, slots, "{line}");
                    assert_eq!(block.entries.capacity(), capacity);
                    assert_eq!(block.value_capacity_word, (u(3) << 8) | 0x13);
                    assert_eq!(block.id_capacity_word, (u(3) << 8) | 0x3f);
                    if let Some(&(id, value)) = block.entries.last() {
                        let defaults = vec![[0; 4]; usize::from(id) + 1];
                        assert_eq!(block.float("native", u32::from(id), 0, &defaults).unwrap().to_bits(), value[0]);
                    }
                    operations[0] += 1;
                }
                "S" => {
                    let group = material_parameter_group(u(1) as u8, fields[2].parse().unwrap());
                    if u(3) != 0 { assert_eq!(group, MaterialParameterGroup::Base); }
                    else if let MaterialParameterGroup::Effect(selector) = group {
                        assert_eq!(i32::from(selector) * 32, fields[4].parse::<i32>().unwrap());
                    } else { panic!("{line}"); }
                    operations[1] += 1;
                }
                "P" => operations[2] += 1,
                _ => panic!("{line}"),
            }
        }
        assert_eq!(operations, [630, 768, 65536]);
    }

    #[test]
    fn prepared_material_groups_feed_the_existing_property_getter() {
        let block = |word| ParameterBlock { entries: vec![(0, [word, 0, 0, 0])],
            value_count_word: 1, id_count_word: 1, value_capacity_word: 256, id_capacity_word: 256 };
        let mut material = MaterialParameterBlocks { base: block(0x3f000000),
            effects: vec![block(0x3f800000)], flags_49: 0x40 };
        assert_eq!(material.selected(-1).unwrap().float("value", 0, 0, &[[0; 4]]).unwrap(), 0.5);
        assert_eq!(material.selected(0).unwrap().float("value", 0, 0, &[[0; 4]]).unwrap(), 1.0);
        assert!(material.selected(-2).is_err());
        assert!(material.selected(1).is_err());
        material.flags_49 = 0;
        assert_eq!(material.selected(127).unwrap().float("value", 0, 0, &[[0; 4]]).unwrap(), 0.5);
        let mut unsupported = block(0);
        unsupported.value_capacity_word = 0;
        assert!(unsupported.deduplicate().is_err());
        assert!(unsupported.filter(&[]).is_err());
    }

    #[test]
    fn resource_union_matches_original() {
        let mut checked = 0;
        for line in include_str!("../analysis/original_static_resource_bounds_cases.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let count = fields[0].parse::<usize>().unwrap();
            let f = |index: usize| f32::from_bits(fields[index].parse().unwrap());
            let previous = std::array::from_fn(|side| std::array::from_fn(|axis| f(1 + side * 3 + axis)));
            let surfaces: Vec<_> = (0..count).map(|surface| std::array::from_fn(|side| std::array::from_fn(|axis| f(7 + surface * 6 + side * 3 + axis)))).collect();
            let actual = accumulate_resource_bounds(previous, &surfaces);
            let expected: [[f32; 3]; 2] = std::array::from_fn(|side| std::array::from_fn(|axis| f(7 + count * 6 + side * 3 + axis)));
            assert_eq!(actual.map(|row| row.map(f32::to_bits)), expected.map(|row| row.map(f32::to_bits)), "{line}");
            checked += 1;
        }
        assert_eq!(checked, 73);
    }

    #[test]
    fn helpers_match_installed_executable() {
        let rows = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.01, 1.0],
            [0.0, 0.0, -0.101, 0.0],
        ];
        let state = crate::tome::RenderQueryState {
            render_rows: rows,
            cropped_rows: rows,
            camera_point: [0.0; 3],
            camera_imprecision: [0.6; 3],
            winding_flip: false,
            viewport: [0, 0, 64, 64],
            maximum_depth: 0.0,
            depth_grid: None,
        };
        let query = NoDepthAabbQuery::from_state(&state).unwrap();
        let mut checked = 0;
        for line in include_str!("../analysis/original_render_model_cases.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let u = |i: usize| fields[i].parse::<u32>().unwrap();
            let f = |i: usize| fields[i].parse::<f32>().unwrap();
            match fields[0] {
                "set" | "clear" => {
                    let mut flags = u(1);
                    if fields[0] == "set" {
                        request_flags(&mut flags, u(2));
                    } else {
                        clear_flags(&mut flags, u(2));
                    }
                    assert_eq!(flags, u(3), "{line}");
                }
                "fade" => {
                    let result = distance_fade(f32::from_bits(u(1)), f32::from_bits(u(2)), f32::from_bits(u(3)));
                    let expected = f32::from_bits(u(4));
                    if expected.is_nan() {
                        assert!(result.is_nan(), "{line}");
                    } else {
                        assert_eq!(result.to_bits(), expected.to_bits(), "{line}");
                    }
                }
                "tree" => {
                    let mut root = VisibilityNode {
                        model_flags: Some(u(1)),
                        children: vec![VisibilityNode {
                            model_flags: None,
                            children: vec![VisibilityNode { model_flags: Some(u(1)), children: vec![] }],
                        }],
                    };
                    root.set_visible(u(2) != 0);
                    assert_eq!(root.model_flags, Some(u(3)), "{line}");
                    assert_eq!(root.children[0].children[0].model_flags, Some(u(4)), "{line}");
                }
                "patch" => {
                    assert_eq!(square_patch_visible(&query, [f(1), f(2), f(3)], f(4), f(5), f(6)).unwrap(), u(7) != 0, "{line}");
                }
                kind => panic!("unknown native model-helper case {kind}"),
            }
            checked += 1;
        }
        assert_eq!(checked, 476);
    }

    #[test]
    fn registry_prefix_matches_installed_original_instructions() {
        let mut count = 0;
        for row in include_str!("../analysis/original_model_registry_cases.txt").lines() {
            let fields: Vec<&str> = row.split_whitespace().collect();
            match fields[0] {
                "resource" | "empty" => {
                    let bounds = std::array::from_fn(|side| std::array::from_fn(|axis| f32::from_bits(fields[1 + side * 3 + axis].parse().unwrap())));
                    let actual = if fields[0] == "empty" { static_resource_bounds(None) } else { static_resource_bounds(Some(resource_bounds(&bounds))) };
                    assert_eq!(actual.map(|side| side.map(f32::to_bits)), bounds.map(|side| side.map(f32::to_bits)));
                    if fields[0] == "resource" { assert!(std::ptr::eq(actual, &bounds)); }
                }
                "prefix" => {
                    let flags = fields[1].parse().unwrap();
                    let shape = fields[2].parse().unwrap();
                    let region = std::array::from_fn(|axis| f32::from_bits(fields[3 + axis].parse().unwrap()));
                    let camera = std::array::from_fn(|axis| f32::from_bits(fields[7 + axis].parse().unwrap()));
                    let accepted = model_eligible(flags, shape) && in_model_region(camera, region);
                    assert_eq!(accepted, fields[10] == "1", "native registry case {count}");
                }
                _ => panic!("unknown original registry case"),
            }
            count += 1;
        }
        assert_eq!(count, 16_388);
        // A prepared record stays eligible even when pending bit1 is set.
        assert!(model_eligible(5, 0));
        assert!(!model_eligible(1, 0x10));
    }

    #[test]
    fn geometry_matches_installed_original_operations() {
        let mut checked = 0;
        for line in include_str!("../analysis/original_model_geometry_cases.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let bits = |index: usize| fields[index].parse::<u32>().unwrap();
            let value = |index: usize| f32::from_bits(bits(index));
            let vector = |start: usize| std::array::from_fn(|axis| value(start + axis));
            let basis = |start: usize| std::array::from_fn(|axis| vector(start + axis * 3));
            let (actual, expected): (Vec<f32>, Vec<f32>) = match fields[0] {
                "base" => { assert_eq!(base_callback() as u32, bits(1)); checked += 1; continue; }
                "matrix" => (affine_rows(vector(1), basis(4), vector(13)).into_iter().flatten().collect(), (16..32).map(value).collect()),
                "bounds" | "static_bounds" => (transformed_bounds([vector(1), vector(4)], vector(7), basis(10), vector(19)).into_iter().flatten().collect(), (22..28).map(value).collect()),
                "region" => (range_region([vector(1), vector(4)], value(7)).to_vec(), (8..12).map(value).collect()),
                kind => panic!("unknown native model geometry case {kind}"),
            };
            assert_eq!(actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), "{line}");
            checked += 1;
        }
        assert_eq!(checked, 355);
    }

    #[test]
    fn masked_requests_do_not_fill_partially_set_masks() {
        let mut flags = 2;
        request_flags(&mut flags, 6);
        assert_eq!(flags, 2);
        clear_flags(&mut flags, 6);
        assert_eq!(flags, 1);
        clear_flags(&mut flags, 1);
        assert_eq!(flags, 0);
    }

    #[test]
    fn visibility_reaches_children_without_parent_model() {
        let mut root = VisibilityNode {
            model_flags: None,
            children: vec![VisibilityNode { model_flags: Some(0), children: vec![] }],
        };
        root.set_visible(true);
        assert_eq!(root.children[0].model_flags, Some(3));
        root.set_visible(false);
        assert_eq!(root.children[0].model_flags, Some(1));
    }

    #[test]
    fn fade_matches_endpoints_and_unordered_machine_comparisons() {
        assert_eq!(distance_fade(0.0, 1.0, -1.0), 0.0);
        assert_eq!(distance_fade(0.0, 1.0, 0.5), 0.5);
        assert_eq!(distance_fade(0.0, 1.0, 2.0), 1.0);
        assert!(distance_fade(0.0, 0.0, 0.0).is_nan());
    }

    #[test]
    fn surface_reduction_clears_only_derived_bits_and_ors_all_surfaces() {
        let a = SurfaceFlags { byte_48: 0, byte_4b: 4, byte_4c: 0, parameter_14229d460: 0.0 };
        let b = SurfaceFlags { byte_48: 1, byte_4b: 0, byte_4c: 0x41, parameter_14229d460: 0.002 };
        assert_eq!(reduce_surface_flags([0xff, 0xff], &[]), [0x0f, 0xfe]);
        assert_eq!(reduce_surface_flags([0xff, 0xff], &[a]), [0x0f, 0xfe]);
        assert_eq!(reduce_surface_flags([0, 4], &[b, a]), [0xf0, 5]);
    }
}
