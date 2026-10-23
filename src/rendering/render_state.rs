//! Native nine-word render-state decoding.
//!
//! FUN_1402bbfa0 keys blend by bytes 0..17, depth/stencil by 17..23,
//! and raster by 23..36. The overlap is intentional: word 4 is NOT a
//! colour mask, and word 5 combines stencil and raster fields.
//! Factory ports: 1402c4ef0, 1402c52d0, 1402c53e0; enum maps:
//! 1402c4e10, 1402c4e80. Values below are the original D3D11 enums.
//! The renderer's hash/cache binding shortcut is not implemented here.

pub type RenderState = [u32; 9];

/// FUN_1402c4dd0 resets the decoded fields, retaining unused blend bits.
pub fn reset_render_state(state: &mut RenderState) {
    for target in &mut state[..4] {
        *target = (*target & 0xffe0_0802) | 0x01e0_0802;
    }
    state[4..].copy_from_slice(&[0xe00e_0000, 0x32ff_ff1e, 0, 0, 0]);
}

/// 316670's depth/stencil classification before the near/far colour draws.
/// These passes replace only 0x04 (near) or 0x08 (far); reference is separate.
pub fn camera_blur_stencil_state(far: bool) -> RenderState {
    let mut state = [0; 9];
    reset_render_state(&mut state);
    state[0] &= 0xfe1f_fffe;
    state[4] = (state[4] & 0xff) | 0xe80e_8000;
    state[5] = if far {
        (state[5] & 0xfc08_08ee) | 0x0008_082e
    } else {
        (state[5] & 0xfc04_04ef) | 0x0004_042f
    };
    state
}

/// 316670's stencil restriction applied to the selected colour material.
pub fn camera_blur_colour_state(mut state: RenderState, far: bool) -> RenderState {
    state[4] = (state[4] & 0xff) | 0x4004_0000;
    state[5] = if far {
        (state[5] & 0xfc08_08ef) | 0x0008_082f
    } else {
        (state[5] & 0xfc04_04ef) | 0x0004_042f
    };
    state
}

/// Resolve a material's authored `state { ... }` against the actual MAIN
/// program state (1402d5610/1402d662d). Only root-level state is consumed:
/// similarly named option/parameter values are not drawing commands. No state
/// preserves the nine MAIN words exactly. Multiple state blocks are explicitly
/// unsupported until the original collector's concatenation is verified.
pub fn material_render_state(source: &str, base: RenderState) -> Result<RenderState, String> {
    let tokens = crate::entities::declaration_lexemes(source)?;
    if tokens.first() != Some(&"{") {
        return Err("material declaration must start with {".into());
    }
    let mut depth = 0usize;
    let mut index = 0;
    let mut state = None;
    while index < tokens.len() {
        let token = tokens[index];
        if depth == 1 && token.eq_ignore_ascii_case("state") {
            if state.is_some() {
                return Err("multiple native material state blocks are unresolved".into());
            }
            if tokens.get(index + 1) != Some(&"{") {
                return Err("material state must start with {".into());
            }
            let start = index + 1;
            let mut end = start + 1;
            let mut braces = 1;
            while end < tokens.len() && braces != 0 {
                match tokens[end] {
                    "{" => braces += 1,
                    "}" => braces -= 1,
                    _ => {}
                }
                end += 1;
            }
            if braces != 0 {
                return Err("Unexpected EOF in state block".into());
            }
            state = Some(parse_render_state(
                &tokens[start..end].join(" "),
                Some(base),
            )?);
            index = end;
            continue;
        }
        match token {
            "{" => depth += 1,
            "}" => {
                depth = depth
                    .checked_sub(1)
                    .ok_or("unmatched material closing brace")?
            }
            _ => {}
        }
        index += 1;
    }
    if depth != 0 {
        return Err("unterminated native material declaration".into());
    }
    Ok(state.unwrap_or(base))
}

/// Valid drawing-state blocks consumed by FUN_1402c3bc0. The material caller
/// at 1402d662d supplies MAIN +0x58 in R8: overrides inherit all nine words,
/// not a freshly reset state. None is the original explicit null-base branch.
/// Lexer diagnostic/recovery side effects on malformed input are not emulated;
/// unsupported/malformed blocks return an error instead of publishing a state.
pub fn parse_render_state(source: &str, base: Option<RenderState>) -> Result<RenderState, String> {
    fn take<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Result<&'a str, String> {
        tokens
            .next()
            .ok_or_else(|| "Unexpected EOF in state block".into())
    }
    fn named(token: &str, names: &[(&str, u32)], gl_prefix: bool) -> Result<u32, String> {
        let upper = token.to_ascii_uppercase();
        let mut token = upper.as_str();
        if gl_prefix {
            while let Some(value) = token.strip_prefix("GL_") {
                token = value;
            }
        }
        names
            .iter()
            .find(|(name, _)| *name == token)
            .map(|(_, value)| *value)
            .ok_or_else(|| format!("invalid drawing-state value {token:?}"))
    }
    fn comparison(token: &str) -> Result<u32, String> {
        named(
            token,
            &[
                ("NEVER", 0),
                ("LESS", 1),
                ("EQUAL", 2),
                ("LEQUAL", 3),
                ("GREATER", 4),
                ("NOTEQUAL", 5),
                ("GEQUAL", 6),
                ("ALWAYS", 7),
            ],
            true,
        )
    }
    fn operation(token: &str) -> Result<u32, String> {
        named(
            token,
            &[("ADD", 0), ("SUB", 1), ("MIN", 2), ("MAX", 3)],
            true,
        )
    }
    fn stencil(token: &str) -> Result<u32, String> {
        named(
            token,
            &[
                ("KEEP", 0),
                ("ZERO", 1),
                ("REPLACE", 2),
                ("INCR_SAT", 3),
                ("DECR_SAT", 4),
                ("INVERT", 5),
                ("INCR", 6),
                ("DECR", 7),
            ],
            true,
        )
    }
    let mut tokens = crate::entities::declaration_lexemes(source)?.into_iter();
    if take(&mut tokens)? != "{" {
        return Err("drawing state must start with {".into());
    }
    let mut state = base.unwrap_or_else(|| {
        let mut value = [0; 9];
        reset_render_state(&mut value);
        value
    });
    loop {
        let key = take(&mut tokens)?.to_ascii_lowercase();
        match key.as_str() {
            "}" => {
                state[0] = (state[0] & !1) | u32::from(crate::material::blend_enabled(state[0]));
                return Ok(state);
            }
            "blend" | "alphablend" => {
                let mut a = crate::material::parse_blend_factor(take(&mut tokens)?)?;
                let mut b = crate::material::parse_blend_factor(take(&mut tokens)?)?;
                let (mask, shift) = if key == "alphablend" {
                    // 140166830 is a returning diagnostic stub; the following
                    // original instructions still substitute colour by alpha.
                    let alpha = |value| match value {
                        2 => 4,
                        3 => 5,
                        8 => 6,
                        9 => 7,
                        _ => value,
                    };
                    a = alpha(a);
                    b = alpha(b);
                    (0x7f800, 11)
                } else {
                    (0x1fe, 1)
                };
                state[0] = (state[0] & !mask) | ((a | (b << 4)) << shift);
            }
            "blendop" | "alphablendop" => {
                let value = operation(take(&mut tokens)?)?;
                let shift = if key == "blendop" { 9 } else { 19 };
                state[0] = (state[0] & !(3 << shift)) | (value << shift);
            }
            "depthfunc" => {
                state[5] = (state[5] & !7) | comparison(take(&mut tokens)?)?;
            }
            "wireframe" => state[5] |= 0x0400_0000,
            "alphacoverage" | "depthwrite" | "depthenable" | "colorwrite" | "alphawrite" => {
                let value = take(&mut tokens)?
                    .parse::<i32>()
                    .map_err(|_| format!("invalid integer for {key}"))?
                    != 0;
                let (word, mask) = match key.as_str() {
                    "alphacoverage" => (4, 1),
                    "depthwrite" => (5, 0x10),
                    "depthenable" => (5, 8),
                    "colorwrite" => (0, 0x00e0_0000),
                    _ => (0, 0x0100_0000),
                };
                state[word] = (state[word] & !mask) | if value { mask } else { 0 };
            }
            "cull" => {
                let value = named(
                    take(&mut tokens)?,
                    &[("NONE", 0), ("FRONT", 1), ("BACK", 2)],
                    false,
                )?;
                state[5] = (state[5] & !0x0300_0000) | (value << 24);
            }
            "frontstencilop" | "backstencilop" => {
                let fail = stencil(take(&mut tokens)?)?;
                let depth_fail = stencil(take(&mut tokens)?)?;
                let pass = stencil(take(&mut tokens)?)?;
                let shift = if key == "frontstencilop" { 8 } else { 20 };
                state[4] = (state[4] & !(0x1ff << shift))
                    | ((fail | (depth_fail << 3) | (pass << 6)) << shift);
                state[5] |= 0x20;
            }
            "frontstencilfunc" | "backstencilfunc" => {
                let value = comparison(take(&mut tokens)?)?;
                let shift = if key == "frontstencilfunc" { 17 } else { 29 };
                state[4] = (state[4] & !(7 << shift)) | (value << shift);
                state[5] |= 0x20;
            }
            _ => return Err(format!("unsupported drawing-state option {key:?}")),
        }
    }
}

/// FUN_140319900's surface-facing override. Only the surface's low bit
/// replaces raster FrontCounterClockwise; all other state bits survive.
pub fn prepared_surface_state(mut state: RenderState, surface_flags: u8) -> RenderState {
    state[5] ^= ((u32::from(surface_flags) << 27) ^ state[5]) & 0x0800_0000;
    state
}

/// Original CVTTSS2SI result, including INT_MIN on NaN/out-of-range.
pub(crate) fn cvtt_f32(value: f32) -> u32 {
    if value >= -2147483648.0 && value < 2147483648.0 {
        value as i32 as u32
    } else {
        0x8000_0000
    }
}

/// FUN_140293470, used by FUN_1402996f0 when requested shadow bias/clip
/// differs from the world's cached state. Only these fields are changed;
/// source blend/stencil/cull bits not in the masks survive. Globals are
/// explicit owner inputs, not guessed renderer defaults.
pub fn world_shadow_draw_state(
    source: RenderState,
    surface_flags: u8,
    depth_bias: f32,
    slope_scale: f32,
    clamp: f32,
    depth_clip: u8,
) -> RenderState {
    let mut state = prepared_surface_state(source, surface_flags);
    state[0] &= 0xfe1f_fffe;
    state[6] = cvtt_f32(depth_bias);
    state[8] = slope_scale.to_bits();
    state[5] = (state[5] & 0xefff_fff9) | (u32::from(depth_clip & 1) << 28) | 1;
    state[7] = clamp.to_bits();
    state
}

/// Literal state leaves adjacent to the world-shadow override. Keeping their
/// native addresses avoids assigning an unverified pass/category meaning.
/// FUN_1402934e0; the two-byte store starts at byte0x15, not a word boundary.
pub fn world_state_2934e0(mut state: RenderState) -> RenderState {
    state[5] = (state[5] & 0xfcff_ffea) | 0x22;
    state[4] = (state[4] & 0xff) | 0xe80e_8000;
    state[5] = (state[5] & !0x00ff_ff00) | 0x0004_0000;
    state
}

/// FUN_140293500. FUN_1402934b0 additionally invokes 3253f0/3253d0;
/// those callbacks are not covered by this leaf.
pub fn world_state_293500(mut state: RenderState) -> RenderState {
    state[5] = (state[5] & 0xffff_ffee) | 6;
    state
}

/// FUN_140293510, installed by 2a8140 in the world sky/cloud cache consumed
/// by 2a09f0. Color alpha-blends while MRT1 adds the red fog-density mask.
pub fn sky_cloud_draw_state(mut state: RenderState) -> RenderState {
    state[5] = (state[5] & 0xffff_ffee) | 0xe;
    state[4] |= 2;
    state[0] = (state[0] & 0xffff_f8a9) | 0xa9;
    state[1] = (state[1] & 0xfe20_8823) | 0x0020_8823;
    state
}

/// 309020 world fog and 309b50 radial scattering exclude stencil bit 0x40.
/// The caller supplies stencil reference 0x40 and the current fog depth target.
pub fn fog_draw_state(mut state: RenderState) -> RenderState {
    state[4] = (state[4] & 0xff) | 0xa00a_0000;
    state[5] = (state[5] & 0xff40_40ff) | 0x0040_4020;
    state
}

/// 307530's area-volume renderMinMax submission, with no depth attachment.
pub fn fog_interval_draw_state(mut state: RenderState) -> RenderState {
    state[4] |= 2;
    state[5] = (state[5] & 0xffff_ffee) | 0xe;
    state[0] = (state[0] & 0xffe0_8c23) | 0x8c23;
    state
}

/// 30a800 selects RGB blending while preserving the sun program's alpha state.
pub fn sun_sprite_draw_state(mut state: RenderState, additive: bool) -> RenderState {
    state[0] = if additive {
        (state[0] & 0xffff_f823) | 0x23
    } else {
        (state[0] & 0xffff_f8a9) | 0xa9
    };
    state
}

/// FUN_140293540; caller supplies actual DAT_1432f8cac.
pub fn world_state_293540(state: RenderState, clamp: f32) -> RenderState {
    let mut state = world_state_293500(state);
    state[6] = 0;
    state[8] = 0x3ca3_d70a;
    state[7] = clamp.to_bits();
    state
}

/// FUN_140293570.
pub fn world_state_293570(mut state: RenderState) -> RenderState {
    state[5] = (state[5] & 0xffff_ffea) | 2;
    state
}

/// FUN_1402fcff0's per-model depth partition, before generic surface-facing
/// preparation. `model_depth_hack` is model owner +0x18b8, NOT a material-name
/// heuristic. Only a positive value writes stencil bit zero with reference 1.
/// The original source render state is otherwise retained, including depth.
/// This is the draw-state subset, not the callback's resource/copy lifecycle.
pub fn main_depth_surface_state(
    mut state: RenderState,
    model_depth_hack: f32,
) -> (RenderState, u8) {
    state[0] = (state[0] & 0xfeff_f802) | 0x00e0_0002;
    let hacked = model_depth_hack > 0.0;
    if hacked {
        state[5] = (state[5] & 0xff01_01ff) | 0x0001_0120;
        state[4] = (state[4] & 0xff) | 0xe80e_8000;
    }
    (state, u8::from(hacked))
}

/// 2a6650's cached world depth state, consumed by 29bd30 (including water).
/// The cache first applies the world record's facing bit, then 293310.
/// Unlike model water, this retains the material's depth/stencil fields;
/// 29bd30 binds the cached objects with stencil reference zero.
pub fn world_depth_state(source: RenderState, surface_flags: u8) -> RenderState {
    let mut state = prepared_surface_state(source, surface_flags);
    state[5] |= 0x10;
    state[0] = (state[0] & 0xfe7f_fffe) | 0x0060_0000;
    state
}

/// 2fd8f0's model-water depth state. The caller supplies stencil reference 4
/// through 319900 -> 2bb550 -> 2bb470 -> 2bbfa0, independently of this packet.
/// The water vertex program uses the material's ordinary TransformVertex.
pub fn water_depth_state() -> RenderState {
    let mut state = [0; 9];
    reset_render_state(&mut state);
    state[0] &= 0xfe1f_ffff;
    state[4] = (state[4] & 0xff) | 0xe80e_8000;
    state[5] = (state[5] & 0xfc04_00fe) | 0x0004_0026;
    state
}

/// 2fcd90: first boat-volume pass, before water depth. Source is the forced
/// arkBoatVolume MAIN program state; stencil reference is 8.
pub fn boat_volume_depth_state(mut state: RenderState) -> RenderState {
    state[0] &= 0xfe1f_fffe;
    state[4] = (state[4] & 0xff) | 0x000a_8800;
    state[5] = (state[5] & 0xfc08_08fc) | 0x0008_083c;
    state
}

/// 2fcb70: second boat-volume pass, also used after the depthAlpha copy on
/// the main camera and reduced-alpha targets. Model stencil reference remains 8.
pub fn boat_volume_stencil_state(mut state: RenderState) -> RenderState {
    state[0] &= 0xfe1f_fffe;
    state[4] = (state[4] & 0xff) | 0xe28e_2800;
    state[5] = (state[5] & 0xfc02_02ec) | 0x0002_022c;
    state
}

/// 293400/2937d0, called by the world boat list (29aba0). The caller first
/// applies the world surface's facing bit. Its packet masks are equivalent to
/// the model helpers, but the stencil pass binds reference 2 instead of 8.
pub fn world_boat_volume_state(state: RenderState, stencil_pass: bool) -> (RenderState, u8) {
    if stencil_pass {
        (boat_volume_stencil_state(state), 2)
    } else {
        (boat_volume_depth_state(state), 8)
    }
}

/// 2fdea0's shared ocean/volume underwater mark. Stencil reference is 0x42;
/// bit 1 excludes boat interiors, and only bit 6 may be replaced.
pub fn underwater_stencil_state(mut state: RenderState) -> RenderState {
    state[0] &= 0xfe1f_fffe;
    state[4] = (state[4] & 0xff) | 0xa80a_8000;
    state[5] = (state[5] & 0xfc40_02ef) | 0x0040_022f;
    state
}

/// 2feef0 restores opaque depth only where (stencil & 12)==8, after the
/// boat/water draws. The copy shader is the same one used by 34f740;
/// the caller supplies stencil reference 8 and the immutable opaque source.
pub fn boat_depth_restore_state() -> RenderState {
    let mut state = [0; 9];
    reset_render_state(&mut state);
    state[0] &= 0xfe1f_fffe;
    state[4] = (state[4] & 0xff) | 0x4004_0000;
    state[5] = (state[5] & 0xfc00_0cff) | 0x0000_0c3f;
    state
}

/// FUN_1402c4e10, including its out-of-range ALWAYS branch.
pub fn compare_function(code: u32) -> u32 {
    if code < 7 {
        code + 1
    } else {
        8
    }
}

/// FUN_1402c4e80, including its out-of-range KEEP branch.
pub fn stencil_operation(code: u32) -> u32 {
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
pub struct BlendTarget {
    pub enabled: bool,
    pub color_source: u32,
    pub color_destination: u32,
    pub color_operation: u32,
    pub alpha_source: u32,
    pub alpha_destination: u32,
    pub alpha_operation: u32,
    pub write_mask: u8,
}

impl BlendTarget {
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
pub struct BlendState {
    pub alpha_to_coverage: bool,
    pub independent_blend: bool,
    /// Targets 1..7 are not initialized/used by the native factory when
    /// IndependentBlendEnable is false; do not invent values for them.
    pub targets: [Option<BlendTarget>; 8],
}

pub fn blend_state(state: &RenderState) -> BlendState {
    let independent_blend = state[4] & 2 != 0;
    let mut targets = [None; 8];
    targets[0] = Some(BlendTarget::from_word(state[0]));
    if independent_blend {
        for (target, word) in targets[1..4].iter_mut().zip(&state[1..4]) {
            *target = Some(BlendTarget::from_word(*word));
        }
        targets[4..].fill(Some(BlendTarget::UNUSED_INDEPENDENT_TARGET));
    }
    BlendState {
        alpha_to_coverage: state[4] & 1 != 0,
        independent_blend,
        targets,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StencilFace {
    pub fail: u32,
    pub depth_fail: u32,
    pub pass: u32,
    pub compare: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepthStencilState {
    pub depth_enabled: bool,
    pub depth_write: bool,
    pub depth_compare: u32,
    pub stencil_enabled: bool,
    pub stencil_read_mask: u8,
    pub stencil_write_mask: u8,
    pub front: StencilFace,
    pub back: StencilFace,
}

pub fn depth_stencil_state(state: &RenderState) -> DepthStencilState {
    let stencil = state[4];
    let depth = state[5];
    DepthStencilState {
        depth_enabled: depth & 8 != 0,
        depth_write: depth & 16 != 0,
        depth_compare: compare_function(depth & 7),
        stencil_enabled: depth & 32 != 0,
        stencil_read_mask: (depth >> 8) as u8,
        stencil_write_mask: (depth >> 16) as u8,
        front: StencilFace {
            fail: stencil_operation(stencil >> 8 & 7),
            depth_fail: stencil_operation(stencil >> 11 & 7),
            pass: stencil_operation(stencil >> 14 & 7),
            compare: compare_function(stencil >> 17 & 7),
        },
        back: StencilFace {
            fail: stencil_operation(stencil >> 20 & 7),
            depth_fail: stencil_operation(stencil >> 23 & 7),
            pass: stencil_operation(stencil >> 26 & 7),
            compare: compare_function(stencil >> 29),
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterState {
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

pub fn raster_state(state: &RenderState) -> RasterState {
    let raster = state[5];
    RasterState {
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
pub fn screen_shadow_pass_states(source: RenderState) -> [RenderState; 2] {
    let mut first = source;
    first[0] = (first[0] & 0xfe5f_fffe) | 0x0040_0000;
    first[5] = (first[5] & 0xfc01_01ef) | 0x0001_012f;
    first[4] = (first[4] & 0xff) | 0x4004_0000;
    let mut second = first;
    second[4] = (second[4] & 0x1ff1_ffff) | 0xa00a_0000;
    [first, second]
}

/// Descriptor arithmetic of FUN_140378280, after a sampler-cache miss.
/// Input is the original nine-word canonical sampler record; output is the
/// thirteen-word D3D11_SAMPLER_DESC, preserving all float bits. This does not
/// implement the function's cache, lock, device creation or disabled-renderer
/// path. 768 offline original executions verify the reduction independently.
pub fn sampler_descriptor(canonical: &[u32; 9], image_anisotropy: f32) -> [u32; 13] {
    let flags = canonical[0];
    let comparison = (flags >> 12) & 7;
    let min = (flags >> 2) & 3;
    let mip = (flags >> 4) & 3;
    let mut filter = if min == 2 {
        (u32::from(comparison != 0) << 7) | 0x55
    } else {
        ((u32::from(comparison != 0) << 3 | u32::from(flags & 3 == 0)) << 2 | u32::from(min == 0))
            << 2
            | u32::from(mip == 0)
    };
    let anisotropy = if mip != 0 || image_anisotropy < 1.0 {
        let own = f32::from_bits(canonical[8]);
        if own < 1.0 {
            1
        } else {
            filter = 0x55;
            cvtt_f32(own)
        }
    } else {
        cvtt_f32(image_anisotropy)
    };
    let address = |shift: u32| match (flags >> shift) & 3u32 {
        0 => 1, // WRAP
        1 => 3, // CLAMP
        2 => 4, // BORDER
        3 => 2, // MIRROR
        _ => unreachable!(),
    };
    [
        filter,
        address(6),
        address(8),
        address(10),
        canonical[5],
        anisotropy,
        comparison + 1,
        canonical[1],
        canonical[2],
        canonical[3],
        canonical[4],
        canonical[6],
        canonical[7],
    ]
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    pub(super) fn descriptor_fields(state: &RenderState) -> [u32; 24] {
    let d = depth_stencil_state(state);
    let r = raster_state(state);
    [
        u32::from(d.depth_enabled), u32::from(d.depth_write), d.depth_compare,
        u32::from(d.stencil_enabled), u32::from(d.stencil_read_mask), u32::from(d.stencil_write_mask),
        d.front.fail, d.front.depth_fail, d.front.pass, d.front.compare,
        d.back.fail, d.back.depth_fail, d.back.pass, d.back.compare,
        r.fill_mode, r.cull_mode, u32::from(r.front_counter_clockwise),
        r.depth_bias as u32, r.depth_bias_clamp_bits, r.slope_scaled_depth_bias_bits,
        u32::from(r.depth_clip), u32::from(r.scissor), u32::from(r.multisample), u32::from(r.antialiased_line),
    ]
    }

    #[test]
    fn boat_depth_restoration_matches_executed_descriptor_factories() {
        let row: Vec<u32> = include_str!("../../tests/fixtures/boat_depth_restore_state.txt")
            .split_whitespace().map(|s| u32::from_str_radix(s, 16).unwrap()).collect();
        assert_eq!(row.len(), 25);
        assert_eq!(row[0], 8);
        assert_eq!(&descriptor_fields(&boat_depth_restore_state()), &row[1..]);
    }

    #[test]
    fn water_and_boat_depth_states_match_executed_descriptor_factories() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/water_depth_states.txt").lines() {
            let row: Vec<u32> = line.split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap()).collect();
            assert_eq!(row.len(), 35);
            let source: RenderState = row[1..10].try_into().unwrap();
            let (state, reference) = match row[0] {
                0 => (water_depth_state(), 4),
                1 => (boat_volume_depth_state(source), 8),
                2 => (boat_volume_stencil_state(source), 8),
                _ => unreachable!(),
            };
            assert_eq!(row[10], reference);
            let fields = descriptor_fields(&state);
            assert_eq!(&fields, &row[11..], "water/boat operation{} case{count}", row[0]);
            count += 1;
        }
        assert_eq!(count, 96);
    }

    #[test]
    fn world_boat_states_match_executed_packets_and_references() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/world_boat_states.txt").lines() {
            let words: Vec<_> = line.split_whitespace().map(|v| u32::from_str_radix(v,16).unwrap()).collect();
            assert_eq!(words.len(),20);
            let (state,reference) = world_boat_volume_state(words[1..10].try_into().unwrap(), words[0] != 0);
            assert_eq!(state, words[10..19]);
            assert_eq!(u32::from(reference), words[19]);
            count += 1;
        }
        assert_eq!(count,96);
    }

    #[test]
    fn state_parser_matches_token_stream_kernel() {
        fn words(text: &str) -> [u32; 9] {
            text.split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect::<Vec<_>>()
                .try_into()
                .unwrap()
        }
        let mut count = 0;
        for (index, line) in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/render_state_tokens_cases.txt"
        ))
        .lines()
        .enumerate()
        {
            let mut columns = line.split('\t');
            let base = columns.next().unwrap();
            let base = (base != "-").then(|| words(base));
            let source = columns.next().unwrap();
            let expected = words(columns.next().unwrap());
            assert!(columns.next().is_none());
            assert_eq!(
                parse_render_state(source, base).unwrap(),
                expected,
                "original token-stream kernel case {index}: {source}"
            );
            count += 1;
        }
        assert_eq!(count, 768);
    }

    #[test]
    fn authored_material_state_is_token_scoped_and_inherits_main() {
        let base = [
            0x01e0_0803,
            0x1234,
            0x5678,
            0x9abc,
            0xe00e_0000,
            0x32ff_ff1e,
            17,
            23,
            29,
        ];
        let source = "{ options { blend ONE state 1 } parms { cull FRONT } state { blend SRC_ALPHA ONE_MINUS_SRC_ALPHA depthWrite 0 cull NONE } mainProgram arkdefault }";
        let actual = material_render_state(source, base).unwrap();
        let expected = parse_render_state(
            "{ blend SRC_ALPHA ONE_MINUS_SRC_ALPHA depthWrite 0 cull NONE }",
            Some(base),
        )
        .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(&actual[1..4], &base[1..4]);
        assert_eq!(&actual[6..], &base[6..]);
        assert_eq!(
            material_render_state("{ options { blend ZERO state 1 } }", base).unwrap(),
            base
        );
        assert!(material_render_state("{ state { } state { } }", base).is_err());
    }

    #[test]
    fn state_parser_shares_comment_rules_and_rejects_unresolved_input() {
        let plain = parse_render_state("{ blend ONE ZERO cull FRONT }", None).unwrap();
        assert_eq!(
            parse_render_state(
                "/* header */ { blend /* a */ ONE ZERO // b\n cull FRONT }",
                None
            )
            .unwrap(),
            plain
        );
        for source in [
            "",
            "{",
            "{ cull GL_BACK }",
            "{ blend ONE }",
            "{ unsupported 1 }",
            "{ depthWrite maybe }",
        ] {
            assert!(parse_render_state(source, None).is_err(), "{source}");
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn world_depth_state_matches_cache_builder_and_factories() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/world_water_depth_states.txt").lines() {
            let words = line
                .split_whitespace()
                .map(|value| u32::from_str_radix(value, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 43);
            let source = words[1..10].try_into().unwrap();
            let state = super::world_depth_state(source, words[0] as u8);
            assert_eq!(state, words[10..19], "world depth case{count}");
            assert_eq!(super::parser_tests::descriptor_fields(&state), words[19..]);
            count += 1;
        }
        assert_eq!(count, 256);
    }

    #[test]
    fn sun_sprite_blend_preserves_installed_state_overrides() {
        for line in include_str!("../../tests/fixtures/sun_sprite_state.txt").lines() {
            let words = line
                .split_whitespace()
                .map(|value| u32::from_str_radix(value, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 19);
            let state: [u32; 9] = words[1..10].try_into().unwrap();
            assert_eq!(
                super::sun_sprite_draw_state(state, words[0] != 0),
                words[10..]
            );
        }
    }
    use super::*;

    #[test]
    fn world_state_leaves_match_full_execution() {
        let mut cases = 0;
        for (case, line) in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/world_shadow_state_cases.txt"
        ))
        .lines()
        .enumerate()
        {
            let words: Vec<_> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            assert_eq!(words.len(), 24);
            let source: [u32; 9] = words[1..10].try_into().unwrap();
            let face = words[10] as u8;
            let bias = f32::from_bits(words[11]);
            let slope = f32::from_bits(words[12]);
            let clamp = f32::from_bits(words[13]);
            let clip = words[14] as u8;
            let actual = match words[0] {
                0x293470 => world_shadow_draw_state(source, face, bias, slope, clamp, clip),
                0x2934e0 => world_state_2934e0(source),
                0x293500 => world_state_293500(source),
                0x293510 => sky_cloud_draw_state(source),
                0x293540 => world_state_293540(source, clamp),
                0x293570 => world_state_293570(source),
                operation => panic!("unknown original leaf {operation:x}"),
            };
            assert_eq!(
                &actual,
                &words[15..24],
                "original world state case{case}, leaf{:x}",
                words[0]
            );
            cases += 1;
        }
        assert_eq!(cases, 1536);
    }

    #[test]
    fn sampler_descriptors_match_executable() {
        let mut count = 0;
        for (case, line) in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sampler_descriptor_cases.txt"
        ))
        .lines()
        .enumerate()
        {
            let words: Vec<_> = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect();
            assert_eq!(words.len(), 23);
            let input: [u32; 9] = words[1..10].try_into().unwrap();
            assert_eq!(
                sampler_descriptor(&input, f32::from_bits(words[0])),
                words[10..],
                "original sampler case {case}"
            );
            count += 1;
        }
        assert_eq!(count, 768);
    }

    #[test]
    fn main_depth_stencil_bit_tracks_actual_depth_hack_not_mesh_type() {
        let mut source = [0; 9];
        reset_render_state(&mut source);
        for hack in [0.0, -0.0, -1.0, f32::NEG_INFINITY, f32::NAN] {
            let (state, reference) = main_depth_surface_state(source, hack);
            assert_eq!(reference, 0);
            assert_eq!(&state[1..], &source[1..]);
        }
        for hack in [f32::MIN_POSITIVE, 0.01, 1.0, f32::INFINITY] {
            let (state, reference) = main_depth_surface_state(source, hack);
            let depth = depth_stencil_state(&state);
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
                depth_stencil_state(&source).depth_compare
            );
            assert_eq!(depth.depth_write, depth_stencil_state(&source).depth_write);
            assert_eq!(&state[6..], &source[6..]);
        }
        for flags in 0..=255 {
            let state = prepared_surface_state([u32::MAX; 9], flags);
            assert_eq!(raster_state(&state).front_counter_clockwise, flags & 1 != 0);
            assert_eq!(state[5] & !0x0800_0000, u32::MAX & !0x0800_0000);
            assert!(state[..5]
                .iter()
                .chain(&state[6..])
                .all(|word| *word == u32::MAX));
        }
    }

    #[test]
    fn enum_defaults_match_switches() {
        for code in 0..256 {
            assert_eq!(compare_function(code), if code < 7 { code + 1 } else { 8 });
            assert_eq!(
                stencil_operation(code),
                if (1..=7).contains(&code) { code + 1 } else { 1 }
            );
        }
    }

    #[test]
    fn screen_draws_only_green_and_split_stencil_bit_one() {
        for source in [[0; 9], [u32::MAX; 9], [0xdead_beef; 9]] {
            let passes = screen_shadow_pass_states(source);
            for (index, state) in passes.iter().enumerate() {
                let blend = blend_state(state).targets[0].unwrap();
                assert!(!blend.enabled);
                assert_eq!(blend.write_mask, 2);
                let depth = depth_stencil_state(state);
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
                assert_eq!(raster_state(state).cull_mode, 1);
                assert_eq!(&state[6..], &source[6..]);
            }
        }
    }

    #[test]
    fn all_three_factories_match_machine_code() {
        let mut cases = 0;
        let mut enums = 0;
        let mut resets = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/render_state_cases.txt"
        ))
        .lines()
        {
            let mut parts = line.split_whitespace();
            let kind = parts.next().unwrap();
            let values: Vec<u32> = parts.map(|s| u32::from_str_radix(s, 16).unwrap()).collect();
            if kind == "ENUM" {
                assert_eq!(compare_function(values[0]), values[1]);
                assert_eq!(stencil_operation(values[0]), values[2]);
                enums += 1;
                continue;
            }
            let mut state: RenderState = values[..9].try_into().unwrap();
            if kind == "RESET" {
                reset_render_state(&mut state);
                assert_eq!(state.as_slice(), &values[9..], "state reset case {resets}");
                resets += 1;
                continue;
            }
            let blend = blend_state(&state);
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
            let depth = depth_stencil_state(&state);
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
            let raster = raster_state(&state);
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
            assert_eq!(actual, values, "descriptor case {cases}");
            cases += 1;
        }
        assert_eq!((cases, enums, resets), (1286, 259, 1026));
    }

    #[test]
    fn independent_target_tail_is_default_not_target_zero() {
        let mut state = [0; 9];
        state[4] = 3;
        let blend = blend_state(&state);
        assert!(blend.alpha_to_coverage && blend.independent_blend);
        assert_eq!(blend.targets[0].unwrap().write_mask, 0);
        assert_eq!(
            blend.targets[4].unwrap(),
            BlendTarget::UNUSED_INDEPENDENT_TARGET
        );
        state[4] = 0;
        assert!(blend_state(&state).targets[1..].iter().all(Option::is_none));
    }
}
