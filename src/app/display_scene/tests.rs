use super::*;
#[test]
#[ignore = "requires installed level archives; prepares the actual gameplay display owner"]
fn installed_gameplay_display_inputs_prepare_for_every_level() {
    use crate::assets::level::LevelResources;
    let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let mut count = 0;
    let mut area_count = 0;
    for path in catalog.paths_with_suffix(".bwm") {
        let resources = LevelResources::resolve(&catalog, &path).unwrap();
        let Some(bytes) = resources.read_entities(&catalog).unwrap() else {
            continue;
        };
        let doc = crate::entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
        let parameters = if let Some(name) = crate::entities::world_environment(&doc) {
            let bytes = catalog
                .read(&format!("generated/decls/env/{name}.decl"))
                .unwrap()
                .unwrap();
            crate::entities::environment_render_parameters_with_inheritance(
                &name,
                &String::from_utf8_lossy(&bytes),
                |parent| {
                    catalog
                        .read(&format!("generated/decls/env/{parent}.decl"))
                        .map(|v| v.map(|b| String::from_utf8_lossy(&b).into_owned()))
                        .map_err(|e| e.to_string())
                },
            )
            .unwrap()
        } else {
            EnvironmentRenderParameters { layers: Vec::new() }
        };
        let areas = crate::rendering::environment::load_area_environments(&doc, &catalog).unwrap();
        let mut images = Assets::<Image>::default();
        let mut scene = DisplayScene::load(&catalog, &parameters, &areas, &mut images)
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        let resident_count = images.len();
        let camera = scene.camera_effects
            .update(&areas, [0., 0., 0.], 1., 0.016)
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        let combine = camera.settings.combine(camera.far_clip);
        assert!(combine.far.into_iter().all(f32::is_finite), "{path}");
        let (frame, cube, weight, exposure, lens) = scene
            .update(&areas, [0., 0., 0.], 1., 0.016, 0.0)
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(
            images.len(),
            resident_count,
            "frame allocated an image: {path}"
        );
        for handle in [
            &lens.dirt.from,
            &lens.dirt.to,
            &lens.profile.from,
            &lens.profile.to,
        ] {
            let image = images.get(handle).unwrap();
            assert_eq!(image.texture_descriptor.dimension, TextureDimension::D2);
            assert!(image.data.as_ref().is_some_and(|v| !v.is_empty()), "{path}");
        }
        let dirt = images.get(&lens.dirt.from).unwrap();
        assert_eq!(
            dirt.texture_descriptor.format,
            TextureFormat::Bc7RgbaUnormSrgb,
            "{path}"
        );
        assert_eq!(
            (
                dirt.texture_descriptor.size.width,
                dirt.texture_descriptor.size.height
            ),
            (1280, 720),
            "{path}"
        );
        assert!(
            lens.veil_size.is_finite()
                && lens.veil_strength.is_finite()
                && lens.profile.weight.is_finite(),
            "{path}"
        );
        if lens.settings.skip_tone_mapping || !lens.settings.exposure_use_tone_mapping_white_level {
            assert_eq!(lens.white_level, 1.0);
        }
        assert_eq!(exposure.adaptation.y, lens.white_level);
        assert_eq!(frame.use_curve, !lens.settings.skip_tone_mapping);
        assert_eq!(
            lens.use_default_dirt,
            lens.dirt.from.id() == lens.dirt.to.id()
        );
        assert!(
            frame.gamma.is_finite() && frame.brightness.is_finite() && weight.is_finite(),
            "{path}"
        );
        exposure.validate().unwrap();
        let image = images.get(&cube).unwrap();
        assert_eq!(
            image.texture_descriptor.dimension,
            TextureDimension::D3,
            "{path}"
        );
        assert_eq!(
            image.texture_descriptor.size,
            Extent3d {
                width: 32,
                height: 32,
                depth_or_array_layers: 32
            },
            "{path}"
        );
        assert!(
            frame
                .curve
                .toe
                .into_iter()
                .chain(frame.curve.shoulder)
                .all(f32::is_finite),
            "{path}"
        );
        eprintln!(
            "display prepared {path}: {} areas, {} resident display images",
            areas.len(),
            images.len()
        );
        area_count += areas.len();
        count += 1;
    }
    assert!(count >= 36);
    eprintln!("{count} actual gameplay display owners / {area_count} area dictionaries prepared");
}
