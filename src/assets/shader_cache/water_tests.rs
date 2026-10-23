//! Exercise the actual application selector against every installed water source.
use super::{CachedWaterDepthClass, ProgramLibrary};
use crate::{material, render_program, shader_cache};

// The selector is intentionally independent of the host renderer. Reuse its
// exact production code here; this suite needs archives, not a graphics device.
#[path = "../../app/material_loading/water.rs"]
mod selector;

fn inputs(
    library: &mut ProgramLibrary,
    bytes: &[u8],
) -> (
    String,
    render_program::SerializedParameterBlock,
    crate::render_state::RenderState,
) {
    let surface = crate::material_surface::Surface::parse(bytes);
    let main = surface.main_program.unwrap();
    let values = material::material_option_parameters(bytes, |name| {
        library.parameter_kinds.get(name.as_bytes())
            == Some(&render_program::RenderParameterKind::Option)
    })
    .unwrap();
    let options = render_program::SerializedParameterBlock {
        flags: 0,
        parameters: values
            .into_iter()
            .map(|(name, v)| render_program::SerializedParameter {
                name: name.into_bytes(),
                kind: render_program::RenderParameterKind::Option,
                value: render_program::SerializedParameterValue::Words([v as u32, 0, 0, 0]),
            })
            .collect(),
    };
    let base = library
        .definition(main.as_bytes())
        .unwrap()
        .unwrap()
        .render_state();
    let state =
        crate::render_state::material_render_state(&crate::entities::declaration_text(bytes), base)
            .unwrap();
    (main, options, state)
}

fn set(block: &mut render_program::SerializedParameterBlock, name: &[u8], value: u32) {
    block.parameters.push(render_program::SerializedParameter {
        name: name.to_vec(),
        kind: render_program::RenderParameterKind::Option,
        value: render_program::SerializedParameterValue::Words([value, 0, 0, 0]),
    });
}

#[test]
#[ignore = "requires installed water materials and resident colour stages"]
fn installed_world_water_colour_preserves_selected_program_and_stages() {
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = crate::archive::Catalog::open(&base).unwrap();
    let mut library =
        ProgramLibrary::open(&base, shader_cache::ShaderCacheBackend::Pc, &catalog).unwrap();
    let mut count = 0;
    for row in include_str!("../../../tests/fixtures/water_world_colour_stages.txt").lines() {
        let mut words = row.split_whitespace();
        let path = words.next().unwrap();
        let expected: Vec<_> = words.map(|v| v.parse::<u32>().unwrap()).collect();
        let bytes = catalog.read(path).unwrap().unwrap();
        let (main, options, state) = inputs(&mut library, &bytes);
        let source = selector::world_colour_source(&mut library, &bytes, &main, &options, state)
            .unwrap_or_else(|e| panic!("{path}: {e}"))
            .unwrap();
        assert_eq!(source.program, main);
        let class = source
            .class
            .as_ref()
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        let probes = library
            .option_value(
                main.as_bytes(),
                &source.options,
                b"water/UseIBLProbesOption",
            )
            .unwrap()
            != 0;
        assert_eq!(
            *class,
            if probes {
                crate::rendering::water::program::WorldWaterColourClass::ProbeHorizontal
            } else {
                crate::rendering::water::program::WorldWaterColourClass::Horizontal
            },
            "{path}"
        );
        assert!(source.colour_options.horizontal, "{path}");
        let view = crate::assets::environment::ParameterReader {
            catalog: &catalog,
            parameters: &crate::assets::entities::EnvironmentRenderParameters {
                layers: Vec::new(),
            },
        };
        let settings = source.settings(&view).unwrap();
        assert!(!settings.sky_cube.is_empty());
        assert!(!settings.caustics.image.is_empty());
        assert_eq!(
            source.variant_hash,
            library
                .specialization_hash(main.as_bytes(), &source.options)
                .unwrap()
                .unwrap(),
        );
        let mut stages = source
            .variant
            .stages
            .iter()
            .filter_map(|stage| match stage.bytecode {
                render_program::StageBytecode::Absent => None,
                render_program::StageBytecode::Cached(id) => Some((stage.kind, id)),
                _ => panic!("{path}: expected resident cached stage"),
            })
            .collect::<Vec<_>>();
        stages.sort_unstable();
        assert_eq!(stages, vec![(0, expected[0]), (1, expected[1])], "{path}");
        assert_eq!(
            source.parameters[0].parameters.len(),
            library
                .definition(main.as_bytes())
                .unwrap()
                .unwrap()
                .defaults
                .parameters
                .len()
        );
        count += 1;
    }
    assert_eq!(count, 21);
}

#[test]
#[ignore = "requires installed program definitions and variants"]
fn world_water_colour_selection_preserves_nulls_and_bank_finalization() {
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = crate::archive::Catalog::open(&base).unwrap();
    let mut library =
        ProgramLibrary::open(&base, shader_cache::ShaderCacheBackend::Pc, &catalog).unwrap();
    let bytes = catalog
        .read("generated/decls/material/models/campaign/dust/water_surface_dust_01.material.decl")
        .unwrap()
        .unwrap();
    let (main, options, state) = inputs(&mut library, &bytes);
    let select = |library: &mut ProgramLibrary,
                  extra: &str,
                  options: &render_program::SerializedParameterBlock| {
        let mut declaration = crate::entities::declaration_text(&bytes).into_owned();
        let end = declaration.rfind('}').unwrap();
        declaration.insert_str(end, &format!("\n{extra}\n"));
        selector::world_colour_source(library, declaration.as_bytes(), &main, options, state)
    };
    for value in ["0", "\"\""] {
        // A null alpha program is ignored unless a non-null SSS slot selects it.
        assert!(
            select(&mut library, &format!("mainAlphaProgram {value}"), &options)
                .unwrap()
                .is_some()
        );
        assert!(select(
            &mut library,
            &format!("sssProgram arkWater\nmainAlphaProgram {value}"),
            &options
        )
        .unwrap()
        .is_none());
        assert!(select(
            &mut library,
            &format!("sssProgram {value}\nmainAlphaProgram 0"),
            &options
        )
        .unwrap()
        .is_some());
    }
    let alternate = select(&mut library, "sssProgram arkWater", &options)
        .unwrap()
        .unwrap();
    assert_ne!(
        library
            .registered_option_value(&alternate.options, b"isAlpha")
            .unwrap(),
        0
    );
    let mut changed = options.clone();
    set(&mut changed, b"useDirectInstancing", 1);
    assert!(select(&mut library, "", &changed).unwrap().is_none());
    let mut changed = options.clone();
    set(&mut changed, b"invisible", 1);
    assert!(select(&mut library, "", &changed).unwrap().is_none());
    // Class1 clears slot0, but does not clear the selected slot2.
    assert!(select(&mut library, "sssProgram arkWater", &changed)
        .unwrap()
        .is_some());
    let mut changed = options.clone();
    set(&mut changed, b"isDecal", 1);
    assert!(select(&mut library, "sssProgram arkWater", &changed)
        .unwrap()
        .is_none());
}

#[test]
#[ignore = "requires installed game archives and shader cache"]
fn installed_water_depth_banks_select_the_verified_stage_pairs() {
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = crate::archive::Catalog::open(&base).unwrap();
    let mut library =
        ProgramLibrary::open(&base, shader_cache::ShaderCacheBackend::Pc, &catalog).unwrap();
    let mut materials = 0;
    for path in catalog.paths_containing("generated/decls/material/") {
        let bytes = catalog.read(&path).unwrap().unwrap();
        if crate::material_surface::Surface::parse(&bytes).program()
            != crate::material_surface::Program::Water
        {
            continue;
        }
        let (main, options, state) = inputs(&mut library, &bytes);
        let sources = selector::depth_sources(&mut library, &bytes, &main, &options, state);
        let repeated = selector::depth_sources(&mut library, &bytes, &main, &options, state);
        for (bank, class, vs) in [
            (0, CachedWaterDepthClass::Rigid, 14476),
            (1, CachedWaterDepthClass::World, 14487),
        ] {
            let source = sources[bank]
                .as_ref()
                .unwrap_or_else(|e| panic!("{path} bank{bank}: {e}"))
                .as_ref()
                .unwrap();
            assert_eq!(source.class, class, "{path}");
            assert!(std::sync::Arc::ptr_eq(
                source,
                repeated[bank].as_ref().unwrap().as_ref().unwrap()
            ));
            let stages: Vec<_> = source
                .variant
                .stages
                .iter()
                .filter_map(|s| match s.bytecode {
                    render_program::StageBytecode::Cached(id) => Some((s.kind, id)),
                    _ => None,
                })
                .collect();
            assert!(stages.contains(&(0, vs)), "{path}: {stages:?}");
            assert!(stages.contains(&(1, 31)), "{path}: {stages:?}");
        }
        materials += 1;
    }
    assert_eq!(materials, 21);
}

#[test]
#[ignore = "requires installed game archives and shader cache"]
fn water_depth_admission_preserves_nulls_finalization_and_bank_exclusion() {
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = crate::archive::Catalog::open(&base).unwrap();
    let mut library =
        ProgramLibrary::open(&base, shader_cache::ShaderCacheBackend::Pc, &catalog).unwrap();
    let bytes = catalog
        .read("generated/decls/material/models/campaign/dust/water_surface_dust_01.material.decl")
        .unwrap()
        .unwrap();
    let (main, options, state) = inputs(&mut library, &bytes);
    for value in ["0", "\"\""] {
        let declaration = format!("mainProgram {main}\nzPrePassProgram {value}\n");
        let sources =
            selector::depth_sources(&mut library, declaration.as_bytes(), &main, &options, state);
        assert!(
            sources.iter().all(|s| matches!(s, Ok(None))),
            "explicit null {value}: {sources:?}"
        );
    }
    for name in [b"invisible".as_slice(), b"isDecal"] {
        let mut changed = options.clone();
        set(&mut changed, name, 1);
        assert!(
            selector::depth_sources(&mut library, &bytes, &main, &changed, state)
                .iter()
                .all(|s| matches!(s, Ok(None))),
            "{name:?}"
        );
    }
    let mut changed = options.clone();
    set(&mut changed, b"useDirectInstancing", 1);
    assert!(matches!(
        selector::depth_sources(&mut library, &bytes, &main, &changed, state)[1],
        Ok(None)
    ));
    let mut changed = options.clone();
    set(&mut changed, b"useMultiInstancing", 1);
    let banks = selector::depth_sources(&mut library, &bytes, &main, &changed, state);
    assert!(banks[0]
        .as_ref()
        .unwrap_err()
        .contains("incompatible World vertex ABI"));
    assert_eq!(
        banks[1].as_ref().unwrap().as_ref().unwrap().class,
        CachedWaterDepthClass::World
    );
    // An authored incompatible source is never replaced with MAIN's water VS.
    let declaration = format!("mainProgram {main}\nzPrePassProgram arkDefault\n");
    let banks =
        selector::depth_sources(&mut library, declaration.as_bytes(), &main, &options, state);
    assert!(banks.iter().all(Result::is_err), "{banks:?}");
}
