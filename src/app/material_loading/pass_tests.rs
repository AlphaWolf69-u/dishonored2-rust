use super::*;

#[test]
#[ignore = "requires installed decal material, MAIN definitions and option defaults"]
fn installed_decal_routes_keep_ordinary_decals_before_atmosphere() {
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = archive::Catalog::open(&base).unwrap();
    let mut library = Ok(shader_cache::ProgramLibrary::open(
        &base,
        shader_cache::ShaderCacheBackend::Pc,
        &catalog,
    )
    .unwrap());
    let program = catalog
        .read("generated/decls/renderprog/arkdecal.decl")
        .unwrap()
        .unwrap();
    let captured = catalog
        .read("generated/decls/material/models/environment/generic/decals/street_floors/leaves_dirt_01.material.decl")
        .unwrap()
        .unwrap();
    for (bytes, expected) in std::iter::once((captured, 1 << 9)).chain((0..4).map(|kind| {
        (
            format!("{{\nmainProgram arkDecal\ndecalType {kind}\n}}\n").into_bytes(),
            if matches!(kind, 0 | 3) { 1 << 9 } else { 8 },
        )
    })) {
        let source =
            resolve_shadow_source_inputs(Some(&mut library), &bytes, &program, Some("arkDecal"))
                .unwrap();
        let flags =
            forward_fog_flags(library.as_mut().unwrap(), &bytes, "arkDecal", &source).unwrap();
        assert_eq!(
            flags & ((1 << 9) | 8),
            expected,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
    }
}

#[test]
#[ignore = "requires installed sort constants, MAIN definitions and option defaults"]
fn installed_boat_routes_preserve_world_and_model_source_rules() {
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = archive::Catalog::open(&base).unwrap();
    let mut library =
        shader_cache::ProgramLibrary::open(&base, shader_cache::ShaderCacheBackend::Pc, &catalog)
            .unwrap();
    for (adjust, extra, expected) in [
        (0., "", [true, true]),
        (0.5, "", [false, true]),
        (20., "", [false, false]),
        (
            0.,
            "state { blend SRC_ALPHA ONE_MINUS_SRC_ALPHA }",
            [false, true],
        ),
        (0., "needsWind 1", [false, true]),
        (0., "hairType 1", [true, false]),
        (0., "skyType 1", [false, false]),
        (0., "isDecal 1", [false, false]),
        (0., "sssProgram arkDefault", [true, false]),
        (0., "rendering/darkVisionLayer 1", [false, true]),
    ] {
        let (header, root) = if extra.starts_with("state") {
            (extra, "")
        } else {
            ("", extra)
        };
        let source = format!("{{\n{header}\nparms {{ stageSort sortBoatVolume }}\nmainProgram arkDefault\nstageSortAdjust {adjust}\n{root}\n}}");
        let sort = library
            .material_stage_sort(&catalog, b"arkDefault", &source)
            .unwrap();
        let actual = boat_volume_routes(
            &mut library,
            &catalog,
            source.as_bytes(),
            "arkDefault",
            sort,
        )
        .unwrap();
        assert_eq!(actual, expected, "adjust={adjust}, overrides={extra}");
    }
}
