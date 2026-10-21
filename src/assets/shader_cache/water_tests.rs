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
