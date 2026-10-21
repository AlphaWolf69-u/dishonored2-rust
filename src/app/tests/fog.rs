use super::*;

#[test]
#[ignore = "requires installed environments, area shapes and atmosphere images"]
fn installed_fog_scene_assets_and_area_tracks() {
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = archive::Catalog::open(&base).unwrap();
    let mut programs =
        shader_cache::ProgramLibrary::open(&base, shader_cache::ShaderCacheBackend::Pc, &catalog)
            .unwrap();
    let read = |name: &str| -> Result<Option<String>, String> {
        catalog
            .read(&format!("generated/decls/env/{name}.decl"))
            .map(|v| v.map(|v| entities::declaration_text(&v).into_owned()))
            .map_err(|e| e.to_string())
    };
    let mut count = 0;
    for resource in catalog.paths_with_suffix(".entities") {
        let bytes = catalog.read(&resource).unwrap().unwrap();
        let document = entities::parse(&entities::declaration_text(&bytes)).unwrap();
        let Some(name) = entities::world_environment(&document) else {
            continue;
        };
        let source = read(&name).unwrap().unwrap();
        let world =
            entities::environment_render_parameters_with_inheritance(&name, &source, read).unwrap();
        let lighting = entities::environment_lighting_with_inheritance(&source, read).unwrap();
        let sun = lighting
            .sun_theta_degrees
            .zip(lighting.sun_phi_degrees)
            .zip(lighting.sun_color)
            .map(|((theta, phi), color)| {
                (
                    Vec3::from_array(render_shadow::environment_sun_direction(theta, phi)),
                    Vec3::from_array(render_light::sun_ground_color(color)),
                )
            });
        let areas =
            crate::rendering::environment::load_area_environments(&document, &catalog).unwrap();
        let mut images = Assets::default();
        let mut fog = super::super::fog_scene::FogScene::load(
            &catalog,
            &mut programs,
            &world,
            &areas,
            sun,
            7000.,
            &mut images,
        )
        .unwrap_or_else(|e| panic!("{resource}: {e}"));
        let position = entities::player_start(&document)
            .map(|p| p.position)
            .unwrap_or([0.; 3]);
        for point in [position, [1.0e7; 3], position] {
            let settings = fog.update(&areas, 1., point, 1. / 60., 7000.).unwrap();
            let shadow = fog.shadow_frame(&settings.shadow).unwrap();
            assert!(
                images.get(&shadow.image).unwrap().data.is_some(),
                "{resource}: lost projected shadow image"
            );
            assert_eq!(shadow.extent, settings.shadow.extent);
            let curve = fog.image(&settings.color_lut).unwrap();
            let noise = fog.image(&settings.noise_map).unwrap();
            let sun = fog.image(&settings.sun_sprite.image).unwrap();
            assert!(
                images.get(&sun).unwrap().data.is_some(),
                "{resource}: lost resident sun image"
            );
            let curve = images.get(&curve).unwrap();
            if curve.texture_descriptor.format == TextureFormat::Rgba8Unorm {
                assert_eq!(
                    curve.data.as_deref(),
                    material::intrinsic_image_pixels("_default").as_deref(),
                    "{resource}: only the original missing-image generator may replace a LUT"
                );
            } else {
                assert_eq!(curve.texture_descriptor.format, TextureFormat::R16Float);
            }
            assert_eq!(
                images.get(&noise).unwrap().texture_descriptor.format,
                TextureFormat::Rgba8Unorm
            );
            assert!(settings.fog.distance.is_finite() && settings.fog.vertical.is_finite());
        }
        println!(
            "Prepared fog scene {resource}: {} areas, {} resident images",
            areas.len(),
            images.len()
        );
        count += 1;
    }
    assert!(count >= 30, "only {count} level environments audited");
}

#[test]
#[ignore = "requires installed sky/cloud/interior materials and shader cache"]
fn installed_atmosphere_materials_retain_geometry_and_images() {
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
    let mut count = 0;
    let mut skies = 0;
    let mut waters = 0;
    for resource in catalog.paths_containing("generated/decls/material/") {
        let bytes = catalog.read(&resource).unwrap().unwrap();
        let surface = material_surface::Surface::parse(&bytes);
        let boat_volume = surface.program() == material_surface::Program::BoatVolume
            || entities::declaration_text(&bytes)
                .to_ascii_lowercase()
                .contains("sortboatvolume");
        if !matches!(
            surface.program(),
            material_surface::Program::InteriorVolume
                | material_surface::Program::BoatVolume
                | material_surface::Program::Sky
                | material_surface::Program::Clouds
                | material_surface::Program::Water
        ) && !boat_volume
        {
            continue;
        }
        let path = resource
            .strip_prefix("generated/decls/material/")
            .unwrap()
            .strip_suffix(".decl")
            .unwrap();
        let resolved = resolver
            .get(path, 0, [0.; 3])
            .unwrap_or_else(|| panic!("lost interior volume material {path}"));
        if surface.program() == material_surface::Program::Water && !boat_volume {
            let MaterialHandle::Water(name) = &resolved.handle else {
                panic!("water material retained a forward route: {path}");
            };
            assert_eq!(name.as_ref(), path);
            assert!(resolved.diffuse_image_slot.is_none());
            assert!(!resolved.vertex_lighting);
            let inputs = resolver.water_materials.0.get(name).unwrap()
                .as_ref().unwrap_or_else(|error|panic!("{path}: {error}"));
            assert!(inputs.main_program.eq_ignore_ascii_case("arkWater"));
            assert_eq!(inputs.declaration.as_ref(), bytes.as_slice());
            assert!(resolver.get_world(path, 0, [1.;3]).is_some_and(|m| m.handle == resolved.handle));
            assert!(resolver.get(path, 0, [2.;3]).is_some_and(|m| m.handle == resolved.handle));
            waters += 1;
            continue;
        }
        if boat_volume {
            assert!(
                matches!(&resolved.handle, MaterialHandle::BoatVolume(name) if name.as_ref()==path)
            );
            let cached = resolver.cache.get(path).unwrap().as_ref().unwrap();
            let stage_sort = cached
                .stage_sort
                .as_ref()
                .unwrap_or_else(|e| panic!("{path}: {e}"));
            assert_eq!(*stage_sort, [80f32.to_bits(), 0]);
            assert!(cached.images.is_empty());
            assert!(resolved.diffuse_image_slot.is_none());
            assert!(!resolved.vertex_lighting);
            let shadow = resolver
                .shadow_materials
                .0
                .get(&resolved.handle)
                .unwrap()
                .as_ref()
                .unwrap_or_else(|e| panic!("{path}: {e}"));
            assert_eq!(
                shadow.world_gather_flags48().unwrap() & 1,
                0,
                "boat proxy must not enter the ordinary caster gather: {path}"
            );
            let again = resolver.get(path, 0, [1.; 3]).unwrap();
            assert!(again.handle == resolved.handle);
            let world = resolver.get_world(path, 0, [2.; 3]).unwrap();
            assert!(
                world.handle == resolved.handle,
                "compatible world/model boat routes should share their material"
            );
            println!("Boat depth/stencil material retained: {path}");
            count += 1;
            continue;
        }
        if matches!(
            surface.program(),
            material_surface::Program::Sky | material_surface::Program::Clouds
        ) {
            let MaterialHandle::Sky(handle) = &resolved.handle else {
                panic!("sky material retained a forward route: {path}");
            };
            let sky = resolver.sky_materials.get(handle).unwrap();
            assert_eq!(
                sky.clouds,
                surface.program() == material_surface::Program::Clouds
            );
            assert_eq!(sky.project_far, !sky.clouds || sky.sky[4].x != 0.);
            assert!(sky.depth_offset.is_finite());
            assert!(sky.depth_uv.is_finite());
            let compiled = material::image_load_resource(&catalog, &surface.sky_depth).unwrap();
            assert!(
                resolver
                    .diffuse_image_paths
                    .iter()
                    .any(|path| *path == compiled),
                "missing sky depth {path}: {compiled}"
            );
            assert!(!resolved.vertex_lighting);
            println!(
                "SKY {path}: clouds={} depth={} offset={}",
                sky.clouds, surface.sky_depth, sky.depth_offset
            );
            skies += 1;
            continue;
        }
        assert!(matches!(&resolved.handle, MaterialHandle::Interior(name) if name.as_ref()==path));
        assert!(!resolved.vertex_lighting);
        assert!(resolved.diffuse_image_slot.is_none());
        let shadow = resolver
            .shadow_materials
            .0
            .get(&resolved.handle)
            .unwrap()
            .as_ref()
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        let flags = shadow
            .world_gather_flags48()
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        println!("Interior fog {path}: shadow gather flags={flags:02x}");
        count += 1;
    }
    assert!(count > 0);
    assert!(resolver.probe_materials.is_empty());
    assert!(resolver.decal_materials.is_empty());
    assert!(skies > 0);
    assert!(waters > 0);
    assert_eq!(waters, resolver.water_materials.0.len());
    assert_eq!(skies, resolver.sky_materials.len());
    println!(
        "Preserved {count} interior/boat and {skies} sky/cloud materials without forward-colour allocations"
    );
}
