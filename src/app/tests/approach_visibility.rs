use super::*;

#[test]
#[ignore = "requires installed material and renderParm declarations"]
fn installed_decal_depth_offsets_use_shared_uniform_resolution() {
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = archive::Catalog::open(&base).unwrap();
    let mut library =
        shader_cache::ProgramLibrary::open(&base, shader_cache::ShaderCacheBackend::Pc, &catalog)
            .unwrap();
    let program = catalog
        .read("generated/decls/renderprog/arkdecal.decl")
        .unwrap()
        .unwrap();
    assert!(material::program_option(b"{}", &program, "isDecal"));
    for name in ["arkDecal", "arkdecal", "ARKDECAL", "\"arkDecal\""] {
        for parameters in [
            "zbias -0.0001",
            "ZBIAS -0.0001",
            "otherUniform 3 zbias { -0.0001, 0, 0, 0 }",
            "options { zbias -1 } zbias -0.0001",
        ] {
            let bytes = format!("{{\nmainProgram {name}\n{parameters}\n}}");
            let surface = material_surface::Surface::parse(bytes.as_bytes());
            assert!(surface
                .main_program
                .as_deref()
                .unwrap()
                .eq_ignore_ascii_case("arkDecal"));
            let [[bias, ..]] =
                material_float_vectors(&mut library, &catalog, bytes.as_bytes(), ["zbias"])
                    .unwrap();
            assert_eq!(bias, -0.0001, "{bytes}");
        }
    }
    assert_eq!(
        material_float_vectors(
            &mut library,
            &catalog,
            b"{\nmainProgram arkdecal\n}",
            ["zbias"]
        )
        .unwrap(),
        [[0.0; 4]]
    );
    let mut spellings = std::collections::BTreeMap::<String, usize>::new();
    let mut defaults = 0;
    for path in catalog.paths_containing("generated/decls/material/") {
        let bytes = catalog.read(&path).unwrap().unwrap();
        let surface = material_surface::Surface::parse(&bytes);
        let Some(main) = surface
            .main_program
            .as_deref()
            .filter(|name| name.eq_ignore_ascii_case("arkDecal"))
        else {
            continue;
        };
        *spellings.entry(main.to_owned()).or_default() += 1;
        let resolved = material_float_vectors(&mut library, &catalog, &bytes, ["zbias"]).unwrap();
        let overrides =
            entities::material_float_parameters(&entities::declaration_text(&bytes), |name| {
                name == "zbias"
            })
            .unwrap();
        let expected = overrides
            .last()
            .map(|(_, words)| words.map(f32::from_bits))
            .unwrap_or([0.0; 4]);
        defaults += usize::from(overrides.is_empty());
        assert_eq!(resolved[0], expected, "decal Uniform binding {path}");
    }
    assert!(spellings.get("arkdecal").copied().unwrap_or(0) > 0);
    assert!(spellings.get("arkDecal").copied().unwrap_or(0) > 0);
    println!("Installed decal depth offsets: spellings={spellings:?} default_zero={defaults}");

    // Exercise the actual loader, not only the shared Uniform decoder: these
    // assets mix arkdecal/arkDecal spellings. The old loader lost the former's
    // bias even though decoding the Uniform on its own already worked.
    let mut programs = Ok(library);
    let mut fog_test_buffers = Assets::default();
    let mut fog_test_images = Assets::default();
    let mut resolver = MaterialResolver {
        fog_material: crate::rendering::bevy_backend::fog_material::FogMaterialBindings::disabled(
            &mut fog_test_images,
            &mut fog_test_buffers,
        ),
        reflections: None,
        reflection_brdf_lookup: None,
        reflection_planes: default(),
        material_exposure: default(),
        anisotropic_sampler_image: default(),
        linear_clamp_image: default(),
        vista_sun: [Vec4::ZERO; 2],
        environment: None,
        catalog: Some(&catalog),
        programs: Some(&mut programs),
        sky_materials: &mut Assets::default(),
        probe_materials: &mut Assets::default(),
        decal_materials: &mut Assets::default(),
        probe_sh_buffers: &mut Assets::default(),
        probe_sh_buffer: default(),
        vertex_probe_buffer: default(),
        shadow_buffer: default(),
        local_shadow_buffer: default(),
        shadow_map: default(),
        probe_sh_records: default(),
        probes: None,
        cache: default(),
        shadow_materials: default(),
        vegetation: default(),
        water_materials: default(),
        water_model_colours: default(),
        water_stencil_materials: default(),
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
    for path in [
        "models/environment/generic/decals/walls/dcl_plaster_holes_bottom_par_02.material",
        "models/environment/generic/decals/walls/dcl_plaster_leaks_03_dark.material",
        "models/environment/generic/decals/street_floors/leaves_dirt_01.material",
        "models/environment/generic/decals/ghost_sign_01/ghost_sign_razina_01.material",
    ] {
        let resolved = resolver
            .get(path, 0, [-27.582226, -26.091814, -17.269775])
            .unwrap();
        let MaterialHandle::Decal(handle) = resolved.handle else {
            panic!("expected decal {path}")
        };
        let bias = resolver
            .decal_materials
            .get(&handle)
            .unwrap()
            .extension
            .bias;
        assert_eq!(bias, -0.0001, "loaded vertex Uniform {path}");
        println!("Resolved decal {path}: vertex zbias={bias}");
    }
}

#[test]
fn short_range_lights_keep_finite_clusters_when_approached() {
    use bevy::camera::{primitives::Frustum, CameraProjection};
    use bevy::light::cluster::{ClusterableObjects, Clusters, GlobalClusterSettings};
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::asset::AssetPlugin::default(),
        bevy::gizmos::GizmoPlugin,
        bevy::light::LightPlugin,
    ));
    app.init_asset::<Image>();
    app.init_asset::<Mesh>();
    app.init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>();
    app.insert_resource(GlobalClusterSettings {
        supports_storage_buffers: true,
        clustered_decals_are_usable: false,
        gpu_clustering: None,
        max_uniform_buffer_clusterable_objects: 256,
        view_cluster_bindings_max_indices: 16384,
    });
    let camera = app.world_mut().spawn(Camera3d::default()).id();
    let lights: Vec<_> = [Vec3::new(-0.2, 0.2, 0.0), Vec3::new(0.2, -0.2, 0.0)]
        .into_iter()
        .map(|position| {
            app.world_mut()
                .spawn((
                    PointLight {
                        range: 1.0,
                        ..default()
                    },
                    GlobalTransform::from(Transform::from_translation(position)),
                    ViewVisibility::VISIBLE,
                ))
                .id()
        })
        .collect();
    for distance in [6.0, 4.0, 2.0, 0.5, 2.0, 6.0] {
        let pose = Transform::from_xyz(0.0, 0.0, distance);
        let mut projection = camera_projection::GameProjection::new(110.0, 0.01, 7000.0);
        projection.update(1920.0, 1080.0);
        let clip = projection.get_clip_from_view();
        let frustum = Frustum(
            bevy::math::primitives::ViewFrustum::from_clip_from_world_custom_far(
                &(clip * pose.to_matrix().inverse()),
                &pose.translation,
                &pose.back(),
                7000.0,
            ),
        );
        let mut value = Camera::default();
        value.viewport = Some(bevy::camera::Viewport {
            physical_size: UVec2::new(1920, 1080),
            ..default()
        });
        value.computed.clip_from_view = clip;
        app.world_mut()
            .entity_mut(camera)
            .insert((value, GlobalTransform::from(pose), frustum));
        // Assignment's far bound comes from the previous frame.
        for _ in 0..3 {
            app.update();
        }
        let clusters = app.world().get::<Clusters>(camera).unwrap();
        let factors = bevy::light::cluster::assign::calculate_cluster_factors(
            clusters.near,
            clusters.far,
            clusters.dimensions.z as f32,
            false,
        );
        assert!(factors.is_finite(), "distance={distance} {clusters:?}");
        assert_eq!(clusters.dimensions.z == 1, distance < 5.0);
        let ClusterableObjects::Cpu(cells) = &clusters.clusterable_objects else {
            panic!("expected CPU assignment")
        };
        for light in &lights {
            assert!(
                cells.iter().any(|cell| cell.iter().any(|id| id == light)),
                "light disappeared at distance {distance}"
            );
        }
    }
}

#[test]
#[ignore = "requires installed archives and saved visibility reports"]
fn saved_dust_approach_visibility_queries() {
    bevy::tasks::ComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
    let catalog =
        archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let level = crate::assets::level::LevelResources::resolve(&catalog, "dust_p_lowchaos").unwrap();
    let bytes = level.read_tome(&catalog).unwrap().unwrap();
    let tree = tome::SpatialTree::parse(&bytes).unwrap();
    let world = TomeWorld {
        sphere_graph: tome::SphereCellGraph::parse(&bytes),
        gate_states: tome::initial_gate_states(&bytes).unwrap(),
        query_data: tome::DirectTomeQueryData::parse(&bytes, &tree).unwrap(),
        bytes,
        tree,
        query_job_count: tome::DEFAULT_QUERY_JOB_COUNT,
        query_pool: None,
    };
    let bwm = world_mesh::parse(&level.read_world(&catalog).unwrap()).unwrap();
    let document = entities::parse(&String::from_utf8_lossy(
        &level.read_entities(&catalog).unwrap().unwrap(),
    ))
    .unwrap();
    let lights = entities::placed_lights_with_inheritance(&document, |path| {
        catalog
            .read(path)
            .map(|v| v.map(|b| String::from_utf8_lossy(&b).into_owned()))
            .map_err(|e| e.to_string())
    })
    .unwrap();
    let mut bwm_instances = vec![Vec::new(); bwm.visibility.len()];
    let mut materials = Vec::new();
    for object in &bwm.objects {
        for (matrix, instances) in bwm_instances
            .iter_mut()
            .enumerate()
            .take(object.matrix_end)
            .skip(object.matrix_start)
        {
            instances.push(materials.len());
            materials.push((matrix, &object.material_path));
        }
    }
    let mapping = TomeRenderInstanceMap::from_world_mapping(&bwm, &bwm_instances);
    let settings = game_settings::read_view_settings().unwrap().1;
    for (number, stamp) in [
        1792344320893u64,
        1792344323794,
        1792344327429,
        1792344333849,
        1792344421365,
        1792344424409,
        1792344436193,
    ]
    .into_iter()
    .enumerate()
    {
        let report =
            fs::read_to_string(format!("artifacts/collision-reports/collision-{stamp}.txt"))
                .unwrap();
        let transform = f8_camera_pose(&report).unwrap();
        let pose = game_pose_from_view_transform(&transform);
        let fov = f8_reference_fov(&report).unwrap();
        let rows =
            tome::world_visibility_query_matrix(fov, 1920, 1080, 1.0, pose, 0.01, 7000.0).unwrap();
        let input = tome::OrdinaryDirectQueryInput {
            camera_point: [pose[0][3], pose[1][3], pose[2][3]],
            render_rows: rows,
            cropped_rows: rows,
            initial_tile_rect: [0, 0, 8, 16],
            camera_imprecision: [settings.umbra_distance_imprecision; 3],
            object_imprecision: settings.umbra_distance_imprecision,
            minimum_area_metric: 25.0 / (1920.0 * 1080.0),
            global_distance_threshold: settings.umbra_accurate_threshold,
            object_distance_scale: view_distance_scale(
                &Projection::custom(camera_projection::GameProjection::new(fov, 0.01, 7000.0)),
                UVec2::new(1920, 1080),
                Some(&ViewSettings(Ok(settings.clone()))),
            )
            .unwrap(),
            winding_flip: tome::query_matrix_winding_flip(rows),
            gate_states: Some(&world.gate_states),
            output_capacity: tome::WORLD_QUERY_OUTPUT_CAPACITY,
        };
        let jobs = run_tome_render_jobs(&world, &input);
        let output = tome::merge_render_query_outputs(&jobs).unwrap().unwrap();
        let state = output.state.as_ref().unwrap();
        let query = tome::RenderAabbQuery::from_state(state).unwrap();
        let mut visible = vec![false; materials.len()];
        mapping
            .mark_visible_instances(output.indices(), &mut visible)
            .unwrap();
        mapping
            .apply_spatial_queries(Some(state), &mut visible)
            .unwrap();
        let ids = mapping
            .visible_authored_ids(&world.bytes, output.indices())
            .unwrap();
        println!(
            "capture {} objects={} unique_cells={}",
            number + 1,
            output.indices().len(),
            output
                .cell_indices()
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        );
        let mut projection = camera_projection::GameProjection::new(fov, 0.01, 7000.0);
        bevy::camera::CameraProjection::update(&mut projection, 1920.0, 1080.0);
        let clip = bevy::camera::CameraProjection::get_clip_from_view(&projection)
            * transform.to_matrix().inverse();
        for light in &lights {
            if Vec3::from_array(light.position).distance(Vec3::new(-83.7, -51.7, -26.1)) > 4.0 {
                continue;
            }
            let linear = Color::srgb_from_array(light.color).to_linear();
            let Some(preset) = light_preset(light, [linear.red, linear.green, linear.blue]) else {
                continue;
            };
            let matrix = local_light_clip_to_world(
                game_to_view(light.position),
                light.rotation,
                light.kind,
                preset.range,
                (light.kind == entities::LightType::Spot)
                    .then(|| authored_light_cosines(light.angular_cutoff, light.angular_falloff).0),
            )
            .unwrap();
            let (low, high) = light_projection_bounds(matrix).unwrap();
            let bounds = [[low.x, -high.z, low.y], [high.x, -low.z, high.y]];
            let sphere = [
                light.position[0] + preset.center_offset[0],
                light.position[1] + preset.center_offset[1],
                light.position[2] + preset.center_offset[2],
                preset.range,
            ];
            let cell = world
                .query_data
                .point_cell_id(&world.bytes, &world.tree, [sphere[0], sphere[1], sphere[2]])
                .unwrap();
            let mask =
                tome::cell_visibility_mask(world.query_data.cell_count(), output.cell_indices())
                    .unwrap();
            let result = world
                .sphere_graph
                .as_ref()
                .unwrap()
                .query_spheres(
                    &world.bytes,
                    &mut tome::SphereCellWorker::default(),
                    &[sphere],
                    &[cell],
                    &mask,
                    Some(&world.gate_states),
                    None,
                    1,
                )
                .unwrap();
            println!(
                "  light={} hash={} preset={:?} position={:?} sphere={sphere:?} idpass={} aabb={:?} spherepass={} volume={:?}",
                light.id,
                light.definition_name_hash,
                light.preset,
                light.position,
                local_light_id_pass(
                    light.kind,
                    light.culling,
                    light.definition_name_hash,
                    Some(&ids)
                ),
                query.visibility_and_area(bounds).unwrap(),
                result.count,
                light_volume_intersects_camera(clip, matrix)
            );
            if number >= 4 {
                assert!(local_light_id_pass(
                    light.kind,
                    light.culling,
                    light.definition_name_hash,
                    Some(&ids)
                ));
                assert!(query.visibility_and_area(bounds).unwrap().0);
                assert_eq!(result.count, 1);
                assert_eq!(light_volume_intersects_camera(clip, matrix), Some(true));
            }
        }
        for (index, (matrix, material)) in materials.iter().enumerate() {
            let bounds = bwm.visibility[*matrix].bounds;
            let point = [-88.3, -55.75, -26.0];
            let near = (0..3).all(|axis| {
                bounds[0][axis] - 0.4 <= point[axis] && point[axis] <= bounds[1][axis] + 0.4
            });
            if near
                && (material.contains("decal")
                    || material.contains("poster")
                    || material.contains("plaster"))
            {
                println!(
                    "  matrix={matrix} visible={} bounds={bounds:?} material={material}",
                    visible[index]
                );
                if number < 4 {
                    assert!(visible[index], "reported wall surface must remain selected");
                }
            }
        }
        let mut raw = b"DH2DQ001".to_vec();
        for value in state.viewport {
            raw.extend_from_slice(&value.to_le_bytes());
        }
        for value in state
            .render_rows
            .iter()
            .flatten()
            .chain(state.cropped_rows.iter().flatten())
            .chain(state.camera_imprecision.iter())
            .chain(std::iter::once(&state.maximum_depth))
            .chain([0.0f32; 6].iter())
            .chain(state.depth_grid.as_ref().unwrap().iter())
        {
            raw.extend_from_slice(&value.to_le_bytes());
        }
        raw.extend_from_slice(&123.0f32.to_le_bytes());
        raw.extend_from_slice(&(world.query_data.object_count() as u32).to_le_bytes());
        for index in 0..world.query_data.object_count() as u32 {
            let bounds = tome::object_bounds(&world.bytes, index).unwrap();
            let (accepted, area) = query.visibility_and_area(bounds).unwrap();
            raw.extend_from_slice(&index.to_le_bytes());
            for value in bounds.iter().flatten() {
                raw.extend_from_slice(&value.to_le_bytes());
            }
            raw.extend_from_slice(&u32::from(accepted).to_le_bytes());
            raw.extend_from_slice(&area.to_le_bytes());
        }
        let fixture = format!("artifacts/dust-approach-{}-depth.bin", number + 1);
        fs::write(&fixture, raw).unwrap();
        let game = std::process::Command::new(
            env::var("DH2_ORACLE_PYTHON").unwrap_or_else(|_| "python".into()),
        )
        .args(["analysis/probe_original_depth_aabb.py", "--state", &fixture])
        .output()
        .unwrap();
        assert!(
            game.status.success(),
            "installed depth-query parity: {}{}",
            String::from_utf8_lossy(&game.stdout),
            String::from_utf8_lossy(&game.stderr)
        );
        print!("{}", String::from_utf8_lossy(&game.stdout));
    }
}
