use super::*;
use crate::app::*;
#[test]
#[ignore = "requires installed level archives and water images; prepares actual gameplay inputs"]
fn installed_water_view_inputs_prepare_for_every_level() {
    use crate::assets::environment::ParameterSource;
    use crate::assets::level::LevelResources;
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = archive::Catalog::open(&base).unwrap();
    let mut programs =
        shader_cache::ProgramLibrary::open(&base, shader_cache::ShaderCacheBackend::Pc, &catalog);
    let mut probe_materials = Assets::default();
    let mut decal_materials = Assets::default();
    let mut sh_buffers = Assets::default();
    let parameters = entities::EnvironmentRenderParameters { layers: Vec::new() };
    let mut fog_test_images = Assets::default();
    let mut resolver = MaterialResolver {
        fog_material: crate::rendering::bevy_backend::fog_material::FogMaterialBindings::disabled(
            &mut fog_test_images,
            &mut sh_buffers,
        ),
        reflections: None,
        reflection_brdf_lookup: None,
        reflection_planes: default(),
        material_exposure: default(),
        anisotropic_sampler_image: default(),
        linear_clamp_image: default(),
        vista_sun: [Vec4::ONE, Vec4::X],
        environment: Some(&parameters),
        catalog: Some(&catalog),
        programs: Some(&mut programs),
        sky_materials: &mut Assets::default(),
        probe_materials: &mut probe_materials,
        decal_materials: &mut decal_materials,
        probe_sh_buffers: &mut sh_buffers,
        probe_sh_buffer: default(),
        vertex_probe_buffer: default(),
        shadow_buffer: default(),
        local_shadow_buffer: default(),
        shadow_map: default(),
        probe_sh_records: default(),
        probes: None,
        cache: default(),
        shadow_materials: default(),
        water_materials: default(),
        boat_materials: default(),
        diffuse_image_slots: default(),
        diffuse_image_paths: default(),
        misses: 0,
        suppressed_transparent: 0,
        cutout: 0,
        blended: 0,
        probe_samples: 0,
        probe_sample_misses: 0,
        lit: true,
    };

    let mut material_count = 0;
    for path in catalog.paths_with_suffix(".material.decl") {
        let bytes = catalog.read(&path).unwrap().unwrap();
        let Some(main) = crate::material::authored_program_override(&bytes, "mainProgram") else {
            continue;
        };
        if !main.eq_ignore_ascii_case("arkWater") {
            continue;
        }
        let name = path
            .strip_prefix("generated/decls/material/")
            .unwrap()
            .strip_suffix(".decl")
            .unwrap();
        resolver
            .get(name, 0, [0.; 3])
            .unwrap_or_else(|| panic!("missing water material {name}"));
        material_count += 1;
    }
    let materials = std::mem::take(&mut resolver.water_materials);
    drop(resolver);
    assert_eq!(material_count, 21);
    let mut count = 0;
    let mut area_count = 0;
    for path in catalog.paths_with_suffix(".bwm") {
        let resources = LevelResources::resolve(&catalog, &path).unwrap();
        let Some(bytes) = resources.read_entities(&catalog).unwrap() else {
            continue;
        };
        let document = crate::entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
        let read = |name: &str| -> Result<Option<String>, String> {
            catalog
                .read(&format!("generated/decls/env/{name}.decl"))
                .map(|v| v.map(|v| String::from_utf8_lossy(&v).into_owned()))
                .map_err(|e| e.to_string())
        };
        let parameters = if let Some(name) = crate::entities::world_environment(&document) {
            crate::entities::environment_render_parameters_with_inheritance(
                &name,
                &read(&name).unwrap().unwrap(),
                read,
            )
            .unwrap()
        } else {
            EnvironmentRenderParameters { layers: Vec::new() }
        };
        let areas =
            crate::rendering::environment::load_area_environments(&document, &catalog).unwrap();
        let (mut scene, sources) = WaterScene::load(&catalog, &parameters, &areas, &materials)
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        let camera = Entity::from_raw_u32(7).unwrap();
        let other_camera = Entity::from_raw_u32(8).unwrap();
        let first = ViewMatrices::from_camera(
            [1.0, 2.0, 3.0],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            bevy::math::Mat4::IDENTITY.to_cols_array_2d(),
        );
        let second = ViewMatrices::from_camera(
            [4.0, 5.0, 6.0],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            bevy::math::Mat4::IDENTITY.to_cols_array_2d(),
        );
        assert_eq!(scene.camera_frame(camera, 10, first).previous, first);
        assert_eq!(scene.camera_frame(camera, 11, second).previous, first);
        assert_eq!(scene.camera_frame(camera, 11, first).previous, first);
        // A replacement camera is a new view, not motion from the old camera.
        assert_eq!(
            scene.camera_frame(other_camera, 11, second).previous,
            second
        );
        let retained = sources.clone();
        let check = |view: &crate::rendering::water::parameters::WaterViewParameters| {
            assert!(sources
                .skies
                .contains_key(&view.image_source("water/skycubemap").unwrap()));
            assert!(sources
                .caustics
                .contains_key(&view.image_source("water/causticstex").unwrap()));
            crate::rendering::water::WaterSettings::resolve(view).unwrap();
        };
        let frame = scene.update(&areas, [0.; 3], 1.0, 0.016).unwrap();
        check(&frame);
        for source in materials.0.values().filter_map(|v| v.as_ref().ok()) {
            let settings = source.settings(&frame).unwrap();
            assert!(sources.skies.contains_key(&settings.sky_cube));
            assert!(sources.caustics.contains_key(&settings.caustics.image));
            let world = source.world_colour.as_ref().unwrap().as_ref().unwrap();
            let settings = world.settings(&frame).unwrap();
            assert!(sources.skies.contains_key(&settings.sky_cube));
            assert!(sources.caustics.contains_key(&settings.caustics.image));
        }
        for (i, (_, area)) in scene.areas.iter().enumerate() {
            let frame = scene
                .tracks
                .update(&scene.world, 1.0, &[(i, area, 0.0)], 1.0);
            check(&frame);
            area_count += 1;
        }
        assert!(Arc::ptr_eq(&sources.skies, &retained.skies));
        assert!(Arc::ptr_eq(&sources.caustics, &retained.caustics));
        count += 1;
    }
    assert_eq!(count, 36);
    assert_eq!(area_count, 2812);
    eprintln!("Water view inputs: {count} levels, {area_count} area snapshots, {material_count} materials; resident named cube/array identities retained");
}
