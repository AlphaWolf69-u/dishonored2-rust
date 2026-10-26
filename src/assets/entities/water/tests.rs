use super::*;

#[test]
fn absent_water_fields_match_executed_declaration_constructor() {
    let document = super::super::parse(
        r#"Version 6
component { cpntWater defaults {} }
entity { entityDef pool {class="arkAreaVolume";edit={m_componentDecls={
item_add["water"]={mapComponentDecl="defaults";}}}} }
"#,
    )
    .unwrap();
    let bindings = water_bindings(&document, |_| {
        panic!("map-local declaration needs no archive read")
    })
    .unwrap();
    let water = &bindings[0];
    let mut actual = vec![
        water.scheduling.update_mask,
        water.scheduling.start_phases,
        water.scheduling.priority as u32,
        u32::from(water.scheduling.flags),
        u32::from(water.water_material.is_some()),
        u32::from(water.ocean_material.is_some()),
        water.density.to_bits(),
    ];
    actual.extend(water.current.map(f32::to_bits));
    actual.extend([water.wavelet_amplitude.to_bits(), u32::from(water.is_ocean)]);
    let expected: Vec<_> =
        include_str!("../../../../tests/fixtures/water_declaration_defaults.txt")
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
    assert_eq!(actual, expected);
}

#[test]
fn water_inheritance_keeps_spawn_admission_and_partial_vector_overlays() {
    let document = super::super::parse(r#"Version 6
component { cpntWater shared {inherit="flow.cpntwater";edit={m_waterCurrent={y=-3;} m_startOnEntitySpawn=false;} } }
entity { entityDef pool {class="arkAreaVolume";edit={m_componentDecls={
item_add["water"]={mapComponentDecl="shared";inlineDecl={m_waterCurrent={z=2;} m_isOcean=true;}}
}}} }
"#).unwrap();
    let bindings = water_bindings(&document, |path| {
        assert_eq!(path, "generated/decls/cpntwater/flow.cpntwater.decl");
        Ok(Some(
            r#"{edit={m_waterPhysicsMaterial="water";m_oceanPhysicsMaterial="ocean";
m_waterDensity=700.0f; m_waterCurrent={x=1;y=4;z=5;}
m_entityContext="ENT_CONTEXT_FLAG_ALIVE ENT_CONTEXT_FLAG_POSSESSED";
m_autoPauseLODLevel="LOD_LEVEL_HIDDEN";}}"#
                .into(),
        ))
    })
    .unwrap();
    let water = &bindings[0];
    assert_eq!(water.current, [1., -3., 2.]);
    assert_eq!(water.density, 700.);
    assert_eq!(water.scheduling.start_phases, 17);
    assert_eq!(water.scheduling.priority, 6);
    assert!(!water.scheduling.starts_on_spawn());
    let source = water
        .startup(|name| match name {
            "water" => Ok(11),
            "ocean" => Ok(22),
            _ => panic!("unexpected material"),
        })
        .unwrap();
    assert_eq!(source.materials, [Some(11), Some(22)]);
    assert!(source.is_ocean);
    assert_eq!(source.current, water.current);
    assert_eq!(source.density, water.density);
}

#[test]
#[ignore = "requires installed level and component archives"]
fn installed_water_declarations_resolve_all_authored_inputs() {
    use crate::assets::{archive::Catalog, level::LevelResources};
    let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let paths: std::collections::BTreeSet<_> = catalog
        .paths_with_suffix(".bwm")
        .iter()
        .map(|path| LevelResources::resolve(&catalog, path).unwrap().entities)
        .collect();
    let mut total = 0;
    for path in &paths {
        let bytes = catalog.read(path).unwrap().unwrap();
        let document =
            super::super::parse(&super::super::declaration_text(&bytes)).unwrap();
        let bindings = water_bindings(&document, |path| {
            catalog
                .read(path)
                .map(|v| v.map(|b| super::super::declaration_text(&b).into_owned()))
                .map_err(|e| e.to_string())
        })
        .unwrap_or_else(|e| panic!("{path}: {e}"));
        for water in &bindings {
            assert!(water.density.is_finite() && water.wavelet_amplitude.is_finite());
            assert!(water.current.iter().all(|v| v.is_finite()));
            for name in [&water.water_material, &water.ocean_material]
                .into_iter()
                .flatten()
            {
                assert!(
                    catalog.contains(&format!("generated/decls/physicsmaterial/{name}.decl")),
                    "{path}: missing {name}"
                );
            }
        }
        total += bindings.len();
    }
    assert_eq!(paths.len(), 36);
    assert!(total > 0);
    println!(
        "{} installed levels; {total} authored water components",
        paths.len()
    );
}
