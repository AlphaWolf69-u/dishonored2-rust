use super::*;
use crate::assets::{entities, environment::ParameterReader, level::LevelResources};

#[test]
#[ignore = "requires installed level archives; audits the prepared ocean schema across every level/area"]
fn installed_ocean_environment_prepares_all_level_and_area_inputs() {
    let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let read = |name: &str| {
        catalog
            .read(&format!("generated/decls/env/{name}.decl"))
            .map(|v| v.map(|v| entities::declaration_text(&v).into_owned()))
            .map_err(|e| e.to_string())
    };
    let mut levels = 0;
    let mut count = 0;
    let mut checks = 0;
    for path in catalog.paths_with_suffix(".bwm") {
        let level = LevelResources::resolve(&catalog, &path).unwrap();
        let Some(bytes) = level.read_entities(&catalog).unwrap() else {
            continue;
        };
        let doc = entities::parse(&entities::declaration_text(&bytes)).unwrap();
        let Some(name) = entities::world_environment(&doc) else {
            continue;
        };
        let text = read(&name).unwrap().unwrap();
        let parameters =
            entities::environment_render_parameters_with_inheritance(&name, &text, read).unwrap();
        let areas = crate::rendering::environment::load_area_environments(&doc, &catalog).unwrap();
        let mut ocean = OceanEnvironment::load(&catalog, &parameters, &areas)
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        for (prepared, source) in std::iter::once((&ocean.world, &parameters)).chain(
            ocean
                .areas
                .iter()
                .zip(&areas)
                .map(|((_, p), a)| (p, &a.parameters)),
        ) {
            let reader = ParameterReader {
                catalog: &catalog,
                parameters: source,
            };
            for (name, value) in &prepared.values {
                match value {
                    EnvironmentValue::Float(v) => {
                        assert_eq!(v.to_array(), reader.vector(name).unwrap(), "{path}:{name}")
                    }
                    EnvironmentValue::Boolean(v) => {
                        assert_eq!(*v, reader.boolean(name).unwrap(), "{path}:{name}")
                    }
                    EnvironmentValue::Image(v) => assert!(
                        crate::material::image_load_resource(
                            &catalog,
                            &reader.image_source(name).unwrap()
                        )
                        .unwrap()
                        .eq_ignore_ascii_case(v),
                        "{path}:{name}"
                    ),
                }
                checks += 1;
            }
        }
        for image in ocean.image_sources() {
            assert!(
                catalog.contains(image)
                    || crate::material::intrinsic_image_pixels(image).is_some()
                    || crate::material::intrinsic_cube_image(image).is_some(),
                "{path}:{image}"
            );
        }
        for (i, (_, area)) in ocean.areas.iter().enumerate() {
            let frame = ocean
                .tracks
                .update(&ocean.world, 0., &[(i, area, 0.)], 1.)
                .unwrap();
            OceanSettings::resolve(frame).unwrap();
            count += 1;
        }
        for position in [[0.; 3], [1.0e7; 3], [0.; 3]] {
            let settings = ocean.update(&areas, position, 1., 1. / 60.).unwrap();
            assert!(settings.surface.scale.iter().all(|v| v.is_finite()));
        }
        levels += 1;
    }
    assert_eq!((levels, count), (36, 2812));
    println!("{levels} levels/{count} areas/{checks} typed ocean inputs; prepared identities resolve without frame archive reads");
}
