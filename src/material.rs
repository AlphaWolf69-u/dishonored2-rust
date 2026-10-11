//! Resolve a Void material's base-colour image from its text declaration.

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
    let text = String::from_utf8_lossy(bytes);
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
    let text = String::from_utf8_lossy(bytes);
    let mut values = std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let mut fields = line.split_whitespace();
        let Some(key) = fields.next() else { continue };
        if key.to_ascii_lowercase().starts_with("hlsl_") { break; }
        if let Some(value) = fields.next() {
            values.insert(key.to_ascii_lowercase(), value.to_ascii_lowercase());
        }
    }
    values
}

pub fn program_option(bytes: &[u8], program: &[u8], key: &str) -> bool {
    let mut values = render_fields(program);
    values.extend(render_fields(bytes));
    values.get(&key.to_ascii_lowercase()).is_some_and(|v| v == "1")
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
    if name.rsplit(['/', '\\']).next().unwrap_or(name).contains('.') {
        name.to_owned()
    } else {
        format!("{name}{}{extension}", if extension.starts_with('.') { "" } else { "." })
    }
}

/// FUN_1402f1480's constant-color token spelling and FUN_1402ebcd0's suffix.
/// Unsupported image-program branches are not mapped to guessed filenames.
pub fn image_source_name(name: &str) -> Option<String> {
    let name = name.trim().trim_matches('"');
    if name.get(..4).is_some_and(|v| v.eq_ignore_ascii_case("ipr_")) {
        let (function, args) = name.split_once('(')?;
        if !function.eq_ignore_ascii_case("ipr_constantcolor") { return None; }
        let args: Vec<_> = args.strip_suffix(')')?.split(',').map(str::trim).collect();
        if args.len() != 4 || args.iter().any(|v| v.starts_with('-') || v.starts_with('+') || v.parse::<f32>().ok().is_none_or(|f| !f.is_finite())) { return None; }
        Some(format!("{function}( {}, {}, {}, {}).imageprog", args[0], args[1], args[2], args[3]))
    } else if !name.is_empty() && !name.starts_with('_') && !name.starts_with('#') {
        Some(append_default_extension(name, "tga"))
    } else { None }
}

/// FUN_1402f1480's constant-color generation branch: truncate float*255,
/// clamp bytes, and repeat the pixel across a 4x4 RGBA8 image. The finite
/// representable-input guard belongs to this Rust boundary, not native parsing.
pub fn constant_color_pixels(color: [f32; 4]) -> Option<[u8; 64]> {
    let scaled = color.map(|value| value * 255.0);
    if scaled.iter().any(|&v| !v.is_finite() || !(-2147483648.0..2147483648.0).contains(&v)) { return None; }
    let pixel = scaled.map(|value| (value as i32).clamp(0, 255) as u8);
    Some(std::array::from_fn(|i| pixel[i % 4]))
}

/// Prefer the source->compiled mapping stored by the game itself.
pub fn compiled_image_stem(catalog: &crate::archive::Catalog, name: &str) -> Option<String> {
    if let Some(source) = image_source_name(name) {
        if let Some(resource) = catalog.image_resource(&source) { return Some(resource.to_owned()); }
    }
    // Older extensionless ordinary resources remain valid. Do not discard
    // unresolved options or invent a compiled image-program filename.
    if name.contains('$') || name.get(..4).is_some_and(|v| v.eq_ignore_ascii_case("ipr_")) { return None; }
    let declaration = format!("diffusemap {name}");
    diffuse_image(declaration.as_bytes())?.strip_suffix("_mip0").map(str::to_owned)
}

pub fn diffuse_image(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
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
    let text = String::from_utf8_lossy(bytes);
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
    let text = String::from_utf8_lossy(bytes);
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
    let text = String::from_utf8_lossy(bytes);
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
    // Void's zbias is a fraction of normalized depth. The GPU pipeline uses
    // integer depth-bias units; 24-bit conversion preserves the authored scale.
    if decal {
        Some(
            original
                .map(|value| (-value * 16_777_216.0).round().clamp(1.0, 4096.0))
                .unwrap_or(1678.0),
        )
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(image_source_name("models/tree_d$nomips"), Some("models/tree_d$nomips.tga".into()));
        assert_eq!(image_source_name("models/tree_n.tga$nomips"), Some("models/tree_n.tga$nomips".into()));
        assert_eq!(append_default_extension("dir.with.dot/tree", ".tga"), "dir.with.dot/tree.tga");
        assert_eq!(append_default_extension("dir\\\\tree.dds", "tga"), "dir\\\\tree.dds");
        assert_eq!(image_source_name("ipr_constantcolor(0.501961, 0.501961, 0, 1)"), Some("ipr_constantcolor( 0.501961, 0.501961, 0, 1).imageprog".into()));
        assert_eq!(image_source_name("ipr_constantColor( .5, .5, .5, 0)"), Some("ipr_constantColor( .5, .5, .5, 0).imageprog".into()));
        assert_eq!(image_source_name("ipr_unknown(1,2,3,4)"), None);
        assert_eq!(image_source_name("ipr_constantcolor(-1,0,0,1)"), None);
        let pixels = constant_color_pixels([-1.0, 0.5, 1.0, 2.0]).unwrap();
        for pixel in pixels.chunks_exact(4) { assert_eq!(pixel, &[0, 127, 255, 255]); }
        assert_eq!(constant_color_pixels([f32::NAN; 4]), None);
    }
    #[test]
    fn default_extension_matches_original_owned_string_cases() {
        let decode = |hex: &str| {
            if hex == "-" { return String::new(); }
            let bytes = (0..hex.len()).step_by(2).map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap()).collect();
            String::from_utf8(bytes).unwrap()
        };
        let fixture = include_str!("../analysis/original_image_name_cases.txt");
        for line in fixture.lines() {
            let row: Vec<_> = line.split_whitespace().collect();
            let name = decode(row[0]);
            let extension = decode(row[1]);
            let expected = decode(row[2]);
            assert_eq!(append_default_extension(&name, &extension), expected, "{name:?} {extension:?}");
            assert_eq!(u32::from_str_radix(row[3], 16).unwrap() & 0x1fffffff, expected.len() as u32);
            assert_eq!(u32::from_str_radix(row[3], 16).unwrap() & 0xe0000000, 0x60000000);
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
        assert_eq!(transparency_with_program(b"state {\n}\n", program), Transparency::Blend);
        assert!(program_option(b"", program, "isDecal"));
        assert!(!program_option(b"isdecal 0\n", program, "isDecal"));
        assert_eq!(transparency(b"blend SRC_ALPHA ONE_MINUS_SRC_ALPHA\n"), Transparency::Blend);
        assert_eq!(transparency(b"stageSort SortDecal\n"), Transparency::Blend);
        assert_eq!(transparency_with_program(b"blend ONE ZERO\n", program), Transparency::Opaque);
        assert_eq!(transparency_with_program(b"stageSort sortCoverage\n", program), Transparency::Blend);
    }

    #[test]
    fn native_visibility_and_alpha_options_inherit_and_allow_explicit_overrides() {
        let program = b"options {\ninvisible 1\nalphaTest 1\n}\nhlsl_prefix {\ninvisible 0\nalphaTest 0\n";
        assert!(program_option(b"", program, "invisible"));
        assert!(program_option(b"", program, "alphaTest"));
        assert_eq!(transparency_with_program(b"", program), Transparency::Cutout);
        let visible_opaque = b"invisible 0\nALPHATEST 0 // explicit material overrides\n";
        assert!(!program_option(visible_opaque, program, "invisible"));
        assert!(!program_option(visible_opaque, program, "alphaTest"));
        assert_eq!(transparency_with_program(visible_opaque, program), Transparency::Opaque);
        assert!(program_option(b"Invisible 1\n", b"invisible 0\n", "invisible"));
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
            Some(1678.0)
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
