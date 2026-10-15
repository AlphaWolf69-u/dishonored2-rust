//! Native nine-word render-state decoding.
//!
//! FUN_1402bbfa0 keys blend by bytes 0..17, depth/stencil by 17..23,
//! and raster by 23..36. The overlap is intentional: word 4 is NOT a
//! colour mask, and word 5 combines stencil and raster fields.
//! Factory ports: 1402c4ef0, 1402c52d0, 1402c53e0; enum maps:
//! 1402c4e10, 1402c4e80. Values below are the original D3D11 enums.
//! The renderer's hash/cache binding shortcut is not implemented here.

pub type NativeRenderState = [u32; 9];

/// FUN_1402c4dd0 resets the decoded fields, retaining unused blend bits.
pub fn reset_native_render_state(state: &mut NativeRenderState) {
    for target in &mut state[..4] {
        *target = (*target & 0xffe0_0802) | 0x01e0_0802;
    }
    state[4..].copy_from_slice(&[0xe00e_0000, 0x32ff_ff1e, 0, 0, 0]);
}

/// FUN_140319900's surface-facing override. Only the surface's low bit
/// replaces raster FrontCounterClockwise; all other state bits survive.
pub fn prepared_surface_state(
    mut state: NativeRenderState,
    surface_flags: u8,
) -> NativeRenderState {
    state[5] ^= ((u32::from(surface_flags) << 27) ^ state[5]) & 0x0800_0000;
    state
}

/// FUN_1402fcff0's per-model depth partition, before generic surface-facing
/// preparation. `model_depth_hack` is model owner +0x18b8, NOT a material-name
/// heuristic. Only a positive value writes stencil bit zero with reference 1.
/// The original source render state is otherwise retained, including depth.
/// This is the draw-state subset, not the callback's resource/copy lifecycle.
pub fn main_depth_surface_state(
    mut state: NativeRenderState,
    model_depth_hack: f32,
) -> (NativeRenderState, u8) {
    state[0] = (state[0] & 0xfeff_f802) | 0x00e0_0002;
    let hacked = model_depth_hack > 0.0;
    if hacked {
        state[5] = (state[5] & 0xff01_01ff) | 0x0001_0120;
        state[4] = (state[4] & 0xff) | 0xe80e_8000;
    }
    (state, u8::from(hacked))
}

/// FUN_1402c4e10, including its out-of-range ALWAYS branch.
pub fn native_compare_function(code: u32) -> u32 {
    if code < 7 {
        code + 1
    } else {
        8
    }
}

/// FUN_1402c4e80, including its out-of-range KEEP branch.
pub fn native_stencil_operation(code: u32) -> u32 {
    if (1..=7).contains(&code) {
        code + 1
    } else {
        1
    }
}

fn blend_factor(code: u32) -> u32 {
    if code == 0 {
        1
    } else if (2..=9).contains(&code) {
        code + 1
    } else {
        2
    }
}

fn blend_operation(code: u32) -> u32 {
    [1, 2, 4, 5][(code & 3) as usize]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeBlendTarget {
    pub enabled: bool,
    pub color_source: u32,
    pub color_destination: u32,
    pub color_operation: u32,
    pub alpha_source: u32,
    pub alpha_destination: u32,
    pub alpha_operation: u32,
    pub write_mask: u8,
}

impl NativeBlendTarget {
    pub fn from_word(word: u32) -> Self {
        Self {
            enabled: word & 1 != 0,
            color_source: blend_factor(word >> 1 & 15),
            color_destination: blend_factor(word >> 5 & 15),
            color_operation: blend_operation(word >> 9),
            alpha_source: blend_factor(word >> 11 & 15),
            alpha_destination: blend_factor(word >> 15 & 15),
            alpha_operation: blend_operation(word >> 19),
            write_mask: (word >> 21 & 15) as u8,
        }
    }

    // DAT_141b15da0 and the explicit tail in 1402c4ef0.
    const UNUSED_INDEPENDENT_TARGET: Self = Self {
        enabled: false,
        color_source: 2,
        color_destination: 1,
        color_operation: 1,
        alpha_source: 2,
        alpha_destination: 1,
        alpha_operation: 1,
        write_mask: 15,
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeBlendState {
    pub alpha_to_coverage: bool,
    pub independent_blend: bool,
    /// Targets 1..7 are not initialized/used by the native factory when
    /// IndependentBlendEnable is false; do not invent values for them.
    pub targets: [Option<NativeBlendTarget>; 8],
}

pub fn native_blend_state(state: &NativeRenderState) -> NativeBlendState {
    let independent_blend = state[4] & 2 != 0;
    let mut targets = [None; 8];
    targets[0] = Some(NativeBlendTarget::from_word(state[0]));
    if independent_blend {
        for (target, word) in targets[1..4].iter_mut().zip(&state[1..4]) {
            *target = Some(NativeBlendTarget::from_word(*word));
        }
        targets[4..].fill(Some(NativeBlendTarget::UNUSED_INDEPENDENT_TARGET));
    }
    NativeBlendState {
        alpha_to_coverage: state[4] & 1 != 0,
        independent_blend,
        targets,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeStencilFace {
    pub fail: u32,
    pub depth_fail: u32,
    pub pass: u32,
    pub compare: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeDepthStencilState {
    pub depth_enabled: bool,
    pub depth_write: bool,
    pub depth_compare: u32,
    pub stencil_enabled: bool,
    pub stencil_read_mask: u8,
    pub stencil_write_mask: u8,
    pub front: NativeStencilFace,
    pub back: NativeStencilFace,
}

pub fn native_depth_stencil_state(state: &NativeRenderState) -> NativeDepthStencilState {
    let stencil = state[4];
    let depth = state[5];
    NativeDepthStencilState {
        depth_enabled: depth & 8 != 0,
        depth_write: depth & 16 != 0,
        depth_compare: native_compare_function(depth & 7),
        stencil_enabled: depth & 32 != 0,
        stencil_read_mask: (depth >> 8) as u8,
        stencil_write_mask: (depth >> 16) as u8,
        front: NativeStencilFace {
            fail: native_stencil_operation(stencil >> 8 & 7),
            depth_fail: native_stencil_operation(stencil >> 11 & 7),
            pass: native_stencil_operation(stencil >> 14 & 7),
            compare: native_compare_function(stencil >> 17 & 7),
        },
        back: NativeStencilFace {
            fail: native_stencil_operation(stencil >> 20 & 7),
            depth_fail: native_stencil_operation(stencil >> 23 & 7),
            pass: native_stencil_operation(stencil >> 26 & 7),
            compare: native_compare_function(stencil >> 29),
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeRasterState {
    pub fill_mode: u32,
    pub cull_mode: u32,
    pub front_counter_clockwise: bool,
    pub depth_bias: i32,
    pub depth_bias_clamp_bits: u32,
    pub slope_scaled_depth_bias_bits: u32,
    pub depth_clip: bool,
    pub scissor: bool,
    pub multisample: bool,
    pub antialiased_line: bool,
}

pub fn native_raster_state(state: &NativeRenderState) -> NativeRasterState {
    let raster = state[5];
    NativeRasterState {
        fill_mode: (!(raster >> 26) & 1) | 2,
        cull_mode: match raster >> 24 & 3 {
            0 => 1,
            1 => 2,
            _ => 3,
        },
        front_counter_clockwise: raster & (1 << 27) != 0,
        depth_bias: state[6] as i32,
        depth_bias_clamp_bits: state[7],
        slope_scaled_depth_bias_bits: state[8],
        depth_clip: raster & (1 << 28) != 0,
        scissor: true,
        multisample: raster & (1 << 30) != 0,
        antialiased_line: raster & (1 << 31) != 0,
    }
}

/// The two screen-shadow draw overrides in FUN_14032c7f0. Both preserve
/// AO and packed depth, write only G, and use read-only depth/KEEP stencil.
/// First stencil == 1; second stencil != 1 (read mask 1). The callback
/// supplies reference 1 to FUN_1402bb470 for BOTH draws.
pub fn screen_shadow_pass_states(source: NativeRenderState) -> [NativeRenderState; 2] {
    let mut first = source;
    first[0] = (first[0] & 0xfe5f_fffe) | 0x0040_0000;
    first[5] = (first[5] & 0xfc01_01ef) | 0x0001_012f;
    first[4] = (first[4] & 0xff) | 0x4004_0000;
    let mut second = first;
    second[4] = (second[4] & 0x1ff1_ffff) | 0xa00a_0000;
    [first, second]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_depth_stencil_bit_tracks_actual_depth_hack_not_mesh_type() {
        let mut source = [0; 9];
        reset_native_render_state(&mut source);
        for hack in [0.0, -0.0, -1.0, f32::NEG_INFINITY, f32::NAN] {
            let (state, reference) = main_depth_surface_state(source, hack);
            assert_eq!(reference, 0);
            assert_eq!(&state[1..], &source[1..]);
        }
        for hack in [f32::MIN_POSITIVE, 0.01, 1.0, f32::INFINITY] {
            let (state, reference) = main_depth_surface_state(source, hack);
            let depth = native_depth_stencil_state(&state);
            assert_eq!(reference, 1);
            assert!(depth.stencil_enabled);
            assert_eq!((depth.stencil_read_mask, depth.stencil_write_mask), (1, 1));
            for face in [depth.front, depth.back] {
                assert_eq!(face.compare, 8);
                assert_eq!(face.pass, 3);
                assert_eq!((face.fail, face.depth_fail), (1, 1));
            }
            assert_eq!(
                depth.depth_compare,
                native_depth_stencil_state(&source).depth_compare
            );
            assert_eq!(
                depth.depth_write,
                native_depth_stencil_state(&source).depth_write
            );
            assert_eq!(&state[6..], &source[6..]);
        }
        for flags in 0..=255 {
            let state = prepared_surface_state([u32::MAX; 9], flags);
            assert_eq!(
                native_raster_state(&state).front_counter_clockwise,
                flags & 1 != 0
            );
            assert_eq!(state[5] & !0x0800_0000, u32::MAX & !0x0800_0000);
            assert!(state[..5]
                .iter()
                .chain(&state[6..])
                .all(|word| *word == u32::MAX));
        }
    }

    #[test]
    fn enum_defaults_match_native_switches() {
        for code in 0..256 {
            assert_eq!(
                native_compare_function(code),
                if code < 7 { code + 1 } else { 8 }
            );
            assert_eq!(
                native_stencil_operation(code),
                if (1..=7).contains(&code) { code + 1 } else { 1 }
            );
        }
    }

    #[test]
    fn native_screen_draws_only_green_and_split_stencil_bit_one() {
        for source in [[0; 9], [u32::MAX; 9], [0xdead_beef; 9]] {
            let passes = screen_shadow_pass_states(source);
            for (index, state) in passes.iter().enumerate() {
                let blend = native_blend_state(state).targets[0].unwrap();
                assert!(!blend.enabled);
                assert_eq!(blend.write_mask, 2);
                let depth = native_depth_stencil_state(state);
                assert!(depth.depth_enabled);
                assert!(!depth.depth_write);
                assert_eq!(depth.depth_compare, 8);
                assert!(depth.stencil_enabled);
                assert_eq!((depth.stencil_read_mask, depth.stencil_write_mask), (1, 1));
                assert_eq!(depth.front.compare, if index == 0 { 3 } else { 6 });
                assert_eq!(depth.back.compare, depth.front.compare);
                assert_eq!(
                    (depth.front.fail, depth.front.depth_fail, depth.front.pass),
                    (1, 1, 1)
                );
                assert_eq!(
                    (depth.back.fail, depth.back.depth_fail, depth.back.pass),
                    (1, 1, 1)
                );
                assert_eq!(native_raster_state(state).cull_mode, 1);
                assert_eq!(&state[6..], &source[6..]);
            }
        }
    }

    #[test]
    fn all_three_factories_match_original_machine_code() {
        let mut cases = 0;
        let mut enums = 0;
        let mut resets = 0;
        for line in include_str!("../analysis/original_render_state_cases.txt").lines() {
            let mut parts = line.split_whitespace();
            let kind = parts.next().unwrap();
            let values: Vec<u32> = parts.map(|s| u32::from_str_radix(s, 16).unwrap()).collect();
            if kind == "ENUM" {
                assert_eq!(native_compare_function(values[0]), values[1]);
                assert_eq!(native_stencil_operation(values[0]), values[2]);
                enums += 1;
                continue;
            }
            let mut state: NativeRenderState = values[..9].try_into().unwrap();
            if kind == "RESET" {
                reset_native_render_state(&mut state);
                assert_eq!(
                    state.as_slice(),
                    &values[9..],
                    "native state reset case {resets}"
                );
                resets += 1;
                continue;
            }
            let blend = native_blend_state(&state);
            let mut actual = state.to_vec();
            actual.extend([
                u32::from(blend.alpha_to_coverage),
                u32::from(blend.independent_blend),
            ]);
            for target in blend.targets.iter().flatten() {
                actual.extend([
                    u32::from(target.enabled),
                    target.color_source,
                    target.color_destination,
                    target.color_operation,
                    target.alpha_source,
                    target.alpha_destination,
                    target.alpha_operation,
                    u32::from(target.write_mask),
                ]);
            }
            let depth = native_depth_stencil_state(&state);
            actual.extend([
                u32::from(depth.depth_enabled),
                u32::from(depth.depth_write),
                depth.depth_compare,
                u32::from(depth.stencil_enabled),
                u32::from(depth.stencil_read_mask),
                u32::from(depth.stencil_write_mask),
            ]);
            for face in [depth.front, depth.back] {
                actual.extend([face.fail, face.depth_fail, face.pass, face.compare]);
            }
            let raster = native_raster_state(&state);
            actual.extend([
                raster.fill_mode,
                raster.cull_mode,
                u32::from(raster.front_counter_clockwise),
                raster.depth_bias as u32,
                raster.depth_bias_clamp_bits,
                raster.slope_scaled_depth_bias_bits,
                u32::from(raster.depth_clip),
                u32::from(raster.scissor),
                u32::from(raster.multisample),
                u32::from(raster.antialiased_line),
            ]);
            assert_eq!(actual, values, "native descriptor case {cases}");
            cases += 1;
        }
        assert_eq!((cases, enums, resets), (1286, 259, 1026));
    }

    #[test]
    fn independent_target_tail_is_native_default_not_target_zero() {
        let mut state = [0; 9];
        state[4] = 3;
        let blend = native_blend_state(&state);
        assert!(blend.alpha_to_coverage && blend.independent_blend);
        assert_eq!(blend.targets[0].unwrap().write_mask, 0);
        assert_eq!(
            blend.targets[4].unwrap(),
            NativeBlendTarget::UNUSED_INDEPENDENT_TARGET
        );
        state[4] = 0;
        assert!(native_blend_state(&state).targets[1..]
            .iter()
            .all(Option::is_none));
    }
}
