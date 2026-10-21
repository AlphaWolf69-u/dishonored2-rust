use super::*;
use crate::assets::entities;

#[test]
#[ignore = "requires installed level environments and image declarations"]
fn installed_ocean_settings_resolve_all_levels() {
    let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let read = |name: &str| -> Result<Option<String>, String> {
        catalog
            .read(&format!("generated/decls/env/{name}.decl"))
            .map(|bytes| bytes.map(|bytes| entities::declaration_text(&bytes).into_owned()))
            .map_err(|e| e.to_string())
    };
    let mut levels = 0;
    let mut enabled = 0;
    for path in catalog.paths_with_suffix(".entities") {
        let bytes = catalog.read(&path).unwrap().unwrap();
        let document = entities::parse(&entities::declaration_text(&bytes)).unwrap();
        let Some(name) = entities::world_environment(&document) else {
            continue;
        };
        let source = read(&name).unwrap().unwrap();
        let environment =
            entities::environment_render_parameters_with_inheritance(&name, &source, read).unwrap();
        let settings = OceanSettings::load(&catalog, &environment)
            .unwrap_or_else(|error| panic!("{path}: {error}"));
        // These are input identities, not substitutes for FFT/collision output.
        for image in [
            &settings.surface.shore_distance_image,
            &settings.surface.wave_amplitude_image,
            &settings.surface.foam_image,
            &settings.projected_shadow.image,
        ] {
            let resource = crate::material::image_load_resource(&catalog, image)
                .unwrap_or_else(|| panic!("{path}: unresolved ocean image {image}"));
            assert!(
                resource != "_default" || image == "_default",
                "{path}: missing actual ocean image{image}"
            );
            if crate::material::intrinsic_image_pixels(&resource).is_none() {
                let bytes = catalog.read(&resource).unwrap().unwrap();
                crate::texture::BimageInfo::parse(&bytes)
                    .unwrap_or_else(|error| panic!("{path}: {resource}: {error}"));
            }
        }
        enabled += usize::from(settings.view.enabled);
        levels += 1;
        eprintln!(
            "{path}: ocean={}, volume={}, level={}, wind={:?}",
            settings.view.enabled,
            settings.view.water_volume,
            settings.view.ocean_water_level,
            settings.view.wind
        );
    }
    assert_eq!(levels, 36);
    eprintln!("Resolved{levels} installed world environments;{enabled} enable ocean rendering");
}
