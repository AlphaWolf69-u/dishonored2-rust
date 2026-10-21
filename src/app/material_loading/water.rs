//! Slot4 depth programs, separate from material MAIN colour and drawing state.
use super::{material, render_program, shader_cache};
use std::sync::Arc;

pub(super) type DepthSource = Result<Option<Arc<shader_cache::CachedWaterDepthSource>>, String>;

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
