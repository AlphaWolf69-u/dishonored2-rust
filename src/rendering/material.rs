//! Resolve a Void material's base-colour image from its text declaration.

/// Ordinary material constructor's three banks (1402d66a0..1402d66f0).
/// GetInteger reads the unmodified material block/registered defaults here,
/// not the later pass-specialized dictionary. Bank2 is the world +0x250 bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstancingBankOverride {
    Unchanged,
    Direct,
    Multi,
}

pub fn instancing_bank_override(
    bank: u8,
    use_multi: bool,
    use_direct: bool,
) -> Result<Option<InstancingBankOverride>, String> {
    match bank {
        0 => Ok(Some(InstancingBankOverride::Unchanged)),
        1 if use_multi => Ok(None),
        1 => Ok(Some(InstancingBankOverride::Direct)),
        2 if use_direct => Ok(None),
        2 => Ok(Some(InstancingBankOverride::Multi)),
        _ => Err("native material has exactly three instancing banks".into()),
    }
}

/// The source of a shadow shader specialization, not an engine pointer or a
/// RenderDoc resource number. Compile `isShadow` and, when enabled, `isCloth`
/// against this source before publishing an executable program bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowProgramSource {
    Disabled,
    Authored,
    MainProgram,
}

/// FUN_1402c3bc0's first render-target blend-enable bit. Blend operations do
/// not participate: ONE/ZERO for both colour and alpha disables blending.
/// This also defines the bit consumed at material +0x2c8 by 1402d5210.
pub fn native_blend_enabled(state: u32) -> bool {
    state & 0x1e != 2 || state & 0x1e0 != 0 || state & 0x7800 != 0x800 || state & 0x78000 != 0
}

/// FUN_1402d5210 at 1402d686d..1402d68bb and 1402d6924..1402d694d:
/// a blended decal of type
/// 0 or 3 does NOT specialize isAlpha. isDecal is main-program fallback
/// handle 14229d200 (name node 14229d1f0); decalType is the material-only
/// handle 14229d1e0 (node 14229d1d0), masked to two bits in +0x4a.
/// The caller supplies the already-derived first render-target blend flag.
pub fn shadow_is_alpha(
    main_blend_state: u32,
    is_decal: bool,
    decal_type: u32,
    sky_type: u32,
) -> bool {
    let suppress_decal = is_decal && matches!(decal_type & 3, 0 | 3);
    main_blend_state & 1 != 0 && !suppress_decal && sky_type & 3 == 0
}

/// Runtime material class at 1402d6e59..1402d6ee8. `implicit_main`
/// is the initial MAIN HasParm miss (the constructor's bVar5), not a missing
/// compiled shader. `invisible` is GetInteger with the MAIN RPD fallback.
/// Editor/reload paths that retain an existing class are outside this helper.
pub fn material_program_class(
    main_present: bool,
    implicit_main: bool,
    main_blend_state: u32,
    invisible: bool,
) -> u8 {
    let blended = main_blend_state & 1 != 0;
    if !main_present || invisible || (implicit_main && blended) {
        1
    } else if blended {
        2
    } else {
        3
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialProgramFinalization {
    /// Slots cleared in EACH of the three 22-entry native banks.
    pub clear_slots: u32,
    /// Class 1 clears pointers but, unlike the second branch, retains +0x48 bit0.
    pub clear_shadow_flag: bool,
}

/// Exact three-bank finalization at 1402d6eec..1402d6f39. This runs AFTER
/// selecting/compiling the sources, including authored shadow overrides.
/// Slot 11 (cloth) is not slot 10 (rigid) and is not cleared by this loop.
pub fn finalize_material_programs(
    material_class: u8,
    hair_type: bool,
    sky_type: u32,
) -> MaterialProgramFinalization {
    if material_class & 3 == 1 {
        MaterialProgramFinalization {
            clear_slots: (1 << 0) | (1 << 4) | (1 << 10),
            clear_shadow_flag: false,
        }
    } else if (material_class & 3 == 2 && !hair_type) || sky_type & 3 == 1 {
        MaterialProgramFinalization {
            clear_slots: 1 << 10,
            clear_shadow_flag: true,
        }
    } else {
        MaterialProgramFinalization {
            clear_slots: 0,
            clear_shadow_flag: false,
        }
    }
}

/// FUN_1402d5210's source resolution for slots 10 and 11 only
/// (1402d615f..1402d6288). This is not shader compilation or vertex-format
/// classification; alpha testing and custom vertex code still belong to the
/// chosen source. `override_present` distinguishes an authored null program
/// from an absent declaration, as the original HasParm test does.
/// `is_decal` is material +0x48 bit4 (handle14229d200), which suppresses
/// ALL slots except 0/6 before HasParm. It is NOT the invisible option.
/// `hair_type` is DAT_14229d340 (node 14229d330 -> "hairType" at
/// 141b17ec0), material +0x4a bit 4. It is NOT "skyType": that distinct
/// node 14229d350/handle14229d360 points to 141b17ed0.
pub fn shadow_program_source(
    cloth_slot: bool,
    no_shadow: bool,
    is_decal: bool,
    override_present: bool,
    hair_type: bool,
    main_blend_state: u32,
    cloth_enabled: bool,
) -> ShadowProgramSource {
    // The noShadow test is on slot 10 only. Do not silently extend it to the
    // authored cloth bank: the original loop handles that bank separately.
    if is_decal || (!cloth_slot && no_shadow) {
        return ShadowProgramSource::Disabled;
    }
    if override_present {
        return ShadowProgramSource::Authored;
    }
    if (hair_type || !native_blend_enabled(main_blend_state)) && (!cloth_slot || cloth_enabled) {
        ShadowProgramSource::MainProgram
    } else {
        ShadowProgramSource::Disabled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transparency {
    Opaque,
    Cutout,
    Coverage,
    Blend,
}

/// Editor/debug colour swatches are geometry guides, not authored visible
/// surfaces. The installed models use these for occluders and volume proxies.
pub fn is_editor_helper(path: &str) -> bool {
    path.starts_with("models/templates/colors/")
        || path.eq_ignore_ascii_case("NULL")
        || path.ends_with("/invisible.material")
}

/// Screen gates are distant-view impostors. Their material's black diffusemap
/// is a placeholder for a particle shader with distance fade and fx/diffuse.
/// The viewer loads the full level, so these impostors must not be drawn over it.
pub fn is_screen_gate_impostor(path: &str, bytes: &[u8]) -> bool {
    let text = crate::entities::native_declaration_text(bytes);
    path.contains("/optim/screen_gates/")
        && text.lines().any(|line| {
            let mut fields = line.split_whitespace();
            fields.next() == Some("fx/opt/view_dependent_fade") && fields.next() == Some("1")
        })
        && text
            .lines()
            .any(|line| line.trim_start().starts_with("fx/diffuse"))
}

pub fn transparency(bytes: &[u8]) -> Transparency {
    transparency_with_program(bytes, &[])
}

/// Render-program declarations contain default options, parms and state before
/// their HLSL blocks. A material need not repeat the program's blend state.
fn render_fields(bytes: &[u8]) -> std::collections::HashMap<String, String> {
    let text = crate::entities::native_declaration_text(bytes);
    let mut values = std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let mut fields = line.split_whitespace();
        let Some(key) = fields.next() else { continue };
        if key.to_ascii_lowercase().starts_with("hlsl_") {
            break;
        }
        if let Some(value) = fields.next() {
            values.insert(key.to_ascii_lowercase(), value.to_ascii_lowercase());
        }
    }
    values
}

/// Material-only numeric Option overrides. RPD defaults remain a fallback;
/// flattening program source text here would override its compiled defaults.
/// The declaration registry, not spelling or texture values, determines kind.
pub fn material_option_parameters(
    bytes: &[u8],
    is_option: impl FnMut(&str) -> bool,
) -> Result<Vec<(String, i32)>, String> {
    // Native FUN_1403446d0 reads tokens, not lines. In real materials an
    // alphaTest pair may share a line with hasBumpMap; dropping the latter
    // selects a nonexistent specialization and blocks the whole shadow bank.
    crate::entities::material_integer_parameters(
        &crate::entities::native_declaration_text(bytes),
        is_option,
    )
}

/// Slot 10's numeric overrides, after reading the UNMODIFIED material block.
/// 1402d68bb..1402d697d computes noScreenShadows from isAlpha OR the selected
/// source's noScreenShadows/translucency_hq/enable/translucency/enable options.
/// The caller resolves those fallbacks before this function overwrites keys.
/// Alpha-test bit 0 instead comes from MAIN (material +0x4b bit 4).
/// This is not the complete parameter-block parser or the three-row bank.
pub fn rigid_shadow_option_parameters(
    bytes: &[u8],
    native_alpha_test: i32,
    native_is_alpha: bool,
    native_no_screen_shadows: bool,
    is_option: impl FnMut(&str) -> bool,
) -> Result<Vec<(String, i32)>, String> {
    let mut options = material_option_parameters(bytes, is_option)?;
    // Last-wins overrides; signature order comes from the program handles.
    options.push(("isAlpha".into(), i32::from(native_is_alpha)));
    options.push((
        "noScreenShadows".into(),
        i32::from(native_no_screen_shadows),
    ));
    options.push(("isShadow".into(), 1));
    if native_alpha_test & 1 != 0 {
        options.push(("forcePixelShader".into(), 1));
    }
    Ok(options)
}

/// FUN_1402c4a70 token values; FUN_1402c3bc0 defaults bits 24..25 to 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CullMode {
    None = 0,
    Front = 1,
    Back = 2,
}

pub fn parse_cull_mode(token: &str) -> Result<CullMode, String> {
    match token.to_ascii_uppercase().as_str() {
        "NONE" => Ok(CullMode::None),
        "FRONT" => Ok(CullMode::Front),
        "BACK" => Ok(CullMode::Back),
        _ => Err(format!("bad cull mode {token}")),
    }
}

pub fn cull_mode_with_program(bytes: &[u8], program: &[u8]) -> Result<CullMode, String> {
    let mut values = render_fields(program);
    let mut authored = render_fields(bytes);
    // FUN_1402d7b30 converts these legacy raster aliases, not twoSidedLighting.
    if authored
        .get("version")
        .and_then(|v| v.parse::<u32>().ok())
        .is_some_and(|v| v < 3)
    {
        if authored.get("twosided").is_some_and(|v| v == "1") {
            authored.insert("cull".into(), "none".into());
        }
        if authored.get("backsided").is_some_and(|v| v == "1") {
            authored.insert("cull".into(), "front".into());
        }
    }
    values.extend(authored);
    values
        .get("cull")
        .map_or(Ok(CullMode::Back), |v| parse_cull_mode(v))
}

pub fn program_option(bytes: &[u8], program: &[u8], key: &str) -> bool {
    let mut values = render_fields(program);
    values.extend(render_fields(bytes));
    values
        .get(&key.to_ascii_lowercase())
        .is_some_and(|v| v == "1")
}

/// FUN_1402c4630: uppercase, repeatedly remove leading GL_, then resolve the
/// original ten factors. FUN_140145120's prefix removal is equivalent to this
/// strip_prefix loop; it does not remove GL_ in the middle of a token.
pub fn parse_blend_factor(token: &str) -> Result<u32, String> {
    let upper = token.to_ascii_uppercase();
    let mut token = upper.as_str();
    while let Some(rest) = token.strip_prefix("GL_") {
        token = rest;
    }
    match token {
        "ZERO" => Ok(0),
        "ONE" => Ok(1),
        "SRC_COLOR" => Ok(2),
        "ONE_MINUS_SRC_COLOR" => Ok(3),
        "SRC_ALPHA" => Ok(4),
        "ONE_MINUS_SRC_ALPHA" => Ok(5),
        "DST_ALPHA" => Ok(6),
        "ONE_MINUS_DST_ALPHA" => Ok(7),
        "DST_COLOR" => Ok(8),
        "ONE_MINUS_DST_COLOR" => Ok(9),
        _ => Err(format!("bad blend mode {token}")),
    }
}

/// The factor fields consumed by FUN_1402d5210's shadow-source selector.
/// FUN_1402c3bc0 initializes each target to 0x01e00802 and applies program
/// state followed by material overrides. Sort, operations and alpha coverage
/// do NOT enable blending. This is not the complete nine-word raster state.
pub fn shadow_blend_factor_state(bytes: &[u8], program: &[u8]) -> Result<u32, String> {
    shadow_blend_factor_overrides(shadow_blend_factor_overrides(0x01e0_0802, program)?, bytes)
}

/// Apply material render-state overrides to the actual MAIN RPD state.
/// The source-text wrapper above is useful without a resident library, but
/// production must not replace the serialized program's state with defaults.
pub fn shadow_blend_factor_overrides(mut word: u32, source: &[u8]) -> Result<u32, String> {
    let text = crate::entities::native_declaration_text(source);
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let mut tokens = line.split_whitespace();
        let Some(key) = tokens.next() else {
            continue;
        };
        let key = key.to_ascii_lowercase();
        if key.starts_with("hlsl_") {
            break;
        }
        let (mask, shift) = match key.as_str() {
            "blend" => (0x1fe, 1),
            "alphablend" => (0x7f800, 11),
            _ => continue,
        };
        let mut a = parse_blend_factor(
            tokens
                .next()
                .ok_or_else(|| format!("missing {key} source factor"))?,
        )?;
        let mut b = parse_blend_factor(
            tokens
                .next()
                .ok_or_else(|| format!("missing {key} destination factor"))?,
        )?;
        if key == "alphablend" {
            // Machine code at 1402c3ef6..3f38 / 3f60..3fa2 proves
            // colour->alpha substitution after each diagnostic call.
            // Ghidra wrongly treats those calls as noreturn; 140166830
            // is a returning no-op in this executable, so no log is needed.
            let alpha = |factor| match factor {
                2 => 4,
                3 => 5,
                8 => 6,
                9 => 7,
                _ => factor,
            };
            a = alpha(a);
            b = alpha(b);
        }
        word = (word & !mask) | ((a | (b << 4)) << shift);
    }
    word = (word & !1) | u32::from(native_blend_enabled(word));
    Ok(word)
}

/// Preserve a material's authored program override presence, including a
/// quoted empty value. FUN_140343c90's HasParm distinguishes absent and null;
/// render-program defaults do not create an authored shadow override.
pub fn authored_program_override(bytes: &[u8], key: &str) -> Option<String> {
    render_fields(bytes)
        .get(&key.to_ascii_lowercase())
        .map(|value| value.trim_matches('"').to_owned())
}

/// Inputs resolved by FUN_1402d5210 for the original rigid shadow bank.
/// This records source selection, not a guess about an arbitrary vertex shader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShadowSourceInputs {
    pub source: ShadowProgramSource,
    pub cloth_source: ShadowProgramSource,
    /// Source option behind material +0x49 bit 0 (slot 4), NOT alphaTest.
    pub enable_refraction: bool,
    /// Source option behind material +0x49 bit 1, consumed by shadow bias.
    pub translucency_hq: bool,
    /// Low bit of materialEffects/enable, material +0x49 bit 6.
    /// The actual specialized override banks still must be compiled.
    pub material_effects: bool,
    pub override_program: Option<String>,
    pub cloth_override_program: Option<String>,
    pub main_blend_state: u32,
    /// Actual MAIN definition inherited by the material's drawing-state block
    /// (1402d662d -> 1402c3bc0, material +0x2c8). A textual source-only parse
    /// cannot supply the registered program's other eight words.
    pub main_render_state: Option<crate::render_state::NativeRenderState>,
    pub hair_type: bool,
    pub sky_type: u32,
    pub is_cloth: bool,
    pub use_skinning: bool,
    pub use_skinning8: bool,
    /// MAIN-fallback needsWind, handle14229d220/node14229d210; +0x48 bit6.
    /// This controls world auxiliary records, not an inferred shader family.
    pub needs_wind: bool,
}

pub fn shadow_source_inputs(bytes: &[u8], program: &[u8]) -> Result<ShadowSourceInputs, String> {
    let mut values = render_fields(program);
    values.extend(render_fields(bytes));
    let integer = |key: &str| -> Result<i32, String> {
        values.get(key).map_or(Ok(0), |value| {
            value
                .parse::<i32>()
                .map_err(|_| format!("unsupported shadow parameter {key}={value}"))
        })
    };
    let no_shadow = integer("noshadow")? != 0;
    let is_decal = integer("isdecal")? & 1 != 0;
    // Constructor masks bit 0 of hairType and the two low bits of skyType.
    let hair_type = integer("hairtype")? & 1 != 0;
    let sky_type = integer("skytype")? as u32 & 3;
    let is_cloth = integer("iscloth")? & 1 != 0;
    let main_blend_state = shadow_blend_factor_state(bytes, program)?;
    let override_program = authored_program_override(bytes, "shadowProgram");
    let cloth_override_program = authored_program_override(bytes, "shadowClothProgram");
    let source = shadow_program_source(
        false,
        no_shadow,
        is_decal,
        override_program.is_some(),
        hair_type,
        main_blend_state,
        is_cloth,
    );
    let cloth_source = shadow_program_source(
        true,
        no_shadow,
        is_decal,
        cloth_override_program.is_some(),
        hair_type,
        main_blend_state,
        is_cloth,
    );
    Ok(ShadowSourceInputs {
        source,
        cloth_source,
        enable_refraction: integer("alpha/enablerefraction")? != 0,
        translucency_hq: integer("translucency_hq/enable")? != 0,
        material_effects: integer("materialeffects/enable")? & 1 != 0,
        override_program,
        cloth_override_program,
        main_blend_state,
        main_render_state: None,
        hair_type,
        sky_type,
        is_cloth,
        use_skinning: integer("useskinning")? != 0,
        use_skinning8: integer("useskinning8")? != 0,
        needs_wind: integer("needswind")? & 1 != 0,
    })
}

impl ShadowSourceInputs {
    /// FUN_1402996f0 copies material +0x2c8's nine words, changes winding from
    /// object +0x10, then calls 140293470. Do not replace inherited cull,
    /// stencil or depth fields with the shadow program's/default state.
    pub fn world_draw_state(
        &self,
        surface_flags: u8,
        depth_bias: f32,
        slope_scale: f32,
        clamp: f32,
        depth_clip: u8,
    ) -> Result<crate::render_state::NativeRenderState, String> {
        let state = self
            .main_render_state
            .ok_or("native world shadow material has no resolved MAIN drawing state")?;
        Ok(crate::render_state::world_shadow_draw_state(
            state,
            surface_flags,
            depth_bias,
            slope_scale,
            clamp,
            depth_clip,
        ))
    }

    /// The two +0x48 bits read by FUN_1402996f0's world gather. The parser
    /// sets bit 6 from the low bit of needsWind (FUN_1402d5210;
    /// analysis/decompiled/chunk_000113.c:2021–2022),
    /// and finalization sets bit 0 from the ACTUAL base shadow program pointer
    /// at material +0xf0 (not the world bank +0x250 or a resident-cache hit).
    /// The owner supplies pointer existence; missing serialized bytecode is
    /// not evidence of a null pointer. All unrelated material bits survive.
    pub fn shadow_gather_flags48(&self, existing: u8, base_shadow_program_exists: bool) -> u8 {
        (existing & !0x41) | u8::from(base_shadow_program_exists) | (u8::from(self.needs_wind) << 6)
    }

    /// The three material +0x49 bits consumed by the native depth/shadow draw
    /// selectors. 1402d5210 reads Alpha/EnableRefraction in slot 4 via
    /// handle 14229d580, translucency_hq/enable via 14229d700, and
    /// materialEffects/enable via 14229d5a0 before constructing effect banks.
    /// This does not synthesize program handles or claim a complete runtime
    /// material flags byte: the other bits belong to different draw paths.
    pub fn shadow_draw_flags49(&self) -> u8 {
        u8::from(self.enable_refraction)
            | (u8::from(self.translucency_hq) << 1)
            | (u8::from(self.material_effects) << 6)
    }
}

pub fn transparency_with_program(bytes: &[u8], program: &[u8]) -> Transparency {
    let mut values = render_fields(program);
    values.extend(render_fields(bytes));
    let cutout = values.get("alphatest").is_some_and(|v| v == "1");
    let sort = values.get("stagesort").map(String::as_str);
    // Explicit blend state is independent of stage ordering. In particular,
    // arkDecal defaults to SRC_ALPHA even if a material changes stageSort.
    let blend = values.get("blend").map_or(
        matches!(sort, Some("sorttrans" | "sortwater" | "sortdecal")),
        |v| v == "src_alpha",
    );
    if blend {
        Transparency::Blend
    } else if sort == Some("sortcoverage") {
        Transparency::Coverage
    } else if cutout {
        Transparency::Cutout
    } else {
        Transparency::Opaque
    }
}

/// FUN_140141ad0: any dot after the last separator suppresses the extension.
/// Options such as `$nomips` stay part of the source name.
pub fn append_default_extension(name: &str, extension: &str) -> String {
    if name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .contains('.')
    {
        name.to_owned()
    } else {
        format!(
            "{name}{}{extension}",
            if extension.starts_with('.') { "" } else { "." }
        )
    }
}

/// FUN_1402f1480's constant-color token spelling and FUN_1402ebcd0's suffix.
/// Unsupported image-program branches are not mapped to guessed filenames.
pub fn image_source_name(name: &str) -> Option<String> {
    let name = name.trim().trim_matches('"');
    if name
        .get(..4)
        .is_some_and(|v| v.eq_ignore_ascii_case("ipr_"))
    {
        let (function, args) = name.split_once('(')?;
        if !function.eq_ignore_ascii_case("ipr_constantcolor") {
            return None;
        }
        let args: Vec<_> = args.strip_suffix(')')?.split(',').map(str::trim).collect();
        if args.len() != 4
            || args.iter().any(|v| {
                v.starts_with('-')
                    || v.starts_with('+')
                    || v.parse::<f32>().ok().is_none_or(|f| !f.is_finite())
            })
        {
            return None;
        }
        Some(format!(
            "{function}( {}, {}, {}, {}).imageprog",
            args[0], args[1], args[2], args[3]
        ))
    } else if !name.is_empty() && !name.starts_with('_') && !name.starts_with('#') {
        Some(append_default_extension(name, "tga"))
    } else {
        None
    }
}

/// FUN_1402f1480's constant-color generation branch: truncate float*255,
/// clamp bytes, and repeat the pixel across a 4x4 RGBA8 image. The finite
/// representable-input guard belongs to this Rust boundary, not native parsing.
pub fn constant_color_pixels(color: [f32; 4]) -> Option<[u8; 64]> {
    let scaled = color.map(|value| value * 255.0);
    if scaled
        .iter()
        .any(|&v| !v.is_finite() || !(-2147483648.0..2147483648.0).contains(&v))
    {
        return None;
    }
    let pixel = scaled.map(|value| (value as i32).clamp(0, 255) as u8);
    Some(std::array::from_fn(|i| pixel[i % 4]))
}

/// Prefer the source->compiled mapping stored by the game itself.
pub fn compiled_image_stem(catalog: &crate::archive::Catalog, name: &str) -> Option<String> {
    if matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "_default" | "_black" | "_white"
    ) {
        return Some(name.trim().to_ascii_lowercase());
    }
    if name
        .trim()
        .get(..4)
        .is_some_and(|s| s.eq_ignore_ascii_case("ipr_"))
    {
        if let Some(resource) = catalog.image_program_resource(name) {
            return Some(resource.into());
        }
    }
    if let Some(source) = image_source_name(name) {
        if let Some(resource) = catalog.image_resource(&source) {
            return Some(resource.to_owned());
        }
    }
    // Older extensionless ordinary resources remain valid. Do not discard
    // unresolved options or invent a compiled image-program filename.
    if name.contains('$')
        || name
            .get(..4)
            .is_some_and(|v| v.eq_ignore_ascii_case("ipr_"))
    {
        return None;
    }
    let declaration = format!("diffusemap {name}");
    diffuse_image(declaration.as_bytes())?
        .strip_suffix("_mip0")
        .map(|path| path.replace('\\', "/"))
}

/// Image_intrinsic.cpp: FUN_1402f5fc0, 1402f5630 and 1402f6a60.
/// Intrinsics use linear RGBA8 (native packed format0x54a), not a white
/// unbound-texture placeholder. The default is a16x16 grey image/white border.
pub fn intrinsic_image_pixels(name: &str) -> Option<Vec<u8>> {
    let name = name.to_ascii_lowercase();
    if !matches!(name.as_str(), "_default" | "_black" | "_white") {
        return None;
    }
    Some(
        (0..16 * 16)
            .flat_map(|i| {
                if name == "_black" {
                    [0; 4]
                } else if name == "_white"
                    || i / 16 == 0
                    || i / 16 == 15
                    || i % 16 == 0
                    || i % 16 == 15
                {
                    [255; 4]
                } else {
                    [32, 32, 32, 255]
                }
            })
            .collect(),
    )
}

#[test]
fn intrinsic_images_match_original_generator_pixels() {
    let default = intrinsic_image_pixels("_default").unwrap();
    assert_eq!(default.len(), 1024);
    // Original1402f5fc0 captured by analysis/probe_original_default_image.py:
    // all six cube faces and sixteen volume slices have this same16x16 image.
    for (i, p) in default.chunks_exact(4).enumerate() {
        assert_eq!(
            p,
            if i / 16 == 0 || i / 16 == 15 || i % 16 == 0 || i % 16 == 15 {
                &[255; 4]
            } else {
                &[32, 32, 32, 255]
            }
        );
    }
    assert!(intrinsic_image_pixels("_white")
        .unwrap()
        .iter()
        .all(|v| *v == 255));
    assert!(intrinsic_image_pixels("_black")
        .unwrap()
        .iter()
        .all(|v| *v == 0));
    assert!(intrinsic_image_pixels("_unknown").is_none());
}

pub fn diffuse_image(bytes: &[u8]) -> Option<String> {
    let text = crate::entities::native_declaration_text(bytes);
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let mut fields = line.split_whitespace();
        if fields
            .next()
            .is_some_and(|key| key.eq_ignore_ascii_case("diffusemap"))
        {
            let path = fields.next()?.trim_matches('"');
            if !path.is_empty() && !path.starts_with('#') {
                // Legacy extensionless fallback. Live resolution first uses
                // compiled_image_stem and the complete original source alias.
                let path = path.split('$').next().unwrap_or(path);
                let path = path
                    .strip_suffix(".tga")
                    .or_else(|| path.strip_suffix(".TGA"))
                    .or_else(|| path.strip_suffix(".dds"))
                    .unwrap_or(path);
                return Some(format!("generated/image/{path}.bimage7_mip0"));
            }
        }
    }
    None
}

/// Void's explicit layer-zero tint, expressed in linear colour space.
pub fn layer0_color_constant(bytes: &[u8]) -> Option<[f32; 4]> {
    let text = crate::entities::native_declaration_text(bytes);
    let mut enabled = false;
    let mut color = None;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let mut fields = line.split_whitespace();
        let Some(key) = fields.next() else {
            continue;
        };
        match key {
            key if key.eq_ignore_ascii_case("use_layer0_colorconstant") => {
                enabled = fields.next() == Some("1");
            }
            key if key.eq_ignore_ascii_case("layer0_colorconstant") => {
                let values = line.split(['{', '}']).nth(1)?;
                let numbers = values
                    .split(',')
                    .map(|value| value.trim().parse::<f32>())
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?;
                if numbers.len() == 4 && numbers.iter().all(|n| n.is_finite()) {
                    color = Some([numbers[0], numbers[1], numbers[2], numbers[3]]);
                }
            }
            _ => {}
        }
    }
    enabled.then_some(color).flatten()
}

/// Which packed vertex-colour channel masks layer zero, if the material uses one.
pub fn layer0_vertex_mask_channel(bytes: &[u8]) -> Option<usize> {
    let text = crate::entities::native_declaration_text(bytes);
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let mut fields = line.split_whitespace();
        if fields
            .next()
            .is_some_and(|key| key.eq_ignore_ascii_case("layer0_vtxcolormaskchannel"))
        {
            return fields
                .next()?
                .parse::<usize>()
                .ok()
                .filter(|&channel| channel < 4);
        }
    }
    None
}

pub fn decal_depth_bias(bytes: &[u8]) -> Option<f32> {
    let text = crate::entities::native_declaration_text(bytes);
    let mut decal = false;
    let mut original = None;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("mainProgram") => decal = fields.next() == Some("arkDecal"),
            Some("zbias") => original = fields.next().and_then(|value| value.parse::<f32>().ok()),
            _ => {}
        }
    }
    // arkDecal.hlsl_vp_main subtracts zbias.x from clip-space Z BEFORE
    // perspective division. Keep its sign and scale; a rasterizer offset
    // instead changes the result with the depth-buffer format and distance.
    if decal {
        Some(original.unwrap_or(0.0))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_material_bank_overrides_preserve_skip_and_set_order() {
        use InstancingBankOverride::*;
        for (multi, direct, expected) in [
            (false, false, [Some(Unchanged), Some(Direct), Some(Multi)]),
            (true, false, [Some(Unchanged), None, Some(Multi)]),
            (false, true, [Some(Unchanged), Some(Direct), None]),
            (true, true, [Some(Unchanged), None, None]),
        ] {
            for bank in 0..3 {
                assert_eq!(
                    instancing_bank_override(bank, multi, direct).unwrap(),
                    expected[bank as usize]
                );
            }
        }
        assert!(instancing_bank_override(3, false, false).is_err());
        assert!(
            !shadow_source_inputs(b"needsWind 2\n", b"")
                .unwrap()
                .needs_wind
        );
        assert!(
            shadow_source_inputs(b"needsWind 3\n", b"")
                .unwrap()
                .needs_wind
        );
        assert!(
            shadow_source_inputs(b"", b"needsWind 1\n")
                .unwrap()
                .needs_wind
        );
        assert!(
            !shadow_source_inputs(b"needsWind 0\n", b"needsWind 1\n")
                .unwrap()
                .needs_wind
        );
        for wind in [false, true] {
            let mut inputs = shadow_source_inputs(b"", b"").unwrap();
            inputs.needs_wind = wind;
            for existing in 0..=u8::MAX {
                for base_program_exists in [false, true] {
                    let flags = inputs.shadow_gather_flags48(existing, base_program_exists);
                    assert_eq!(flags & !0x41, existing & !0x41);
                    assert_eq!(flags & 1 != 0, base_program_exists);
                    assert_eq!(flags & 0x40 != 0, wind);
                }
            }
        }
    }

    #[test]
    fn shadow_alpha_decal_suppression_matches_original_instructions() {
        // Captured by analysis/probe_original_shadow_alpha.py. Only the
        // original constructor's predicate blocks execute, not the whole ctor.
        let mut count = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/shadow_alpha_cases.txt"
        ))
        .lines()
        {
            let mut fields = line.split_ascii_whitespace();
            let blend = u32::from_str_radix(fields.next().unwrap(), 16).unwrap();
            let decal = fields.next().unwrap().parse::<u32>().unwrap() != 0;
            let kind = fields.next().unwrap().parse().unwrap();
            let sky = fields.next().unwrap().parse().unwrap();
            let expected = fields.next().unwrap().parse::<u32>().unwrap() != 0;
            assert!(fields.next().is_none());
            assert_eq!(shadow_is_alpha(blend, decal, kind, sky), expected, "{line}");
            count += 1;
        }
        assert_eq!(count, 4096);
    }

    #[test]
    fn runtime_material_class_uses_main_presence_and_native_blend_bit() {
        for main in [false, true] {
            for implicit in [false, true] {
                for invisible in [false, true] {
                    for word in [0, 1, 0x01e0_0802, 0x01e0_08a9, 0xffff_fffe, 0xffff_ffff] {
                        let class = material_program_class(main, implicit, word, invisible);
                        let expected = match (main, implicit, word & 1 != 0, invisible) {
                            (false, _, _, _) | (_, _, _, true) | (_, true, true, _) => 1,
                            (_, _, true, _) => 2,
                            _ => 3,
                        };
                        assert_eq!(class, expected);
                    }
                }
            }
        }
    }

    #[test]
    fn material_finalization_matches_original_three_bank_instructions() {
        // Only 1402d6eec..1402d6f39 executes in the probe: no constructor,
        // compiler callbacks, input or live game memory writes.
        let mut count = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/material_program_finalization_cases.txt"
        ))
        .lines()
        {
            let fields: Vec<_> = line.split_ascii_whitespace().collect();
            assert_eq!(fields.len(), 6);
            let class = fields[0].parse().unwrap();
            let hair = fields[1] == "1";
            let sky = fields[2].parse().unwrap();
            let before = u8::from_str_radix(fields[3], 16).unwrap();
            let mask = u32::from_str_radix(fields[4], 16).unwrap();
            let after = u8::from_str_radix(fields[5], 16).unwrap();
            let result = finalize_material_programs(class, hair, sky);
            assert_eq!(result.clear_slots, mask, "{line}");
            assert_eq!(
                if result.clear_shadow_flag {
                    before & !1
                } else {
                    before
                },
                after,
                "{line}"
            );
            count += 1;
        }
        assert_eq!(count, 2048);
    }

    #[test]
    fn material_finalization_preserves_cloth_and_class_one_shadow_flag() {
        for class in 0..16 {
            for hair in [false, true] {
                for sky in 0..16 {
                    let result = finalize_material_programs(class, hair, sky);
                    assert_eq!(result.clear_slots & (1 << 11), 0);
                    if class & 3 == 1 {
                        assert_eq!(result.clear_slots, 0x411);
                        assert!(!result.clear_shadow_flag);
                    } else {
                        let clears = (class & 3 == 2 && !hair) || sky & 3 == 1;
                        assert_eq!(result.clear_slots, if clears { 0x400 } else { 0 });
                        assert_eq!(result.clear_shadow_flag, clears);
                    }
                }
            }
        }
        // Live native material header: flags48..4b=0e,00,00,88,
        // MAIN state=01e008a9; all three rigid/cloth shadow slots were null.
        let class = material_program_class(true, false, 0x01e0_08a9, false);
        assert_eq!(class, (0x88 >> 2) & 3);
        assert_eq!(
            finalize_material_programs(class, false, 0).clear_slots,
            1 << 10
        );
    }

    #[test]
    fn native_shadow_blend_factors_inherit_without_sort_inference() {
        let tokens = [
            "ZERO",
            "ONE",
            "SRC_COLOR",
            "ONE_MINUS_SRC_COLOR",
            "SRC_ALPHA",
            "ONE_MINUS_SRC_ALPHA",
            "DST_ALPHA",
            "ONE_MINUS_DST_ALPHA",
            "DST_COLOR",
            "ONE_MINUS_DST_COLOR",
        ];
        for (factor, name) in tokens.iter().enumerate() {
            assert_eq!(parse_blend_factor(name).unwrap(), factor as u32);
            assert_eq!(
                parse_blend_factor(&format!("gl_gl_{}", name.to_ascii_lowercase())).unwrap(),
                factor as u32
            );
        }
        assert!(parse_blend_factor("SRC_ALPHA_SATURATE").is_err());
        assert!(parse_blend_factor("ONE_GL_").is_err());
        let default =
            shadow_blend_factor_state(b"stageSort sortTrans\nalphaCoverage 1\n", b"").unwrap();
        assert_eq!(default, 0x01e00802);
        assert!(!native_blend_enabled(default));
        let program = b"blend SRC_ALPHA ONE_MINUS_SRC_ALPHA\nalphaBlend ONE ZERO\nhlsl_prefix {\nblend invalid ignored\n";
        assert!(native_blend_enabled(
            shadow_blend_factor_state(b"", program).unwrap()
        ));
        assert_eq!(
            shadow_blend_factor_state(b"blend ONE ZERO\n", program).unwrap(),
            default
        );
        let remapped =
            shadow_blend_factor_state(b"alphaBlend SRC_COLOR ONE_MINUS_DST_COLOR\n", b"").unwrap();
        assert_eq!((remapped >> 11) & 15, 4);
        assert_eq!((remapped >> 15) & 15, 7);
        assert!(native_blend_enabled(remapped));
        assert_eq!(
            authored_program_override(b"shadowProgram \"\"\n", "shadowProgram"),
            Some(String::new())
        );
        assert_eq!(authored_program_override(b"", "shadowProgram"), None);
        assert!(shadow_blend_factor_state(b"blend ONE\n", b"").is_err());
        use super::ShadowProgramSource::*;
        let blended = b"blend SRC_ALPHA ONE_MINUS_SRC_ALPHA\n";
        assert_eq!(shadow_source_inputs(b"", blended).unwrap().source, Disabled);
        let hair = shadow_source_inputs(b"hairType 1\n", blended).unwrap();
        assert_eq!(hair.source, MainProgram);
        let sky = shadow_source_inputs(b"skyType 1\n", blended).unwrap();
        assert_eq!(sky.sky_type, 1);
        assert_eq!(sky.source, Disabled); // skyType is NOT the native hairType branch.
        assert_eq!(
            shadow_source_inputs(b"noShadow 2\nshadowProgram \"custom\"\n", b"")
                .unwrap()
                .source,
            Disabled
        );
        assert_eq!(
            shadow_source_inputs(b"shadowProgram \"\"\n", b"")
                .unwrap()
                .source,
            Authored
        );
        assert!(shadow_source_inputs(b"hairType unknown\n", b"").is_err());
    }

    #[test]
    fn decal_slot_suppression_precedes_authored_programs() {
        // 1402d61a0..: +0x48 bit4 suppresses ALL slots except 0/6,
        // before HasParm; neither rigid nor cloth overrides bypass it.
        let source = shadow_source_inputs(
            b"isDecal 1\nshadowProgram \"custom\"\nshadowClothProgram \"cloth\"\nisCloth 1\n",
            b"",
        )
        .unwrap();
        assert_eq!(source.source, ShadowProgramSource::Disabled);
        assert_eq!(source.cloth_source, ShadowProgramSource::Disabled);
        // Invisible affects draw eligibility elsewhere; it is not this flag.
        assert_eq!(
            shadow_source_inputs(b"invisible 1\n", b"").unwrap().source,
            ShadowProgramSource::MainProgram
        );
    }

    #[test]
    fn shadow_material_blend_overrides_retain_actual_rpd_state() {
        let rpd = 0x01e0_08a9;
        assert_eq!(shadow_blend_factor_overrides(rpd, b"").unwrap(), rpd);
        assert_eq!(
            shadow_blend_factor_overrides(rpd, b"blend ONE ZERO\n").unwrap(),
            0x01e0_0802
        );
        assert_eq!(
            shadow_blend_factor_overrides(0x01e0_0802, b"blend SRC_ALPHA ONE_MINUS_SRC_ALPHA\n")
                .unwrap(),
            rpd
        );
        // Explicit blend operations do not set the first blend-enable flag.
        assert_eq!(
            shadow_blend_factor_overrides(0x01e0_0802, b"blendOp ADD\n").unwrap(),
            0x01e0_0802
        );
    }

    #[test]
    fn world_shadow_draw_requires_complete_main_state_and_preserves_unmodified_words() {
        let mut source = shadow_source_inputs(b"{}", b"{}").unwrap();
        assert!(source.world_draw_state(1, 2.5, 3.0, 0.25, 1).is_err());
        let state = [
            0xffff_ffff,
            0x1234_5678,
            0x8765_4321,
            0x1122_3344,
            0x5566_7788,
            0xa755_abff,
            0xdead_beef,
            0xaaaa_aaaa,
            0xbbbb_bbbb,
        ];
        source.main_render_state = Some(state);
        let actual = source.world_draw_state(1, 2.5, 3.0, 0.25, 1).unwrap();
        let mut expected = state;
        expected[0] &= 0xfe1f_fffe;
        expected[5] = (((state[5] & !0x0800_0000) | 0x0800_0000) & 0xefff_fff9) | 0x1000_0001;
        expected[6] = 2; // Original CVTT truncation, not rounding.
        expected[7] = 0.25f32.to_bits();
        expected[8] = 3.0f32.to_bits();
        assert_eq!(actual, expected);
        assert_eq!(source.main_render_state, Some(state));
        assert_eq!(&actual[1..5], &state[1..5]);
    }

    #[test]
    fn shadow_source_retains_native_selector_bits_and_cloth_presence() {
        let program = b"Alpha/EnableRefraction 2\ntranslucency_hq/enable 1\nmaterialEffects/enable 3\nisCloth 1\n";
        let inputs = shadow_source_inputs(b"alphaTest 0\n", program).unwrap();
        assert_eq!(inputs.shadow_draw_flags49(), 0x43);
        assert_eq!(inputs.cloth_source, ShadowProgramSource::MainProgram);
        // These bits are not inferred from alpha testing, blend sort or skinning.
        assert_eq!(
            shadow_source_inputs(b"alphaTest 1\nuseSkinning 1\n", b"")
                .unwrap()
                .shadow_draw_flags49(),
            0
        );
        assert_eq!(
            shadow_source_inputs(b"materialEffects/enable 2\n", b"")
                .unwrap()
                .shadow_draw_flags49(),
            0
        );
        assert_eq!(
            shadow_source_inputs(b"Alpha/EnableRefraction 0\n", program)
                .unwrap()
                .shadow_draw_flags49(),
            0x42
        );
        // Native noShadow suppresses slot 10, not the separate cloth slot 11.
        let cloth = shadow_source_inputs(b"noShadow 1\nshadowClothProgram \"\"\n", b"").unwrap();
        assert_eq!(cloth.source, ShadowProgramSource::Disabled);
        assert_eq!(cloth.cloth_source, ShadowProgramSource::Authored);
        assert_eq!(cloth.cloth_override_program.as_deref(), Some(""));
    }

    #[test]
    fn parses_diffuse_without_comments() {
        assert_eq!(
            diffuse_image(b"// diffusemap bad\n diffusemap models/stone_d\n"),
            Some("generated/image/models/stone_d.bimage7_mip0".into())
        );
        assert_eq!(
            diffuse_image(b"diffusemap models/tree_d$nomips\n"),
            Some("generated/image/models/tree_d.bimage7_mip0".into())
        );
    }
    #[test]
    fn native_image_sources_preserve_options_and_program_token_spelling() {
        assert_eq!(
            image_source_name("models/tree_d$nomips"),
            Some("models/tree_d$nomips.tga".into())
        );
        assert_eq!(
            image_source_name("models/tree_n.tga$nomips"),
            Some("models/tree_n.tga$nomips".into())
        );
        assert_eq!(
            append_default_extension("dir.with.dot/tree", ".tga"),
            "dir.with.dot/tree.tga"
        );
        assert_eq!(
            append_default_extension("dir\\\\tree.dds", "tga"),
            "dir\\\\tree.dds"
        );
        assert_eq!(
            image_source_name("ipr_constantcolor(0.501961, 0.501961, 0, 1)"),
            Some("ipr_constantcolor( 0.501961, 0.501961, 0, 1).imageprog".into())
        );
        assert_eq!(
            image_source_name("ipr_constantColor( .5, .5, .5, 0)"),
            Some("ipr_constantColor( .5, .5, .5, 0).imageprog".into())
        );
        assert_eq!(image_source_name("ipr_unknown(1,2,3,4)"), None);
        assert_eq!(image_source_name("ipr_constantcolor(-1,0,0,1)"), None);
        let pixels = constant_color_pixels([-1.0, 0.5, 1.0, 2.0]).unwrap();
        for pixel in pixels.chunks_exact(4) {
            assert_eq!(pixel, &[0, 127, 255, 255]);
        }
        assert_eq!(constant_color_pixels([f32::NAN; 4]), None);
    }
    #[test]
    fn default_extension_matches_original_owned_string_cases() {
        let decode = |hex: &str| {
            if hex == "-" {
                return String::new();
            }
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
                .collect();
            String::from_utf8(bytes).unwrap()
        };
        let fixture = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/image_name_cases.txt"
        ));
        for line in fixture.lines() {
            let row: Vec<_> = line.split_whitespace().collect();
            let name = decode(row[0]);
            let extension = decode(row[1]);
            let expected = decode(row[2]);
            assert_eq!(
                append_default_extension(&name, &extension),
                expected,
                "{name:?} {extension:?}"
            );
            assert_eq!(
                u32::from_str_radix(row[3], 16).unwrap() & 0x1fffffff,
                expected.len() as u32
            );
            assert_eq!(
                u32::from_str_radix(row[3], 16).unwrap() & 0xe0000000,
                0x60000000
            );
            assert_eq!(row[4], "c0000400");
        }
        assert_eq!(fixture.lines().count(), 576);
    }

    #[test]
    fn honors_coverage_and_transparency_flags() {
        assert_eq!(
            transparency(b"options { alphatest 1 }"),
            Transparency::Opaque
        );
        assert_eq!(transparency(b"alphatest 1\n"), Transparency::Cutout);
        assert_eq!(transparency(b"stageSort sortwater\n"), Transparency::Blend);
        assert_eq!(transparency(b"stageSort sorttrans\n"), Transparency::Blend);
        assert_eq!(
            transparency(b"stageSort sortcoverage\nalphatest 0\n"),
            Transparency::Coverage
        );
        assert_eq!(transparency(b"// alphatest 1\n"), Transparency::Opaque);
    }
    #[test]
    fn inherits_program_blend_and_decal_option_without_object_names() {
        let program = b"isDecal 1\nstageSort sortDecal\nblend SRC_ALPHA ONE_MINUS_SRC_ALPHA\nhlsl_prefix {\nblend ignored\n";
        assert_eq!(
            transparency_with_program(b"state {\n}\n", program),
            Transparency::Blend
        );
        assert!(program_option(b"", program, "isDecal"));
        assert!(!program_option(b"isdecal 0\n", program, "isDecal"));
        assert_eq!(
            transparency(b"blend SRC_ALPHA ONE_MINUS_SRC_ALPHA\n"),
            Transparency::Blend
        );
        assert_eq!(transparency(b"stageSort SortDecal\n"), Transparency::Blend);
        assert_eq!(
            transparency_with_program(b"blend ONE ZERO\n", program),
            Transparency::Opaque
        );
        assert_eq!(
            transparency_with_program(b"stageSort sortCoverage\n", program),
            Transparency::Blend
        );
    }

    #[test]
    fn native_raster_culling_inherits_and_is_not_a_lighting_option() {
        assert_eq!(cull_mode_with_program(b"", b""), Ok(CullMode::Back));
        assert_eq!(
            cull_mode_with_program(b"state {\n}\n", b"cull FRONT\n"),
            Ok(CullMode::Front)
        );
        assert_eq!(
            cull_mode_with_program(b"cull NoNe // fabric\n", b"cull BACK\n"),
            Ok(CullMode::None)
        );
        assert_eq!(
            cull_mode_with_program(b"twoSidedLighting 1\n", b""),
            Ok(CullMode::Back)
        );
        assert_eq!(
            cull_mode_with_program(b"version 2\ntwoSided 1\n", b""),
            Ok(CullMode::None)
        );
        assert_eq!(
            cull_mode_with_program(b"version 2\nbackSided 1\n", b""),
            Ok(CullMode::Front)
        );
        assert_eq!(
            cull_mode_with_program(b"version 6\ntwoSided 1\n", b""),
            Ok(CullMode::Back)
        );
        assert_eq!(
            cull_mode_with_program(b"cull sideways\n", b""),
            Err("bad cull mode sideways".into())
        );
        assert_eq!(
            cull_mode_with_program(b"", b"hlsl_prefix {\ncull NONE\n"),
            Ok(CullMode::Back)
        );
    }

    #[test]
    fn rigid_shadow_options_keep_authored_values_and_native_last_writes() {
        let bytes = b"useSkinning 0\nforcePixelShader 0\ndiffusemap not_a_number\n";
        let options = rigid_shadow_option_parameters(bytes, 1, false, false, |name| {
            matches!(
                name,
                "useskinning" | "alphatest" | "isshadow" | "forcepixelshader"
            )
        })
        .unwrap();
        let value = |key: &str| {
            options
                .iter()
                .rev()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .unwrap()
                .1
        };
        assert_eq!(value("useSkinning"), 0);
        assert_eq!(value("isShadow"), 1);
        assert_eq!(value("isAlpha"), 0);
        assert_eq!(value("noScreenShadows"), 0);
        assert_eq!(value("forcePixelShader"), 1);
        assert!(
            !options
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("alphaTest")),
            "program defaults belong to the RPD fallback, not material overrides"
        );
        assert!(
            rigid_shadow_option_parameters(b"useSkinning wrong", 1, false, false, |name| name
                == "useskinning")
            .is_err()
        );
    }

    #[test]
    fn shadow_option_inputs_are_not_overwritten_before_native_flag_resolution() {
        let bytes = b"noScreenShadows -2\ntranslucency/enable 3\nskyType 2\nisAlpha 0\n";
        let inputs = material_option_parameters(bytes, |_| true).unwrap();
        assert!(inputs.contains(&("noscreenshadows".into(), -2)));
        assert!(inputs.contains(&("translucency/enable".into(), 3)));
        for alpha_test in [0, 1, 2, -1] {
            for is_alpha in [false, true] {
                for no_screen in [false, true] {
                    let options = rigid_shadow_option_parameters(
                        bytes,
                        alpha_test,
                        is_alpha,
                        no_screen,
                        |_| true,
                    )
                    .unwrap();
                    let value = |key: &str| {
                        options
                            .iter()
                            .rev()
                            .find(|(name, _)| name.eq_ignore_ascii_case(key))
                            .map(|(_, value)| *value)
                    };
                    assert_eq!(value("noScreenShadows"), Some(i32::from(no_screen)));
                    assert_eq!(value("isAlpha"), Some(i32::from(is_alpha)));
                    assert_eq!(
                        value("forcePixelShader"),
                        (alpha_test & 1 != 0).then_some(1)
                    );
                }
            }
        }
    }

    #[test]
    fn native_visibility_and_alpha_options_inherit_and_allow_explicit_overrides() {
        let program =
            b"options {\ninvisible 1\nalphaTest 1\n}\nhlsl_prefix {\ninvisible 0\nalphaTest 0\n";
        assert!(program_option(b"", program, "invisible"));
        assert!(program_option(b"", program, "alphaTest"));
        assert_eq!(
            transparency_with_program(b"", program),
            Transparency::Cutout
        );
        let visible_opaque = b"invisible 0\nALPHATEST 0 // explicit material overrides\n";
        assert!(!program_option(visible_opaque, program, "invisible"));
        assert!(!program_option(visible_opaque, program, "alphaTest"));
        assert_eq!(
            transparency_with_program(visible_opaque, program),
            Transparency::Opaque
        );
        assert!(program_option(
            b"Invisible 1\n",
            b"invisible 0\n",
            "invisible"
        ));
        assert!(!program_option(b"// invisible 1\n", b"", "invisible"));
        assert!(!program_option(b"", b"", "invisible"));
    }

    #[test]
    fn parses_enabled_layer_zero_tint() {
        assert_eq!(
            layer0_color_constant(
                b"use_layer0_colorconstant 1\nlayer0_colorconstant { 0.0466651, 0, 0, 1 }\n"
            ),
            Some([0.0466651, 0.0, 0.0, 1.0])
        );
        assert_eq!(
            layer0_color_constant(b"layer0_colorconstant { 1, 0, 0, 1 }\n"),
            None
        );
    }
    #[test]
    fn reads_decal_bias_and_vertex_mask() {
        assert_eq!(
            layer0_vertex_mask_channel(b"layer0_vtxcolormaskchannel 0\n"),
            Some(0)
        );
        assert_eq!(
            transparency(b"layer0_vtxcolormaskchannel 0\n"),
            Transparency::Opaque
        );
        assert_eq!(
            decal_depth_bias(b"mainProgram arkDecal\nzbias -0.0001\n"),
            Some(-0.0001)
        );
        assert_eq!(decal_depth_bias(b"mainProgram arkDecal\n"), Some(0.0));
        assert_eq!(decal_depth_bias(b"mainProgram arkDecal\nzbias 0\n"), Some(0.0));
        assert_eq!(
            decal_depth_bias(b"mainProgram arkDecal\nzbias 0.25\n"),
            Some(0.25)
        );
    }
    #[test]
    fn distinguishes_editor_colour_from_game_material() {
        assert!(is_editor_helper("models/templates/colors/blue.material"));
        assert!(is_editor_helper("NULL"));
        assert!(is_editor_helper(
            "models/templates/flat_colors/invisible.material"
        ));
        assert!(is_editor_helper(
            "models/testassets/yguilloud/invisible_mesh/invisible.material"
        ));
        assert!(!is_editor_helper(
            "models/environment/vehicles/skiff_01/skiff_hull_01.material"
        ));
    }
    #[test]
    fn identifies_distant_screen_gate_impostor() {
        let path = "models/campaign/eotw/optim/screen_gates/screen_gate_eotw_03_fade.material";
        let decl = b"fx/opt/view_dependent_fade 1\nfx/diffuse models/campaign/eotw/optim/screen_gates/screen_gate_eotw_03_d.tga\n";
        assert!(is_screen_gate_impostor(path, decl));
        assert!(!is_screen_gate_impostor(
            "models/environment/gate.material",
            decl
        ));
        assert!(!is_screen_gate_impostor(
            path,
            b"diffusemap models/templates/colors/black_d.tga"
        ));
    }
}
