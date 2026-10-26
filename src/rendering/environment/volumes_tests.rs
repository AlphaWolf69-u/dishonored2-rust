use super::*;
use std::collections::HashMap;

#[test]
#[ignore = "requires installed level and environment declarations"]
fn installed_environment_volumes_retain_water_profiles_and_trigger_owners() {
    let catalog =
        archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let mut counts = [0usize; 6];
    for path in catalog.paths_with_suffix(".bwm") {
        let level = crate::assets::level::LevelResources::resolve(&catalog, &path).unwrap();
        let bytes = level.read_entities(&catalog).unwrap().unwrap();
        let document = entities::parse(&entities::declaration_text(&bytes)).unwrap();
        let volumes = load_environment_volumes(&document, &catalog).unwrap();
        let bindings = entities::triggers::trigger_bindings(&document, |path| {
            catalog
                .read(path)
                .map(|b| b.map(|b| entities::declaration_text(&b).into_owned()))
                .map_err(|e| e.to_string())
        })
        .unwrap();
        let mut by_entity = HashMap::<&str, Vec<_>>::new();
        for binding in &bindings {
            by_entity
                .entry(&binding.entity_id)
                .or_default()
                .push(binding);
        }
        counts[0] += 1;
        for volume in &volumes {
            let owners = by_entity
                .get(volume.placement.id.as_str())
                .unwrap_or_else(|| {
                    panic!(
                        "{path}: environment {} has no trigger owner",
                        volume.placement.id
                    )
                });
            let water = usize::from(volume.is_water_volume);
            counts[1 + water] += 1;
            counts[3 + water] += usize::from(owners.iter().any(|b| b.collision.keyframed));
            if let Some(material) = &volume.surface_material {
                assert!(
                    catalog.contains(&format!("generated/decls/material/{material}.decl")),
                    "{path}: unresolved water surface material {material}"
                );
            }
            if volume.is_water_volume {
                println!("Water volume {path}: {} material={:?}", volume.placement.id, volume.surface_material);
                counts[5] += usize::from(owners.iter().any(|b| b.water_component));
            }
        }
        if level.entities.contains("dust_p_lowchaos/")
            || level.entities.contains("eotw_p_lowchaos/")
        {
            let ordinary =
                crate::rendering::environment::load_area_environments(&document, &catalog).unwrap();
            assert_eq!(
                ordinary.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
                volumes
                    .iter()
                    .filter(|v| !v.is_water_volume)
                    .map(|v| v.placement.id.as_str())
                    .collect::<Vec<_>>(),
                "{path}"
            );
        }
    }
    assert_eq!(counts[0], 36);
    assert!(counts[1] > 0 && counts[2] > 0);
    println!(
        "worlds/ordinary/water/ordinary-keyframed/water-keyframed/water-with-component={counts:?}"
    );
}
