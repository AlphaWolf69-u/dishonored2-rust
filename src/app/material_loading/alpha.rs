//! Resolve alpha-depth slots while the installed material dictionaries are live.
use super::{material, MaterialResolver};
use crate::rendering::bevy_backend::alpha_depth_scene::AlphaDepthMaterial;
use bevy::prelude::*;
use std::{collections::HashMap, sync::Arc};

#[derive(Resource, Clone, Default)]
pub(in crate::app) struct AlphaDepthMaterials(
    pub Arc<HashMap<super::MaterialHandle, [Result<AlphaDepthMaterial, String>; 3]>>,
);

impl MaterialResolver<'_> {
    pub(in crate::app) fn alpha_depth_materials(&mut self) -> AlphaDepthMaterials {
        let records = self
            .cache
            .named_values()
            .filter_map(|(name, cached)| {
                matches!(cached.handle, super::MaterialHandle::Probe(_))
                    .then_some((name.to_owned(), cached.handle.clone()))
            })
            .collect::<Vec<_>>();
        let mut result = HashMap::new();
        for (name, handle) in records {
            if result.contains_key(&handle) {
                continue;
            }
            let prepared = (|| -> Result<_, String> {
                let catalog = self
                    .catalog
                    .ok_or("alpha-depth material catalog unavailable")?;
                let bytes = catalog
                    .read(&format!("generated/decls/material/{name}.decl"))
                    .map_err(|e| e.to_string())?
                    .ok_or("alpha-depth material declaration absent")?;
                let authored_main = material::authored_program_override(&bytes, "mainProgram");
                let implicit_main = authored_main.is_none();
                let main = authored_main.unwrap_or_else(|| "arkDefault".into());
                let library = self
                    .programs
                    .as_deref_mut()
                    .ok_or("alpha-depth program library unavailable")?
                    .as_mut()
                    .map_err(|error| error.clone())?;
                let reference = super::material_option_overrides(library, &bytes)?;
                let definition = library
                    .definition(main.as_bytes())?
                    .ok_or("alpha-depth MAIN definition absent")?;
                let fallback = definition.defaults.clone();
                let state = crate::render_state::material_render_state(
                    &crate::entities::declaration_text(&bytes),
                    definition.render_state(),
                )?;
                let decal = library.option_value(main.as_bytes(), &reference, b"isDecal")? & 1 != 0;
                let hair = library.option_value(main.as_bytes(), &reference, b"hairType")? & 1 != 0;
                let sky = library.option_value(main.as_bytes(), &reference, b"skyType")? as u32;
                let class = material::material_program_class(
                    true,
                    implicit_main,
                    state[0],
                    library.option_value(main.as_bytes(), &reference, b"invisible")? != 0,
                );
                let cleared = material::finalize_material_programs(class, hair, sky).clear_slots;
                let refraction =
                    library.registered_option_value(&reference, b"Alpha/EnableRefraction")? != 0;
                let effects =
                    library.registered_option_value(&reference, b"materialEffects/enable")? & 1
                        != 0;
                let multi =
                    library.registered_option_value(&reference, b"useMultiInstancing")? != 0;
                let direct =
                    library.registered_option_value(&reference, b"useDirectInstancing")? != 0;
                let selected = [
                    (
                        if refraction { 4 } else { 6 },
                        if refraction {
                            "zPrePassProgram"
                        } else {
                            "zPrePassAlphaProgram"
                        },
                    ),
                    (6, "zPrePassAlphaProgram"),
                ]
                .map(|(slot, key)| {
                    if (slot != 6 && decal) || cleared & (1 << slot) != 0 {
                        return Ok(false);
                    }
                    let selected = material::authored_program_override(&bytes, key)
                        .unwrap_or_else(|| main.clone());
                    if selected.is_empty() || selected == "0" {
                        return Ok(false);
                    }
                    if !selected.eq_ignore_ascii_case(&main) {
                        return Err(format!(
                            "alpha-depth override {selected} differs from MAIN {main}"
                        ));
                    }
                    Ok(true)
                });
                let [full, reduced] = selected;
                let full = full?;
                let reduced = reduced?;
                if (full || reduced)
                    && !matches!(
                        main.to_ascii_lowercase().as_str(),
                        "arkdefault"
                            | "arkvegetation"
                            | "arkvista"
                            | "arksimplesprite"
                            | "particle/filing_container"
                    )
                {
                    return Err(format!("alpha-depth program {main} is not yet integrated"));
                }
                if refraction
                    && !matches!(
                        main.to_ascii_lowercase().as_str(),
                        "arkdefault" | "arkvegetation"
                    )
                {
                    return Err(format!(
                        "alpha-depth refraction output for {main} is not integrated"
                    ));
                }
                // 233a30 initializes model+1c10 to -1. 3119a0 selects the
                // ordinary bank for that selector even when +49 bit6 is set.
                // These are the same base materials submitted by the forward
                // renderer; enabling effect support does not activate an effect.
                let mut glass = Vec4::ZERO;
                if refraction {
                    let names = ["alpha/refractionior", "alpha/normalmask"];
                    let parameters = crate::entities::material_float_parameters(
                        &crate::entities::declaration_text(&bytes),
                        |n| names.contains(&n),
                    )?
                    .into_iter()
                    .map(|(name, words)| crate::render_program::SerializedParameter {
                        name: name.into_bytes(),
                        kind: crate::render_program::RenderParameterKind::Uniform,
                        value: crate::render_program::SerializedParameterValue::Words(words),
                    })
                    .collect();
                    let mut values = crate::render_program::SerializedParameterBlock {
                        flags: 0,
                        parameters,
                    };
                    let ior = f32::from_bits(
                        library.registered_float_vector(
                            catalog,
                            &values,
                            b"alpha/refractionior",
                            Some(&fallback),
                        )?[0],
                    );
                    let mask = f32::from_bits(
                        library.registered_float_vector(
                            catalog,
                            &values,
                            b"alpha/normalmask",
                            Some(&fallback),
                        )?[0],
                    );
                    let uint = crate::entities::material_uint_parameters(
                        &crate::entities::declaration_text(&bytes),
                        |n| n == "alpha/blursize",
                    )?;
                    for (name, word) in uint {
                        values
                            .parameters
                            .push(crate::render_program::SerializedParameter {
                                name: name.into_bytes(),
                                kind: crate::render_program::RenderParameterKind::Uniform,
                                value: crate::render_program::SerializedParameterValue::Words(word),
                            });
                    }
                    let decl = catalog
                        .read("generated/decls/renderparm/alpha/blursize.decl")
                        .map_err(|e| e.to_string())?
                        .ok_or("missing blur size declaration")?;
                    let default = crate::entities::declaration_uint_default(
                        &crate::entities::declaration_text(&decl),
                    )?;
                    let blur = values.unsigned_integer_value(
                        b"alpha/blursize",
                        0x20,
                        default,
                        Some(&fallback),
                    )?;
                    glass = Vec4::new(ior, blur as f32, mask, 0.);
                }
                if let super::MaterialHandle::Probe(handle) = &handle {
                    if let Some(mut material) = self.probe_materials.get_mut(handle) {
                        material.extension.surface.glass = glass;
                    }
                }
                let base = AlphaDepthMaterial {
                    state,
                    flags49: u8::from(refraction) | (u8::from(effects) << 6),
                    full,
                    reduced,
                };
                Ok(std::array::from_fn(|bank| -> Result<_, String> {
                    let present =
                        material::instancing_bank_override(bank as u8, multi, direct)?.is_some();
                    Ok(AlphaDepthMaterial {
                        full: full && present,
                        reduced: reduced && present,
                        ..base.clone()
                    })
                }))
            })();
            result.insert(
                handle,
                prepared.unwrap_or_else(|error| {
                    std::array::from_fn(|_| Err(format!("{name}: {error}")))
                }),
            );
        }
        AlphaDepthMaterials(Arc::new(result))
    }
}
