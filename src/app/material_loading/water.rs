//! Slot4 depth programs, separate from material MAIN colour and drawing state.
use super::{material, render_program, shader_cache};
use std::sync::Arc;

pub(super) type DepthSource = Result<Option<Arc<shader_cache::CachedWaterDepthSource>>, String>;

/// 29f890's world-bank colour selection, after 2d5210 bank construction and
/// finalization. Keep serialized stages separate from GPU ABI admission.
pub(super) fn world_colour_source(
    library: &mut shader_cache::ProgramLibrary,
    bytes: &[u8],
    main: &str,
    reference: &render_program::SerializedParameterBlock,
    state: crate::render_state::RenderState,
) -> Result<Option<Arc<crate::rendering::water::program::WorldWaterColourSource>>, String> {
    let multi = library.registered_option_value(reference, b"useMultiInstancing")? != 0;
    let direct = library.registered_option_value(reference, b"useDirectInstancing")? != 0;
    if material::instancing_bank_override(2, multi, direct)?.is_none() {
        return Ok(None);
    }
    let decal = library.option_value(main.as_bytes(), reference, b"isDecal")? & 1 != 0;
    let sky = library.option_value(main.as_bytes(), reference, b"skyType")? as u32;
    let hair = library.option_value(main.as_bytes(), reference, b"hairType")? & 1 != 0;
    // The registered Program default for sssProgram is null. Slot12's authored
    // source changes selection even if its draw bucket is handled elsewhere.
    let alternate = material::authored_program_override(bytes, "sssProgram")
        .is_some_and(|name| !name.is_empty() && name != "0");
    let class = material::material_program_class(
        true,
        material::authored_program_override(bytes, "mainProgram").is_none(),
        state[0],
        library.option_value(main.as_bytes(), reference, b"invisible")? != 0,
    );
    if (alternate && decal)
        || (!alternate
            && material::finalize_material_programs(class, hair, sky).clear_slots & 1 != 0)
    {
        return Ok(None);
    }
    let selected = if alternate {
        material::authored_program_override(bytes, "mainAlphaProgram")
            .unwrap_or_else(|| main.to_owned())
    } else {
        main.to_owned()
    };
    if selected.is_empty() || selected == "0" {
        return Ok(None);
    }
    let alpha = material::shadow_is_alpha(
        state[0],
        decal,
        library.registered_option_value(reference, b"decalType")? as u32,
        sky,
    );
    let no_screen = [
        b"noScreenShadows".as_slice(),
        b"translucency_hq/enable",
        b"translucency/enable",
    ]
    .into_iter()
    .try_fold(alpha, |v, name| {
        library
            .option_value(selected.as_bytes(), reference, name)
            .map(|n| v | (n != 0))
    })?;
    let mut options = reference.clone();
    for (name, value) in [
        (b"useMultiInstancing".as_slice(), true),
        (b"isAlpha", alpha || alternate),
        (b"noScreenShadows", no_screen),
    ] {
        options
            .parameters
            .push(render_program::SerializedParameter {
                name: name.to_vec(),
                kind: render_program::RenderParameterKind::Option,
                value: render_program::SerializedParameterValue::Words([u32::from(value), 0, 0, 0]),
            });
    }
    let variant_hash = library
        .specialization_hash(selected.as_bytes(), &options)?
        .ok_or_else(|| format!("world water colour program absent: {selected}"))?;
    let variant = library
        .serialized_variant(selected.as_bytes(), &options)?
        .ok_or_else(|| format!("world water colour needs runtime compilation: {selected}"))?;
    let parameters = colour_parameters(library, bytes, &selected)?;
    Ok(Some(Arc::new(
        crate::rendering::water::program::WorldWaterColourSource {
            program: selected,
            variant_hash,
            variant,
            options,
            parameters,
        },
    )))
}

/// 2d5210: original rigid bank0 and world bank2. Cloth/direct-instancing
/// programs remain distinct and are never substituted with rigid water.
pub(super) fn depth_sources(
    library: &mut shader_cache::ProgramLibrary,
    bytes: &[u8],
    main: &str,
    reference: &render_program::SerializedParameterBlock,
    state: crate::render_state::RenderState,
) -> [DepthSource; 2] {
    let common = (|| -> Result<_, String> {
        let is_decal = library.option_value(main.as_bytes(), reference, b"isDecal")? & 1 != 0;
        let sky = library.option_value(main.as_bytes(), reference, b"skyType")? as u32;
        let hair = library.option_value(main.as_bytes(), reference, b"hairType")? & 1 != 0;
        let class = material::material_program_class(
            true,
            material::authored_program_override(bytes, "mainProgram").is_none(),
            state[0],
            library.option_value(main.as_bytes(), reference, b"invisible")? != 0,
        );
        if is_decal
            || material::finalize_material_programs(class, hair, sky).clear_slots & (1 << 4) != 0
        {
            return Ok(None);
        }
        let selected = material::authored_program_override(bytes, "zPrePassProgram")
            .unwrap_or_else(|| main.to_owned());
        if selected.is_empty() || selected == "0" {
            return Ok(None);
        }
        let alpha = material::shadow_is_alpha(
            state[0],
            is_decal,
            library.registered_option_value(reference, b"decalType")? as u32,
            sky,
        );
        let no_screen = [
            b"noScreenShadows".as_slice(),
            b"translucency_hq/enable",
            b"translucency/enable",
        ]
        .into_iter()
        .try_fold(alpha, |v, name| {
            library
                .option_value(selected.as_bytes(), reference, name)
                .map(|n| v | (n != 0))
        })?;
        // Material+49 bit0 is the registered/material Alpha/EnableRefraction,
        // not alphaTest. It forces a pixel stage specifically in slot4.
        let force_pixel =
            library.registered_option_value(reference, b"Alpha/EnableRefraction")? != 0;
        let multi = library.registered_option_value(reference, b"useMultiInstancing")? != 0;
        let direct = library.registered_option_value(reference, b"useDirectInstancing")? != 0;
        Ok(Some((
            selected,
            alpha,
            no_screen,
            force_pixel,
            multi,
            direct,
        )))
    })();
    std::array::from_fn(|index| {
        let Some((selected, alpha, no_screen, force_pixel, multi, direct)) =
            common.as_ref().map_err(Clone::clone)?
        else {
            return Ok(None);
        };
        let bank = if index == 0 { 0 } else { 2 };
        if material::instancing_bank_override(bank, *multi, *direct)?.is_none() {
            return Ok(None);
        }
        let mut options = reference.clone();
        let mut set = |name: &[u8], value: bool| {
            options
                .parameters
                .push(render_program::SerializedParameter {
                    name: name.to_vec(),
                    kind: render_program::RenderParameterKind::Option,
                    value: render_program::SerializedParameterValue::Words([
                        u32::from(value),
                        0,
                        0,
                        0,
                    ]),
                })
        };
        if bank == 2 {
            set(b"useMultiInstancing", true);
        }
        set(b"isAlpha", *alpha);
        set(b"noScreenShadows", *no_screen);
        set(b"isZPrePass", true);
        if *force_pixel {
            set(b"forcePixelShader", true);
        }
        let source = library
            .water_depth_source(selected.as_bytes(), &options)?
            .ok_or_else(|| format!("water depth slot4 needs runtime compilation: {selected}"))?;
        let expected = if bank == 0 {
            shader_cache::CachedWaterDepthClass::Rigid
        } else {
            shader_cache::CachedWaterDepthClass::World
        };
        if source.class != expected {
            return Err(format!(
                "water depth bank{bank} has incompatible {:?} vertex ABI",
                source.class
            ));
        }
        Ok(Some(source))
    })
}

/// Actual colour program defaults followed by the root material dictionary.
/// The evaluated view remains underneath these scopes at draw preparation.
pub(super) fn colour_parameters(
    library: &mut shader_cache::ProgramLibrary,
    bytes: &[u8],
    main: &str,
) -> Result<[render_program::SerializedParameterBlock; 2], String> {
    use render_program::{
        RenderParameterKind, SerializedParameter, SerializedParameterBlock,
        SerializedParameterValue,
    };
    let defaults = library
        .definition(main.as_bytes())?
        .ok_or("water MAIN definition absent")?
        .defaults
        .clone();
    let text = crate::entities::declaration_text(bytes);
    let numeric = crate::entities::material_float_parameters(&text, |name| {
        crate::rendering::water::FLOAT_PARAMETER_NAMES.contains(&name)
    })?;
    let images = crate::entities::material_resource_parameters(&text, |name| {
        crate::rendering::water::IMAGE_PARAMETER_NAMES.contains(&name)
    })?;
    let mut parameters: Vec<_> = numeric
        .into_iter()
        .map(|(name, words)| SerializedParameter {
            name: name.into_bytes(),
            kind: RenderParameterKind::Uniform,
            value: SerializedParameterValue::Words(words),
        })
        .collect();
    parameters.extend(images.into_iter().map(|(name, image)| SerializedParameter {
        name: name.into_bytes(),
        kind: RenderParameterKind::Texture,
        value: SerializedParameterValue::Name(image.into_bytes()),
    }));
    Ok([
        defaults,
        SerializedParameterBlock {
            flags: 0,
            parameters,
        },
    ])
}
