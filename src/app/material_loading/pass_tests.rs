use super::*;

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
