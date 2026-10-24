use super::*;
use super::{
    controls::*, diagnostics::*, launch::load_installed_shadow_quality, light_culling::*,
    material_loading::*, shadow_preparation::*, state::*, texture_streaming::*, view_geometry::*,
    visibility_queries::*,
};
use crate::{render_resource, tiled_lighting_gpu};
use bevy::camera::PerspectiveProjection;
use bevy::prelude::*;
use std::path::Path;

/// Bevy keys mesh attributes by id: a shared id silently replaces one
/// channel with another and makes every layout check match both.
#[test]
fn custom_vertex_attribute_ids_are_distinct() {
    use crate::rendering::bevy_backend::{shadow_pass, water_world_colour};
    let ids = [
        DH2_BITANGENT,
        DH2_NORMAL_W,
        super::materials::DH2_MODEL_DEPTH_HACK,
        shadow_pass::VEGETATION_NORMAL,
        shadow_pass::VEGETATION_COLOR,
        shadow_pass::VEGETATION_POSITION,
        shadow_pass::SKIN_INDICES_0,
        shadow_pass::SKIN_WEIGHTS_0,
        shadow_pass::SKIN_INDICES_1,
        shadow_pass::SKIN_WEIGHTS_1,
        water_world_colour::NORMAL_W,
    ]
    .map(|attribute| attribute.id);
    for (i, id) in ids.iter().enumerate() {
        assert!(!ids[..i].contains(id), "attribute {i} reuses an id");
    }
}

#[path = "tests/approach_visibility.rs"]
mod approach_visibility;
#[path = "tests/environment_reflection.rs"]
mod environment_reflection;
#[path = "tests/fog.rs"]
mod fog;
#[path = "tests/texture_formats.rs"]
mod texture_formats;

#[test]
#[ignore = "requires installed original catalog and Saved Games config"]
fn actual_shadow_quality_inputs_resolve_from_config_and_assets() {
    let (_, settings) = game_settings::read_shadow_settings().unwrap();
    let preset = settings.quality_preset.unwrap();
    let catalog =
        archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let quality = load_installed_shadow_quality(&catalog, preset)
        .unwrap()
        .unwrap();
    let target = shadow_target_dimensions(settings.parallel_slice_size, &quality).unwrap();
    assert!(quality.cull_distance.is_finite());
    assert_eq!(target.layers, quality.parallel_slice_count + 1);
    assert_eq!(
        target.width,
        settings.parallel_slice_size / (1 << quality.parallel_resolution_divider)
    );
    assert_eq!(target.width, target.height);
    assert_eq!(target.depth_32_bits, quality.depth_32_bits);
    println!("original shadow preset {preset}: {quality:?}; target={target:?}");
}

#[test]
#[ignore = "requires installed game archives"]
fn installed_sky_and_liquid_material_inputs_resolve() {
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = archive::Catalog::open(&base).unwrap();
    let mut library =
        shader_cache::ProgramLibrary::open(&base, shader_cache::ShaderCacheBackend::Pc, &catalog)
            .unwrap();
    let level = crate::assets::level::LevelResources::resolve(&catalog, "dust_p_lowchaos").unwrap();
    let world = world_mesh::parse(&level.read_world(&catalog).unwrap()).unwrap();
    let mut names = world
        .objects
        .iter()
        .map(|o| o.material_path.clone())
        .collect::<std::collections::BTreeSet<_>>();
    names.extend([
        "models/templates/flat_colors/life_01.material".into(),
        "models/templates/flat_colors/mana_01.material".into(),
        "models/weapons/crossbow/bolt_sleep_liquid.material".into(),
        "models/weapons/crossbow/bolt_incendiary_liquid.material".into(),
        "models/weapons/crossbow/bolt_venom_liquid.material".into(),
    ]);
    let mut counts = [0; 3];
    for name in names {
        let bytes = catalog
            .read(&format!("generated/decls/material/{name}.decl"))
            .unwrap()
            .unwrap();
        let surface = material_surface::Surface::parse(&bytes);
        match surface.program() {
            material_surface::Program::Filling => {
                let p = material_float_vectors(
                    &mut library,
                    &catalog,
                    &bytes,
                    crate::rendering::filling::PARAMETER_NAMES,
                )
                .unwrap();
                let clip = material_float_vectors(
                    &mut library,
                    &catalog,
                    &bytes,
                    crate::rendering::filling::CLIP_PARAMETER_NAMES,
                )
                .unwrap();
                assert!(p
                    .iter()
                    .flatten()
                    .chain(clip.iter().flatten())
                    .all(|v| v.is_finite()));
                if name.contains("/flat_colors/") {
                    assert_eq!(p[7][0], 1.0, "authored potion fill level {name}");
                    assert_eq!(
                        p[12],
                        [5., 0.015, 0.01, 1.],
                        "brace-adjacent liquid colour {name}"
                    );
                }
                counts[0] += 1;
            }
            material_surface::Program::Sky | material_surface::Program::Clouds => {
                let p = material_float_vectors(
                    &mut library,
                    &catalog,
                    &bytes,
                    crate::rendering::sky::PARAMETER_NAMES,
                )
                .unwrap();
                assert!(
                    p.iter().flatten().all(|v| v.is_finite()),
                    "sky parameters {name}"
                );
                counts[if surface.program() == material_surface::Program::Sky {
                    1
                } else {
                    2
                }] += 1;
            }
            _ => continue,
        }
        for (_, image) in surface.texture_bindings(false) {
            assert!(
                material::intrinsic_image_pixels(image).is_some()
                    || material::compiled_image_stem(&catalog, image).is_some(),
                "unresolved effect image {name}: {image}"
            );
        }
        println!("installed effect material {:?}: {name}", surface.program());
    }
    assert_eq!(counts[0], 5);
    assert!(counts[1] > 0 && counts[2] > 0, "{counts:?}");
    println!("installed liquid/sky/cloud inputs resolved: {counts:?}");
}

#[test]
fn shadow_model_bounds_keep_hidden_owners_once_not_surface_boxes() {
    let instance = |surface| RenderInstance {
        source: Some(RenderInstanceSource {
            surface,
            world_baked: false,
        }),
        mesh: Handle::default(),
        material: MaterialHandle::Probe(Handle::default()),
        diffuse_image_slot: None,
        probe_index: 0,
        transform: Transform::default(),
        bounds: Aabb::from_min_max(Vec3::splat(1000.0), Vec3::splat(1001.0)),
        pickup_id: None,
        entity: None,
        visible: false,
    };
    let mut scene = SceneRenderQueue::default();
    scene.instances = vec![
        instance(RenderModelSurface::Bmodel {
            placement: 11,
            surface: 0,
        }),
        instance(RenderModelSurface::Bmodel {
            placement: 11,
            surface: 1,
        }),
        instance(RenderModelSurface::Bmd6 {
            placement: 12,
            surface: 0,
            cpu_vertex_declaration: 0,
            gpu_vertex_declaration: 0,
            skin_influences: 1,
            matrix_offset: 0,
        }),
    ];
    let first = [[-1.0, -2.0, -3.0], [4.0, 5.0, 6.0]];
    let second = [[-7.0, -8.0, -9.0], [10.0, 11.0, 12.0]];
    let mut sources = ShadowModelBoundsSources(vec![
        ShadowModelBoundsSource {
            placement: 11,
            instances: 0..2,
            bounds: first,
        },
        ShadowModelBoundsSource {
            placement: 12,
            instances: 2..3,
            bounds: second,
        },
    ]);
    let pickups = PickupState {
        items: Vec::new(),
        world: runtime::WorldState::from_pickups(&[]).unwrap(),
    };
    assert_eq!(
        sources.active_bounds(&scene, &pickups).unwrap(),
        [first, second],
        "hidden/unspawned model owners remain receiver inputs, once per placement"
    );
    for item in &mut scene.instances[..2] {
        item.pickup_id = Some("removed-owner".into());
    }
    assert_eq!(sources.active_bounds(&scene, &pickups).unwrap(), [second]);
    sources.0[1].placement = 99;
    assert!(
        sources.active_bounds(&scene, &pickups).is_err(),
        "no owner substitution"
    );
    sources.0[1].placement = 12;
    sources.0[1].instances.end = 4;
    assert!(
        sources.active_bounds(&scene, &pickups).is_err(),
        "no partial range fallback"
    );
}

#[test]
fn shadow_ordinary_query_retains_unmodified_current_depth() {
    let mut output = tome::RenderQueryObjectOutput::default();
    output.state = Some(tome::RenderQueryState {
        render_rows: Mat4::IDENTITY.to_cols_array_2d(),
        cropped_rows: Mat4::IDENTITY.to_cols_array_2d(),
        camera_point: [1.0, 2.0, 3.0],
        camera_imprecision: [0.6; 3],
        winding_flip: false,
        viewport: [0, 0, 64, 64],
        maximum_depth: 0.75,
        depth_grid: Some(Box::new([0.5; 4096])),
    });
    let mut scene = SceneRenderQueue::default();
    scene.publish_shadow_ordinary_query(&output);
    // The existing no-depth diagnostic is exclusively a light consumer.
    scene.light_query_state = output.state.clone();
    scene.light_query_state.as_mut().unwrap().depth_grid = None;
    output.state.as_mut().unwrap().depth_grid.as_mut().unwrap()[0] = 0.25;
    let state = scene
        .shadow_ordinary_query
        .as_ref()
        .unwrap()
        .state
        .as_ref()
        .unwrap();
    assert_eq!(state.depth_grid.as_ref().unwrap()[0], 0.5);
    assert_eq!(state.maximum_depth, 0.75);
    assert_eq!(state.camera_point, [1.0, 2.0, 3.0]);
    // A fresh, empty ordinary descriptor replaces the previous state too.
    scene.publish_shadow_ordinary_query(&tome::RenderQueryObjectOutput::default());
    assert!(scene
        .shadow_ordinary_query
        .as_ref()
        .unwrap()
        .state
        .is_none());
}

#[test]
fn shadow_ordinary_query_clears_when_camera_is_unavailable() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    let mut scene = SceneRenderQueue::default();
    scene.publish_shadow_ordinary_query(&tome::RenderQueryObjectOutput::default());
    world.insert_resource(scene);
    world.insert_resource(PickupState {
        items: Vec::new(),
        world: runtime::WorldState::from_pickups(&[]).unwrap(),
    });
    world.run_system_once(stream_visible_models).unwrap();
    assert!(
        world
            .resource::<SceneRenderQueue>()
            .shadow_ordinary_query
            .is_none(),
        "a missing view must not retain the previous frame's query"
    );
}

#[test]
fn world_object_retains_its_actual_material_owner() {
    let mut sources = WorldShadowMeshSources {
        objects: vec![WorldShadowMeshSource {
            object_id: 0,
            mesh_index: 0,
            matrix_start: 0,
            matrix_end: 2,
            lod: 0,
            flipped: false,
            mesh: Handle::default(),
            raw_vertex_count: Ok(3),
            material: None,
            material_error: None,
            gather_flags48: None,
        }],
        shared_mesh_count: 1,
    };
    let material = MaterialHandle::Probe(Handle::default());
    assert!(sources.object(0).unwrap().material.is_none());
    sources.bind_material(0, &material).unwrap();
    sources.bind_material(0, &material).unwrap();
    assert!(sources.object(0).unwrap().material.as_ref() == Some(&material));
    assert!(sources
        .bind_material(0, &MaterialHandle::Decal(Handle::default()))
        .is_err());
    assert!(
        sources.object(0).unwrap().material.as_ref() == Some(&material),
        "a conflicting instance must not replace the object's material"
    );
    assert!(sources.bind_material(1, &material).is_err());
}

#[cfg(test)]
fn world_shadow_test_world() -> world_mesh::WorldMesh {
    let matrix = |translation: f32| {
        [
            1.0,
            0.0,
            0.0,
            translation,
            0.0,
            1.0,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
        ]
    };
    world_mesh::WorldMesh {
        meshes: vec![world_mesh::Mesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            packed_channels: vec![[0; 16]; 3],
            indices: vec![0, 1, 2],
        }],
        matrices: vec![
            matrix(0.0),
            matrix(1.0),
            matrix(2.0),
            matrix(3.0),
            matrix(4.0),
        ],
        visibility: (0..5)
            .map(|index| world_mesh::WorldVisibility {
                bounds: [[-1.0; 3], [1.0; 3]],
                umbra_id: index,
                trailing_field: 0x1200 + index as u16,
            })
            .collect(),
        auxiliary_instance_groups: std::array::from_fn(|_| Vec::new()),
        visibility_groups: Vec::new(),
        tome_object_instances: Vec::new(),
        spatial_query_instances: Vec::new(),
        objects: vec![
            world_mesh::WorldObject {
                mesh_index: 0,
                material_path: "world/native-shadow-test.material".into(),
                matrix_start: 0,
                matrix_end: 2,
                flipped: false,
                lod: 3,
            },
            world_mesh::WorldObject {
                mesh_index: 0,
                material_path: "world/native-shadow-test.material".into(),
                matrix_start: 2,
                matrix_end: 4,
                flipped: true,
                lod: 2,
            },
            world_mesh::WorldObject {
                mesh_index: 0,
                material_path: "world/native-shadow-test.material".into(),
                matrix_start: 4,
                matrix_end: 5,
                flipped: false,
                lod: 1,
            },
        ],
    }
}

#[cfg(test)]
fn world_shadow_test_owner(target_image: Handle<Image>) -> ParallelShadowOwner {
    let identity = Mat4::IDENTITY.to_cols_array_2d();
    ParallelShadowOwner {
        target_image,
        dimensions: shadow_pass::ParallelShadowTargetDimensions {
            width: 64,
            height: 64,
            layers: 5,
            depth_32_bits: true,
        },
        whole_view: render_shadow::DirectionalCascade {
            clip: identity,
            atlas: identity,
            inverse_atlas: identity,
            caster_clip: identity,
            camera: [0.0; 3],
            depth_offset: 0.0,
            near_far: [0.0, 1.0],
        },
        whole_view_initial_masks: vec![vec![1, 1, 1, 0, 0], vec![0, 0, 0, 0, 1]],
        hardware_bias: [1.25, 2.5, 0.25],
        shader_bias: [0.0; 3],
        sort_shadows_ccw: true,
        source_flag_04: game_clock::RESET_VIEW_LOCAL_TIME_FLAG,
        cascade_controls: game_settings::ShadowCascadeControls::default(),
        cascade_fars: [4.0, 12.0, 30.0, 75.0],
    }
}

#[cfg(test)]
fn world_shadow_test_material(
    class: shader_cache::CachedWorldShadowClass,
) -> (MaterialHandle, ShadowMaterials) {
    use render_program::{
        D3dVariantData, D3dVariantHeader, RenderParameterKind, SerializedParameter,
        SerializedParameterBlock, SerializedParameterValue, VariantBindings,
    };
    let material = MaterialHandle::Probe(Handle::default());
    let mut main_state = [0; 9];
    main_state[0] = 0x1234_5678;
    main_state[5] = 1 << 27;
    let mut defaults = HashMap::new();
    let vector = |value: [f32; 4]| value.map(f32::to_bits);
    defaults.insert(b"layer0_scalebias".to_vec(), vector([1.0, 1.0, 0.0, 0.0]));
    for (name, value) in [
        (b"vegetanim/mainbendingvar".as_slice(), [1.0, 2.0, 3.0, 4.0]),
        (
            b"vegetanim/mainbendingfreq".as_slice(),
            [2.0, 3.0, 4.0, 5.0],
        ),
        (
            b"vegetanim/mainbendingamplitude".as_slice(),
            [3.0, 4.0, 5.0, 6.0],
        ),
        (
            b"vegetanim/leafminwindstrength".as_slice(),
            [0.1, 0.2, 0.3, 0.4],
        ),
        (b"vegetanim/leaffreq".as_slice(), [4.0, 5.0, 6.0, 7.0]),
        (b"vegetanim/leafamplitude".as_slice(), [5.0, 6.0, 7.0, 8.0]),
        (
            b"vegetanim/branchamplitude".as_slice(),
            [6.0, 7.0, 8.0, 9.0],
        ),
    ] {
        defaults.insert(name.to_vec(), vector(value));
    }
    let uv_override = [2.0, 3.0, 0.25, 0.5].map(f32::to_bits);
    let world_parameters = ShadowMaterialParameters::new(
        SerializedParameterBlock {
            flags: 0,
            parameters: vec![SerializedParameter {
                name: b"layer0_scalebias".to_vec(),
                kind: RenderParameterKind::Uniform,
                value: SerializedParameterValue::Words(uv_override),
            }],
        },
        defaults,
    );
    let variant = D3dVariantData {
        header: D3dVariantHeader {
            source_crc: 0,
            flags: 0,
            shared_names: false,
            include_count: 0,
        },
        includes: Vec::new(),
        parameters: Vec::new(),
        stages: Vec::new(),
        bindings: VariantBindings {
            shared_names: None,
            parameters: Vec::new(),
        },
        layout: [0; 9],
    };
    let world_source = shader_cache::CachedWorldShadowSource {
        class,
        program: b"typed test source; CPU preparation does not compile it".to_vec(),
        variant_hash: 0,
        definition_render_state: main_state,
        variant,
    };
    let source = material::ShadowSourceInputs {
        source: material::ShadowProgramSource::MainProgram,
        cloth_source: material::ShadowProgramSource::Disabled,
        enable_refraction: false,
        translucency_hq: false,
        material_effects: false,
        override_program: None,
        cloth_override_program: None,
        main_blend_state: 0,
        main_render_state: Some(main_state),
        hair_type: false,
        sky_type: 0,
        is_cloth: false,
        use_skinning: false,
        use_skinning8: false,
        needs_wind: class == shader_cache::CachedWorldShadowClass::Vegetation,
    };
    let inputs = ShadowMaterialInputs {
        source,
        rigid_base_source: Ok(None),
        base_slot10_present: Ok(true),
        world_source: Ok(Some(std::sync::Arc::new(world_source))),
        world_parameters: Ok(Some(world_parameters)),
        main_program: Some("native-shadow-test".into()),
        alpha_test: class != shader_cache::CachedWorldShadowClass::Opaque,
        world_uv_set: Some(1),
        diffuse_image_slot: (class != shader_cache::CachedWorldShadowClass::Opaque).then_some(0),
        parameters: Ok(None),
        cull_mode: None,
    };
    (
        material.clone(),
        ShadowMaterials([(material, Ok(inputs))].into_iter().collect()),
    )
}

#[cfg(test)]
fn world_shadow_test_scene(world: world_mesh::WorldMesh) -> SceneData {
    SceneData {
        source_path: PathBuf::from("native-shadow-test.bwm"),
        world,
        player_start: None,
        placed_models: Vec::new(),
        placed_lights: Vec::new(),
        environment_lighting: None,
        environment_render_parameters: None,
        environment_blend_duration: 1.0,
        area_environments: Vec::new(),
        area_fog: Vec::new(),
        rewrite_shadow_settings: crate::platform::rewrite_settings::RewriteShadowSettings {
            minimum_sun_caster_extent: 0.0,
        },
        environment_probes: None,
        shadow_quality_preset: None,
        shadow_quality: None,
        shadow_target_dimensions: None,
        shadow_depth_bias_clamp: None,
        shadow_sort_ccw: None,
        shadow_cascade_controls: None,
        shadow_cascade_fars: None,
        far_shadow_bounds: None,
        pickups: Vec::new(),
        catalog: None,
    }
}

#[test]
fn world_shadow_initial_publish_retries_alpha_and_owns_one_mask_batch() {
    use bevy::ecs::system::RunSystemOnce;
    let (material, materials) =
        world_shadow_test_material(shader_cache::CachedWorldShadowClass::AlphaTested);
    let scene = world_shadow_test_scene(world_shadow_test_world());
    let mut mesh_assets = Assets::<Mesh>::default();
    let mut sources = WorldShadowMeshSources::prepare(&scene.world, &mut mesh_assets);
    sources.bind_material(0, &material).unwrap();
    sources.bind_material(1, &material).unwrap();
    sources.bind_material(2, &material).unwrap();
    let mut image_assets = Assets::<Image>::default();
    let target_image = image_assets.add(Image::default());
    let owner = world_shadow_test_owner(target_image.clone());
    assert_eq!(owner.source_flag_04, game_clock::RESET_VIEW_LOCAL_TIME_FLAG);
    let mut queue = SceneRenderQueue::default();
    queue.diffuse_image_paths = vec!["textures/off-camera-world-alpha.dds".to_string()];
    queue.diffuse_image_handles = vec![None];
    queue.instances.push(RenderInstance {
        source: Some(RenderInstanceSource {
            surface: RenderModelSurface::Bmodel {
                placement: 77,
                surface: 0,
            },
            world_baked: false,
        }),
        mesh: sources.object(0).unwrap().mesh.clone(),
        material: material.clone(),
        diffuse_image_slot: Some(0),
        probe_index: 0,
        transform: Transform::default(),
        bounds: Aabb::from_min_max(Vec3::ZERO, Vec3::ONE),
        pickup_id: None,
        entity: None,
        visible: false,
    });
    let mut world = World::new();
    world.insert_resource(scene);
    world.insert_resource(owner);
    world.insert_resource(sources);
    world.insert_resource(materials);
    world.insert_resource(queue);
    world.insert_resource(mesh_assets);
    world.insert_resource(image_assets);
    world.insert_resource(Assets::<ShaderBuffer>::default());
    world.insert_resource(super::world_instances::WorldMatrices::default());
    world.insert_resource(shadow_pass::ShadowSceneFrame::default());

    world
        .run_system_once(publish_initial_whole_view_depth)
        .unwrap();
    {
        let frame = world.resource::<shadow_pass::ShadowSceneFrame>();
        assert_eq!(frame.revision, 0);
        assert!(matches!(
            &frame.state,
            shadow_pass::ShadowSceneState::Unavailable(error)
                if error.contains("pending through texture streamer")
        ));
        assert_eq!(
            world.resource::<Assets<ShaderBuffer>>().len(),
            0,
            "dependency retries must not leave provisional GPU-input assets"
        );
    }
    assert_eq!(
        world.resource::<SceneRenderQueue>().shadow_diffuse_slots,
        [0],
        "off-camera alpha slots are requested by the existing texture streamer"
    );

    let diffuse = world.resource_mut::<Assets<Image>>().add(Image::default());
    {
        let mut queue = world.remove_resource::<SceneRenderQueue>().unwrap();
        queue.diffuse_image_handles[0] = Some(diffuse);
        let scene = world.resource::<SceneData>();
        let owner = world.resource::<ParallelShadowOwner>();
        let sources = world.resource::<WorldShadowMeshSources>();
        let materials = world.resource::<ShadowMaterials>();
        let meshes = world.resource::<Assets<Mesh>>();
        let images = world.resource::<Assets<Image>>();
        let hidden = prepare_whole_view_masks(
            owner,
            &scene.world,
            sources,
            materials,
            meshes,
            images,
            &mut super::scene_queue::ShadowTextureRequests::new(&queue),
            None,
            Some("clock is not needed by this alpha shader"),
        )
        .unwrap();
        assert_eq!(
            hidden
                .iter()
                .map(|mask| mask.instance_ids.clone())
                .collect::<Vec<_>>(),
            [vec![0, 1, 2], vec![4]],
            "initial BWM masks, not color-visible IDs, own the remap"
        );
        assert_eq!(
            hidden
                .iter()
                .map(|mask| mask.instances.len())
                .collect::<Vec<_>>(),
            [2, 1]
        );
        queue.instances[0].visible = true;
        let visible = prepare_whole_view_masks(
            owner,
            &scene.world,
            sources,
            materials,
            meshes,
            images,
            &mut super::scene_queue::ShadowTextureRequests::new(&queue),
            None,
            Some("clock is not needed by this alpha shader"),
        )
        .unwrap();
        assert_eq!(
            visible
                .iter()
                .map(|mask| mask.instance_ids.clone())
                .collect::<Vec<_>>(),
            [vec![0, 1, 2], vec![4]],
            "changing color visibility cannot change native whole-view IDs"
        );
        world.insert_resource(queue);
    }
    assert_eq!(
        world.resource::<Assets<ShaderBuffer>>().len(),
        0,
        "CPU validation alone must not mutate the ShaderBuffer asset table"
    );
    world
        .run_system_once(publish_initial_whole_view_depth)
        .unwrap();
    {
        let frame = world.resource::<shadow_pass::ShadowSceneFrame>();
        assert_eq!(frame.revision, 1);
        let shadow_pass::ShadowSceneState::WholeViewDepth {
            target_image: published_target,
            dimensions,
            layer,
        } = &frame.state
        else {
            panic!(
                "initial native stage did not publish WholeViewDepth: {:?}",
                frame.state
            );
        };
        assert_eq!(published_target.id(), target_image.id());
        assert_eq!(dimensions.layers, 5);
        assert_eq!(layer.instances.len(), 3);
        assert!(matches!(layer.cache, shadow_pass::ShadowCacheOp::None));
        assert!(matches!(
            layer.ops.depth,
            shadow_pass::ShadowLoad::Clear(1.0)
        ));
        assert_eq!(layer.ops.stencil, shadow_pass::StencilLoad::Clear(0));
        let expected_states = [
            render_state::world_shadow_draw_state(
                world
                    .resource::<ShadowMaterials>()
                    .0
                    .get(&material)
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .source
                    .main_render_state
                    .unwrap(),
                1,
                1.25,
                2.5,
                0.25,
                0,
            ),
            render_state::world_shadow_draw_state(
                world
                    .resource::<ShadowMaterials>()
                    .0
                    .get(&material)
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .source
                    .main_render_state
                    .unwrap(),
                0,
                1.25,
                2.5,
                0.25,
                0,
            ),
            render_state::world_shadow_draw_state(
                world
                    .resource::<ShadowMaterials>()
                    .0
                    .get(&material)
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .source
                    .main_render_state
                    .unwrap(),
                0,
                1.25,
                2.5,
                0.25,
                0,
            ),
        ];
        let mut visible_buffers = Vec::new();
        let mut transform_buffers = Vec::new();
        for (index, instance) in layer.instances.iter().enumerate() {
            assert_eq!(
                instance.program,
                shadow_pass::ShadowProgram::WorldAlphaTested
            );
            assert_eq!(instance.draw_state, expected_states[index]);
            assert_eq!(
                (instance.front_face, instance.cull_mode),
                shadow_pipeline::shadow_mesh_raster(&expected_states[index], index != 0),
                "winding must include source_mesh's index reversal and BWM flipped flag",
            );
            assert_eq!(instance.scissor.width, dimensions.width);
            assert_eq!(instance.scissor.height, dimensions.height);
            let Some(alpha) = &instance.alpha_material else {
                panic!("alpha source binding omitted")
            };
            assert_eq!(alpha.uv_set, 1);
            assert_eq!(
                alpha.uv_scale_bias,
                [2.0, 3.0, 0.25, 0.5],
                "alpha UV state comes from the typed source override, not the default"
            );
            let shadow_pass::ShadowInstanceConstants::World {
                constants,
                visible_indices,
                matrices,
                instance_count,
            } = &instance.per_instance_cb
            else {
                panic!("world alpha packet used a non-world instance ABI")
            };
            let packet_offset = [2, 0, 0][index];
            assert_eq!(
                *constants,
                shadow_pass::world_alpha_shadow_view_cb(
                    &world.resource::<ParallelShadowOwner>().whole_view.clip,
                    packet_offset,
                    [2.0, 3.0, 0.25, 0.5],
                )
            );
            assert_eq!(*instance_count, [1, 2, 1][index]);
            assert_eq!(
                u32::from_le_bytes(constants[64..68].try_into().unwrap()),
                packet_offset,
                "packet remap offset is the selected ID stream start"
            );
            visible_buffers.push(visible_indices.id());
            transform_buffers.push(matrices.id());
        }
        assert_eq!(
            visible_buffers[0], visible_buffers[1],
            "packets in one mask share their exact remap buffer"
        );
        assert_ne!(
            visible_buffers[0], visible_buffers[2],
            "each initial mask owns a distinct ID buffer"
        );
        assert!(
            transform_buffers
                .iter()
                .all(|buffer| *buffer == transform_buffers[0]),
            "source 48-byte BWM affine matrix stream is shared without remapping"
        );
    }
    let buffers_after_publish = world.resource::<Assets<ShaderBuffer>>().len();
    assert_eq!(buffers_after_publish, 3);
    world
        .run_system_once(publish_initial_whole_view_depth)
        .unwrap();
    assert_eq!(
        world.resource::<shadow_pass::ShadowSceneFrame>().revision,
        1,
        "the initial layer is never republished for the same owner generation"
    );
    assert_eq!(
        world.resource::<Assets<ShaderBuffer>>().len(),
        buffers_after_publish,
        "same-owner retry must not leak typed buffers"
    );
    assert!(
        !matches!(
            &world.resource::<shadow_pass::ShadowSceneFrame>().state,
            shadow_pass::ShadowSceneState::Ready { .. }
        ),
        "whole-view depth is not a complete shadow frame"
    );
}

#[test]
fn world_shadow_all_supported_programs_preserve_typed_material_inputs() {
    let world = world_shadow_test_world();
    let owner = world_shadow_test_owner(Handle::default());
    let mut mesh_assets = Assets::<Mesh>::default();
    let mut sources = WorldShadowMeshSources::prepare(&world, &mut mesh_assets);
    let classes = [
        shader_cache::CachedWorldShadowClass::Opaque,
        shader_cache::CachedWorldShadowClass::AlphaTested,
        shader_cache::CachedWorldShadowClass::VistaAlpha,
        shader_cache::CachedWorldShadowClass::Vegetation,
        shader_cache::CachedWorldShadowClass::VegetationFixed,
    ];
    let mut image_assets = Assets::<Image>::default();
    let diffuse = image_assets.add(Image::default());
    for class in classes {
        let (material, materials) = world_shadow_test_material(class);
        sources.bind_material(0, &material).unwrap();
        sources.bind_material(1, &material).unwrap();
        sources.bind_material(2, &material).unwrap();
        let mut queue = SceneRenderQueue::default();
        if class != shader_cache::CachedWorldShadowClass::Opaque {
            queue.diffuse_image_paths = vec!["textures/native-world-test.dds".to_string()];
            queue.diffuse_image_handles = vec![Some(diffuse.clone())];
        }
        let plans = prepare_whole_view_masks(
            &owner,
            &world,
            &sources,
            &materials,
            &mesh_assets,
            &image_assets,
            &mut super::scene_queue::ShadowTextureRequests::new(&queue),
            Some([12.5, 0.125]),
            None,
        )
        .unwrap();
        assert_eq!(
            plans
                .iter()
                .map(|mask| mask.instance_ids.clone())
                .collect::<Vec<_>>(),
            [vec![0, 1, 2], vec![4]]
        );
        if class == shader_cache::CachedWorldShadowClass::Vegetation {
            assert_eq!(plans[0].auxiliary, vec![[0; 8]; 3]);
            assert_eq!(plans[1].auxiliary, vec![[0; 8]]);
        } else {
            assert!(plans.iter().all(|mask| mask.auxiliary.is_empty()));
        }
        let mut buffers = Assets::<ShaderBuffer>::default();
        let layer = build_whole_view_layer(plans, &world, owner.dimensions, &mut buffers,
            &mut super::world_instances::WorldMatrices::default());
        assert_eq!(layer.instances.len(), 3);
        let expected_program = match class {
            shader_cache::CachedWorldShadowClass::Opaque => shadow_pass::ShadowProgram::WorldOpaque,
            shader_cache::CachedWorldShadowClass::AlphaTested => {
                shadow_pass::ShadowProgram::WorldAlphaTested
            }
            shader_cache::CachedWorldShadowClass::VistaAlpha => {
                shadow_pass::ShadowProgram::WorldVistaAlpha
            }
            shader_cache::CachedWorldShadowClass::Vegetation => {
                shadow_pass::ShadowProgram::WorldVegetation
            }
            shader_cache::CachedWorldShadowClass::VegetationFixed => {
                shadow_pass::ShadowProgram::WorldVegetationFixed
            }
        };
        assert!(layer
            .instances
            .iter()
            .all(|instance| instance.program == expected_program));
        if class != shader_cache::CachedWorldShadowClass::Opaque {
            let uv_set = if class == shader_cache::CachedWorldShadowClass::VistaAlpha {
                0
            } else {
                1
            };
            assert!(layer.instances.iter().all(|instance| {
                instance
                    .alpha_material
                    .as_ref()
                    .is_some_and(|material| material.uv_set == uv_set)
            }));
        }
        let wind_buffers = layer
            .instances
            .iter()
            .filter_map(|instance| match &instance.per_instance_cb {
                shadow_pass::ShadowInstanceConstants::WorldVegetation { winds, .. } => {
                    winds.as_ref()
                }
                _ => None,
            })
            .count();
        assert_eq!(
            wind_buffers,
            if class == shader_cache::CachedWorldShadowClass::Vegetation {
                3
            } else {
                0
            }
        );
    }
}

#[test]
#[ignore = "requires installed original catalog/config and local EOTW world/entities"]
fn shadow_owner_uses_level_sources_without_publishing_fake_frame() {
    use bevy::ecs::system::RunSystemOnce;
    let (_, settings) = game_settings::read_shadow_settings().unwrap();
    let preset = settings.quality_preset.unwrap();
    let catalog =
        archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let quality = load_installed_shadow_quality(&catalog, preset)
        .unwrap()
        .unwrap();
    let dimensions = shadow_target_dimensions(settings.parallel_slice_size, &quality).unwrap();
    let source_path = PathBuf::from("artifacts/eotw_p_lowchaos.bwm");
    let document =
        entities::parse(&fs::read_to_string(source_path.with_extension("entities")).unwrap())
            .unwrap();
    let environment = entities::world_environment(&document).unwrap();
    let source = String::from_utf8(
        catalog
            .read(&format!("generated/decls/env/{environment}.decl"))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let read_parent = |parent: &str| {
        catalog
            .read(&format!("generated/decls/env/{parent}.decl"))
            .map(|bytes| bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
            .map_err(|error| error.to_string())
    };
    let lighting = entities::environment_lighting_with_inheritance(&source, read_parent).unwrap();
    let parameters = entities::environment_render_parameters_with_inheritance(
        &environment,
        &source,
        read_parent,
    )
    .unwrap();
    let mut scene = SceneData {
        source_path: source_path.clone(),
        world: world_mesh::parse(&fs::read(&source_path).unwrap()).unwrap(),
        player_start: None,
        placed_models: Vec::new(),
        placed_lights: Vec::new(),
        environment_lighting: Some(lighting),
        environment_render_parameters: Some(parameters),
        environment_blend_duration: 1.0,
        area_environments: Vec::new(),
        area_fog: Vec::new(),
        rewrite_shadow_settings: crate::platform::rewrite_settings::RewriteShadowSettings {
            minimum_sun_caster_extent: 0.0,
        },
        environment_probes: None,
        shadow_quality_preset: Some(preset),
        shadow_quality: Some(quality),
        shadow_target_dimensions: Some(dimensions),
        shadow_depth_bias_clamp: Some(settings.depth_bias_clamp),
        shadow_sort_ccw: Some(settings.sort_shadows_ccw),
        shadow_cascade_controls: Some(settings.cascade_controls),
        shadow_cascade_fars: settings.cascade_fars,
        far_shadow_bounds: entities::world_far_shadow_bounds(&document).unwrap(),
        pickups: Vec::new(),
        catalog: None,
    };
    // Owner initialization must not depend on meshes/color visibility.
    // Native static query records survive even with no renderable geometry.
    scene.world.meshes.clear();
    // The native authored-area branch never reads the fallback root.
    let saved_area = scene.far_shadow_bounds;
    let saved_records = std::mem::take(&mut scene.world.visibility);
    let saved_objects = std::mem::take(&mut scene.world.objects);
    let saved_groups = std::mem::take(&mut scene.world.visibility_groups);
    let mut empty_images = Assets::<Image>::default();
    scene.far_shadow_bounds = Some([[-1.0, -2.0, -3.0], [1.0, 2.0, 3.0]]);
    let empty_owner = ParallelShadowOwner::from_scene(&scene, &mut empty_images).unwrap();
    assert!(empty_owner.whole_view_initial_masks.is_empty());
    assert_eq!(empty_owner.cascade_fars, [4.0, 12.0, 30.0, 75.0]);
    scene.shadow_cascade_fars = Some([3.0, 15.0, 45.0, 90.0]);
    let configured_owner = ParallelShadowOwner::from_scene(&scene, &mut empty_images).unwrap();
    assert_eq!(configured_owner.cascade_fars, [3.0, 15.0, 45.0, 90.0]);
    scene.shadow_cascade_fars = None;
    scene.far_shadow_bounds = Some([[0.0; 3]; 2]);
    assert!(ParallelShadowOwner::from_scene(&scene, &mut empty_images).is_err());
    assert_eq!(empty_images.len(), 1);
    scene.far_shadow_bounds = saved_area;
    scene.world.visibility = saved_records;
    scene.world.objects = saved_objects;
    scene.world.visibility_groups = saved_groups;
    let record_count = scene.world.visibility.len();
    let parents: Vec<usize> = scene
        .world
        .visibility_groups
        .iter()
        .map(|group| group.parent)
        .collect();
    let mut app = App::new();
    app.insert_resource(scene)
        .insert_resource(Assets::<Image>::default())
        .insert_resource(shadow_pass::ShadowSceneFrame::default())
        .add_systems(Startup, initialize_parallel_shadow_owner);
    app.update();
    let initial_target = app
        .world()
        .resource::<ParallelShadowOwner>()
        .target_image
        .id();
    app.update();
    let world = app.world_mut();
    let owner = world.resource::<ParallelShadowOwner>();
    assert_eq!(owner.target_image.id(), initial_target);
    assert_eq!(owner.dimensions, dimensions);
    assert_eq!(
        owner.whole_view_initial_masks.len(),
        record_count.div_ceil(4096)
    );
    for mask in &owner.whole_view_initial_masks {
        assert_eq!(mask.len(), record_count);
        assert!(parents.iter().all(|&parent| mask[parent] == 0));
    }
    assert!(owner
        .whole_view
        .clip
        .iter()
        .flatten()
        .all(|value| value.is_finite()));
    assert!(owner
        .hardware_bias
        .iter()
        .chain(&owner.shader_bias)
        .all(|value| value.is_finite()));
    let images = world.resource::<Assets<Image>>();
    assert_eq!(images.len(), 1);
    let image = images.get(&owner.target_image).unwrap();
    assert_eq!(
        image.texture_descriptor.size.depth_or_array_layers,
        dimensions.layers
    );
    assert_eq!(image.texture_descriptor.size.width, dimensions.width);
    assert!(matches!(
        world.resource::<shadow_pass::ShadowSceneFrame>().state,
        shadow_pass::ShadowSceneState::Unavailable(_)
    ));
    // Missing source data must allocate nothing and never imply Ready.
    world.resource_mut::<SceneData>().environment_lighting = None;
    world
        .run_system_once(initialize_parallel_shadow_owner)
        .unwrap();
    assert!(!world.contains_resource::<ParallelShadowOwner>());
    assert_eq!(world.resource::<Assets<Image>>().len(), 1);
    assert!(matches!(
        world.resource::<shadow_pass::ShadowSceneFrame>().state,
        shadow_pass::ShadowSceneState::Unavailable(_)
    ));
}

fn cache_test_input(gates: Option<&[u32]>) -> tome::OrdinaryDirectQueryInput<'_> {
    tome::OrdinaryDirectQueryInput {
        camera_point: [0.0; 3],
        render_rows: Mat4::IDENTITY.to_cols_array_2d(),
        cropped_rows: Mat4::IDENTITY.to_cols_array_2d(),
        initial_tile_rect: [0, 0, 8, 16],
        camera_imprecision: [0.6; 3],
        object_imprecision: 0.6,
        minimum_area_metric: 25.0 / (1920.0 * 1080.0),
        global_distance_threshold: tome::DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
        object_distance_scale: 1.0,
        winding_flip: false,
        gate_states: gates,
        output_capacity: tome::WORLD_QUERY_OUTPUT_CAPACITY,
    }
}

#[test]
fn tome_query_cache_requires_every_exact_input_and_unchanged_world() {
    let gates = [0u32, 1];
    let input = cache_test_input(Some(&gates));
    let key = || TomeRenderQueryKey::new(&input, 12);
    let cache = TomeRenderQueryCache {
        key: Some(key()),
        ..default()
    };
    assert!(cache.matches(&key(), false));
    assert!(!cache.matches(&key(), true));
    assert!(!cache.matches(&TomeRenderQueryKey::new(&input, 1), false));
    let changed = |input| assert!(!cache.matches(&TomeRenderQueryKey::new(&input, 12), false));
    for i in 0..3 {
        let mut other = input;
        other.camera_point[i] = -0.0; // Signed zero must not alias.
        changed(other);
        other = input;
        other.camera_point[i] = f32::from_bits(1); // Smallest camera change.
        changed(other);
        other = input;
        other.camera_imprecision[i] += 0.001;
        changed(other);
    }
    for row in 0..4 {
        for col in 0..4 {
            let mut other = input;
            other.render_rows[row][col] += 0.001;
            changed(other);
            other = input;
            other.cropped_rows[row][col] += 0.001;
            changed(other);
        }
        let mut other = input;
        other.initial_tile_rect[row] += 1;
        changed(other);
    }
    let mut other = input;
    other.object_imprecision += 0.001;
    changed(other);
    other = input;
    other.minimum_area_metric += 0.001;
    changed(other);
    other = input;
    other.global_distance_threshold += 0.001;
    changed(other);
    other = input;
    other.winding_flip = true;
    changed(other);
    other = input;
    other.output_capacity -= 1;
    changed(other);
    changed(cache_test_input(Some(&[0, 0])));
    changed(cache_test_input(None));
    changed(cache_test_input(Some(&[])));
}

#[test]
fn tome_query_cache_never_keeps_failed_or_partial_job_results() {
    let input = cache_test_input(None);
    let key = || TomeRenderQueryKey::new(&input, 12);
    let mut cache = TomeRenderQueryCache::default();
    let mut output = tome::RenderQueryObjectOutput::default();
    output.state = Some(tome::RenderQueryState::from_input(&input, [0, 0, 16, 16]));
    let success = Ok(Some(output));
    cache.store(key(), &success, &[]);
    assert!(cache.matches(&key(), false));
    let mut copy = cache.output.clone().unwrap();
    copy.state.as_mut().unwrap().depth_grid.as_mut().unwrap()[0] = 1.0;
    assert_eq!(
        cache
            .output
            .as_ref()
            .unwrap()
            .state
            .as_ref()
            .unwrap()
            .depth_grid
            .as_ref()
            .unwrap()[0],
        0.0
    );
    cache.store(key(), &Err("merge failed".into()), &[]);
    assert!(!cache.matches(&key(), false));
    cache.store(key(), &success, &[]);
    let failed_job = tome::RenderQueryJobDiagnostics {
        job_index: 0,
        viewport: None,
        elapsed_ns: 1,
        progress: default(),
        error: Some("worker failed".into()),
    };
    cache.store(key(), &success, &[failed_job]);
    assert!(!cache.matches(&key(), false));
    cache.store(key(), &Ok(None), &[]);
    assert!(cache.matches(&key(), false));
    assert!(cache.output.is_none());
}

#[test]
fn unavailable_or_unready_camera_replaces_model_and_texture_visibility() {
    use bevy::ecs::system::RunSystemOnce;
    for camera_count in [0, 1, 2] {
        let mut world = World::new();
        world.insert_resource(PickupState {
            items: Vec::new(),
            world: runtime::WorldState::default(),
        });
        world.insert_resource(VisibilityElapsedTiming::default());
        let entity = world.spawn(Visibility::Visible).id();
        world.insert_resource(SceneRenderQueue {
            active: 1,
            instances: vec![RenderInstance {
                source: None,
                mesh: Handle::default(),
                material: MaterialHandle::Probe(Handle::default()),
                diffuse_image_slot: None,
                probe_index: 0,
                transform: Transform::default(),
                bounds: Aabb::from_min_max(Vec3::ZERO, Vec3::ONE),
                pickup_id: None,
                entity: Some(entity),
                visible: true,
            }],
            visible_authored_ids: vec![1],
            visible_cell_indices: vec![1],
            visible_cell_mask: vec![2],
            light_query_state: Some(light_test_query_state()),
            ..default()
        });
        for _ in 0..camera_count {
            world.spawn(Camera3d::default());
        }
        world.run_system_once(stream_visible_models).unwrap();
        let scene = world.resource::<SceneRenderQueue>();
        assert_eq!(scene.active, 0, "camera count {camera_count}");
        assert!(!scene.instances[0].visible); // Texture streaming uses this flag too.
        assert_eq!(world.get::<Visibility>(entity), Some(&Visibility::Hidden));
        assert!(scene.visible_authored_ids.is_empty());
        assert!(scene.visible_cell_indices.is_empty());
        assert!(scene.visible_cell_mask.is_empty());
        assert!(scene.light_query_state.is_none());
        assert_eq!(world.resource::<VisibilityElapsedTiming>().samples, 1);
    }
}

#[test]
fn mesh_winding_matches_authored_normals() {
    let points = [
        Vec3::new(-1.0, -1.0, 0.0),
        Vec3::new(1.0, -1.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    ];
    let raw = [0u16, 2, 1]; // Original clockwise front, outward normal +Z.
    let mesh = mesh_from_points(&points, &[[0.0; 2]; 3], &raw);
    assert_eq!(mesh.indices(), Some(&Indices::U16(vec![0, 1, 2])));
    let cross = (points[1] - points[0]).cross(points[2] - points[0]);
    let normals = mesh
        .attribute(Mesh::ATTRIBUTE_NORMAL)
        .unwrap()
        .as_float3()
        .unwrap();
    assert!(normals
        .iter()
        .all(|n| cross.dot(Vec3::from_array(*n)) > 0.0));
    let mut packed = [[0u8; 12]; 3];
    for p in &mut packed {
        p[6] = 127;
        p[8] = 127;
        p[11] = 127;
    }
    let authored = authored_mesh(&points, &[[0.0; 2]; 3], &raw, &packed, |n| n);
    assert_eq!(authored.indices(), mesh.indices());
    let reflected: Vec<Vec3> = points
        .iter()
        .map(|p| *p * Vec3::new(-1.0, 1.0, 1.0))
        .collect();
    let mirrored = authored_mesh(&reflected, &[[0.0; 2]; 3], &raw, &packed, |n| {
        n * Vec3::new(-1.0, 1.0, 1.0)
    });
    assert_eq!(mirrored.indices(), Some(&Indices::U16(raw.to_vec())));
    let cross = (reflected[2] - reflected[0]).cross(reflected[1] - reflected[0]);
    assert!(cross.z > 0.0);
}

#[test]
fn f8_replay_preserves_saved_camera_and_rejects_bad_input() {
    let report = "Camera game_xyz: [-22.121996, 93.85251, 12.256257]\nCamera direction game_xyz: [0.04506988, -0.98096955, 0.18885912]\n";
    let pose = f8_camera_pose(report).unwrap();
    assert_eq!(
        view_to_game(pose.translation),
        [-22.121996, 93.85251, 12.256257]
    );
    let expected = game_to_view([0.04506988, -0.98096955, 0.18885912]).normalize();
    assert!(pose.forward().as_vec3().distance(expected) < 1e-6);
    assert!(f8_camera_pose("").is_none());
    assert!(f8_camera_pose(&report.replace("12.256257", "NaN")).is_none());
    assert!(
        f8_camera_pose(&report.replace("0.04506988, -0.98096955, 0.18885912", "0, 0, 0")).is_none()
    );
    assert_eq!(f8_reference_fov(report), Some(80.0));
    assert_eq!(
        f8_reference_fov(&format!("{report}Reference horizontal FOV degrees: 110\n")),
        Some(110.0)
    );
    for bad in ["0", "180", "NaN", "invalid"] {
        assert!(f8_reference_fov(&format!(
            "{report}Reference horizontal FOV degrees: {bad}\n"
        ))
        .is_none());
    }
    let rolled = format!("{report}Camera up game_xyz: [0.4, 0.2, 1]\n");
    let rolled_pose = f8_camera_pose(&rolled).unwrap();
    assert!(rolled_pose.forward().as_vec3().distance(expected) < 1e-6);
    assert!(rolled_pose.rotation.angle_between(pose.rotation) > 0.01);
    assert!(f8_camera_pose(&format!("{report}Camera up game_xyz: [0,0,0]\n")).is_none());
}

#[test]
fn visibility_uses_actual_camera_fov_not_registered_default() {
    for reference in [55.0, 80.0, 110.0] {
        let projection = Projection::Perspective(PerspectiveProjection {
            fov: tome::render_vertical_fov_radians(reference).unwrap(),
            ..default()
        });
        assert!((reference_fov_from_projection(&projection).unwrap() - reference).abs() < 0.00002);
    }
    assert!(
        reference_fov_from_projection(&Projection::Perspective(PerspectiveProjection {
            fov: f32::NAN,
            ..default()
        }))
        .is_err()
    );
}

#[test]
fn replayed_pose_matches_live_query_matrix() {
    // Raw camera/query metadata captured at the user's 15-FPS location.
    let report = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/dense_camera_pose.f8"
    ));
    let pose = f8_camera_pose(report).unwrap();
    let reference = f8_reference_fov(report).unwrap();
    let projection = Projection::Perspective(PerspectiveProjection {
        fov: tome::render_vertical_fov_radians(reference).unwrap(),
        ..default()
    });
    let rows = tome::world_visibility_query_matrix(
        reference_fov_from_projection(&projection).unwrap(),
        1920,
        1080,
        1.0,
        game_pose_from_view_transform(&pose),
        DEFAULT_NEAR,
        DEFAULT_FAR,
    )
    .unwrap();
    for (index, actual) in rows.into_iter().enumerate() {
        let label = format!("Original query row{index}: ");
        let text = report
            .lines()
            .find_map(|line| line.strip_prefix(&label))
            .unwrap();
        let expected: Vec<f32> = text
            .trim_matches(['[', ']'])
            .split(',')
            .map(|part| part.trim().parse().unwrap())
            .collect();
        for lane in 0..4 {
            assert!(
                (actual[lane] - expected[lane]).abs() < 0.00004,
                "row{index} lane{lane}: {} != {}",
                actual[lane],
                expected[lane]
            );
        }
    }
}

#[test]
#[ignore = "requires installed material/program cache; writes local skin shader evidence"]
fn installed_model_skin_shadow_stage_evidence() {
    use render_program::StageBytecode;
    let base = &crate::assets::installation::game_base().unwrap();
    let catalog = archive::Catalog::open(base).unwrap();
    let mut library =
        shader_cache::ProgramLibrary::open(base, shader_cache::ShaderCacheBackend::Pc, &catalog);
    let directory = Path::new("artifacts/native-actual-skin-shadow-source");
    fs::create_dir_all(directory).unwrap();
    let mut found = HashSet::new();
    let mut materials = 0;
    let mut misses = 0;
    for path in catalog.paths_containing("generated/decls/material/") {
        let bytes = catalog.read(&path).unwrap().unwrap();
        if !entities::declaration_text(&bytes)
            .to_ascii_lowercase()
            .contains("useskinning")
        {
            continue;
        }
        let surface = material_surface::Surface::parse(&bytes);
        let Some(main) = surface.main_program.as_deref() else {
            continue;
        };
        let Some(program) = catalog
            .read(&format!(
                "generated/decls/renderprog/{}.decl",
                main.to_ascii_lowercase()
            ))
            .unwrap()
        else {
            continue;
        };
        let source =
            resolve_shadow_source_inputs(Some(&mut library), &bytes, &program, Some(main)).unwrap();
        if source.is_cloth {
            continue;
        }
        let name = match source.source {
            material::ShadowProgramSource::Disabled => continue,
            material::ShadowProgramSource::Authored => source.override_program.as_deref().unwrap(),
            material::ShadowProgramSource::MainProgram => main,
        };
        if name.is_empty() {
            continue;
        }
        let library = library.as_mut().unwrap();
        let overrides =
            shadow_variant_overrides(library, &bytes, &source, Some(main), name).unwrap();
        let skin4 = library
            .option_value(name.as_bytes(), &overrides, b"useSkinning")
            .unwrap()
            != 0;
        let skin8 = library
            .option_value(name.as_bytes(), &overrides, b"useSkinning8")
            .unwrap()
            != 0;
        if !skin4 && !skin8 {
            continue;
        }
        let Some(variant) = library
            .serialized_variant(name.as_bytes(), &overrides)
            .unwrap()
        else {
            misses += 1;
            eprintln!("SKIN ACTUAL RESIDENT MISS {path}");
            continue;
        };
        materials += 1;
        for stage in variant.stages {
            let data = match stage.bytecode {
                StageBytecode::Absent => continue,
                StageBytecode::Inline(bytes) => bytes,
                StageBytecode::Cached(id) => library.cache.stage(id as usize).unwrap().bytes,
            };
            let hash = data[4..20]
                .iter()
                .map(|v| format!("{v:02x}"))
                .collect::<String>();
            if found.insert(hash.clone()) {
                let file = directory.join(format!(
                    "skin{}-stage{}-{hash}.dxbc",
                    if skin8 { 8 } else { 4 },
                    stage.kind
                ));
                fs::write(&file, &data).unwrap();
                eprintln!("SKIN ACTUAL {} material={path}", file.display());
            }
        }
    }
    eprintln!(
        "SKIN ACTUAL {materials} resident materials, {misses} misses, {} unique stages",
        found.len()
    );
    assert!(!found.is_empty());
}

#[test]
#[ignore = "requires a graphics adapter; validates both authored material pipelines offscreen"]
fn authored_surface_gpu_pipelines_compile() {
    use bevy::render::render_resource::{CachedPipelineState, PipelineDescriptor};
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(bevy::window::WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            })
            .set(bevy::render::RenderPlugin {
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
    )
    .add_plugins(MaterialPlugin::<ProbeMaterial>::default())
    .add_plugins(MaterialPlugin::<DecalMaterial>::default());
    super::sky_sprite::register_renderer(&mut app);
    bevy::asset::load_internal_asset!(
        app,
        PROBE_SHADER_HANDLE,
        "../../assets/shaders/dh2_probe.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        PROBE_PREPASS_SHADER_HANDLE,
        "../../assets/shaders/dh2_probe_prepass.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        INDIRECT_SHADER_HANDLE,
        "../../assets/shaders/dh2_indirect.wgsl",
        crate::rendering::bevy_backend::probe_weights::indirect_shader
    );
    bevy::asset::load_internal_asset!(
        app,
        super::state::REFLECTION_SHADER_HANDLE,
        "../../assets/shaders/dh2_reflection.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        SKY_SHADER_HANDLE,
        "../../assets/shaders/dh2_sky.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        FILLING_SHADER_HANDLE,
        "../../assets/shaders/dh2_filling.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        VISTA_SHADER_HANDLE,
        "../../assets/shaders/dh2_vista.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        SURFACE_SHADER_HANDLE,
        "../../assets/shaders/dh2_surface.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        DECAL_SHADER_HANDLE,
        "../../assets/shaders/dh2_decal.wgsl",
        Shader::from_wgsl
    );
    shadow_sampling::register_shaders(&mut app);
    crate::rendering::bevy_backend::vegetation_wind::register_shaders(&mut app);
    crate::rendering::bevy_backend::fog_material::register_shaders(&mut app);
    app.add_plugins(auto_exposure::ExposurePlugin);
    app.finish();
    app.cleanup();
    let material_exposure = app
        .world_mut()
        .resource_mut::<Assets<ShaderBuffer>>()
        .add(auto_exposure::initial_material_exposure());
    let fog_bindings = app
        .world_mut()
        .resource_scope(|world, mut images: Mut<Assets<Image>>| {
            crate::rendering::bevy_backend::fog_material::FogMaterialBindings::disabled(
                &mut images,
                &mut world.resource_mut::<Assets<ShaderBuffer>>(),
            )
        });
    let fog_parameters = fog_bindings.parameters.clone();
    let target = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::new_target_texture(
            64,
            64,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
    let mut mesh = Mesh::from(Cuboid::new(1.0, 1.0, 1.0));
    let n = mesh.count_vertices();
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.3, 0.7, 1.0, 1.0]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.25, 0.5]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.75, 0.5]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, vec![[1.0, 0.0, 0.0, 1.0]; n]);
    mesh.insert_attribute(DH2_BITANGENT, vec![[0.0, 1.0, 0.0]; n]);
    mesh.insert_attribute(DH2_NORMAL_W, vec![0.0; n]);
    let mut wind_mesh = mesh.clone();
    wind_mesh.insert_attribute(shadow_pass::VEGETATION_POSITION, vec![[0.0, 0.0, 0.5]; n]);
    wind_mesh.insert_attribute(shadow_pass::VEGETATION_NORMAL, vec![[0.5, -0.5]; n]);
    wind_mesh.insert_attribute(shadow_pass::VEGETATION_COLOR, vec![[0.2, 0.4, 0.6]; n]);
    let wind_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(wind_mesh);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let vegetation_buffers = super::vegetation_wind::VegetationMaterials::new(
        &mut app.world_mut().resource_mut::<Assets<ShaderBuffer>>(),
    );
    let sh = app
        .world_mut()
        .resource_mut::<Assets<ShaderBuffer>>()
        .add(ShaderBuffer::from(vec![[Vec4::ZERO; 9]]));
    let vertex_probes = app
        .world_mut()
        .resource_mut::<Assets<ShaderBuffer>>()
        .add(ShaderBuffer::from(vec![0u32; 8]));
    let shadow = app
        .world_mut()
        .resource_mut::<Assets<ShaderBuffer>>()
        .add(ShaderBuffer::from(
            shadow_sampling::ParallelSamplingUniform::default(),
        ));
    let shadow_map = app.world_mut().resource_mut::<Assets<Image>>().add(
        shadow_pipeline::shadow_depth_image(
            bevy::render::render_resource::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureFormat::Depth24PlusStencil8,
        )
        .unwrap(),
    );
    let local_shadow =
        app.world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .add(ShaderBuffer::from(
                vec![0u32; shadow_sampling::local_sampling_word_count(1)],
            ));
    app.world_mut()
        .insert_resource(shadow_pass::ShadowSceneFrame::default());
    app.world_mut()
        .insert_resource(shadow_sampling::ParallelSamplingBuffer {
            buffer: shadow.clone(),
            unavailable_depth: shadow_map.clone(),
        });
    app.add_systems(
        PostUpdate,
        (
            sync_shadow_target::<ProbeExtension>,
            sync_shadow_target::<DecalExtension>,
        )
            .after(bevy::asset::AssetEventSystems),
    );
    let mut surface = surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(&[]));
    surface.misc = UVec4::new(1, 1, 1, 2 | 128);
    surface.flags = UVec4::new(2, 1, 0, 1);
    let probe = app
        .world_mut()
        .resource_mut::<Assets<ProbeMaterial>>()
        .add(ProbeMaterial {
            base: StandardMaterial::default(),
            extension: ProbeExtension {
                fog_parameters: fog_parameters.clone(),
                fog_objects: fog_bindings.objects.clone(),
                fog_world: fog_bindings.world.clone(),
                fog_canvas: fog_bindings.canvas.clone(),
                fog_curve: fog_bindings.curve.clone(),
                fog_depth: fog_bindings.opaque_depth.clone(),
                reflections: None,
                reflection_brdf_lookup: None,
                reflection_planes: sh.clone(),
                material_exposure: material_exposure.clone(),
                sh,
                vertex_probes,
                shadow: shadow.clone(),
                local_shadow: local_shadow.clone(),
                local_shadow_map: Some(shadow_map.clone()),
                shadow_map: Some(shadow_map.clone()),
                screen_map: None,
                cloud_map: None,
                surface: surface.clone(),
                diffuse0: None,
                diffuse1: None,
                diffuse2: None,
                bump0: None,
                bump1: None,
                bump2: None,
                gloss0: None,
                gloss1: None,
                gloss2: None,
                metallic0: None,
                metallic1: None,
                metallic2: None,
                mask0: None,
                mask1: None,
                mask2: None,
                specular0: None,
                specular1: None,
                specular2: None,
                detail_bump: None,
                emissive_map: None,
                emissive_mask: None,
                anisotropic_sampler_image: None,
                lightmap: None,
                filling_diffuse: None,
                filling_distortion1: None,
                filling_distortion2: None,
                filling_lookup: None,
                filling_normal: None,
                filling_fresnel: None,
                cloud_opacity: None,
                cloud_density: None,
                cloud_back_density: None,
                linear_clamp_image: None,
                vegetation_slots: vegetation_buffers.slots.clone(),
                vegetation_instances: vegetation_buffers.instances.clone(),
            },
        });
    // Establish the target, then add a material without a binding while the
    // target stays unchanged. The event-driven sync must repair it before
    // render extraction, without a full material scan every frame.
    app.update();
    let mut late = app
        .world()
        .resource::<Assets<ProbeMaterial>>()
        .get(&probe)
        .unwrap()
        .clone();
    late.extension.shadow_map = None;
    let late = app
        .world_mut()
        .resource_mut::<Assets<ProbeMaterial>>()
        .add(late);
    app.update();
    assert_eq!(
        app.world()
            .resource::<Assets<ProbeMaterial>>()
            .get(&late)
            .unwrap()
            .extension
            .shadow_map
            .as_ref(),
        Some(&shadow_map)
    );
    let mut vegetation = app
        .world()
        .resource::<Assets<ProbeMaterial>>()
        .get(&probe)
        .unwrap()
        .clone();
    vegetation.extension.surface =
        surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(
            b"mainProgram arkVegetation\nhasBumpMap 1\nhasGlossMap 1\nalphaTest 1\n",
        ));
    vegetation.extension.surface.misc.w = 1 | 4;
    vegetation.base.alpha_mode = AlphaMode::AlphaToCoverage;
    vegetation.base.double_sided = true;
    let mut cutout = vegetation.clone();
    cutout.base.alpha_mode = AlphaMode::Mask(0.5);
    let cutout = app
        .world_mut()
        .resource_mut::<Assets<ProbeMaterial>>()
        .add(cutout);
    let vegetation = app
        .world_mut()
        .resource_mut::<Assets<ProbeMaterial>>()
        .add(vegetation);
    let decal_probe_buffers = {
        let p = app
            .world()
            .resource::<Assets<ProbeMaterial>>()
            .get(&probe)
            .unwrap();
        (p.extension.sh.clone(), p.extension.vertex_probes.clone())
    };
    let decal = app
        .world_mut()
        .resource_mut::<Assets<DecalMaterial>>()
        .add(DecalMaterial {
            base: StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                ..default()
            },
            extension: DecalExtension {
                fog_parameters: fog_parameters.clone(),
                fog_objects: fog_bindings.objects.clone(),
                fog_world: fog_bindings.world.clone(),
                fog_canvas: fog_bindings.canvas.clone(),
                fog_curve: fog_bindings.curve.clone(),
                fog_depth: fog_bindings.opaque_depth.clone(),
                reflections: None,
                reflection_brdf_lookup: None,
                reflection_planes: decal_probe_buffers.0.clone(),
                material_exposure: material_exposure.clone(),
                bias: 0.0,
                sh: decal_probe_buffers.0,
                vertex_probes: decal_probe_buffers.1,
                shadow,
                local_shadow,
                shadow_map: Some(shadow_map.clone()),
                local_shadow_map: Some(shadow_map.clone()),
                screen_map: None,
                cloud_map: None,
                surface,
                diffuse0: None,
                diffuse1: None,
                diffuse2: None,
                bump0: None,
                bump1: None,
                bump2: None,
                gloss0: None,
                gloss1: None,
                gloss2: None,
                metallic0: None,
                metallic1: None,
                metallic2: None,
                mask0: None,
                mask1: None,
                mask2: None,
                specular0: None,
                specular1: None,
                specular2: None,
                detail_bump: None,
                emissive_map: None,
                emissive_mask: None,
                anisotropic_sampler_image: None,
                lightmap: None,
                filling_diffuse: None,
                filling_distortion1: None,
                filling_distortion2: None,
                filling_lookup: None,
                filling_normal: None,
                filling_fresnel: None,
                cloud_opacity: None,
                cloud_density: None,
                cloud_back_density: None,
                linear_clamp_image: None,
            },
        });
    app.world_mut().spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(probe.clone()),
        Transform::from_xyz(-0.6, 0.0, 0.0),
    ));
    app.world_mut().spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(decal.clone()),
        Transform::from_xyz(0.6, 0.0, 0.0),
    ));
    app.world_mut().spawn((
        Mesh3d(wind_mesh.clone()),
        MeshMaterial3d(vegetation),
        Transform::from_xyz(0.0, 0.6, 0.0),
    ));
    app.world_mut().spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(cutout),
        Transform::from_xyz(0.0, -0.6, 0.0),
    ));
    // Compile both colour fog routes with disabled, valid shared inputs.
    for (flags, position) in [
        (8, Vec3::new(0.6, 0.6, 0.)),
        (56, Vec3::new(-0.6, -0.6, 0.)),
    ] {
        let mut value = app
            .world()
            .resource::<Assets<ProbeMaterial>>()
            .get(&probe)
            .unwrap()
            .clone();
        value.extension.surface.misc.w |= flags;
        value.base.alpha_mode = AlphaMode::Blend;
        let value = app
            .world_mut()
            .resource_mut::<Assets<ProbeMaterial>>()
            .add(value);
        app.world_mut().spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(value),
            Transform::from_translation(position),
        ));
    }
    let camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            DepthPrepass,
            bevy::render::view::NoIndirectDrawing,
            auto_exposure::MaterialExposure(material_exposure.clone()),
            Msaa::Sample4,
            bevy::camera::RenderTarget::Image(target.clone().into()),
            Transform::from_xyz(0.0, 0.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .id();
    let mut validated = 0;
    let mut prepass_validated = 0;
    let mut fog_validated = 0;
    let mut wind_validated = [0; 2];
    for _ in 0..100 {
        app.update();
        let cache = app
            .sub_app(bevy::render::RenderApp)
            .world()
            .resource::<bevy::render::render_resource::PipelineCache>();
        validated = 0;
        prepass_validated = 0;
        fog_validated = 0;
        wind_validated = [0; 2];
        for p in cache.pipelines() {
            if let CachedPipelineState::Err(e) = &p.state {
                panic!("GPU shader failure: {e}");
            }
            if let (CachedPipelineState::Ok(_), PipelineDescriptor::RenderPipelineDescriptor(d)) =
                (&p.state, &p.descriptor)
            {
                let defined = |wanted: &str| {
                    d.vertex.shader_defs.iter().any(|value| {
                        matches!(value, bevy::shader::ShaderDefVal::Bool(name, true) if name == wanted)
                    })
                };
                if defined("DH2_VEGETATION_WIND") {
                    wind_validated[usize::from(defined("PREPASS_PIPELINE"))] += 1;
                }
                if let Some(fragment) = d.fragment.as_ref() {
                    if fragment.shader == PROBE_SHADER_HANDLE
                        || fragment.shader == DECAL_SHADER_HANDLE
                        || fragment.shader == PROBE_PREPASS_SHADER_HANDLE
                    {
                        validated += 1;
                    }
                    if fragment.shader == PROBE_PREPASS_SHADER_HANDLE {
                        prepass_validated += 1;
                    }
                    if fragment.shader==PROBE_SHADER_HANDLE && fragment.shader_defs.iter().any(
                        |value| matches!(value,bevy::shader::ShaderDefVal::UInt(name,flags) if name=="DH2_ALPHA_FLAGS" && flags&8!=0)
                    ) {fog_validated+=1;}
                }
            }
        }
        if validated >= 7
            && prepass_validated > 0
            && fog_validated >= 2
            && wind_validated.iter().all(|count| *count > 0)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        validated >= 7 && prepass_validated > 0 && fog_validated>=2,
        "surface, decal, vegetation, prepass and both fog routes must compile: {validated} colour / {prepass_validated} prepass / {fog_validated} fog"
    );
    assert!(
        wind_validated.iter().all(|count| *count > 0),
        "forward and prepass vegetation wind must compile: {wind_validated:?}"
    );

    app.world_mut().entity_mut(camera).insert(Msaa::Off);
    // Sample the actual prepass attachment, not the main-pass depth that
    // subsequent forward drawing can overwrite. It has TEXTURE_BINDING
    // but no COPY_SRC, so a one-invocation compute probe reads its center.
    fn all_prepass_depth(app: &mut App) -> Vec<u32> {
        use bevy::core_pipeline::prepass::ViewPrepassTextures;
        use bevy::render::{
            render_resource::*,
            renderer::{RenderDevice, RenderQueue},
        };
        let world = app.sub_app_mut(bevy::render::RenderApp).world_mut();
        let mut views = world.query::<&ViewPrepassTextures>();
        let view = views
            .single(world)
            .unwrap()
            .depth
            .as_ref()
            .unwrap()
            .texture
            .texture
            .create_view(&TextureViewDescriptor {
                aspect: TextureAspect::DepthOnly,
                ..default()
            });
        let device = world.resource::<RenderDevice>();
        let queue = world.resource::<RenderQueue>();
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("test prepass depth readback"),
            source: ShaderSource::Wgsl(
                r#"
                    @group(0) @binding(0) var depth: texture_depth_2d;
                    @group(0) @binding(1) var<storage, read_write> result: array<u32>;
                    @compute @workgroup_size(8,8) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                        result[id.y * 64u + id.x] = bitcast<u32>(textureLoad(depth, vec2<i32>(id.xy), 0));
                    }
                "#
                .into(),
            ),
        });
        let pipeline =
            device
                .wgpu_device()
                .create_compute_pipeline(&RawComputePipelineDescriptor {
                    label: Some("test prepass depth readback"),
                    layout: None,
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: default(),
                    cache: None,
                });
        let output = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 64 * 64 * 4,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 64 * 64 * 4,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let bind = device
            .wgpu_device()
            .create_bind_group(&BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: output.as_entire_binding(),
                    },
                ],
            });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(8, 8, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 64 * 64 * 4);
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let bytes = readback.slice(..).get_mapped_range();
        let depth = bytes
            .chunks_exact(4)
            .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        drop(bytes);
        readback.unmap();
        depth
    }

    fn center_depth(app: &mut App) -> f32 {
        f32::from_bits(all_prepass_depth(app)[32 * 64 + 32])
    }

    let entities: Vec<_> = app
        .world_mut()
        .query_filtered::<Entity, With<Mesh3d>>()
        .iter(app.world())
        .collect();
    for entity in entities {
        app.world_mut().despawn(entity);
    }
    let image = |alpha| {
        Image::new_fill(
            bevy::render::render_resource::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[255, 255, 255, alpha],
            TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::default(),
        )
    };
    let solid = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(image(255));
    let empty = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(image(0));
    let translucent = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(image(191));
    let split = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::new_fill(
            bevy::render::render_resource::Extent3d {
                width: 2,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[255, 255, 255, 0, 255, 255, 255, 255],
            TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::default(),
        ));
    let mut material = app
        .world()
        .resource::<Assets<ProbeMaterial>>()
        .get(&probe)
        .unwrap()
        .clone();
    material.base = StandardMaterial {
        alpha_mode: AlphaMode::Mask(0.5),
        ..default()
    };
    material.extension.surface =
        surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(
            b"mainProgram arkDefault\nalphaTest 1\nDefaultAlphaTest {0.5}\n",
        ));
    material.extension.surface.misc.w = 1;
    material.extension.diffuse0 = Some(solid);
    material.extension.diffuse1 = Some(empty.clone());
    let mut tinted = material.clone();
    tinted.extension.surface.colors[0].w = 0.0;
    tinted.extension.surface.mask_settings[0] = UVec4::new(0, 3, 1, 0);
    let mut layered = material.clone();
    layered.extension.surface.flags.x = 1;
    layered.extension.surface.mask_settings[1] = UVec4::new(0, 3, 0, 0);
    let mut gloss_alpha = material.clone();
    gloss_alpha.extension.diffuse0 = Some(empty.clone());
    gloss_alpha.extension.surface.flags.z = 1;
    let mut vegetation = material.clone();
    vegetation.extension.diffuse0 = Some(translucent);
    vegetation.extension.surface.misc.z = 2;
    vegetation.extension.surface.alpha.x = 0.9;
    let mut uv0 = material.clone();
    uv0.extension.diffuse0 = Some(split);
    let mut uv1 = uv0.clone();
    uv1.extension.surface.uv_sets[0].x = 1;
    let mut scaled = uv1.clone();
    scaled.extension.surface.uv[0] = Vec4::new(0.0, 0.0, 0.25, 0.5);
    let mut opaque = material.clone();
    opaque.base.alpha_mode = AlphaMode::Opaque;
    opaque.extension.diffuse0 = Some(empty);
    opaque.extension.surface.misc.w = 0;
    // Distinct lighting/layer keys share geometry but not reference depth
    // pipelines. The material-free path must batch their transforms correctly.
    let mut companion = opaque.clone();
    companion.extension.surface.flags.x = 2;
    companion.extension.surface.flags.y = 1;
    let companion = app
        .world_mut()
        .resource_mut::<Assets<ProbeMaterial>>()
        .add(companion);
    let companion_entity = app
        .world_mut()
        .spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(companion.clone()),
            Transform::from_xyz(-1.2, 0.0, 0.0),
        ))
        .id();
    fn opaque_depth_batches(app: &App) -> usize {
        use bevy::core_pipeline::prepass::Opaque3dPrepass;
        use bevy::render::render_phase::ViewBinnedRenderPhases;
        app.sub_app(bevy::render::RenderApp)
            .world()
            .resource::<ViewBinnedRenderPhases<Opaque3dPrepass>>()
            .values()
            .map(|phase| {
                phase
                    .direct_batch_count()
                    .expect("fixture uses direct draws")
            })
            .sum()
    }
    let mut coverage_opaque = opaque.clone();
    coverage_opaque.base.alpha_mode = AlphaMode::AlphaToCoverage;
    let cases = [
        ("coverage material with opaque Z", coverage_opaque, true),
        ("opaque alpha", material.clone(), true),
        ("color constant must not erase depth", tinted, true),
        ("layer1 must not erase depth", layered, true),
        (
            "gloss alpha must not turn a hole opaque",
            gloss_alpha,
            false,
        ),
        (
            "vegetation Z uses DefaultAlphaTest, not forward 0.5",
            vegetation,
            false,
        ),
        ("layer0 UV0 selects transparent texel", uv0, false),
        ("layer0 UV1 selects opaque texel", uv1, true),
        ("layer0 scale/bias applies before clipping", scaled, false),
        ("opaque Z variant never samples alpha", opaque, true),
    ];
    let mut forward = material.clone();
    forward.base.unlit = true;
    let handle = app
        .world_mut()
        .resource_mut::<Assets<ProbeMaterial>>()
        .add(material);
    let entity = app
        .world_mut()
        .spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(handle.clone()),
            Transform::default(),
        ))
        .id();
    for (label, material, expected) in cases {
        let expect_material_free = matches!(
            material.base.alpha_mode,
            AlphaMode::Opaque | AlphaMode::AlphaToCoverage
        ) && !surface_gpu::SurfaceKey::from(&material.extension.surface)
            .depth_needs_material();
        app.sub_app_mut(bevy::render::RenderApp)
            .world_mut()
            .remove_resource::<bevy::pbr::DepthPrepassMaterialKeys>();
        *app.world_mut()
            .resource_mut::<Assets<ProbeMaterial>>()
            .get_mut(&handle)
            .unwrap() = material;
        let _ = app
            .world_mut()
            .resource_mut::<Assets<ProbeMaterial>>()
            .get_mut(&companion)
            .unwrap()
            .into_inner();
        for _ in 0..6 {
            app.update();
        }
        if label == "opaque Z variant never samples alpha" {
            assert_eq!(
                opaque_depth_batches(&app),
                2,
                "reference must use separate material pipelines"
            );
        }
        let reference = all_prepass_depth(&mut app);
        let depth = f32::from_bits(reference[32 * 64 + 32]);
        app.sub_app_mut(bevy::render::RenderApp)
            .world_mut()
            .insert_resource(super::materials::depth_prepass_material_keys(true));
        // AssetMut::into_inner emits the modification event needed to respecialize.
        for asset in [&handle, &companion] {
            let _ = app
                .world_mut()
                .resource_mut::<Assets<ProbeMaterial>>()
                .get_mut(asset)
                .unwrap()
                .into_inner();
        }
        for _ in 0..6 {
            app.update();
        }
        let actual = all_prepass_depth(&mut app);
        assert!(
            reference == actual,
            "{label}: shared depth pipeline changed {} pixels",
            reference
                .iter()
                .zip(&actual)
                .filter(|(a, b)| a != b)
                .count()
        );
        let keys = app
            .sub_app(bevy::render::RenderApp)
            .world()
            .resource::<bevy::pbr::DepthPrepassMaterialKeys>();
        assert!(
            keys.normalized_keys() > 0,
            "{label}: depth normalization must actually run"
        );
        if expect_material_free {
            assert!(
                keys.omitted_materials() > 0,
                "{label}: material-free depth must actually run"
            );
        }
        if label == "opaque Z variant never samples alpha" {
            assert_eq!(
                opaque_depth_batches(&app),
                1,
                "material-free depth must actually combine both instances"
            );
        }
        assert!(depth.is_finite(), "{label}: nonfinite depth {depth}");
        assert_eq!(
            depth > 0.0,
            expected,
            "{label}: actual prepass depth {depth}"
        );
    }
    app.world_mut().despawn(companion_entity);
    // A valid prepass silhouette alone cannot catch a later forward discard.
    app.world_mut().entity_mut(camera).remove::<DepthPrepass>();
    app.world_mut()
        .entity_mut(camera)
        .get_mut::<Camera>()
        .unwrap()
        .clear_color = bevy::camera::ClearColorConfig::Custom(Color::BLACK);
    let mut tint = forward.clone();
    tint.extension.surface.colors[0].w = 0.0;
    tint.extension.surface.mask_settings[0] = UVec4::new(0, 3, 1, 0);
    let mut layer = forward.clone();
    layer.extension.surface.flags.x = 1;
    layer.extension.surface.mask_settings[1] = UVec4::new(0, 3, 0, 0);
    let mut hole = forward.clone();
    hole.extension.diffuse0 = hole.extension.diffuse1.clone();
    let mut vertex = forward.clone();
    vertex.extension.surface.misc.y = 1;
    let mut zero_alpha = app
        .world()
        .resource::<Assets<Mesh>>()
        .get(&mesh)
        .unwrap()
        .clone();
    let n = zero_alpha.count_vertices();
    zero_alpha.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.3, 0.7, 1.0, 0.0]; n]);
    let zero_alpha = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(zero_alpha);
    // Isolate one face: reversing a closed cube still exposes its far face.
    let mut face = app
        .world()
        .resource::<Assets<Mesh>>()
        .get(&mesh)
        .unwrap()
        .clone();
    let normals = face
        .attribute(Mesh::ATTRIBUTE_NORMAL)
        .unwrap()
        .as_float3()
        .unwrap();
    let indices: Vec<u32> = face.indices().unwrap().iter().map(|i| i as u32).collect();
    let indices: Vec<u32> = indices
        .chunks_exact(3)
        .filter(|t| normals[t[0] as usize][2] > 0.5)
        .flatten()
        .copied()
        .collect();
    assert_eq!(indices.len(), 6);
    // Exercise the actual CW asset -> CCW render adapter for both sides.
    let front: Vec<u16> = indices
        .chunks_exact(3)
        .flat_map(|t| [t[0] as u16, t[2] as u16, t[1] as u16])
        .collect();
    face.insert_indices(render_indices(&front));
    let mut reversed = face.clone();
    reversed.insert_indices(render_indices(
        &indices.iter().map(|i| *i as u16).collect::<Vec<_>>(),
    ));
    let mut mirrored = mirrored_render_mesh(face.clone(), &front, true);
    let positions = mirrored
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap()
        .iter()
        .map(|p| [-p[0], p[1], p[2]])
        .collect::<Vec<_>>();
    mirrored.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    let mirrored = app.world_mut().resource_mut::<Assets<Mesh>>().add(mirrored);
    let face = app.world_mut().resource_mut::<Assets<Mesh>>().add(face);
    let reversed = app.world_mut().resource_mut::<Assets<Mesh>>().add(reversed);
    let mut back_cull = forward.clone();
    back_cull.base.cull_mode = Some(bevy::render::render_resource::Face::Back);
    let mut front_cull = forward.clone();
    front_cull.base.cull_mode = Some(bevy::render::render_resource::Face::Front);
    let mut no_cull = forward.clone();
    no_cull.base.cull_mode = None;
    fn forward_center(app: &mut App, target: &Handle<Image>) -> [u8; 4] {
        let (tx, rx) = std::sync::mpsc::channel();
        app.world_mut()
            .spawn(Screenshot::image(target.clone()))
            .observe(
                move |event: On<bevy::render::view::screenshot::ScreenshotCaptured>| {
                    let bytes = event.image.data.as_ref().unwrap();
                    tx.send(
                        <[u8; 4]>::try_from(&bytes[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 4])
                            .unwrap(),
                    )
                    .unwrap();
                },
            );
        for _ in 0..200 {
            app.update();
            if let Ok(pixel) = rx.try_recv() {
                return pixel;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("forward pixel readback did not complete");
    }
    for (label, material, mesh, expected) in [
        ("raw diffuse solid", forward, mesh.clone(), true),
        ("raw diffuse hole", hole, mesh.clone(), false),
        (
            "material tint alpha is not a second clip",
            tint,
            mesh.clone(),
            true,
        ),
        (
            "layer alpha is not a second clip",
            layer,
            mesh.clone(),
            true,
        ),
        (
            "vertex alpha is not a second clip",
            vertex,
            zero_alpha,
            true,
        ),
        (
            "BACK keeps mirrored CW fronts",
            back_cull.clone(),
            mirrored,
            true,
        ),
        (
            "BACK keeps front-facing triangles",
            back_cull.clone(),
            face.clone(),
            true,
        ),
        (
            "BACK rejects reverse coincident triangles",
            back_cull,
            reversed.clone(),
            false,
        ),
        (
            "FRONT rejects front-facing triangles",
            front_cull.clone(),
            face.clone(),
            false,
        ),
        (
            "FRONT keeps reverse-facing triangles",
            front_cull,
            reversed.clone(),
            true,
        ),
        (
            "NONE keeps front-facing triangles",
            no_cull.clone(),
            face.clone(),
            true,
        ),
        (
            "NONE keeps reverse-facing triangles",
            no_cull,
            reversed,
            true,
        ),
    ] {
        *app.world_mut()
            .resource_mut::<Assets<ProbeMaterial>>()
            .get_mut(&handle)
            .unwrap() = material;
        app.world_mut().entity_mut(entity).insert(Mesh3d(mesh));
        for _ in 0..6 {
            app.update();
        }
        let pixel = forward_center(&mut app, &target);
        assert_eq!(pixel[0] > 20, expected, "{label}: forward pixel {pixel:?}");
    }
    // Exercise the actual alpha-blended decal fragment path. A late
    // streamed decal must bind the same sun map as opaque materials, and
    // sun visibility must attenuate its direct lighting before blending.
    let mut receiving = app
        .world()
        .resource::<Assets<DecalMaterial>>()
        .get(&decal)
        .unwrap()
        .clone();
    receiving.base = StandardMaterial {
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    };
    receiving.extension.surface = surface_gpu::SurfaceUniform::from(
        &material_surface::Surface::parse(b"mainProgram arkDecal\n"),
    );
    receiving.extension.diffuse0 = app
        .world()
        .resource::<Assets<ProbeMaterial>>()
        .get(&handle)
        .unwrap()
        .extension
        .diffuse0
        .clone();
    receiving.extension.surface.shadow = UVec4::new(1, 0, 1, 2);
    receiving.extension.shadow_map = None;
    let shadow_buffer = receiving.extension.shadow.clone();
    let receiving = app
        .world_mut()
        .resource_mut::<Assets<DecalMaterial>>()
        .add(receiving);
    app.world_mut().despawn(entity);
    app.world_mut().spawn((
        MeshMaterial3d(receiving.clone()),
        Mesh3d(face),
        Transform::default(),
    ));
    app.world_mut().insert_resource(GlobalAmbientLight::NONE);
    app.world_mut().spawn((
        DirectionalLight {
            illuminance: 8000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::default(),
    ));
    for _ in 0..6 {
        app.update();
    }
    for _ in 0..100 {
        app.update();
        let cache = app
            .sub_app(bevy::render::RenderApp)
            .world()
            .resource::<bevy::render::render_resource::PipelineCache>();
        if cache
            .pipelines()
            .all(|p| matches!(p.state, CachedPipelineState::Ok(_)))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        app.world()
            .resource::<Assets<DecalMaterial>>()
            .get(&receiving)
            .unwrap()
            .extension
            .shadow_map
            .as_ref(),
        Some(&shadow_map)
    );
    for _ in 0..6 {
        app.update();
    }
    let lit = forward_center(&mut app, &target);
    let mut shadowed = shadow_sampling::ParallelSamplingUniform::default();
    // LESS against depth>=1 cannot pass at reference2, independent of the
    // unavailable texture's contents. The source-backed PCF is still used.
    shadowed.world_to_shadow = [Mat4::from_cols_array_2d(&[
        [0., 0., 0., 0.5],
        [0., 0., 0., 0.5],
        [0., 0., 0., 2.],
        [0., 0., 0., 1.],
    ]); 5];
    shadowed.cascade_fars = Vec4::splat(10.0);
    shadowed.shadow_size = Vec4::new(1., 1., 0., 0.);
    shadowed.config = UVec4::new(1, 0, 0, 1);
    shadowed.quality = IVec4::new(2, 1, 0, 0);
    *app.world_mut()
        .resource_mut::<Assets<ShaderBuffer>>()
        .get_mut(&shadow_buffer)
        .unwrap() = ShaderBuffer::from(shadowed);
    for _ in 0..6 {
        app.update();
    }
    let dark = forward_center(&mut app, &target);
    let energy = |p: [u8; 4]| p[..3].iter().map(|&v| u32::from(v)).sum::<u32>();
    assert!(
        energy(lit) > 50 && energy(dark) * 5 < energy(lit),
        "decal must receive native sun visibility: lit={lit:?} shadowed={dark:?}"
    );
    eprintln!(
        "GPU decal native shadow attenuation: lit={lit:?}, shadowed={dark:?}; late material atlas association passed"
    );
    // Compile the separate native specular-map branch, including layered
    // reuse. A valid shadow remains required after this material change.
    {
        let mut materials = app.world_mut().resource_mut::<Assets<DecalMaterial>>();
        let extension = &mut materials.get_mut(&receiving).unwrap().extension;
        extension.surface.specular_reuse = UVec4::new(0, 1, 2, 1);
        extension.surface.flags.x = 2;
        extension.specular0 = extension.diffuse0.clone();
    }
    for _ in 0..6 {
        app.update();
    }
    let specular_dark = forward_center(&mut app, &target);
    assert!(
        energy(specular_dark) * 5 < energy(lit),
        "specular-map decal must retain shadow receiving: {specular_dark:?}"
    );
    // Native arkDefault writes finalized lighting RGB independently of
    // diffuse alpha. arkDecal applies coverage once, in hardware blending.
    // Exercise both through the real material draw with no direct light.
    app.world_mut().entity_mut(camera).insert((
        bevy::camera::Exposure { ev100: 0.0 },
        bevy::core_pipeline::tonemapping::Tonemapping::None,
    ));
    let probe_buffer = app
        .world()
        .resource::<Assets<DecalMaterial>>()
        .get(&receiving)
        .unwrap()
        .extension
        .sh
        .clone();
    let mut coefficients = [Vec4::ZERO; 9];
    coefficients[0] = Vec4::splat(0.25 / render_light::ARKANE_TO_WORLD_LUMINANCE);
    *app.world_mut()
        .resource_mut::<Assets<ShaderBuffer>>()
        .get_mut(&probe_buffer)
        .unwrap() = ShaderBuffer::from(vec![coefficients]);
    let mut pixels = Vec::new();
    for (mode, alpha) in [
        (AlphaMode::Opaque, 1.0),
        (AlphaMode::Opaque, 0.0),
        (AlphaMode::Blend, 1.0),
        (AlphaMode::Blend, 0.5),
        (AlphaMode::Blend, 0.0),
    ] {
        {
            let mut materials = app.world_mut().resource_mut::<Assets<DecalMaterial>>();
            let mut material = materials.get_mut(&receiving).unwrap();
            material.base.alpha_mode = mode;
            material.extension.surface = surface_gpu::SurfaceUniform::from(
                &material_surface::Surface::parse(b"mainProgram arkDecal\n"),
            );
            material.extension.surface.shadow = UVec4::new(1, 0, 1, 2);
            material.extension.surface.colors[0].w = alpha;
            material.extension.surface.mask_settings[0] = UVec4::new(0, 3, 1, 0);
            material.extension.specular0 = None;
        }
        for _ in 0..12 {
            app.update();
        }
        pixels.push(forward_center(&mut app, &target));
    }
    assert!(
        energy(pixels[0]) > 30,
        "probe lighting must be visible: {pixels:?}"
    );
    for c in 0..3 {
        assert!(
            (i32::from(pixels[0][c]) - [52, 51, 51][c]).abs() <= 2,
            "HDR must convert to world luminance once: {pixels:?}"
        );
    }
    for c in 0..3 {
        assert!(
            (i32::from(pixels[0][c]) - i32::from(pixels[1][c])).abs() <= 2,
            "opaque indirect light must ignore coverage alpha: {pixels:?}"
        );
    }
    let linear = |v: u8| {
        let x = f32::from(v) / 255.0;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    for c in 0..3 {
        assert!(
            (linear(pixels[3][c]) / linear(pixels[2][c]) - 0.5).abs() < 0.06,
            "decal indirect light must blend alpha once: {pixels:?}"
        );
    }
    assert!(
        energy(pixels[4]) < 10,
        "zero-coverage decal must remain transparent: {pixels:?}"
    );
    eprintln!("GPU native indirect coverage: opaque alpha1/0, blended alpha1/0.5/0 = {pixels:?}");
    // Verify native exposure reaches the displayed pixel through the HDR
    // camera and final tonemapping, rather than only updating its state.
    app.world_mut()
        .resource_mut::<Assets<DecalMaterial>>()
        .get_mut(&receiving)
        .unwrap()
        .extension
        .surface
        .colors[0]
        .w = 1.0;
    app.world_mut()
        .entity_mut(camera)
        .insert(bevy::core_pipeline::tonemapping::Tonemapping::Reinhard);
    let mut displayed = Vec::new();
    for luminance in [1.0, 4.0] {
        app.world_mut()
            .entity_mut(camera)
            .insert(auto_exposure::ExposureParameters {
                range: Vec4::new(luminance, luminance, 0.0, 1.0),
                adaptation: Vec4::new(0.0, 1.0, 1.0, 1.0),
                camera: Vec4::new(30.0, 0.0, 0.0, 0.0),
            });
        assert!(app.world().entity(camera).contains::<bevy::camera::Hdr>());
        for _ in 0..100 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        displayed.push(forward_center(&mut app, &target));
    }
    let brighter = linear(displayed[0][0]);
    let darker = linear(displayed[1][0]);
    assert!(
        brighter > 0.0001 && (brighter / darker - 4.0).abs() < 1.0,
        "exposure must reach final pixel once: {displayed:?}"
    );
    eprintln!("GPU native exposure final display: luminance1/4={displayed:?}");
    auto_exposure::verify_gpu_math(&mut app);
    vista_lighting_matches_reference(&mut app);
    filling_matches_reference(&mut app);
    probe_shader_matches(&mut app);
    environment_reflection::shader_matches(&mut app);
    tiled_depth_shader_matches_reference(&mut app);
    // Exercise ordinary emissive-map/mask sampling through the submitted
    // material, with no direct or probe lighting. Emission is independent
    // of diffuse coverage until the hardware decal blend.
    let entities = app
        .world_mut()
        .query_filtered::<Entity, Or<(With<Mesh3d>, With<DirectionalLight>)>>()
        .iter(app.world())
        .collect::<Vec<_>>();
    for e in entities {
        app.world_mut().despawn(e);
    }
    app.world_mut()
        .entity_mut(camera)
        .remove::<(auto_exposure::ExposureParameters, bevy::camera::Hdr)>()
        .insert((
            bevy::camera::Exposure { ev100: 0.0 },
            bevy::core_pipeline::tonemapping::Tonemapping::None,
        ));
    *app.world_mut()
        .resource_mut::<Assets<ShaderBuffer>>()
        .get_mut(&probe_buffer)
        .unwrap() = ShaderBuffer::from(vec![[Vec4::ZERO; 9]]);
    let pixel_image = |rgba: [u8; 4]| {
        Image::new_fill(
            Extent3d::default(),
            TextureDimension::D2,
            &rgba,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::default(),
        )
    };
    let white = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(pixel_image([255; 4]));
    let emission = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(pixel_image([255, 128, 64, 0]));
    let mask = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(pixel_image([191, 0, 0, 0]));
    let mut emitting = app
        .world()
        .resource::<Assets<ProbeMaterial>>()
        .get(&handle)
        .unwrap()
        .clone();
    emitting.base = StandardMaterial { ..default() };
    emitting.extension.sh = probe_buffer;
    emitting.extension.surface=surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(
            b"hasEmissiveMap 1\nhasEmissiveMaskMap 1\nemissiveIntensity 0.001\nemissiveClipParms {0.5,0.1}\nuse_Layer0_ColorConstant 1\n"));
    emitting.extension.diffuse0 = Some(white);
    emitting.extension.emissive_map = Some(emission);
    emitting.extension.emissive_mask = Some(mask.clone());
    let emitting = app
        .world_mut()
        .resource_mut::<Assets<ProbeMaterial>>()
        .add(emitting);
    app.world_mut().spawn((
        Mesh3d(mesh),
        MeshMaterial3d(emitting.clone()),
        Transform::default(),
    ));
    let mut emitted = Vec::new();
    for alpha in [1.0, 0.0] {
        app.world_mut()
            .resource_mut::<Assets<ProbeMaterial>>()
            .get_mut(&emitting)
            .unwrap()
            .extension
            .surface
            .colors[0]
            .w = alpha;
        for _ in 0..80 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        emitted.push(forward_center(&mut app, &target));
    }
    for c in 0..3 {
        let expected =
            0.001 * render_light::ARKANE_TO_WORLD_LUMINANCE * [1.0, 128.0 / 255.0, 64.0 / 255.0][c]
                / 1.2;
        assert!(
            (linear(emitted[0][c]) - expected).abs() < 0.006,
            "emission units/channel{c}: {emitted:?}"
        );
        assert!(
            (i32::from(emitted[0][c]) - i32::from(emitted[1][c])).abs() <= 2,
            "opaque emission must ignore diffuse alpha: {emitted:?}"
        );
    }
    *app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&mask)
        .unwrap() = pixel_image([64, 0, 0, 0]);
    for _ in 0..20 {
        app.update();
    }
    let masked = forward_center(&mut app, &target);
    assert!(energy(masked) < 10, "emission mask must clip: {masked:?}");
    eprintln!("GPU native emission: alpha1/0={emitted:?}, masked={masked:?}");
    // Exercise the actual vista material layout, UV1 lightmap sampling and
    // unconditional forward alpha clip. UV0 deliberately addresses the other
    // lightmap texel, so accidentally using UV0 cannot pass this check.
    let mesh_handle = app
        .world_mut()
        .query::<&Mesh3d>()
        .single(app.world())
        .unwrap()
        .0
        .clone();
    {
        let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
        let mut mesh = meshes.get_mut(&mesh_handle).unwrap();
        let count = mesh.count_vertices();
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.25, 0.5]; count]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.75, 0.5]; count]);
    }
    let baked = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::new(
            Extent3d {
                width: 2,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![4, 8, 12, 0, 16, 24, 32, 0],
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::default(),
        ));
    let diffuse = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(pixel_image([128, 64, 32, 255]));
    {
        let mut materials = app.world_mut().resource_mut::<Assets<ProbeMaterial>>();
        let mut m = materials.get_mut(&emitting).unwrap();
        m.extension.surface = surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(
            b"mainProgram arkVista\nmetallicMinMax {0,0}\nwardRoughness {0,0}\n",
        ));
        m.extension.surface.vista = [
            Vec4::new(0.1, 0., 0., 0.),
            Vec4::new(-1., 1., 0., 1.),
            Vec4::new(0., 1., 0., 0.),
            Vec4::new(1., 0., 0.01, 0.),
        ];
        m.extension.diffuse0 = Some(diffuse.clone());
        m.extension.lightmap = Some(baked);
    }
    for _ in 0..100 {
        app.update();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let vista = forward_center(&mut app, &target);
    for c in 0..3 {
        let expected = [128., 64., 32.][c] / 255. * [16., 24., 32.][c] / 255.
            * 0.1
            * render_light::ARKANE_TO_WORLD_LUMINANCE
            / 1.2;
        assert!(
            (linear(vista[c]) - expected).abs() < 0.006,
            "vista UV1 channel{c}: {vista:?}, expected{expected}"
        );
    }
    *app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&diffuse)
        .unwrap() = pixel_image([128, 64, 32, 0]);
    for _ in 0..30 {
        app.update();
    }
    let clipped = forward_center(&mut app, &target);
    assert!(
        energy(clipped) < 10,
        "vista forward alpha clip: {clipped:?}"
    );
    eprintln!("GPU vista UV1 lightmap and alpha clip: {vista:?}, {clipped:?}");
    {
        let black = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(pixel_image([0, 0, 0, 255]));
        let liquid_diffuse = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(pixel_image([64, 128, 192, 255]));
        let lookup = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(pixel_image([255, 64, 32, 255]));
        let mut materials = app.world_mut().resource_mut::<Assets<ProbeMaterial>>();
        let mut m = materials.get_mut(&emitting).unwrap();
        m.base.unlit = true;
        m.extension.surface = surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(
            b"mainProgram particle/filing_container\n",
        ));
        let p = &mut m.extension.surface;
        // Keep the native pixel-lit clip branch; isolate procedural colour
        // from lighting with the host material's debug-unlit switch.
        p.filling_flags = UVec4::new(1, 1, 0, 0);
        p.filling[2] = Vec4::ONE;
        p.filling[3] = Vec4::ONE;
        p.filling[4].w = 1.;
        p.filling[5] = Vec4::new(0., 2., 0.1, 0.);
        p.filling[6] = Vec4::ONE;
        p.filling[7] = Vec4::new(1., 1., 1., 0.001);
        p.filling[8] = Vec4::new(1., 0.5, 1., 0.03);
        p.filling[9].w = 100000.;
        m.extension.filling_diffuse = Some(liquid_diffuse);
        m.extension.filling_distortion1 = Some(black.clone());
        m.extension.filling_distortion2 = Some(black);
        m.extension.filling_lookup = Some(lookup);
    }
    app.world_mut().entity_mut(camera).insert(DepthPrepass);
    let mut filled = Vec::new();
    let mut liquid_depth = Vec::new();
    for (level, fresnel) in [(1., false), (0., false), (1., true)] {
        {
            let mut materials = app.world_mut().resource_mut::<Assets<ProbeMaterial>>();
            let mut m = materials.get_mut(&emitting).unwrap();
            m.extension.surface.filling[4].w = level;
            m.extension.surface.filling_flags.x = if fresnel { 3 } else { 1 };
        }
        for _ in 0..40 {
            app.update();
        }
        filled.push(forward_center(&mut app, &target));
        liquid_depth.push(center_depth(&mut app));
    }
    assert!(
        energy(filled[0]) > 100,
        "liquid shader must draw colour: {filled:?}"
    );
    assert!(energy(filled[1]) < 10, "empty liquid must clip: {filled:?}");
    assert!(
        energy(filled[2]) > 100,
        "front-facing Fresnel liquid must remain visible: {filled:?}"
    );
    assert!(
        liquid_depth[0] > 0.0 && liquid_depth[1] == 0.0 && liquid_depth[2] > 0.0,
        "liquid prepass fill: {liquid_depth:?}"
    );
    let mut view_fade = Vec::new();
    for (end, visible) in [(1.0, true), (8.0, false)] {
        {
            let mut materials = app.world_mut().resource_mut::<Assets<ProbeMaterial>>();
            let mut material = materials.get_mut(&emitting).unwrap();
            let surface = &mut material.extension.surface;
            surface.filling_flags.x = 17;
            surface.filling[8].w = 0.7;
            surface.filling[10].z = 0.0;
            surface.filling[10].w = end;
        }
        for _ in 0..80 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        let color = forward_center(&mut app, &target);
        let depth = center_depth(&mut app);
        assert_eq!(energy(color) > 100, visible, "view fade color: {color:?}");
        assert_eq!(depth > 0.0, visible, "view fade prepass: {depth}");
        view_fade.push((color, depth));
    }
    // Resume the preceding Fresnel case without the independent fade option.
    {
        let mut materials = app.world_mut().resource_mut::<Assets<ProbeMaterial>>();
        let mut material = materials.get_mut(&emitting).unwrap();
        material.extension.surface.filling_flags.x = 3;
        material.extension.surface.filling[8].w = 0.03;
    }
    eprintln!("GPU mesh-particle clip-W view fade: {view_fade:?}");
    app.world_mut()
        .resource_mut::<Assets<ProbeMaterial>>()
        .get_mut(&emitting)
        .unwrap()
        .extension
        .surface
        .filling[8]
        .y = 100.0;
    for _ in 0..40 {
        app.update();
    }
    let fresnel_clipped = forward_center(&mut app, &target);
    assert!(
        energy(fresnel_clipped) < 10 && center_depth(&mut app) > 0.0,
        "Fresnel clips forward only: {fresnel_clipped:?}"
    );
    app.world_mut().entity_mut(camera).remove::<DepthPrepass>();
    eprintln!("GPU filling full/empty/Fresnel: {filled:?}");
    {
        let mut materials = app.world_mut().resource_mut::<Assets<ProbeMaterial>>();
        let mut m = materials.get_mut(&emitting).unwrap();
        m.extension.surface.filling_flags = UVec4::new(1, 0, 0, 0);
        m.extension.surface.filling[10].y = 1.0;
    }
    let mut compensated = Vec::new();
    for factor in [1.0_f32, 0.5] {
        *app.world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .get_mut(&material_exposure)
            .unwrap() = ShaderBuffer::from(vec![factor, 1., 0., 0., 0., 0., 0., 0.]);
        for _ in 0..40 {
            app.update();
        }
        compensated.push(forward_center(&mut app, &target));
    }
    for channel in 0..3 {
        let ratio = linear(compensated[1][channel]) / linear(compensated[0][channel]);
        assert!(
            (ratio - 2.0).abs() < 0.1,
            "unlit liquid reads native exposure state: {compensated:?}"
        );
    }
    eprintln!("GPU unlit liquid exposure compensation: {compensated:?}");
    {
        let mut materials = app.world_mut().resource_mut::<Assets<ProbeMaterial>>();
        let mut m = materials.get_mut(&emitting).unwrap();
        m.extension.surface = surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(
            b"mainProgram arkSky\n",
        ));
        m.extension.surface.sky[0].x = 0.001;
        m.extension.surface.sky[1] = Vec4::new(30., 60., 0.1, 0.);
        m.extension.diffuse0 = Some(diffuse.clone());
    }
    for _ in 0..40 {
        app.update();
    }
    let sky_pixel = forward_center(&mut app, &target);
    assert!(
        energy(sky_pixel) > 30,
        "authored sky must draw at far depth: {sky_pixel:?}"
    );
    {
        let mut materials = app.world_mut().resource_mut::<Assets<ProbeMaterial>>();
        let mut m = materials.get_mut(&emitting).unwrap();
        m.extension.surface.misc.z = 8;
        m.extension.surface.sky[2].z = 1.;
        m.extension.surface.sky[3].w = 1.;
        m.extension.surface.sky[4].x = 1.;
        m.extension.surface.sky[5] = Vec4::new(0., 1., 0., 0.75);
        m.base.alpha_mode = AlphaMode::Blend;
    }
    for _ in 0..40 {
        app.update();
    }
    let _ = forward_center(&mut app, &target);
    // The celestial pass must be visible against the far plane and occluded
    // by ordinary scene depth. Its screen size follows native aspect scaling.
    let existing = app
        .world_mut()
        .query_filtered::<Entity, With<Mesh3d>>()
        .iter(app.world())
        .collect::<Vec<_>>();
    for e in &existing {
        app.world_mut().entity_mut(*e).insert(Visibility::Hidden);
    }
    let image = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(pixel_image([255; 4]));
    let celestial = app
        .world_mut()
        .resource_mut::<Assets<super::sky_sprite::CelestialSprite>>()
        .add(super::sky_sprite::comparison_material(image));
    let quad = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Rectangle::new(2., 2.));
    let celestial = app
        .world_mut()
        .spawn((
            Mesh3d(quad),
            MeshMaterial3d(celestial),
            Transform::default(),
            bevy::camera::visibility::NoFrustumCulling,
        ))
        .id();
    for _ in 0..40 {
        app.update();
    }
    let disc = forward_center(&mut app, &target);
    assert!(energy(disc) > 80, "celestial disc must draw: {disc:?}");
    let mut occluding = app
        .world()
        .resource::<Assets<ProbeMaterial>>()
        .get(&emitting)
        .unwrap()
        .clone();
    occluding.base.alpha_mode = AlphaMode::Opaque;
    occluding.extension.surface = surface_gpu::SurfaceUniform::from(
        &material_surface::Surface::parse(b"mainProgram arkDefault\n"),
    );
    let occluding = app
        .world_mut()
        .resource_mut::<Assets<ProbeMaterial>>()
        .add(occluding);
    let occluder = app
        .world_mut()
        .spawn((
            Mesh3d(mesh_handle.clone()),
            MeshMaterial3d(occluding),
            Transform::default(),
        ))
        .id();
    for _ in 0..40 {
        app.update();
    }
    let covered = forward_center(&mut app, &target);
    assert_ne!(disc, covered, "scene geometry must occlude celestial disc");
    eprintln!("GPU sky/disc/depth: {sky_pixel:?} / {disc:?} / {covered:?}");
    app.world_mut().despawn(celestial);
    for _ in 0..12 {
        app.update();
    }
    let without_disc = forward_center(&mut app, &target);
    if covered != without_disc {
        use bevy::render::render_resource::PipelineDescriptor;
        let cache = app
            .sub_app(bevy::render::RenderApp)
            .world()
            .resource::<bevy::render::render_resource::PipelineCache>();
        for p in cache.pipelines() {
            if let PipelineDescriptor::RenderPipelineDescriptor(d) = &p.descriptor {
                if let Some(f) = &d.fragment {
                    if f.shader == PROBE_SHADER_HANDLE {
                        let program = f
                            .shader_defs
                            .iter()
                            .find(|v| format!("{v:?}").contains("DH2_SURFACE_ENABLED"));
                        let far = d
                            .vertex
                            .shader_defs
                            .iter()
                            .filter(|v| format!("{v:?}").contains("DH2_PROJECT_FAR"))
                            .collect::<Vec<_>>();
                        eprintln!(
                            "occlusion pipeline program={program:?} far={far:?} depth={:?}",
                            d.depth_stencil
                        );
                    }
                }
            }
        }
    }
    assert_eq!(
        covered, without_disc,
        "occluded disc must contribute no light"
    );
    app.world_mut().despawn(occluder);
    for e in &existing {
        app.world_mut().entity_mut(*e).insert(Visibility::Visible);
    }
    {
        *app.world_mut()
            .resource_mut::<Assets<Image>>()
            .get_mut(&diffuse)
            .unwrap() = pixel_image([128, 64, 32, 255]);
        let mut materials = app.world_mut().resource_mut::<Assets<ProbeMaterial>>();
        let mut m = materials.get_mut(&emitting).unwrap();
        m.extension.surface.misc.z = 9;
        m.base.alpha_mode = AlphaMode::Blend;
    }
    for _ in 0..40 {
        app.update();
    }
    let sprite_pixel = forward_center(&mut app, &target);
    assert!(
        energy(sprite_pixel) > 40,
        "textured vertex-colour sprite must draw: {sprite_pixel:?}"
    );
    eprintln!("GPU simple sprite: {sprite_pixel:?}");
    // The test fragment exposes the production surface evaluator's result;
    // it does not duplicate its layer/detail sampling implementation.
    app.world_mut()
        .resource_mut::<Assets<Shader>>()
        .insert(
            PROBE_SHADER_HANDLE.id(),
            Shader::from_wgsl(
                r#"
            #import bevy_pbr::forward_io::{VertexOutput,FragmentOutput}
            #import dh2_surface::evaluate_surface
            @fragment fn fragment(v:VertexOutput)->FragmentOutput {
                let s=evaluate_surface(v.uv,vec2(0.0),vec4(1.0));
                var out:FragmentOutput;out.color=vec4(s.normal_xy,s.metallic,1.0);return out;
            }
        "#,
                "surface map probe",
            ),
        )
        .unwrap();
    let base_normal = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(pixel_image([64, 128, 0, 0]));
    let detail_normal = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(pixel_image([128, 64, 0, 0]));
    let zero = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(pixel_image([0; 4]));
    {
        let mut materials = app.world_mut().resource_mut::<Assets<ProbeMaterial>>();
        let mut material = materials.get_mut(&emitting).unwrap();
        material.extension.surface=surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(
                b"hasBumpMap 1\nhasDetailBumpMap 1\ndetailBumpmapIntensity 0.5\nextraLayer 1\nLayer1_BumpReuseLayer 1\nhasMetallicMap 1\nmetallicMinMax {0,1}\n"));
        material.extension.bump0 = Some(base_normal);
        material.extension.detail_bump = Some(detail_normal);
        material.extension.metallic1 = Some(zero);
    }
    for _ in 0..80 {
        app.update();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let detail = forward_center(&mut app, &target);
    for (c, expected) in [128.0 / 255.0, 160.0 / 255.0, 0.0].into_iter().enumerate() {
        assert!(
            (linear(detail[c]) - expected).abs() < 0.006,
            "layer/detail map sample channel{c}: {detail:?}"
        );
    }
    eprintln!("GPU native detail/layer metallic sampling: {detail:?}");

    // The reported sign sits just behind its receiver. A constant raster
    // bias on Depth32Float cannot reproduce arkDecal's homogeneous offset.
    let all_meshes: Vec<_> = app
        .world_mut()
        .query_filtered::<Entity, With<Mesh3d>>()
        .iter(app.world())
        .collect();
    for entity in all_meshes {
        app.world_mut()
            .entity_mut(entity)
            .insert(Visibility::Hidden);
    }
    *app.world_mut()
        .resource_mut::<Assets<Shader>>()
        .get_mut(&PROBE_SHADER_HANDLE)
        .unwrap() = Shader::from_wgsl(
        include_str!("../../assets/shaders/dh2_probe.wgsl"),
        "decal-depth-receiver",
    );
    let mut receiver = app
        .world()
        .resource::<Assets<ProbeMaterial>>()
        .get(&emitting)
        .unwrap()
        .clone();
    receiver.base = StandardMaterial {
        base_color: Color::srgb(1.0, 0.0, 0.0),
        unlit: true,
        ..default()
    };
    receiver.extension.surface.misc.z = 0;
    let receiver = app
        .world_mut()
        .resource_mut::<Assets<ProbeMaterial>>()
        .add(receiver);
    let mut overlay = app
        .world()
        .resource::<Assets<DecalMaterial>>()
        .get(&decal)
        .unwrap()
        .clone();
    overlay.base = StandardMaterial {
        base_color: Color::srgb(0.0, 1.0, 0.0),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    };
    overlay.extension.surface.misc.z = 0;
    let overlay = app
        .world_mut()
        .resource_mut::<Assets<DecalMaterial>>()
        .add(overlay);
    let quad = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Rectangle::new(2.0, 2.0));
    let fog_receiver_entity = app.world_mut().spawn((
        Mesh3d(quad.clone()),
        MeshMaterial3d(receiver.clone()),
        Transform::default(),
    )).id();
    let overlay_entity = app
        .world_mut()
        .spawn((
            Mesh3d(quad),
            MeshMaterial3d(overlay.clone()),
            Transform::from_xyz(0.0, 0.0, -0.01),
        ))
        .id();
    app.world_mut()
        .entity_mut(camera)
        .insert(Projection::custom(camera_projection::GameProjection::new(
            110.0, 0.01, 7000.0,
        )));
    for distance in [2.0, 4.0, 8.0, 16.0] {
        app.world_mut()
            .entity_mut(camera)
            .insert(Transform::from_xyz(0.0, 0.0, distance));
        for (bias, green) in [(0.0, false), (-0.0001, true), (0.0001, false)] {
            app.world_mut()
                .resource_mut::<Assets<DecalMaterial>>()
                .get_mut(&overlay)
                .unwrap()
                .extension
                .bias = bias;
            for _ in 0..24 {
                app.update();
            }
            let pixel = forward_center(&mut app, &target);
            assert_eq!(
                pixel[1] > pixel[0],
                green,
                "decal receiver distance={distance} zbias={bias} pixel={pixel:?}"
            );
        }
    }
    eprintln!(
        "GPU decal clip-space bias: zero/negative/positive offsets at distances 2/4/8/16 passed"
    );
    app.world_mut()
        .entity_mut(overlay_entity)
        .insert(Visibility::Hidden);
    let previous_lights: Vec<_> = app
        .world_mut()
        .query_filtered::<Entity, Or<(With<DirectionalLight>, With<PointLight>, With<SpotLight>)>>()
        .iter(app.world())
        .collect();
    for entity in previous_lights {
        app.world_mut()
            .entity_mut(entity)
            .insert(Visibility::Hidden);
    }
    {
        let mut materials = app.world_mut().resource_mut::<Assets<ProbeMaterial>>();
        let mut material = materials.get_mut(&receiver).unwrap();
        material.base.base_color = Color::WHITE;
        material.base.unlit = false;
    }
    let point = app
        .world_mut()
        .spawn((
            PointLight {
                intensity: 2000.0,
                range: 1.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 0.5),
        ))
        .id();
    for gpu_clustering in [false, true] {
        app.world_mut()
            .resource_mut::<bevy::light::cluster::GlobalClusterSettings>()
            .gpu_clustering =
            gpu_clustering.then_some(bevy::light::cluster::GlobalClusterGpuSettings {
                initial_z_slice_list_capacity: 4096,
                initial_index_list_capacity: 16384,
            });
        let mut candle_pixels = Vec::new();
        for distance in [6.0, 4.0, 2.0, 0.75] {
            app.world_mut()
                .entity_mut(camera)
                .insert(Transform::from_xyz(0.0, 0.0, distance));
            for _ in 0..24 {
                app.update();
                if gpu_clustering {
                    // Cluster far-Z statistics return asynchronously from the
                    // GPU, then cross extraction before the next main-world
                    // grid assignment. Drain that work before judging the
                    // new camera position rather than the previous one.
                    app.sub_app(bevy::render::RenderApp).world()
                        .resource::<bevy::render::renderer::RenderDevice>()
                        .poll(bevy::render::render_resource::PollType::Wait {
                            submission_index: None,
                            timeout: Some(std::time::Duration::from_secs(30)),
                        }).unwrap();
                }
            }
            let clusters = app
                .world()
                .get::<bevy::light::cluster::Clusters>(camera)
                .unwrap();
            let factors = bevy::light::cluster::assign::calculate_cluster_factors(
                clusters.near,
                clusters.far,
                clusters.dimensions.z as f32,
                false,
            );
            assert!(
                factors.is_finite(),
                "local-light grid must remain finite at distance {distance}: {clusters:?}"
            );
            if distance < 5.0 {
                assert_eq!(clusters.dimensions.z, 1,
                    "camera distance={distance}, GPU assignment={gpu_clustering}, grid={clusters:?}");
            }
            let on = forward_center(&mut app, &target);
            app.world_mut().entity_mut(point).insert(Visibility::Hidden);
            for _ in 0..6 {
                app.update();
            }
            let off = forward_center(&mut app, &target);
            assert!(
                energy(on) > energy(off) + 20,
                "nearby point light disappeared at distance {distance}: on={on:?} off={off:?}"
            );
            app.world_mut()
                .entity_mut(point)
                .insert(Visibility::Visible);
            candle_pixels.push(on);
        }
        eprintln!(
            "GPU point light crossing first depth slice (GPU assignment={gpu_clustering}): {candle_pixels:?}"
        );
    }
    // Exercise shared fog inputs through real materials, including the vertex
    // route on geometry without tangents. Only fog readiness/colour changes.
    app.world_mut().entity_mut(camera).insert((
        Transform::from_xyz(0., 0., 4.),
        bevy::camera::Exposure { ev100: 0. },
        bevy::core_pipeline::tonemapping::Tonemapping::None,
    ));
    {
        let mut materials = app.world_mut().resource_mut::<Assets<ProbeMaterial>>();
        let mut value = materials.get_mut(&receiver).unwrap();
        value.base.unlit = true;
        value.base.base_color = Color::WHITE;
        value.base.alpha_mode = AlphaMode::Blend;
        value.extension.surface.misc = UVec4::ZERO;
    }
    let fog_image = |rgb: [f32; 3]| {
        let data: Vec<u8> = rgb
            .into_iter()
            .chain([1.])
            .flat_map(|value| half::f16::from_f32(value).to_bits().to_le_bytes())
            .collect();
        Image::new_fill(
            Extent3d::default(),
            TextureDimension::D2,
            &data,
            TextureFormat::Rgba16Float,
            Default::default(),
        )
    };
    let red_fog = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(fog_image([1. / 1024., 0., 0.]));
    let blue_fog = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(fog_image([0., 0., 1. / 1024.]));
    for flags in [8, 56] {
        app.world_mut()
            .resource_mut::<Assets<ProbeMaterial>>()
            .get_mut(&receiver)
            .unwrap()
            .extension
            .surface
            .misc
            .w = flags;
        let mut pixels = Vec::new();
        for (ready, image) in [
            (false, red_fog.clone()),
            (true, red_fog.clone()),
            (true, blue_fog.clone()),
            (false, blue_fog.clone()),
        ] {
            let mut parameters =
                crate::rendering::bevy_backend::fog_material::FogMaterialParameters::disabled();
            parameters.control = Vec4::new(u8::from(ready) as f32, 0.01, 0., 0.);
            parameters.unproject = Vec4::new(1., 1., 0., 1.);
            app.world_mut()
                .resource_mut::<Assets<ShaderBuffer>>()
                .get_mut(&fog_parameters)
                .unwrap()
                .set_data(parameters);
            app.world_mut()
                .resource_mut::<Assets<ProbeMaterial>>()
                .get_mut(&receiver)
                .unwrap()
                .extension
                .fog_world = Some(image);
            for _ in 0..24 {
                app.update();
            }
            pixels.push(forward_center(&mut app, &target));
        }
        assert_eq!(
            pixels[0], pixels[3],
            "disabling material fog must restore the original output: {flags} {pixels:?}"
        );
        assert!(
            i32::from(pixels[1][0]) > i32::from(pixels[1][2]) + 5
                && i32::from(pixels[2][2]) > i32::from(pixels[2][0]) + 5,
            "material fog route {flags} did not sample/composite shared colour: {pixels:?}"
        );
    }
    // Actual material stages must address both tag domains and clear a
    // receiver after its selected local volume is removed.
    use crate::rendering::bevy_backend::{fog::AreaFogFrame, fog_objects::FogObjectParameters};
    let blank_fog = app.world_mut().resource_mut::<Assets<Image>>().add(fog_image([0.; 3]));
    let area = |color: Vec3| AreaFogFrame {
        meshes: Vec::new(), world_from_local: Mat4::IDENTITY,
        origin: Vec3::ZERO, bounds: [[-1.; 3], [1.; 3]],
        parameters: crate::rendering::fog::alpha::VolumeFog {
            volume_type: Vec3::X, distance: Vec4::new(0., 1., 4., 0.),
            color_start: color, color_end: color, density_factor: 1., falloff: 0.,
            scattering_direction: Vec3::ZERO, scattering_phase: 0.,
            scattering_color: Vec3::ZERO, scattering_intensity: 0.,
        },
    };
    for flags in [8, 56] {
        let mut pixels = Vec::new();
        for (tag, active) in [(3, false), (3, true), (0x80000001, true), (3, false)] {
            let mut records = vec![FogObjectParameters::default(); 10];
            if active {
                records[3] = FogObjectParameters::selected(&area(Vec3::X), [0., 100., 4.]);
                records[9] = FogObjectParameters::selected(&area(Vec3::Z), [0., 100., 4.]);
            }
            app.world_mut().resource_mut::<Assets<ShaderBuffer>>()
                .get_mut(&fog_bindings.objects).unwrap().set_data(records);
            let mut parameters = crate::rendering::bevy_backend::fog_material::FogMaterialParameters::disabled();
            parameters.control = Vec4::new(1., 0.01, 0., 8.);
            parameters.unproject = Vec4::new(1., 1., 0., 1.);
            app.world_mut().resource_mut::<Assets<ShaderBuffer>>()
                .get_mut(&fog_parameters).unwrap().set_data(parameters);
            {
                let mut materials = app.world_mut().resource_mut::<Assets<ProbeMaterial>>();
                let mut material = materials.get_mut(&receiver).unwrap();
                material.base.base_color = Color::BLACK;
                material.extension.surface.misc.w = flags;
                material.extension.fog_world = Some(blank_fog.clone());
            }
            app.world_mut().entity_mut(fog_receiver_entity).insert(MeshTag(tag));
            for _ in 0..24 { app.update(); }
            pixels.push(forward_center(&mut app, &target));
        }
        assert_eq!(pixels[0], pixels[3], "area fog removal route {flags}: {pixels:?}");
        assert!(i32::from(pixels[1][0]) > i32::from(pixels[1][2]) + 5
            && i32::from(pixels[2][2]) > i32::from(pixels[2][0]) + 5,
            "area fog tag domains route {flags}: {pixels:?}");
    }
}

#[test]
#[ignore = "requires installed original renderParm declarations and image catalog"]
fn installed_omitted_surface_maps_resolve_constant_images() {
    let catalog =
        archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let sampler = surface_sampler_image(Some(&catalog)).unwrap();
    let ImageSampler::Descriptor(sampler) = sampler.sampler else {
        panic!("sampler missing")
    };
    assert_eq!(sampler.anisotropy_clamp, 4);
    assert_eq!(sampler.lod_max_clamp, 13.0);
    let defaults = material_surface::Surface::parse(b"");
    let mut parameters = Vec::new();
    for (i, layer) in defaults.layers.iter().enumerate() {
        for (suffix, value) in [
            ("diffusemap", &layer.diffuse),
            ("bumpmap", &layer.bump),
            ("glossmap", &layer.gloss),
            ("metallicmap", &layer.metallic),
            ("maskmap", &layer.mask),
            ("specularmap", &layer.specular),
        ] {
            let key = if i == 0 && suffix != "maskmap" {
                suffix.into()
            } else {
                format!("layer{i}_{suffix}")
            };
            parameters.push((key, value.as_deref().unwrap()));
        }
    }
    parameters.extend([
        (
            "occlusionmap".into(),
            defaults.occlusion.as_deref().unwrap(),
        ),
        ("detailbumpmap".into(), defaults.detail_bump.as_str()),
        ("emissivemap".into(), defaults.emissive_map.as_str()),
        ("emissivemaskmap".into(), defaults.emissive_mask.as_str()),
        ("lightmap".into(), defaults.lightmap.as_str()),
    ]);
    for (key, source) in parameters {
        let bytes = catalog
            .read(&format!("generated/decls/renderparm/{key}.decl"))
            .unwrap()
            .unwrap();
        let text = entities::declaration_text(&bytes)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        let source_key = source
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        assert!(
            text.ends_with(&format!("{source_key}}}")),
            "texture default mismatch {key}: {source} vs {text}"
        );
        let stem = material::compiled_image_stem(&catalog, source).unwrap();
        let image = read_streamed_diffuse_image(&catalog, &stem, 0)
            .unwrap()
            .unwrap();
        assert!(!image.uploads.is_empty(), "default must upload: {key}");
        if source == "_default" {
            assert_eq!(image.level.format, texture::ImageFormat::Rgba8);
        }
    }
    let perlin = material::compiled_image_stem(&catalog, "ipr_noise_perlin(32, 32, 5, 3)").unwrap();
    assert!(read_streamed_diffuse_image(&catalog, &perlin, 0)
        .unwrap()
        .is_some());
    assert_eq!(
        material::compiled_image_stem(
            &catalog,
            "models\\rendering\\powers\\darkVisionFlowMap_n.png"
        )
        .unwrap(),
        "generated/image/models/rendering/powers/darkvisionflowmap_n.bimage7"
    );
    for name in [
        "specularmap",
        "layer1_specularmap",
        "layer2_specularmap",
        "layer1_glossmap",
        "layer2_glossmap",
        "layer1_metallicmap",
        "layer2_metallicmap",
    ] {
        let bytes = catalog
            .read(&format!("generated/decls/renderparm/{name}.decl"))
            .unwrap()
            .unwrap();
        let text = entities::declaration_text(&bytes)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        assert!(
            text.contains("ipr_constantcolor(0,0,0,0)"),
            "default changed: {name}: {text}"
        );
    }
    let bytes=catalog.read("generated/decls/material/models/environment/archi/karnaca/street_gate_01/gate_roof_01.material.decl").unwrap().unwrap();
    let surface = material_surface::Surface::parse(&bytes);
    for name in [&surface.layers[1].gloss, &surface.layers[1].metallic] {
        let name = name.as_deref().unwrap();
        let stem = material::compiled_image_stem(&catalog, name).unwrap();
        assert_eq!(
            stem,
            "generated/image/ipr_constantcolor__0__0__0__0_.bimage7"
        );
        let image = read_streamed_diffuse_image(&catalog, &stem, 0)
            .unwrap()
            .unwrap();
        assert!(
            !image.uploads.is_empty(),
            "constant must reach texture uploader"
        );
    }
}

#[test]
#[ignore = "requires installed Dust District fog lookup texture"]
fn installed_fog_lookup_preserves_half_float_uploads() {
    let catalog =
        archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let stem = material::compiled_image_stem(
        &catalog,
        "env/campaign/dust/dust_env/dust_ext01/atm/worlddistancefog/colorlut",
    )
    .unwrap();
    let bytes = catalog.read(&stem).unwrap().unwrap();
    let chain = texture::compressed_inline_chain(&bytes).unwrap();
    assert_eq!(chain.info.format, texture::ImageFormat::R16Float);
    let image = read_streamed_diffuse_image(&catalog, &stem, 0)
        .unwrap()
        .unwrap();
    assert_eq!(
        image.image.texture_descriptor.format,
        TextureFormat::R16Float
    );
    assert_eq!(streamed_block_bytes(TextureFormat::R16Float), Some(2));
    assert_eq!(
        image
            .uploads
            .iter()
            .flat_map(|mip| mip.payload.iter().copied())
            .collect::<Vec<_>>(),
        chain.payload
    );
    let values = chain
        .payload
        .chunks_exact(2)
        .map(|v| half::f16::from_le_bytes([v[0], v[1]]).to_f32())
        .collect::<Vec<_>>();
    assert!(values.len() > 1);
    assert!(values.iter().all(|v| v.is_finite()));
    assert!(values.windows(2).any(|v| v[0] != v[1]));
}

// Execute the actual shared WGSL functions, not a test-only reimplementation.
// Compare the complete one/two-sided packed paths against installed HLSL
// executed by D3D11, plus native PNW vertex-reference fetch/interpolation.
fn tiled_depth_shader_matches_reference(app: &mut App) {
    tiled_depth_shader_matches_case(app, None);
    for id in 0..64 {
        tiled_depth_shader_matches_case(app, Some(id));
    }
    eprintln!("GPU TLF depth: 4 mixed tiles and 64 original-HLSL cases passed");
    let mut raw_light = [0_u8; 0xe8];
    raw_light[0] = 1;
    raw_light[0x60..0x64].copy_from_slice(&3_u32.to_le_bytes());
    assert!(tiled_lighting::includes_local_light(0, 3, 0, raw_light[0]));
    raw_light[0x30..0x34].copy_from_slice(&(-2.0_f32).to_le_bytes());
    raw_light[0x80..0x84].copy_from_slice(&1.0_f32.to_le_bytes());
    raw_light[0x9c..0xa0].copy_from_slice(&1.0_f32.to_le_bytes());
    let mut lighting = [0_u32; 28];
    let mut used_shadows = 0;
    assert_eq!(
        render_light::prepare_tlf_lighting_packet(
            &raw_light,
            0,
            false,
            &mut used_shadows,
            &mut lighting
        ),
        (6, None)
    );
    assert_eq!(lighting[2], (-2.0_f32).to_bits());
    assert_eq!(lighting[24], u32::MAX);
    let mut point = [0_u32; 12];
    tiled_lighting::fill_light_cull_packet([0.0, 0.0, -2.0], &lighting, &mut point);
    for scattering in [false, true] {
        assert_eq!(
            tiled_cull_gpu_lists(app, &point, scattering),
            [vec![0], vec![], vec![], vec![]]
        );
    }
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/tlf_cull_cases.txt"
    ));
    let mut checked = 0;
    for case in fixture.split("\nCASE ").skip(1) {
        let header = case.lines().next().unwrap();
        let packets = case
            .lines()
            .find_map(|line| line.strip_prefix("LIGHTS id "))
            .unwrap();
        let mut lights = Vec::new();
        for (id, packet) in packets.split_whitespace().enumerate() {
            let (index, hex) = packet.split_once(':').unwrap();
            assert_eq!(index.parse::<usize>().unwrap(), id);
            assert_eq!(hex.len(), 96);
            for word in hex.as_bytes().chunks_exact(8) {
                lights.push(u32::from_str_radix(std::str::from_utf8(word).unwrap(), 16).unwrap());
            }
        }
        let mut expected: [Vec<u32>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for line in case.lines().filter(|line| line.starts_with("ACCEPT type=")) {
            let (kind, ids) = line
                .strip_prefix("ACCEPT type=")
                .unwrap()
                .split_once(" ids=")
                .unwrap();
            expected[kind.trim().parse::<usize>().unwrap()] = ids
                .trim()
                .split(',')
                .filter(|id| !id.is_empty())
                .map(|id| id.trim().parse().unwrap())
                .collect();
        }
        let scattering = header.contains("scatter=1");
        let actual = tiled_cull_gpu_lists(app, &lights, scattering);
        assert_eq!(actual, expected, "original HLSL {header}");
        if scattering {
            assert_eq!(
                tiled_cull_gpu_lists_with_prior(app, &lights, true, true),
                expected,
                "normal/scatter must share the pool without a counter reset between passes"
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 2);
    eprintln!("GPU TLF culling: 512 original light packets and packed node chains passed");
    tiled_light_iterator_matches_reference(app);
}

fn tiled_light_iterator_matches_reference(app: &mut App) {
    use bevy::render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    };
    let render = app.sub_app(RenderApp).world();
    let device = render.resource::<RenderDevice>();
    let queue = render.resource::<RenderQueue>();
    let mut pool = Vec::new();
    let mut heads = [tiled_lighting::END_OF_LIGHT_LIST; 16];
    // Two by two tiles, with four horizontal type planes. Distinct chains
    // expose tile/type addressing and preserve deliberately unsorted IDs.
    for (head_index, head) in heads.iter_mut().enumerate() {
        for i in 0..40 {
            let node =
                tiled_lighting::PackedLightNode::new(*head, (i * 37 + head_index as u32) & 511);
            *head = pool.len() as u32;
            pool.push(node);
        }
    }
    let mut source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/shaders/dh2_tlf_list.wgsl"
    ))
    .lines()
    .filter(|line| !line.starts_with('#'))
    .collect::<Vec<_>>()
    .join("\n");
    source.push_str(
        r#"
@group(0) @binding(0) var<storage> pool: array<u32>;
@group(0) @binding(1) var<storage> heads: array<u32>;
@group(0) @binding(2) var<storage, read_write> result: array<u32>;
@compute @workgroup_size(64)
fn exercise_lists(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= 64u { return; }
    let tile = gid.x & 3u;
    let kind = (gid.x >> 2u) & 3u;
    let maximum = array<u32, 4>(0u, 1u, 32u, 64u)[gid.x >> 4u];
    let pixel = vec2<u32>((tile & 1u) * 16u + 11u, (tile >> 1u) * 16u + 7u);
    let coordinate = light_head_coordinate(pixel, kind, 4u, 2u);
    var list = begin_light_list(heads[coordinate.x + coordinate.y * 8u]);
    let base = gid.x * 65u;
    while has_light(list, maximum) {
        let packed = pool[list.head];
        let index = advance_light(&list, packed);
        result[base + list.count] = index;
    }
    result[base] = list.count;
}
"#,
    );
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("PC LightContext consumer"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: Some("PC LightContext consumer"),
            layout: None,
            module: &module,
            entry_point: Some("exercise_lists"),
            compilation_options: default(),
            cache: None,
        });
    let bytes = |words: &[u32]| {
        words
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect::<Vec<_>>()
    };
    let pool_buffer = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &bytes(&pool.iter().map(|node| node.0).collect::<Vec<_>>()),
        usage: BufferUsages::STORAGE,
    });
    let heads_buffer = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &bytes(&heads),
        usage: BufferUsages::STORAGE,
    });
    let size = 64 * 65 * 4;
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let group = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: pool_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: heads_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: output.as_entire_binding(),
                },
            ],
        });
    let mut encoder = device.create_command_encoder(&default());
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
    device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    let mapped = readback.slice(..).get_mapped_range();
    let words = mapped
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
        .collect::<Vec<_>>();
    let grid = tiled_lighting::TileGrid::new(32, 32, 16).unwrap();
    for case in 0..64usize {
        let tile = case & 3;
        let coordinate = grid.light_head_coordinate(
            [((tile & 1) * 16 + 11) as u32, ((tile >> 1) * 16 + 7) as u32],
            ((case >> 2) & 3) as u32,
        );
        let head = heads[(coordinate[0] + coordinate[1] * 8) as usize];
        let mut expected = Vec::new();
        tiled_lighting::visit_light_list(&pool, head, [0, 1, 32, 64][case >> 4], |id| {
            expected.push(id)
        })
        .unwrap();
        assert_eq!(
            words[case * 65] as usize,
            expected.len(),
            "PC LightContext case {case}"
        );
        assert_eq!(
            &words[case * 65 + 1..case * 65 + 1 + expected.len()],
            expected.as_slice(),
            "PC LightContext order {case}"
        );
    }
    drop(mapped);
    readback.unmap();
    eprintln!("GPU TLF PC consumer: 64 tile/type/limit/order cases passed");
}

fn tiled_cull_gpu_lists(app: &mut App, lights: &[u32], scattering: bool) -> [Vec<u32>; 4] {
    tiled_cull_gpu_lists_with_prior(app, lights, scattering, false)
}

fn tiled_cull_gpu_lists_with_prior(
    app: &mut App,
    lights: &[u32],
    scattering: bool,
    prior_normal: bool,
) -> [Vec<u32>; 4] {
    assert!(!prior_normal || scattering);
    use bevy::render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    };
    use std::sync::mpsc;
    assert_eq!(lights.len() % 12, 0);
    let count = lights.len() / 12;
    assert!(count > 0 && count <= tiled_lighting::LIGHT_BUFFER_CAPACITY);
    let render = app.sub_app_mut(RenderApp).world_mut();
    render.init_resource::<tiled_lighting_gpu::TlfPipelines>();
    let device = render.resource::<RenderDevice>();
    let queue = render.resource::<RenderQueue>();
    let pipeline = &render.resource::<tiled_lighting_gpu::TlfPipelines>().cull;
    let mut params = vec![
        0.0f32.to_bits(),
        0.0f32.to_bits(),
        1.0f32.to_bits(),
        1.0f32.to_bits(),
    ];
    params.extend([1.0f32.to_bits(); 4]);
    for _ in 0..2 {
        for row in 0..4 {
            for column in 0..4 {
                params.push(if row == column { 1.0f32.to_bits() } else { 0 });
            }
        }
    }
    params.extend([count.div_ceil(64) as u32 * 64, u32::from(scattering), 0, 0]);
    assert_eq!(params.len(), 44);
    let bytes = |values: &[u32]| {
        values
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<u8>>()
    };
    let uniform = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &bytes(&params),
        usage: BufferUsages::UNIFORM,
    });
    let buffers = tiled_lighting_gpu::TlfBuffers::new(device);
    // The original-HLSL fixture supplies cull records. Reconstruct its
    // corresponding lighting fields to exercise the shared producer/upload
    // path too; the 1-light case additionally dispatches all 63 padding lanes.
    let lighting = lights
        .chunks_exact(12)
        .map(|cull| {
            let mut record = [0_u32; 28];
            record[..3].copy_from_slice(&cull[..3]);
            for (src, dst) in (3..11).zip([7, 8, 9, 10, 13, 16, 17, 18]) {
                record[dst] = cull[src];
            }
            record[25] = cull[11];
            record
        })
        .collect::<Vec<_>>();
    let identity = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    assert_eq!(
        buffers
            .upload_lighting_packets(queue, &identity, &lighting)
            .unwrap(),
        count.div_ceil(64) * 64
    );
    let packets = buffers
        .resource(render_resource::resource_name_key("lightToPutInTiles"))
        .unwrap();
    let counter = buffers
        .resource(render_resource::resource_name_key(
            "lightNodePoolNextFreeEntry",
        ))
        .unwrap();
    let pool = buffers
        .resource(render_resource::resource_name_key("lightNodePool"))
        .unwrap();
    assert!(buffers
        .resource(render_resource::resource_name_key("missingResource"))
        .is_none());
    let heads = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &bytes(&[tiled_lighting::END_OF_LIGHT_LIST; 4]),
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
    });
    // Native fixed allocations persist across all frames below.
    let depth = device.create_texture(&TextureDescriptor {
        label: None,
        size: Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba32Float,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        depth.as_image_copy(),
        &bytes(&[-0.5f32, -10.0, -4.0, f32::from_bits(u32::MAX)].map(f32::to_bits)),
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(16),
            rows_per_image: Some(1),
        },
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let view = depth.create_view(&default());
    let layout = pipeline.get_bind_group_layout(0);
    let group = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: packets.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: heads.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: counter.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: pool.as_entire_binding(),
                },
            ],
        });
    let prior_heads = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &bytes(&[tiled_lighting::END_OF_LIGHT_LIST; 4]),
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
    });
    params[41] = 0;
    let prior_uniform = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &bytes(&params),
        usage: BufferUsages::UNIFORM,
    });
    let prior_group = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: prior_uniform.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: packets.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: prior_heads.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: counter.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: pool.as_entire_binding(),
                },
            ],
        });
    let pool_words = count * if prior_normal { 2 } else { 1 };
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 36 + pool_words as u64 * 4,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut previous = None;
    for frame in 0..3 {
        // Each original-HLSL fixture starts with empty heads. They are
        // caller-owned inputs; cull_lights only exchanges/appends them.
        queue.write_buffer(&heads, 0, &bytes(&[tiled_lighting::END_OF_LIGHT_LIST; 4]));
        queue.write_buffer(
            &prior_heads,
            0,
            &bytes(&[tiled_lighting::END_OF_LIGHT_LIST; 4]),
        );
        let mut encoder = device.create_command_encoder(&default());
        // Native351780 clears once BEFORE both passes, never between them.
        buffers.begin_frame(&mut encoder);
        if prior_normal {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &prior_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&heads, 0, &readback, 0, 16);
        encoder.copy_buffer_to_buffer(&prior_heads, 0, &readback, 16, 16);
        encoder.copy_buffer_to_buffer(counter, 0, &readback, 32, 4);
        encoder.copy_buffer_to_buffer(pool, 0, &readback, 36, pool_words as u64 * 4);
        queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        readback.slice(..).map_async(MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let bytes = readback.slice(..).get_mapped_range();
        let words: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        let used = words[8] as usize;
        assert!(used <= pool_words);
        let mut seen = HashSet::new();
        let mut result = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for chain in 0..if prior_normal { 8 } else { 4 } {
            let kind = chain % 4;
            let mut cursor = words[chain];
            while cursor != tiled_lighting::END_OF_LIGHT_LIST {
                assert!((cursor as usize) < used);
                assert!(
                    seen.insert(cursor),
                    "duplicate/cyclic TLF node or counter reset between passes"
                );
                let packed = tiled_lighting::PackedLightNode(words[9 + cursor as usize]);
                let id = packed.light_index();
                assert!((id as usize) < count);
                assert_eq!((lights[id as usize * 12 + 11] >> 4) & 7, kind as u32);
                if chain < 4 {
                    result[kind].push(id);
                }
                cursor = packed.next();
            }
            if chain < 4 {
                result[kind].sort_unstable();
            }
        }
        if prior_normal {
            assert!(
                seen.len() > result.iter().map(Vec::len).sum::<usize>(),
                "normal prefix did not exercise shared allocation"
            );
        }
        assert_eq!(seen.len(), used);
        drop(bytes);
        readback.unmap();
        if let Some(before) = &previous {
            assert_eq!(
                &result, before,
                "TLF buffers accumulated stale links on frame {frame}"
            );
        }
        previous = Some(result);
    }
    previous.unwrap()
}

fn depth_copy_shader_matches_fixture(
    device: &bevy::render::renderer::RenderDevice,
    queue: &bevy::render::renderer::RenderQueue,
    pipelines: &tiled_lighting_gpu::TlfPipelines,
    source: &bevy::render::render_resource::Texture,
    expected: &[f32],
    entry: &str,
) {
    use bevy::render::render_resource::*;
    let array_source = entry == "fragment";
    let output_size = if entry == "downscale" { 16 } else { 32 };
    let module = &pipelines.depth_copy;
    // Caller-owned fixture layout; not the original 32-byte vertex ABI.
    let vertex_layout = [RawVertexBufferLayout {
        array_stride: 20,
        step_mode: VertexStepMode::Vertex,
        attributes: &[
            VertexAttribute {
                format: VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            VertexAttribute {
                format: VertexFormat::Float32x2,
                offset: 12,
                shader_location: 1,
            },
        ],
    }];
    let pipeline = if array_source {
        device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("depthcopy fixture"),
            layout: None,
            vertex: RawVertexState {
                module,
                entry_point: Some("vertex"),
                compilation_options: default(),
                buffers: &vertex_layout,
            },
            fragment: Some(RawFragmentState {
                module,
                entry_point: Some(entry),
                compilation_options: default(),
                targets: &[],
            }),
            primitive: default(),
            multisample: default(),
            multiview_mask: None,
            cache: None,
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::Always),
                stencil: default(),
                bias: default(),
            }),
        })
    } else {
        pipelines.depth_copy_pipeline(
            device,
            TextureFormat::Depth32Float,
            entry == "downscale",
            &vertex_layout,
        )
    };
    let destination = device.create_texture(&TextureDescriptor {
        label: Some("copied depth fixture"),
        size: Extent3d {
            width: output_size,
            height: output_size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Depth32Float,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let source_view = source.create_view(&TextureViewDescriptor {
        dimension: Some(if array_source {
            TextureViewDimension::D2Array
        } else {
            TextureViewDimension::D2
        }),
        ..default()
    });
    let destination_view = destination.create_view(&default());
    let sampler = device.create_sampler(&SamplerDescriptor {
        min_filter: FilterMode::Nearest,
        mag_filter: FilterMode::Nearest,
        ..default()
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 32 * 256,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    for scale_bias in [
        [1_f32, 1., 0., 0.],
        [0.5, 0.5, 0.25, 0.25],
        [-1., 1., 1., 0.],
    ] {
        let mut vertex_data = [
            -1_f32, -1., 0., 0., 1., 3., -1., 0., 2., 1., -1., 3., 0., 0., -1.,
        ];
        if !array_source {
            for v in vertex_data.chunks_mut(5) {
                v[3] = v[3] * scale_bias[0] + scale_bias[2];
                v[4] = v[4] * scale_bias[1] + scale_bias[3];
            }
        }
        let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &vertex_data
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
            usage: BufferUsages::VERTEX,
        });
        let parameters = array_source.then(|| {
            let mut words = scale_bias.map(f32::to_bits).to_vec();
            words.extend([0; 4]);
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: None,
                contents: &words
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
                usage: BufferUsages::UNIFORM,
            })
        });
        let mut bindings = vec![
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&source_view),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Sampler(&sampler),
            },
        ];
        if let Some(parameters) = &parameters {
            bindings.push(BindGroupEntry {
                binding: 2,
                resource: parameters.as_entire_binding(),
            });
        }
        let bind = device
            .wgpu_device()
            .create_bind_group(&BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &bindings,
            });
        let mut encoder = device.create_command_encoder(&default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &[],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &destination_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.333),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.set_vertex_buffer(0, *vertices.slice(..));
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &destination,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::DepthOnly,
            },
            TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(32),
                },
            },
            Extent3d {
                width: output_size,
                height: output_size,
                depth_or_array_layers: 1,
            },
        );
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let bytes = readback.slice(..).get_mapped_range();
        for y in 0..output_size as usize {
            for x in 0..output_size as usize {
                let footprint = if entry == "downscale" { 0.5 } else { 0.0 };
                let ratio = 32.0 / output_size as f32;
                let sx = (((x as f32 + 0.5) * ratio * scale_bias[0] + 32. * scale_bias[2])
                    - footprint)
                    .floor() as i32;
                let sy = (((y as f32 + 0.5) * ratio * scale_bias[1] + 32. * scale_bias[3])
                    - footprint)
                    .floor() as i32;
                let sample = |dx: i32, dy: i32| {
                    expected[(sy + dy).clamp(0, 31) as usize * 32 + (sx + dx).clamp(0, 31) as usize]
                };
                let predicted = if entry == "downscale" {
                    sample(0, 0)
                        .min(sample(1, 0))
                        .min(sample(0, 1).min(sample(1, 1)))
                } else {
                    sample(0, 0)
                };
                let at = y * 256 + x * 4;
                assert_eq!(
                    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
                    predicted.to_bits(),
                    "depthcopy {entry} {scale_bias:?} pixel{x},{y}"
                );
            }
        }
        drop(bytes);
        readback.unmap();
    }
}

fn tiled_depth_shader_matches_case(app: &mut App, case: Option<usize>) {
    use bevy::render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    };
    let world = app.sub_app_mut(bevy::render::RenderApp).world_mut();
    world.init_resource::<tiled_lighting_gpu::TlfPipelines>();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let pipeline = &world.resource::<tiled_lighting_gpu::TlfPipelines>().depth;
    let make_depth = || {
        device.create_texture(&TextureDescriptor {
            label: None,
            size: Extent3d {
                width: 32,
                height: 32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    };
    let opaque = make_depth();
    let alpha = make_depth();
    let output = device.create_texture(&TextureDescriptor {
        label: None,
        size: Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba32Float,
        usage: TextureUsages::STORAGE_BINDING | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let reference = case.map(tiled_lighting::depth_case);
    let unproject = reference.as_ref().map_or([-0.1, 0.00001], |v| v.1);
    let mut depths = [Vec::<f32>::new(), Vec::<f32>::new()];
    for y in 0..32 {
        for x in 0..32 {
            let tile = x / 16 + 2 * (y / 16);
            if let Some((input, _, _)) = &reference {
                let pixel = (y % 16) * 16 + x % 16;
                depths[0].push(input[pixel]);
                depths[1].push(input[pixel + 256]);
            } else {
                depths[0].push(if tile == 3 {
                    0.0
                } else {
                    ((x + y) % 4 + 1) as f32 * 0.25
                });
                depths[1].push(if tile == 3 || (x + y) % 3 == 0 {
                    0.0
                } else {
                    0.125 + ((x * 3 + y) % 5) as f32 * 0.125
                });
            }
        }
    }
    // Depth32Float cannot be Queue::write_texture'd. Populate actual
    // depth attachments through the rasterizer, as the engine does.
    let fill_module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("synthetic depth fixture"),
        source: ShaderSource::Wgsl(
            r#"
                @group(0) @binding(0) var<storage,read> depths:array<f32>;
                @vertex fn vertex(@builtin(vertex_index) id:u32)->@builtin(position) vec4<f32> {
                    let p=array<vec2<f32>,3>(vec2<f32>(-1,-1),vec2<f32>(3,-1),vec2<f32>(-1,3));
                    return vec4<f32>(p[id],0,1);
                }
                @fragment fn fragment(@builtin(position) p:vec4<f32>)->@builtin(frag_depth) f32 {
                    return depths[u32(p.y)*32u+u32(p.x)];
                }
            "#
            .into(),
        ),
    });
    let fill_pipeline = device
        .wgpu_device()
        .create_render_pipeline(&RawRenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: RawVertexState {
                module: &fill_module,
                entry_point: Some("vertex"),
                compilation_options: default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &fill_module,
                entry_point: Some("fragment"),
                compilation_options: default(),
                targets: &[],
            }),
            primitive: default(),
            multisample: default(),
            multiview_mask: None,
            cache: None,
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::Always),
                stencil: default(),
                bias: default(),
            }),
        });
    let mut fill_encoder = device.create_command_encoder(&default());
    for (texture, values) in [(&opaque, &depths[0]), (&alpha, &depths[1])] {
        let bytes = values
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        let data = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &bytes,
            usage: BufferUsages::STORAGE,
        });
        let bind = device
            .wgpu_device()
            .create_bind_group(&BindGroupDescriptor {
                label: None,
                layout: &fill_pipeline.get_bind_group_layout(0),
                entries: &[BindGroupEntry {
                    binding: 0,
                    resource: data.as_entire_binding(),
                }],
            });
        let view = texture.create_view(&default());
        let mut pass = fill_encoder.begin_render_pass(&RenderPassDescriptor {
            label: None,
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &view,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(0.0),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&fill_pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
    queue.submit([fill_encoder.finish()]);
    if case.is_none() {
        let pipelines = world.resource::<tiled_lighting_gpu::TlfPipelines>();
        for entry in ["fragment", "identity", "downscale"] {
            depth_copy_shader_matches_fixture(device, queue, pipelines, &opaque, &depths[0], entry);
        }
    }
    let params = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &[
            unproject[0].to_bits(),
            unproject[1].to_bits(),
            0,
            0,
            16,
            0,
            0,
            0,
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>(),
        usage: BufferUsages::UNIFORM,
    });
    let sampler = device.create_sampler(&SamplerDescriptor {
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        ..default()
    });
    let opaque_view = opaque.create_view(&default());
    let alpha_view = alpha.create_view(&default());
    let output_view = output.create_view(&default());
    let bind = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&opaque_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&alpha_view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&sampler),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: params.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: BindingResource::TextureView(&output_view),
                },
            ],
        });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 512,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&default());
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(2, 2, 1);
    }
    encoder.copy_texture_to_buffer(
        TexelCopyTextureInfo {
            texture: &output,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(2),
            },
        },
        Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: 1,
        },
    );
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback.slice(..).map_async(MapMode::Read, move |result| {
        tx.send(result).unwrap();
    });
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    for ty in 0..2 {
        for tx in 0..2 {
            let mut tile = Vec::new();
            for source in &depths {
                for y in ty * 16..ty * 16 + 16 {
                    for x in tx * 16..tx * 16 + 16 {
                        tile.push(source[y * 32 + x]);
                    }
                }
            }
            let expected = reference.as_ref().map(|v| v.2).unwrap_or_else(|| {
                tiled_lighting::reduce_depth_tile(&tile, unproject).map(f32::to_bits)
            });
            for lane in 0..4 {
                let at = ty * 256 + tx * 16 + lane * 4;
                let actual = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
                if lane == 3 {
                    assert_eq!(
                        actual, expected[lane],
                        "case{case:?} tile{tx},{ty} depth mask"
                    );
                } else {
                    assert!(
                        actual.abs_diff(expected[lane]) <= 1,
                        "case{case:?} tile{tx},{ty} endpoint{lane}: {actual:08x} vs {:08x}",
                        expected[lane]
                    );
                }
            }
        }
    }
    drop(bytes);
    readback.unmap();
}

fn probe_shader_matches(app: &mut App) {
    use bevy::render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    };
    let world = app.sub_app_mut(bevy::render::RenderApp).world_mut();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let mut source = crate::rendering::bevy_backend::probe_weights::indirect_source(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/shaders/dh2_indirect.wgsl"
    )))
    .replace("#define_import_path dh2_indirect", "")
    .replace("#{MATERIAL_BIND_GROUP}", "0");
    source.push_str(r#"
@group(0) @binding(120) var<storage, read_write> result: array<vec4<f32>>;
fn view_vector(v: vec3<f32>) -> vec3<f32> { return vec3(v.x,v.z,-v.y); }
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let id = invocation.x;
    let sh = vertex_sh(id, 0u, vec3(0.0));
    let d = vec3<f32>(vec3<i32>(i32(id%5u)-2,i32((id/5u)%5u)-2,i32((id/25u)%5u)-2))*0.25*select(1.0,3.0,id>=32u);
    var t = vec3(1.0,0.0,0.0); var b = vec3(0.0,1.0,0.0); var n = vec3(0.0,0.0,1.0);
    if (id & 1u) != 0u { t = vec3(0.0,1.0,0.0); b = vec3(-1.0,0.0,0.0); }
    if (id & 2u) != 0u { b = -b; n = -n; }
    let ao = f32(id%5u)*0.25;
    let one = pack_vertex_irradiance(sh,view_vector(t),view_vector(b),view_vector(n),false);
    let two = pack_vertex_irradiance(sh,view_vector(t),view_vector(b),view_vector(n),true);
    let at = id*9u;
    result[at] = vec4(irradiance(d,sh),0.0);
    result[at+1u] = vec4(one.luminance,0.0,0.0,0.0);
    result[at+2u] = one.chroma_a; result[at+3u] = one.chroma_b;
    result[at+4u] = vec4(unpack_diffuse(one,view_vector(d),view_vector(t),view_vector(b),view_vector(n),ao,false),0.0);
    result[at+5u] = vec4(two.luminance,0.0,0.0,0.0);
    result[at+6u] = two.chroma_a; result[at+7u] = two.chroma_b;
    result[at+8u] = vec4(unpack_diffuse(two,view_vector(d),view_vector(t),view_vector(b),view_vector(n),ao,true),0.0);
    let albedo=vec3(f32(id%17u),f32((id*3u)%17u),f32((id*7u)%17u))/16.0;
    result[64u*9u+4u*9u+id]=vec4(evaluate_diffuse_response(albedo,f32(id%9u)/8.0,
        f32((id/3u)%5u)/4.0,vec3(.3,.5,.7),(id&1u)==0u,vec3(.04,.02,.08),vec3(.01,.02,.03)),0.0);
    if id < 4u {
        var p = vec3(0.25);
        if id == 1u { p = vec3(1.0,0.0,0.0); }
        if id == 2u { p = vec3(-0.25,1.25,-0.5); }
        if id == 3u { p = vec3(0.0); }
        let lighting = vertex_lighting(0x80000000u,0u,view_vector(p));
        let interpolated = lighting.sh;
        for (var i = 0u; i < 9u; i++) { result[64u*9u+id*9u+i] = vec4(interpolated[i],0.0); }
        result[64u*9u+4u*9u+64u+id] = vec4(f32(lighting.cube_index),lighting.reflection_gain,0.0,0.0);
    }
}
"#);
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("original PNW vertex SH numerical parity"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: default(),
            cache: None,
        });
    let mut sh = Vec::new();
    for id in 0..64 {
        for i in 0..9 {
            for c in 0..4 {
                sh.push(if c == 3 || id == 0 {
                    0.0f32
                } else {
                    (((id * 17 + i * 5 + c * 3) % 19) as i32 - 9) as f32 / 16.0
                });
            }
        }
    }
    let sh_bytes = sh.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
    let coefficients = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &sh_bytes,
        usage: BufferUsages::STORAGE,
    });
    // One tetrahedron with four primary IDs and exact unit-axis positions.
    let mut words = vec![
        10u32, 11, 12, 18, 1, 1, 1, 4, 34, 4, 0, 0, 0, 1, 2, 3, 0x00010000, 0x00030002,
    ];
    for i in 0..4 {
        for axis in 0..3 {
            words.push((if i == axis { 1.0f32 } else { 0.0f32 }).to_bits());
        }
        words.push(255u32 << (8 * i));
    }
    for (index, gain) in [(2u32, 0.25f32), (1, 0.5), (3, 0.75), (0, 1.0)] {
        words.extend([index, gain.to_bits()]);
    }
    let word_bytes = words
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &word_bytes,
        usage: BufferUsages::STORAGE,
    });
    let size = (64 * 9 + 4 * 9 + 64 + 4) * 16;
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let bind = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 100,
                    resource: coefficients.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 119,
                    resource: vertices.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 120,
                    resource: output.as_entire_binding(),
                },
            ],
        });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(64, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback.slice(..).map_async(MapMode::Read, move |result| {
        tx.send(result).unwrap();
    });
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    let actual = bytes
        .chunks_exact(4)
        .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
        .collect::<Vec<_>>();
    drop(bytes);
    readback.unmap();
    for (id, expected) in [[2.0, 0.625], [2.0, 0.25], [1.0, 0.6875], [0.0, 1.0]].into_iter().enumerate() {
        let at = (64 * 9 + 4 * 9 + 64 + id) * 4;
        assert_eq!(actual[at], expected[0], "world reflection probe ID case{id}");
        assert!((actual[at + 1] - expected[1]).abs() < 1e-6,
            "world reflection gain case{id}: {} != {}", actual[at + 1], expected[1]);
    }
    for (id, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/indirect_lighting_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let expected = line
            .split_whitespace()
            .map(|v| f32::from_bits(u32::from_str_radix(v, 16).unwrap()))
            .collect::<Vec<_>>();
        for (row, game) in [2, 9, 10, 11, 16, 17, 18, 19, 20].into_iter().enumerate() {
            for c in 0..4 {
                let a = actual[(id * 9 + row) * 4 + c];
                let b = expected[game * 4 + c];
                assert!(
                    (a - b).abs() <= 3e-5 * b.abs().max(1.0),
                    "WGSL/native HLSL case{id} row{row} channel{c}: {a} != {b}"
                );
            }
        }
    }
    for (id, weights) in [
        [0.25f32; 4],
        [1.0, 0.0, 0.0, 0.0],
        [-0.25, 1.25, -0.5, 0.5],
        [0.0, 0.0, 0.0, 1.0],
    ]
    .into_iter()
    .enumerate()
    {
        for i in 0..9 {
            for c in 0..3 {
                let expected: f32 = (0..4).map(|p| weights[p] * sh[(p * 9 + i) * 4 + c]).sum();
                let a = actual[(64 * 9 + id * 9 + i) * 4 + c];
                assert!(
                    (a - expected).abs() < 1e-5,
                    "PNW GPU case{id} coefficient{i} channel{c}: {a} != {expected}"
                );
            }
        }
    }
    for (id, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/diffuse_response_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        for (c, word) in line.split_whitespace().enumerate() {
            let expected = f32::from_bits(u32::from_str_radix(word, 16).unwrap());
            let result = actual[(64 * 9 + 4 * 9 + id) * 4 + c];
            assert!(
                (result - expected).abs() < 0.000003,
                "diffuse response case{id} channel{c}: {result} != {expected}"
            );
        }
    }
    eprintln!("GPU native diffuse radiance response: 64 installed-HLSL cases passed");
    eprintln!(
        "GPU native SH parity:64 original HLSL packed one/two-sided cases +4 PNW reference/interpolation cases passed"
    );
}

#[test]
fn authored_mesh_retains_uv_color_and_independent_sheared_basis() {
    use bevy::mesh::VertexAttributeValues;
    let packed = [
        0x00, 0x38, 0x00, 0xbc, 127, 0, 0, 42, 0, 127, 0, 129, 0, 128, 255, 64,
    ];
    let linear = [
        1.0, 2.0, 3.0, 1000.0, 0.0, 1.0, 4.0, 2000.0, 0.0, 0.0, 1.0, 3000.0,
    ];
    let mesh = authored_mesh(&[Vec3::ZERO], &[[0.1, 0.2]], &[], &[packed], |v| {
        transformed_direction(&linear, v)
    });
    let f3 = |attribute| match mesh.attribute(attribute).unwrap() {
        VertexAttributeValues::Float32x3(v) => v[0],
        _ => panic!("wrong vertex type"),
    };
    assert_eq!(f3(Mesh::ATTRIBUTE_NORMAL), [1.0, 0.0, 0.0]);
    assert_eq!(f3(DH2_BITANGENT), [-3.0, -1.0, 4.0]);
    let VertexAttributeValues::Float32(corner) = mesh.attribute(DH2_NORMAL_W).unwrap() else {panic!()};
    assert_eq!(corner[0], 42.0 / 127.0);
    let VertexAttributeValues::Float32x4(t) = mesh.attribute(Mesh::ATTRIBUTE_TANGENT).unwrap()
    else {
        panic!()
    };
    assert_eq!(t[0], [2.0, 0.0, -1.0, -1.0]);
    let VertexAttributeValues::Float32x4(color) = mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap()
    else {
        panic!()
    };
    assert_eq!(color[0], [0.0, 128.0 / 255.0, 1.0, 64.0 / 255.0]);
    let VertexAttributeValues::Float32x2(uv1) = mesh.attribute(Mesh::ATTRIBUTE_UV_1).unwrap()
    else {
        panic!()
    };
    assert_eq!(uv1[0], [0.5, -1.0]);
    // MD6 declarations omit color, not an inferred byte channel.
    let md6_packed: [u8; 12] = packed[..12].try_into().unwrap();
    let md6 = authored_mesh(&[Vec3::ZERO], &[[0.0; 2]], &[], &[md6_packed], |v| v);
    assert!(md6.attribute(Mesh::ATTRIBUTE_COLOR).is_none());
}

#[test]
fn vegetation_channels_keep_local_inputs_on_baked_meshes() {
    use bevy::mesh::VertexAttributeValues;
    let packed = [
        0x00, 0x38, 0x00, 0xbc, 128, 64, 127, 0, 0, 127, 0, 127, 128, 64, 255, 0,
    ];
    let local = [[2.0, 3.0, 4.0]];
    let world = [Vec3::new(100.0, 200.0, 300.0)];
    let mut mesh = authored_mesh(&world, &[[0.1, 0.2]], &[], &[packed], |v| v * 2.0);
    retain_vegetation_channels(&mut mesh, &local, &[packed]).unwrap();
    let f3 = |attribute| match mesh.attribute(attribute).unwrap() {
        VertexAttributeValues::Float32x3(values) => values[0],
        _ => panic!("wrong vertex type"),
    };
    assert_eq!(f3(Mesh::ATTRIBUTE_POSITION), world[0].to_array());
    assert_eq!(f3(Mesh::ATTRIBUTE_NORMAL), [-2.0, 128.0 / 127.0, 2.0]);
    assert_eq!(f3(shadow_pass::VEGETATION_POSITION), local[0]);
    assert_eq!(
        f3(shadow_pass::VEGETATION_COLOR),
        [128.0 / 255.0, 64.0 / 255.0, 1.0]
    );
    let VertexAttributeValues::Float32x2(normal) =
        mesh.attribute(shadow_pass::VEGETATION_NORMAL).unwrap()
    else {
        panic!()
    };
    assert_eq!(normal[0], [-1.0, 64.0 / 127.0]);
    let VertexAttributeValues::Float32x2(uv) = mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap() else {
        panic!()
    };
    assert_eq!(uv[0], [0.1, 0.2]);

    let short: [u8; 12] = packed[..12].try_into().unwrap();
    let mut no_color = authored_mesh(&world, &[[0.0; 2]], &[], &[short], |v| v);
    assert!(retain_vegetation_channels(&mut no_color, &local, &[short]).is_err());
    assert!(no_color
        .attribute(shadow_pass::VEGETATION_POSITION)
        .is_none());
    assert!(retain_vegetation_channels(&mut no_color, &[], &[packed]).is_err());
}

#[test]
fn vegetation_mesh_channels_are_selective_and_shared_once() {
    use bevy::mesh::VertexAttributeValues;
    let packed = [[0u8; 16]];
    let local = [[2.0, 3.0, 4.0]];
    let mut meshes = Assets::<Mesh>::default();
    let handle = meshes.add(source_mesh(&local, &[[0.0; 2]], &[], &packed, false));
    let mut inputs = VegetationMeshInputs::default();
    inputs.prepare(false, &handle, &mut meshes, &local, &packed);
    assert!(inputs.0.is_empty());
    assert!(meshes
        .get(&handle)
        .unwrap()
        .attribute(shadow_pass::VEGETATION_POSITION)
        .is_none());
    inputs.prepare(true, &handle, &mut meshes, &local, &packed);
    assert!(inputs.0[&handle.id()].is_ok());
    let raw_pointer = |mesh: &Mesh| match mesh.attribute(shadow_pass::VEGETATION_POSITION).unwrap()
    {
        VertexAttributeValues::Float32x3(values) => values.as_ptr(),
        _ => panic!("wrong vertex type"),
    };
    let first = raw_pointer(meshes.get(&handle).unwrap());
    inputs.prepare(true, &handle, &mut meshes, &local, &packed);
    assert_eq!(inputs.0.len(), 1);
    assert_eq!(raw_pointer(meshes.get(&handle).unwrap()), first);
}

#[test]
fn displayed_frame_rate_uses_real_time_even_when_gameplay_time_is_paused() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    let mut real = Time::<bevy::time::Real>::default();
    real.advance_by(std::time::Duration::from_millis(1500));
    world.insert_resource(real);
    world.insert_resource(Time::<()>::default());
    world.insert_resource(SceneRenderQueue::default());
    let window = world.spawn(Window::default()).id();
    let overlay = world
        .spawn((Text::new("FPS: measuring"), FrameRateOverlay))
        .id();
    world.run_system_once(show_frame_rate).unwrap();
    assert_eq!(
        world.get::<Window>(window).unwrap().title,
        "Dishonored 2 Rust | 1 FPS | 0 loaded"
    );
    let text = &world.get::<Text>(overlay).unwrap().0;
    assert!(text.starts_with("1 FPS | 1500.0 ms\nRAM "));
    #[cfg(target_os = "windows")]
    assert!(text.contains("GiB | Private "));
}

#[test]
fn local_light_area_threshold_is_32_physical_pixels() {
    let viewport = UVec2::new(800, 600);
    let threshold_range = (LOCAL_LIGHT_MIN_AREA_PIXELS / (viewport.x * viewport.y) as f32).sqrt();
    let area =
        projected_local_light_area_pixels(Mat4::IDENTITY, Vec3::ZERO, threshold_range, viewport)
            .unwrap();
    assert!((area - LOCAL_LIGHT_MIN_AREA_PIXELS).abs() < 0.001);

    let below =
        projected_local_light_area_pixels(Mat4::IDENTITY, Vec3::ZERO, 0.005, viewport).unwrap();
    assert!(below < LOCAL_LIGHT_MIN_AREA_PIXELS);

    let crossing_camera_plane =
        Mat4::from_cols(Vec4::X, Vec4::Y, Vec4::new(0.0, 0.0, 1.0, 1.0), Vec4::ZERO);
    assert!(
        projected_local_light_area_pixels(crossing_camera_plane, Vec3::ZERO, 0.1, viewport,)
            .is_none()
    );

    let near_camera_plane =
        Mat4::from_cols(Vec4::X, Vec4::Y, Vec4::Z, Vec4::new(0.0, 0.0, 0.0, 1.0e-8));
    let area =
        projected_local_light_area_pixels(near_camera_plane, Vec3::ZERO, 1.0e-8, viewport).unwrap();
    assert!((area - (viewport.x * viewport.y) as f32).abs() < 1.0);
}

#[test]
fn light_proximity_bounds_match_max_visible_range_branches() {
    assert_eq!(
        light_proximity_bounds(0.0, None),
        Some((Vec3::ZERO, LIGHT_FALLBACK_CULL_RADIUS))
    );
    assert_eq!(
        light_proximity_bounds(-1.0, None),
        Some((Vec3::ZERO, LIGHT_FALLBACK_CULL_RADIUS))
    );
    let clip_to_world = Mat4::from_cols(
        Vec4::new(2.0, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 4.0, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 6.0, 0.0),
        Vec4::new(4.0, 5.0, 6.0, 1.0),
    );
    assert_eq!(
        light_proximity_bounds(2.0, Some(clip_to_world)),
        Some((Vec3::new(5.0, 7.0, 9.0), 5.0))
    );
    assert_eq!(light_proximity_bounds(2.0, None), None);
}

#[test]
fn light_projection_bounds_are_retained_before_proximity() {
    let matrix = Mat4::from_scale_rotation_translation(
        Vec3::new(-2.0, 4.0, 6.0),
        Quat::IDENTITY,
        Vec3::new(4.0, 5.0, 6.0),
    );
    let bounds = light_projection_bounds(matrix);
    assert_eq!(
        bounds,
        Some((Vec3::new(2.0, 5.0, 6.0), Vec3::new(4.0, 9.0, 12.0)))
    );
    assert_eq!(
        light_proximity_from_cached_bounds(2.0, bounds),
        Some((Vec3::new(3.0, 7.0, 9.0), 5.0))
    );
    assert_eq!(
        light_projection_bounds(Mat4::from_cols_array(&[0.0; 16])),
        None
    );
    // The initialized fallback is independent of projection availability.
    assert_eq!(
        light_proximity_from_cached_bounds(0.0, None),
        Some((Vec3::ZERO, LIGHT_FALLBACK_CULL_RADIUS))
    );
}

#[test]
fn light_clip_to_world_matches_point_and_spot_projection() {
    let view_aligned_game_rotation = [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]];
    let point = local_light_clip_to_world(
        Vec3::new(1.0, 2.0, 3.0),
        view_aligned_game_rotation,
        entities::LightType::Point,
        2.0,
        None,
    )
    .unwrap()
    .transform_point3(Vec3::ZERO);
    assert!((point - Vec3::new(-1.0, 0.0, 1.0)).length() < 0.0001);

    let cosine = 35.0_f32.to_radians().cos();
    let spot = local_light_clip_to_world(
        Vec3::ZERO,
        view_aligned_game_rotation,
        entities::LightType::Spot,
        2.0,
        Some(cosine),
    )
    .unwrap()
    .transform_point3(Vec3::ZERO);
    let expected = -2.0 * 35.0_f32.to_radians().tan();
    assert!((spot - Vec3::new(0.0, expected, expected)).length() < 0.0001);
}

#[test]
fn light_volume_clip_and_proximity_gates_match_bounds() {
    assert_eq!(
        light_volume_intersects_camera(Mat4::IDENTITY, Mat4::IDENTITY),
        Some(true)
    );
    assert_eq!(
        light_volume_intersects_camera(
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(0.5, 0.0, 0.0)),
        ),
        Some(true)
    );
    assert_eq!(
        light_volume_intersects_camera(
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(1.1, 0.0, 0.0)),
        ),
        Some(false)
    );

    assert!(light_proximity_pass(
        Vec3::new(32_000.0, 32_000.0, 0.0),
        Vec3::ZERO,
        LIGHT_FALLBACK_CULL_RADIUS,
    ));
    assert!(!light_proximity_pass(
        Vec3::new(32_768.1, 0.0, 0.0),
        Vec3::ZERO,
        LIGHT_FALLBACK_CULL_RADIUS,
    ));
}

fn add_culling_test_camera(world: &mut World) {
    let mut camera = Camera::default();
    camera.viewport = Some(default());
    camera.viewport.as_mut().unwrap().physical_size = UVec2::splat(128);
    camera.computed.clip_from_view = Mat4::IDENTITY;
    world.spawn((camera, Camera3d::default(), GlobalTransform::default()));
}

fn add_culling_test_light(
    world: &mut World,
    kind: entities::LightType,
    range: f32,
    proximity_bounds: Option<(Vec3, f32)>,
    clip_to_world: Mat4,
) -> Entity {
    world
        .spawn((
            LocalLightCull {
                range,
                kind,
                culling: entities::LightCulling::StaticPosition,
                definition_name_hash: 0,
                linear_color: [1.0; 3],
                projection_bounds: light_projection_bounds(clip_to_world),
                proximity_bounds,
                initial_sphere: None,
                clip_to_world: Some(clip_to_world),
            },
            GlobalTransform::default(),
            Visibility::Inherited,
        ))
        .id()
}

#[test]
#[ignore = "requires installed catalog and EOTW BWM; verifies the real world bank2, not rigid bank0"]
fn world_shadow_sources_resolve_level_materials() {
    let base = &crate::assets::installation::game_base().unwrap();
    let catalog = archive::Catalog::open(base).unwrap();
    let bytes = if let Ok(level) = env::var("DH2_AUDIT_LEVEL") {
        crate::assets::level::LevelResources::resolve(&catalog, &level)
            .unwrap()
            .read_world(&catalog)
            .unwrap()
    } else {
        fs::read("artifacts/eotw_p_lowchaos.bwm").unwrap()
    };
    let world = world_mesh::parse(&bytes).unwrap();
    let mut world_mesh_assets = Assets::<Mesh>::default();
    let mesh_sources = WorldShadowMeshSources::prepare(&world, &mut world_mesh_assets);
    assert_eq!(mesh_sources.objects.len(), world.objects.len());
    let mut checked_meshes = HashSet::new();
    for (object_id, object) in world.objects.iter().enumerate() {
        let source = mesh_sources.object(object_id).unwrap();
        assert_eq!(source.mesh_index, object.mesh_index);
        assert_eq!(source.matrix_start, object.matrix_start);
        assert_eq!(source.matrix_end, object.matrix_end);
        assert_eq!(source.lod, object.lod);
        assert_eq!(source.flipped, object.flipped);
        let decoded = &world.meshes[object.mesh_index];
        assert_eq!(
            source.raw_vertex_count.as_ref().unwrap(),
            &decoded.positions.len()
        );
        if !checked_meshes.insert((object.mesh_index, object.flipped)) {
            continue;
        }
        let mesh = world_mesh_assets.get(&source.mesh).unwrap();
        let bevy::mesh::VertexAttributeValues::Float32x3(forward) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
        else {
            panic!("forward BWM position attribute is missing")
        };
        let bevy::mesh::VertexAttributeValues::Float32x3(raw) =
            mesh.attribute(shadow_pass::VEGETATION_POSITION).unwrap()
        else {
            panic!("local BWM position attribute is missing")
        };
        assert_eq!(raw, &decoded.positions);
        assert_eq!(forward.len(), decoded.positions.len());
        for (&local, converted) in decoded.positions.iter().zip(forward) {
            assert_eq!(*converted, game_to_view(local).to_array());
        }
        let bevy::mesh::VertexAttributeValues::Float32x2(raw_normal) =
            mesh.attribute(shadow_pass::VEGETATION_NORMAL).unwrap()
        else {
            panic!("local BWM normal attribute is missing")
        };
        assert_eq!(raw_normal.len(), decoded.packed_channels.len());
        for (actual, packed) in raw_normal.iter().zip(&decoded.packed_channels) {
            assert_eq!(
                *actual,
                [packed[4], packed[5]].map(|byte| ((byte as i8 as f32) / 127.0).max(-1.0))
            );
        }
        let bevy::mesh::VertexAttributeValues::Float32x3(raw_color) =
            mesh.attribute(shadow_pass::VEGETATION_COLOR).unwrap()
        else {
            panic!("local BWM color attribute is missing")
        };
        assert_eq!(raw_color.len(), decoded.packed_channels.len());
        for (actual, packed) in raw_color.iter().zip(&decoded.packed_channels) {
            assert_eq!(
                *actual,
                [packed[12], packed[13], packed[14]].map(|byte| byte as f32 / 255.0)
            );
        }
    }
    let mut library =
        shader_cache::ProgramLibrary::open(base, shader_cache::ShaderCacheBackend::Pc, &catalog);
    let names: std::collections::BTreeSet<_> = world
        .objects
        .iter()
        .map(|object| &object.material_path)
        .collect();
    let mut counts = [0usize; 5];
    let mut base_slots = [0usize; 2];
    let mut failures = Vec::new();
    for name in names {
        let result: Result<Option<shader_cache::CachedWorldShadowClass>, String> = (|| {
            let bytes = catalog
                .read(&format!("generated/decls/material/{name}.decl"))
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("missing {name}"))?;
            let surface = material_surface::Surface::parse(&bytes);
            let main = surface.main_program.as_deref().ok_or("missing MAIN")?;
            let program = catalog
                .read(&format!(
                    "generated/decls/renderprog/{}.decl",
                    main.to_ascii_lowercase()
                ))
                .map_err(|e| e.to_string())?
                .ok_or("missing MAIN definition")?;
            let inputs =
                resolve_shadow_source_inputs(Some(&mut library), &bytes, &program, Some(main))?;
            let present =
                resolve_shadow_slot_presence(Some(&mut library), &catalog, &bytes, &inputs, Some(main))?;
            base_slots[usize::from(present)] += 1;
            let source =
                resolve_world_shadow_source(Some(&mut library), &bytes, &inputs, Some(main))?;
            if let Some(source) = source {
                let class = source.class.material_parameter_class();
                resolve_shadow_material_parameters(
                    library.as_mut().map_err(|e| e.clone())?,
                    &catalog,
                    &bytes,
                    class,
                    false,
                )?;
                Ok(Some(source.class))
            } else {
                Ok(None)
            }
        })();
        match result {
            Ok(None) => counts[0] += 1,
            Ok(Some(shader_cache::CachedWorldShadowClass::Opaque)) => counts[1] += 1,
            Ok(Some(
                shader_cache::CachedWorldShadowClass::AlphaTested
                | shader_cache::CachedWorldShadowClass::VistaAlpha,
            )) => counts[2] += 1,
            Ok(Some(
                shader_cache::CachedWorldShadowClass::Vegetation
                | shader_cache::CachedWorldShadowClass::VegetationFixed,
            )) => counts[3] += 1,
            Err(error) => {
                counts[4] += 1;
                failures.push(format!("{name}: {error}"));
            }
        }
    }
    eprintln!(
        "actual world shadow bank2 disabled/opaque/alpha/vegetation/unsupported={counts:?}; base slot10 absent/present={base_slots:?}"
    );
    for failure in &failures {
        eprintln!("WORLD SOURCE UNSUPPORTED {failure}");
    }
    assert!(
        failures.is_empty(),
        "{} unresolved actual world shadow materials",
        failures.len()
    );
}

#[test]
#[ignore = "requires installed catalog and actual EOTW BWM material fixture"]
fn shadow_sources_resolve_actual_world_materials() {
    let catalog =
        archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let world = world_mesh::parse(&fs::read("artifacts/eotw_p_lowchaos.bwm").unwrap()).unwrap();
    let mut library = shader_cache::ProgramLibrary::open(
        &crate::assets::installation::game_base().unwrap(),
        shader_cache::ShaderCacheBackend::Pc,
        &catalog,
    );
    let names: std::collections::BTreeSet<_> = world
        .objects
        .iter()
        .map(|object| &object.material_path)
        .collect();
    let mut counts = [0usize; 5]; // disabled, opaque, cutout, vegetation, unsupported
    let mut vegetation_materials = std::collections::BTreeSet::new();
    let mut failures = Vec::new();
    // Explicit synthetic model inputs validate the real material encoder;
    // they are not a claim that every actual entity owns group 3/offset 0.
    let mut clock = game_clock::GameClock::from_bytes([0; game_clock::STATE_SIZE]);
    clock.initialize();
    let clocks = game_clock::GameplayViewClocks::from_clock(&clock);
    let mvp = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    for name in names {
        let result = (|| {
            let bytes = catalog
                .read(&format!("generated/decls/material/{name}.decl"))
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("missing material {name}"))?;
            let surface = material_surface::Surface::parse(&bytes);
            let program = match surface.main_program.as_deref() {
                Some(name) => catalog
                    .read(&format!(
                        "generated/decls/renderprog/{}.decl",
                        name.to_ascii_lowercase()
                    ))
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("missing main program {name}"))?,
                None => return Err("material has no main program".into()),
            };
            let source = resolve_shadow_source_inputs(
                Some(&mut library),
                &bytes,
                &program,
                surface.main_program.as_deref(),
            )?;
            // Read-only native material snapshot, not a path exception:
            // final class 2 / hairType 0 clears all three rigid banks.
            if name == "models/environment/props/generic/clocks_01/clocks_glass.material" {
                assert_eq!(source.source, material::ShadowProgramSource::Disabled);
            }
            let resolved = resolve_rigid_shadow_source(
                Some(&mut library),
                &bytes,
                &source,
                surface.main_program.as_deref(),
            );
            if let Err(error) = &resolved {
                let library = library.as_mut().map_err(|error| error.to_string())?;
                let main = surface.main_program.as_deref().unwrap();
                let selected = source.override_program.as_deref().unwrap_or(main);
                let parameters =
                    shadow_variant_overrides(library, &bytes, &source, Some(main), selected)?;
                let values: Vec<_> = [
                    "noShadow",
                    "invisible",
                    "hairType",
                    "skyType",
                    "useDirectInstancing",
                    "useMultiInstancing",
                    "noScreenShadows",
                    "translucency_hq/enable",
                    "translucency/enable",
                ]
                .into_iter()
                .map(|name| {
                    (
                        name,
                        library.option_value(main.as_bytes(), &parameters, name.as_bytes()),
                    )
                })
                .collect();
                let requested_hash = library
                    .specialization_hash(selected.as_bytes(), &parameters)?
                    .map(|hash| format!("{hash:08x}"));
                let stage_ids = |data: render_program::D3dVariantData| {
                    data.stages
                        .into_iter()
                        .map(|stage| {
                            (
                                stage.kind,
                                match stage.bytecode {
                                    render_program::StageBytecode::Absent => None,
                                    render_program::StageBytecode::Cached(id) => Some(id),
                                    render_program::StageBytecode::Inline(_) => Some(u32::MAX),
                                },
                            )
                        })
                        .collect::<Vec<_>>()
                };
                let base = library
                    .serialized_variant(selected.as_bytes(), &parameters)?
                    .map(stage_ids);
                let mut banks = Vec::new();
                for (enabled, excluded) in [
                    ("useDirectInstancing", "useMultiInstancing"),
                    ("useMultiInstancing", "useDirectInstancing"),
                ] {
                    if library.option_value(main.as_bytes(), &parameters, excluded.as_bytes())? != 0
                    {
                        continue;
                    }
                    let mut instanced = parameters.clone();
                    instanced
                        .parameters
                        .push(render_program::SerializedParameter {
                            name: enabled.as_bytes().to_vec(),
                            kind: render_program::RenderParameterKind::Option,
                            value: render_program::SerializedParameterValue::Words([1, 0, 0, 0]),
                        });
                    let stages = library
                        .serialized_variant(selected.as_bytes(), &instanced)?
                        .map(stage_ids);
                    let hash = library
                        .specialization_hash(selected.as_bytes(), &instanced)?
                        .map(|hash| format!("{hash:08x}"));
                    banks.push((enabled, hash, stages));
                }
                let main_state = library.definition(main.as_bytes())?.unwrap().render_state();
                return Err(format!(
                    "{error}; requested native signature {requested_hash:?}; base {base:?}; blend parsed={:08x} mainRpd={:08x}; effective main options {values:?}; native instance banks {banks:?}",
                    source.main_blend_state, main_state[0]
                ));
            }
            resolved
        })();
        match result {
            Ok(None) => counts[0] += 1,
            Ok(Some(source)) => {
                let material = catalog
                    .read(&format!("generated/decls/material/{name}.decl"))
                    .unwrap()
                    .unwrap();
                let library = library.as_mut().unwrap();
                let parameters = resolve_shadow_material_parameters(
                    library,
                    &catalog,
                    &material,
                    source.class,
                    true,
                )
                .unwrap_or_else(|error| {
                    panic!("{name}: native shadow material parameters: {error}")
                });
                let (overrides, defaults) = parameters.declarations();
                for (parameter, default) in defaults {
                    let expected = library
                        .registered_float_vector(&catalog, overrides, parameter, None)
                        .unwrap();
                    assert_eq!(
                        parameters
                            .vector(parameter, None)
                            .unwrap()
                            .map(f32::to_bits),
                        expected
                    );
                    let fallback = render_program::SerializedParameterBlock {
                        flags: 0,
                        parameters: vec![render_program::SerializedParameter {
                            name: parameter.clone(),
                            kind: render_program::RenderParameterKind::Uniform,
                            value: render_program::SerializedParameterValue::Words(
                                [11.0f32.to_bits(); 4],
                            ),
                        }],
                    };
                    let expected = overrides
                        .vector_words(parameter, 0x30, *default, Some(&fallback))
                        .unwrap();
                    assert_eq!(
                        parameters
                            .vector(parameter, Some(&fallback))
                            .unwrap()
                            .map(f32::to_bits),
                        expected
                    );
                }
                if source.class == shader_cache::CachedRigidShadowClass::Vegetation {
                    let wind = parameters.vegetation(None).unwrap();
                    let uv = parameters.vector(b"layer0_scalebias", None).unwrap();
                    for group in 0..game_clock::GROUP_COUNT {
                        let flags = (group as u8) << 4;
                        let actual = parameters
                            .vegetation_instance_cb(None, &clocks, 0.5, flags, &mvp, 1.0)
                            .unwrap();
                        let local_time = render_model::model_shader_local_time(
                            0.5,
                            clock.time(group).unwrap(),
                            clock.delta(group).unwrap(),
                        );
                        let expected = shadow_pass::vegetation_shadow_instance_cb(
                            uv, &wind, &mvp, local_time, 1.0,
                        );
                        assert_eq!(actual, expected, "{name}: model clock group {group}");
                    }
                    assert!(parameters
                        .vegetation_instance_cb(None, &clocks, 0.5, 0x50, &mvp, 1.0)
                        .is_err());
                }
                match source.class {
                    shader_cache::CachedRigidShadowClass::Opaque => counts[1] += 1,
                    shader_cache::CachedRigidShadowClass::AlphaTested
                    | shader_cache::CachedRigidShadowClass::VistaAlpha => counts[2] += 1,
                    shader_cache::CachedRigidShadowClass::Vegetation => {
                        counts[3] += 1;
                        if vegetation_materials.is_empty() {
                            let definition = library
                                .definition(&source.program)
                                .unwrap()
                                .unwrap()
                                .clone();
                            let material = catalog
                                .read(&format!("generated/decls/material/{name}.decl"))
                                .unwrap()
                                .unwrap();
                            let mut evidence = format!(
                                "material={name}\nprogram={}\ndefaults={:?}\nmaterialSource:\n{}\n",
                                String::from_utf8_lossy(&source.program),
                                definition.defaults,
                                String::from_utf8_lossy(&material)
                            );
                            let mut parameters = std::collections::BTreeSet::new();
                            for stage in &source.variant.stages {
                                let names: Vec<_> = stage
                                    .bindings
                                    .parameters
                                    .iter()
                                    .filter_map(|entry| {
                                        let name = entry.name.bytes(&library.cache.names).unwrap();
                                        let key: Vec<_> = name
                                            .split(|byte| *byte == 0)
                                            .next()
                                            .unwrap()
                                            .iter()
                                            .map(u8::to_ascii_lowercase)
                                            .collect();
                                        (library.parameter_kinds.get(&key)
                                            == Some(&render_program::RenderParameterKind::Uniform))
                                        .then(|| String::from_utf8_lossy(&key).into_owned())
                                    })
                                    .collect();
                                evidence.push_str(&format!(
                                    "stage={} layout={:?} uniforms={names:?}\n",
                                    stage.kind, stage.layout
                                ));
                                parameters.extend(names);
                            }
                            for parameter in parameters {
                                let path = format!(
                                    "generated/decls/renderparm/{}.decl",
                                    parameter.to_ascii_lowercase()
                                );
                                let bytes = catalog.read(&path).unwrap().unwrap();
                                evidence.push_str(&format!(
                                    "\n{parameter}:\n{}\n",
                                    String::from_utf8_lossy(&bytes)
                                ));
                            }
                            fs::write("artifacts/native-vegetation-uniform-evidence.txt", evidence)
                                .unwrap();
                        }
                        vegetation_materials.insert(name.to_string());
                    }
                }
            }
            Err(error) => {
                counts[4] += 1;
                failures.push(format!("{name}: {error}"));
            }
        }
    }
    // Exercise the production mesh/input preparation against every actual
    // world mesh using a verified wind program, including shared LOD data.
    let mut geometry = Assets::<Mesh>::default();
    let mut handles = HashMap::new();
    let mut inputs = VegetationMeshInputs::default();
    for object in &world.objects {
        if !vegetation_materials.contains(&object.material_path) {
            continue;
        }
        let source = &world.meshes[object.mesh_index];
        let handle = handles.entry(object.mesh_index).or_insert_with(|| {
            geometry.add(source_mesh(
                &source.positions,
                &source.uvs,
                &source.indices,
                &source.packed_channels,
                false,
            ))
        });
        inputs.prepare(
            true,
            handle,
            &mut geometry,
            &source.positions,
            &source.packed_channels,
        );
        assert!(
            inputs.0[&handle.id()].is_ok(),
            "{}: {:?}",
            object.material_path,
            inputs.0[&handle.id()]
        );
        let mesh = geometry.get(&*handle).unwrap();
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(raw)) =
            mesh.attribute(shadow_pass::VEGETATION_POSITION)
        else {
            panic!("missing native local positions")
        };
        assert_eq!(raw, &source.positions);
    }
    assert!(vegetation_materials.is_empty() || !inputs.0.is_empty());
    let vertices: usize = inputs
        .0
        .values()
        .map(|result| *result.as_ref().unwrap())
        .sum();
    let geometry_report = format!(
        "vegetation geometry: {} shared meshes, {vertices} vertices, {} decoded attribute bytes",
        inputs.0.len(),
        vertices * std::mem::size_of::<[f32; 8]>()
    );
    eprintln!("{geometry_report}");
    let report = format!(
        "actual BWM shadow material sources disabled/opaque/cutout/vegetation/unsupported={counts:?}\n{geometry_report}\n{}\n",
        failures.join("\n")
    );
    fs::write("artifacts/native-world-shadow-source-coverage.txt", &report).unwrap();
    eprintln!(
        "actual BWM shadow material sources disabled/opaque/cutout/vegetation/unsupported={counts:?}"
    );
    for error in failures.iter().take(25) {
        eprintln!("unsupported shadow source: {error}");
    }
    assert!(
        counts[1] + counts[2] > 0,
        "no actual map materials resolved; this is NOT a complete-coverage assertion"
    );
}

#[test]
fn parsed_light_preset_inputs_reach_conversion() {
    let document = entities::parse(
        r#"Version 6
entity { entityDef lamp { class = "idLight"; edit = {
    spawnPosition = { x = 1; } m_preset = "LIGHT_PRESET_CANDLE";
} } }"#,
    )
    .unwrap();
    let mut light = entities::placed_lights_with_inheritance(&document, |_| Ok(None))
        .unwrap()
        .remove(0);
    let converted = light_preset(&light, [1.0; 3]).unwrap();
    assert_eq!(converted.range, 10.0f32.sqrt());
    assert_eq!(converted.falloff, converted.range * 10.0 / 15.0);
    assert_eq!(converted.color, [5.0 / render_light::normalization(); 3]);
    assert!(light_rgb_nonzero(converted.color));
    assert!(!light_rgb_nonzero(
        light_preset(&light, [0.0; 3]).unwrap().color
    ));
    light.preset = Some("LIGHT_PRESET_100W".into());
    assert_eq!(
        light_preset(&light, [1.0; 3]).unwrap().normalized_intensity,
        5.0
    );
    assert_eq!(light_preset(&light, [1.0; 3]).unwrap().range, 15.0);
    light.preset = Some("LIGHT_PRESET_MOON".into());
    light.distance_cutoff = 1000.0;
    assert_eq!(light_preset(&light, [1.0; 3]).unwrap().range, 1000.0);
    light.preset = Some("LIGHT_PRESET_REFLECTOR".into());
    assert!(light_preset(&light, [1.0; 3]).is_some());
    light.reflector_preset = Some("REFLECTOR_SUN_OFF_50PCT_WALL".into());
    assert!(light_preset(&light, [1.0; 3]).is_some());
    light.reflector_preset = Some("UNKNOWN".into());
    assert!(light_preset(&light, [1.0; 3]).is_none());
    light.preset = Some("UNKNOWN".into());
    assert!(light_preset(&light, [1.0; 3]).is_none());
}

#[test]
fn area_rectangle_preserves_corners_and_emitting_side() {
    // Real balcony light plus identity: compare native +/- half-extent
    // corners with the backend rectangle, including its one-sided normal.
    for rotation in [
        [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        [
            [-0.55556643, 0.8314722, 0.],
            [-0.8314722, -0.55556643, 0.],
            [0., 0., 1.],
        ],
    ] {
        let center = Vec3::new(-41.088326, 23.700006, -83.30427);
        let transform = authored_area_light_transform(center, rotation).unwrap();
        let z = game_to_view(rotation[0]).normalize();
        let x = game_to_view(rotation[1]);
        let y = z.cross(x);
        let half = [0.7587613, 1.5461122];
        let game = [-1., 1.]
            .into_iter()
            .flat_map(|sx| [-1., 1.].map(|sy| center + sx * half[0] * x + sy * half[1] * y))
            .collect::<Vec<_>>();
        for sx in [-1., 1.] {
            for sy in [-1., 1.] {
                let corner =
                    center + sx * half[0] * transform.right() + sy * half[1] * transform.up();
                assert!(game.iter().any(|p| p.distance(corner) < 0.00001));
            }
        }
        assert!(transform.up().cross(*transform.right()).dot(z) > 0.99999);
    }
}

#[test]
fn area_preset_energy_reaches_actual_bevy_extraction_without_double_normalization() {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::pbr::{extract_lights, ExtractedRectLight};
    use bevy::render::{sync_world::RenderEntity, MainWorld};

    let mut main_world = MainWorld::default();
    main_world.init_resource::<PointLightShadowMap>();
    main_world.init_resource::<DirectionalLightShadowMap>();
    let mut render_world = World::new();
    let mut expected = Vec::new();
    for conservative in [false, true] {
        for preset in [6, 11] {
            for width in [1.0, 2.0] {
                let size = [width, 3.0];
                let output = render_light::convert(render_light::PresetInput {
                    kind: 3,
                    preset,
                    reflector: 0,
                    area_energy_conservative: conservative,
                    cutoff: 15.0,
                    falloff: 10.0,
                    cutoff_cosine: 0.5,
                    back_reflection: 0.0,
                    area_size: size,
                    color: [1.0; 3],
                })
                .unwrap();
                let full_size = size.map(|half| 2.0 * half);
                let render_entity = render_world.spawn_empty().id();
                main_world.spawn((
                    RectLight {
                        color: Color::WHITE,
                        intensity: light_backend_intensity(
                            output.normalized_intensity,
                            entities::LightType::Area,
                            full_size,
                        )
                        .unwrap(),
                        width: full_size[0],
                        height: full_size[1],
                        range: output.range,
                    },
                    GlobalTransform::default(),
                    ViewVisibility::VISIBLE,
                    RenderEntity::from(render_entity),
                ));
                expected.push((render_entity, output.normalized_intensity));
            }
        }
    }
    render_world.insert_resource(main_world);
    render_world.run_system_once(extract_lights).unwrap();
    for (entity, intensity) in expected {
        let extracted = render_world.get::<ExtractedRectLight>(entity).unwrap();
        let expected = intensity * 2_000.0 / std::f32::consts::PI;
        assert!((extracted.intensity / expected - 1.0).abs() < 0.000_001);
    }
}

#[test]
fn backend_light_intensity_rejects_overflow_after_conversion() {
    use entities::LightType;
    for kind in [
        LightType::Point,
        LightType::HalfPoint,
        LightType::Spot,
        LightType::Parallel,
    ] {
        assert!(light_backend_intensity(f32::MAX, kind, [1.0; 2]).is_none());
        assert!(light_backend_intensity(f32::NAN, kind, [1.0; 2]).is_none());
        let backend = light_backend_intensity(5.0, kind, [1.0; 2]).unwrap();
        let world = if kind == LightType::Parallel {
            backend
        } else {
            backend / (4.0 * std::f32::consts::PI)
        };
        assert!(
            (world - 696.3).abs() < 0.0001,
            "preset intensity5 in world units: {world}"
        );
    }
    assert!(light_backend_intensity(f32::MAX, LightType::Area, [1.0; 2]).is_none());
    assert!(light_backend_intensity(1.0, LightType::Area, [f32::MAX; 2]).is_none());
    assert!(light_backend_intensity(1.0, LightType::Area, [f32::NAN, 1.0]).is_none());
    let output = render_light::convert(render_light::PresetInput {
        kind: 3,
        preset: 6,
        reflector: 0,
        area_energy_conservative: true,
        cutoff: 15.0,
        falloff: 10.0,
        cutoff_cosine: 0.5,
        back_reflection: 0.0,
        area_size: [1e-18; 2],
        color: [1.0; 3],
    })
    .unwrap();
    assert!(output.normalized_intensity.is_finite());
    assert!((output.normalized_intensity * 2_000.0).is_infinite());
    assert!(
        light_backend_intensity(output.normalized_intensity, LightType::Area, [0.01; 2])
            .unwrap()
            .is_finite()
    );
}

#[test]
fn light_eligibility_matches_installed_instructions() {
    let mut cases = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/light_update_cases.txt"
    ))
    .lines()
    {
        let values = line
            .split_whitespace()
            .map(|word| word.parse::<u32>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(values.len(), 7);
        let color = [values[3], values[4], values[5]].map(f32::from_bits);
        let flags = render_light_update_flags(values[0] as u8, values[1] as u8, values[2], color);
        assert_eq!(flags, values[6] as u8, "{line}");
        if !light_rgb_nonzero(color) {
            assert_eq!(flags & 8, 0);
        }
        cases += 1;
    }
    assert_eq!(cases, 1920);
}

#[test]
fn static_light_id_gate_preserves_bypasses_and_empty_views() {
    use entities::{LightCulling, LightType};
    for kind in [
        LightType::Point,
        LightType::HalfPoint,
        LightType::Spot,
        LightType::Area,
    ] {
        assert!(local_light_id_pass(
            kind,
            LightCulling::StaticPosition,
            42,
            Some(&[1, 42, 42, 100])
        ));
        assert!(!local_light_id_pass(
            kind,
            LightCulling::StaticPosition,
            43,
            Some(&[1, 42, 42, 100])
        ));
        assert!(!local_light_id_pass(
            kind,
            LightCulling::StaticPosition,
            42,
            Some(&[])
        ));
        assert!(local_light_id_pass(
            kind,
            LightCulling::DynamicPosition,
            42,
            Some(&[])
        ));
        assert!(local_light_id_pass(
            kind,
            LightCulling::StaticPosition,
            42,
            None
        ));
    }
    assert!(local_light_id_pass(
        LightType::Parallel,
        LightCulling::StaticPosition,
        42,
        Some(&[])
    ));
}

fn light_test_query_state() -> tome::RenderQueryState {
    let rows = std::array::from_fn(|row| std::array::from_fn(|column| f32::from(row == column)));
    tome::RenderQueryState {
        render_rows: rows,
        cropped_rows: rows,
        camera_point: [0.0; 3],
        camera_imprecision: [0.0; 3],
        winding_flip: false,
        viewport: [0, 0, 64, 64],
        maximum_depth: 0.0,
        depth_grid: None,
    }
}

#[test]
fn world_light_query_preserves_area_then_common_proximity_and_clip_checks() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    world.insert_resource(LocalLightCullEnabled(true));
    world.insert_resource(LocalLightCullStats::default());
    world.insert_resource(SceneRenderQueue {
        visible_authored_ids: vec![0],
        visible_cell_mask: vec![1],
        light_query_state: Some(light_test_query_state()),
        ..default()
    });
    add_culling_test_camera(&mut world);
    let mut lights = Vec::new();
    for bounds in [
        [[0.1, 0.1, 0.2], [0.5, 0.5, 0.6]],
        [[2.0, 0.1, 0.2], [3.0, 0.5, 0.6]],
        [[0.1, 0.1, 0.2], [0.11, 0.11, 0.6]],
    ] {
        let light = add_culling_test_light(
            &mut world,
            entities::LightType::Point,
            10.0,
            Some((Vec3::splat(1.0e6), 0.0)),
            Mat4::IDENTITY,
        );
        let low = game_to_view(bounds[0]);
        let high = game_to_view(bounds[1]);
        world
            .get_mut::<LocalLightCull>(light)
            .unwrap()
            .projection_bounds = Some((low.min(high), low.max(high)));
        lights.push(light);
    }
    world.run_system_once(cull_small_local_lights).unwrap();
    for &light in &lights {
        assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Hidden));
    }
    let stats = world.resource::<LocalLightCullStats>();
    assert!(stats.world_query_bound);
    assert_eq!(stats.world_query_unverified, 0);
    assert_eq!(stats.proximity_culled, 1);
    assert_eq!(stats.clip_culled, 1);
    assert_eq!(stats.area_culled, 1);

    // The common proximity check is required in the Umbra branch too.
    // Its area uses the cached box, NOT the non-Umbra range sphere.
    let mut first = world.get_mut::<LocalLightCull>(lights[0]).unwrap();
    first.proximity_bounds = Some((Vec3::ZERO, 1.0));
    first.range = 1.0e-9;
    drop(first);
    world.run_system_once(cull_small_local_lights).unwrap();
    assert_eq!(
        world.get::<Visibility>(lights[0]),
        Some(&Visibility::Visible)
    );
    assert_eq!(world.resource::<LocalLightCullStats>().proximity_culled, 0);

    // Missing world-area data must not reuse a previous metric. Known
    // common proximity/clip checks remain valid without that descriptor.
    world.resource_mut::<SceneRenderQueue>().light_query_state = None;
    world.run_system_once(cull_small_local_lights).unwrap();
    assert_eq!(
        world.get::<Visibility>(lights[0]),
        Some(&Visibility::Visible)
    );
    for &light in &lights[1..] {
        assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Hidden));
    }
    assert_eq!(
        world
            .resource::<LocalLightCullStats>()
            .world_query_unverified,
        3
    );
    assert_eq!(world.resource::<LocalLightCullStats>().proximity_culled, 2);

    world
        .resource_mut::<SceneRenderQueue>()
        .visible_cell_mask
        .clear();
    world.run_system_once(cull_small_local_lights).unwrap();
    for light in lights {
        assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Hidden));
    }
    assert_eq!(world.resource::<LocalLightCullStats>().proximity_culled, 2);
    assert_eq!(world.resource::<LocalLightCullStats>().area_culled, 1);
}

#[test]
#[ignore = "requires local original-game EOTW Tome fixture"]
fn light_sphere_batch_rejects_hidden_cells_and_retains_unlocated_sources() {
    use bevy::ecs::system::RunSystemOnce;
    let bytes =
        fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
    let tree = tome::SpatialTree::parse(&bytes).unwrap();
    let query_data = tome::DirectTomeQueryData::parse(&bytes, &tree).unwrap();
    let point = [-36.467716, 192.6837, 5.6344576];
    let outside = [1.0e6; 3];
    let cell = query_data
        .point_cell_id(&bytes, &tree, point)
        .unwrap()
        .unwrap();
    assert_eq!(cell, 391);
    assert_eq!(
        query_data.point_cell_id(&bytes, &tree, outside).unwrap(),
        None
    );
    let cell_count = query_data.cell_count();
    let mut world = World::new();
    world.insert_resource(TomeWorld {
        sphere_graph: tome::SphereCellGraph::parse(&bytes),
        gate_states: tome::initial_gate_states(&bytes).unwrap(),
        bytes,
        tree,
        query_data,
        query_job_count: tome::DEFAULT_QUERY_JOB_COUNT,
        query_pool: None,
    });
    world.insert_resource(LocalLightCullEnabled(true));
    world.insert_resource(LocalLightCullStats::default());
    world.insert_resource(SceneRenderQueue {
        visible_authored_ids: vec![0],
        visible_cell_mask: vec![0; cell_count.div_ceil(32)],
        light_query_state: Some(light_test_query_state()),
        ..default()
    });
    add_culling_test_camera(&mut world);
    let lights = [point, outside].map(|point| {
        let light = add_culling_test_light(
            &mut world,
            entities::LightType::Point,
            0.1,
            None,
            Mat4::IDENTITY,
        );
        world
            .get_mut::<LocalLightCull>(light)
            .unwrap()
            .initial_sphere = Some([point[0], point[1], point[2], 0.1]);
        light
    });
    world.run_system_once(cull_small_local_lights).unwrap();
    assert_eq!(
        world.get::<Visibility>(lights[0]),
        Some(&Visibility::Hidden)
    );
    assert_eq!(
        world.get::<Visibility>(lights[1]),
        Some(&Visibility::Visible)
    );
    let stats = world.resource::<LocalLightCullStats>();
    assert_eq!(stats.sphere_candidates, 2);
    assert_eq!(stats.sphere_culled, 1);
    assert_eq!(stats.sphere_unverified, 0);
    assert_eq!(stats.sphere_error, None);

    // A fresh mask containing the source cell retains its light.
    world.resource_mut::<SceneRenderQueue>().visible_cell_mask[cell as usize / 32] |=
        1 << (cell % 32);
    world.run_system_once(cull_small_local_lights).unwrap();
    assert_eq!(
        world.get::<Visibility>(lights[0]),
        Some(&Visibility::Visible)
    );
    assert_eq!(world.resource::<LocalLightCullStats>().sphere_culled, 0);
}

#[test]
fn preset_rgb_is_diagnostic_not_runtime_light_eligibility() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    world.insert_resource(LocalLightCullEnabled(true));
    world.insert_resource(LocalLightCullStats::default());
    world.insert_resource(SceneRenderQueue {
        visible_authored_ids: vec![0],
        visible_cell_mask: vec![1],
        ..default()
    });
    add_culling_test_camera(&mut world);
    let mut lights = Vec::new();
    for kind in [
        entities::LightType::Point,
        entities::LightType::HalfPoint,
        entities::LightType::Spot,
        entities::LightType::Area,
    ] {
        for color in [[0.0, -0.0, 0.0], [0.0, 1.0, 0.0]] {
            let light = add_culling_test_light(
                &mut world,
                kind,
                10.0,
                Some((Vec3::ZERO, LIGHT_FALLBACK_CULL_RADIUS)),
                Mat4::IDENTITY,
            );
            world.get_mut::<LocalLightCull>(light).unwrap().linear_color = color;
            lights.push(light);
        }
    }
    world.run_system_once(cull_small_local_lights).unwrap();
    for light in &lights {
        assert_eq!(world.get::<Visibility>(*light), Some(&Visibility::Visible));
    }
    let stats = world.resource::<LocalLightCullStats>();
    assert_eq!(stats.zero_preset_rgb, 4);
    assert_eq!(stats.static_id_culled, 0);
    assert_eq!(stats.proximity_culled, 0);
    assert_eq!(stats.clip_culled, 0);
    assert_eq!(stats.area_culled, 0);

    world
        .resource_mut::<SceneRenderQueue>()
        .visible_cell_mask
        .clear();
    world.run_system_once(cull_small_local_lights).unwrap();
    assert_eq!(world.resource::<LocalLightCullStats>().zero_preset_rgb, 4);
    for light in lights {
        assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Visible));
    }
}

#[test]
fn local_light_culler_consumes_current_ids_and_fails_open_on_missing_outputs() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    world.insert_resource(LocalLightCullEnabled(true));
    world.insert_resource(LocalLightCullStats::default());
    world.insert_resource(SceneRenderQueue {
        static_world_model_present: false,
        instances: Vec::new(),
        active: 0,
        tome_render_instances: TomeRenderInstanceMap::default(),
        visible_authored_ids: vec![7],
        visible_cell_indices: vec![0],
        visible_cell_mask: vec![1],
        shadow_diffuse_slots: Vec::new(),
        shadow_diffuse_membership: Vec::new(),
        light_query_state: None,
        shadow_ordinary_query: None,
        visibility_suppression_groups: Vec::new(),
        trace_bwm_lod: false,
        world_lods_by_render_instance: Vec::new(),
        last_bwm_lod_visibility_report: None,
        diffuse_image_demands: Vec::new(),
        diffuse_image_paths: Vec::new(),
        diffuse_image_materials: Vec::new(),
        material_images: HashMap::new(),
        material_stage_sorts: HashMap::new(),
        diffuse_image_handles: Vec::new(),
        diffuse_image_pending: Vec::new(),
        diffuse_mip_states: Vec::new(),
        diffuse_image_attempted: Vec::new(),
        diffuse_image_last_visible: Vec::new(),
        texture_stream_frame: 0,
        texture_viewport_pixels: 0,
        diffuse_mip_upgrades: 0,
        preferred_mip: None,
    });
    add_culling_test_camera(&mut world);
    let mut handles = Vec::new();
    for (hash, dynamic) in [(7, false), (8, false), (9, true)] {
        let light = add_culling_test_light(
            &mut world,
            entities::LightType::Point,
            10.0,
            Some((Vec3::ZERO, LIGHT_FALLBACK_CULL_RADIUS)),
            Mat4::IDENTITY,
        );
        let mut component = world.get_mut::<LocalLightCull>(light).unwrap();
        component.definition_name_hash = hash;
        if dynamic {
            component.culling = entities::LightCulling::DynamicPosition;
        }
        handles.push(light);
    }
    world.run_system_once(cull_small_local_lights).unwrap();
    assert_eq!(
        world.get::<Visibility>(handles[0]),
        Some(&Visibility::Visible)
    );
    assert_eq!(
        world.get::<Visibility>(handles[1]),
        Some(&Visibility::Hidden)
    );
    assert_eq!(
        world.get::<Visibility>(handles[2]),
        Some(&Visibility::Visible)
    );
    assert_eq!(world.resource::<LocalLightCullStats>().static_id_culled, 1);
    assert!(
        world
            .resource::<LocalLightCullStats>()
            .static_id_binding_active
    );
    // A valid empty visible-ID result rejects static lights, not dynamic lights.
    world
        .resource_mut::<SceneRenderQueue>()
        .visible_authored_ids
        .clear();
    world.run_system_once(cull_small_local_lights).unwrap();
    assert_eq!(
        world.get::<Visibility>(handles[0]),
        Some(&Visibility::Hidden)
    );
    assert_eq!(
        world.get::<Visibility>(handles[1]),
        Some(&Visibility::Hidden)
    );
    assert_eq!(
        world.get::<Visibility>(handles[2]),
        Some(&Visibility::Visible)
    );
    assert_eq!(world.resource::<LocalLightCullStats>().static_id_culled, 2);
    // An unavailable camera/matrix/mapping must not turn cleared IDs into
    // rejections. Native worker errors instead publish fresh empty lists;
    // that connected pipeline policy is tested separately below.
    world
        .resource_mut::<SceneRenderQueue>()
        .visible_cell_mask
        .clear();
    world.run_system_once(cull_small_local_lights).unwrap();
    for light in handles {
        assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Visible));
    }
    assert_eq!(world.resource::<LocalLightCullStats>().static_id_culled, 0);
    assert!(
        !world
            .resource::<LocalLightCullStats>()
            .static_id_binding_active
    );
}

#[test]
#[ignore = "requires local original-game EOTW Tome fixture"]
fn streamed_light_ids_replace_stale_data_on_worker_errors_and_camera_loss() {
    use bevy::ecs::system::RunSystemOnce;
    let bytes =
        fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
    bevy::tasks::ComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
    let tree = tome::SpatialTree::parse(&bytes).unwrap();
    let query_data = tome::DirectTomeQueryData::parse(&bytes, &tree).unwrap();
    let cell_count = query_data.cell_count();
    let object_count = u32::from_le_bytes(bytes[0x40..0x44].try_into().unwrap()) as usize;
    let mut world = World::new();
    world.insert_resource(TomeWorld {
        sphere_graph: tome::SphereCellGraph::parse(&bytes),
        gate_states: tome::initial_gate_states(&bytes).unwrap(),
        bytes,
        tree,
        query_data,
        query_job_count: tome::DEFAULT_QUERY_JOB_COUNT,
        query_pool: None,
    });
    world.insert_resource(PickupState {
        items: Vec::new(),
        world: runtime::WorldState::default(),
    });
    world.insert_resource(SceneRenderQueue {
        // This focused fixture has no BWM bindings; every selected object
        // can therefore contribute its authored user IDs to light lookup.
        tome_render_instances: TomeRenderInstanceMap::from_authored_bwm_mapping(
            &vec![Vec::new(); object_count],
            &[],
        ),
        ..default()
    });
    world.insert_resource(StreamingDiagnostics {
        frame: 0,
        pending: String::new(),
        history: VecDeque::new(),
        log: None,
        log_path: PathBuf::new(),
        status: String::new(),
        previous_status: String::new(),
        previous_active: 0,
        last_logged: Instant::now(),
        trace_texture_streaming: false,
        texture_events_pending: VecDeque::new(),
        texture_event_history: VecDeque::new(),
        texture_events_dropped: 0,
    });
    world.insert_resource(LocalLightCullEnabled(true));
    world.insert_resource(LocalLightCullStats::default());
    let mut camera = Camera::default();
    camera.viewport = Some(default());
    camera.viewport.as_mut().unwrap().physical_size = UVec2::splat(128);
    camera.computed.clip_from_view = Mat4::IDENTITY;
    let point = game_to_view([-36.467716, 192.6837, 5.6344576]);
    let camera_entity = world
        .spawn((
            camera,
            Camera3d::default(),
            Transform::from_translation(point),
            GlobalTransform::from_translation(point),
        ))
        .id();
    // Keep Local cache and change ticks across actual system invocations.
    let stream = world.register_system(stream_visible_models);
    world.run_system(stream).unwrap();
    assert_eq!(world.resource::<StreamingDiagnostics>().status, "ok");
    assert!(world
        .resource::<StreamingDiagnostics>()
        .pending
        .contains("query_cached=false"));
    let initial_ids = world
        .resource::<SceneRenderQueue>()
        .visible_authored_ids
        .clone();
    let initial_cells = world
        .resource::<SceneRenderQueue>()
        .visible_cell_indices
        .clone();
    let initial_depth = world
        .resource::<SceneRenderQueue>()
        .light_query_state
        .as_ref()
        .unwrap()
        .depth_grid
        .clone();
    world.run_system(stream).unwrap();
    assert!(world
        .resource::<StreamingDiagnostics>()
        .pending
        .contains("query_cached=true"));
    assert_eq!(
        world.resource::<SceneRenderQueue>().visible_authored_ids,
        initial_ids
    );
    assert_eq!(
        world.resource::<SceneRenderQueue>().visible_cell_indices,
        initial_cells
    );
    assert_eq!(
        world
            .resource::<SceneRenderQueue>()
            .light_query_state
            .as_ref()
            .unwrap()
            .depth_grid,
        initial_depth
    );

    // Replacing the resource must invalidate even identical query inputs.
    let unchanged_world = world.remove_resource::<TomeWorld>().unwrap();
    world.insert_resource(unchanged_world);
    world.run_system(stream).unwrap();
    assert!(world
        .resource::<StreamingDiagnostics>()
        .pending
        .contains("query_cached=false"));
    world.run_system(stream).unwrap();
    assert!(world
        .resource::<StreamingDiagnostics>()
        .pending
        .contains("query_cached=true"));

    let projection = world
        .entity_mut(camera_entity)
        .take::<Projection>()
        .unwrap();
    world.run_system(stream).unwrap();
    assert_eq!(
        world.resource::<StreamingDiagnostics>().status,
        "camera_not_ready"
    );
    assert!(world
        .resource::<SceneRenderQueue>()
        .visible_authored_ids
        .is_empty());
    assert!(world
        .resource::<SceneRenderQueue>()
        .light_query_state
        .is_none());
    world.entity_mut(camera_entity).insert(projection);
    world.run_system(stream).unwrap();
    assert!(world
        .resource::<StreamingDiagnostics>()
        .pending
        .contains("query_cached=true"));
    assert_eq!(
        world.resource::<SceneRenderQueue>().visible_authored_ids,
        initial_ids
    );

    let viewport = world
        .get_mut::<Camera>(camera_entity)
        .unwrap()
        .viewport
        .take();
    world.run_system(stream).unwrap();
    assert_eq!(
        world.resource::<StreamingDiagnostics>().status,
        "camera_not_ready"
    );
    assert!(world
        .resource::<SceneRenderQueue>()
        .light_query_state
        .is_none());
    world.get_mut::<Camera>(camera_entity).unwrap().viewport = viewport.clone();
    world
        .get_mut::<Camera>(camera_entity)
        .unwrap()
        .viewport
        .as_mut()
        .unwrap()
        .physical_size = UVec2::ZERO;
    world.run_system(stream).unwrap();
    assert!(world
        .resource::<StreamingDiagnostics>()
        .status
        .starts_with("query_error:"));
    assert!(world
        .resource::<SceneRenderQueue>()
        .light_query_state
        .is_none());
    world.get_mut::<Camera>(camera_entity).unwrap().viewport = viewport;
    world.run_system(stream).unwrap();
    assert!(world
        .resource::<StreamingDiagnostics>()
        .pending
        .contains("query_cached=true"));

    let other_camera = world.spawn(Camera3d::default()).id();
    world.run_system(stream).unwrap();
    assert_eq!(
        world.resource::<StreamingDiagnostics>().status,
        "camera_unavailable"
    );
    assert!(world
        .resource::<SceneRenderQueue>()
        .visible_authored_ids
        .is_empty());
    assert!(world
        .resource::<SceneRenderQueue>()
        .light_query_state
        .is_none());
    world.despawn(other_camera);
    world.run_system(stream).unwrap();
    assert!(world
        .resource::<StreamingDiagnostics>()
        .pending
        .contains("query_cached=true"));
    assert_eq!(
        world.resource::<SceneRenderQueue>().visible_authored_ids,
        initial_ids
    );
    let id = *world
        .resource::<SceneRenderQueue>()
        .visible_authored_ids
        .first()
        .unwrap();
    let light = add_culling_test_light(
        &mut world,
        entities::LightType::Point,
        10.0,
        Some((Vec3::ZERO, LIGHT_FALLBACK_CULL_RADIUS)),
        Mat4::from_translation(point),
    );
    world
        .get_mut::<LocalLightCull>(light)
        .unwrap()
        .definition_name_hash = id;
    world.run_system_once(cull_small_local_lights).unwrap();
    assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Visible));
    assert_eq!(world.resource::<LocalLightCullStats>().static_id_culled, 0);

    // Replay a mixed merge through the same publication stage used by
    // streaming. Failed workers clear their lists; successful siblings
    // still supply fresh IDs/cells to the light gate, never stale seeds.
    let mut jobs = {
        let tome_world = world.resource::<TomeWorld>();
        let pose = game_pose_from_view_transform(world.get::<Transform>(camera_entity).unwrap());
        let rows = tome::world_visibility_query_matrix(
            80.0,
            128,
            128,
            1.0,
            pose,
            DEFAULT_NEAR,
            DEFAULT_FAR,
        )
        .unwrap();
        let input = tome::OrdinaryDirectQueryInput {
            camera_point: view_to_game(point),
            render_rows: rows,
            cropped_rows: rows,
            initial_tile_rect: [0, 0, 8, 16],
            camera_imprecision: [0.6; 3],
            object_imprecision: 0.6,
            minimum_area_metric: 25.0 / (128.0 * 128.0),
            global_distance_threshold: tome::DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
            object_distance_scale: 1.0,
            winding_flip: tome::query_matrix_winding_flip(rows),
            gate_states: Some(&tome_world.gate_states),
            output_capacity: tome::WORLD_QUERY_OUTPUT_CAPACITY,
        };
        run_tome_render_jobs(tome_world, &input)
    };
    jobs[0].result = Err("injected native cleared-worker result".into());
    let merged = tome::merge_render_query_outputs(&jobs).unwrap().unwrap();
    world.resource_scope(|world, mut scene: Mut<SceneRenderQueue>| {
        scene.visible_authored_ids = vec![u32::MAX];
        scene.visible_cell_indices = vec![u32::MAX];
        scene
            .publish_light_query_outputs(world.resource::<TomeWorld>(), &merged)
            .unwrap();
    });
    let scene = world.resource::<SceneRenderQueue>();
    assert!(!scene.visible_authored_ids.is_empty());
    assert_eq!(scene.visible_cell_indices, merged.cell_indices());
    assert!(!scene.visible_authored_ids.contains(&u32::MAX));
    assert_eq!(scene.visible_cell_mask.len(), cell_count.div_ceil(32));
    let id = scene.visible_authored_ids[0];
    world
        .get_mut::<LocalLightCull>(light)
        .unwrap()
        .definition_name_hash = id;
    world.run_system_once(cull_small_local_lights).unwrap();
    assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Visible));
    assert_eq!(world.resource::<LocalLightCullStats>().static_id_culled, 0);
    assert!(
        world
            .resource::<LocalLightCullStats>()
            .static_id_binding_active
    );

    // An outside-root input is native worker error 5. All workers clear
    // their outputs, so merge publishes fresh empty IDs/cells (not the
    // preceding view and not an unavailable-query fail-open substitute).
    let point = game_to_view([1.0e8, 1.0e8, 1.0e8]);
    *world.get_mut::<Transform>(camera_entity).unwrap() = Transform::from_translation(point);
    *world.get_mut::<GlobalTransform>(camera_entity).unwrap() =
        GlobalTransform::from_translation(point);
    world
        .get_mut::<LocalLightCull>(light)
        .unwrap()
        .clip_to_world = Some(Mat4::from_translation(point));
    world.run_system(stream).unwrap();
    assert!(world
        .resource::<StreamingDiagnostics>()
        .pending
        .contains("query_cached=false"));
    world.run_system(stream).unwrap();
    assert!(world
        .resource::<StreamingDiagnostics>()
        .pending
        .contains("query_cached=false"));
    assert!(
        world
            .resource::<StreamingDiagnostics>()
            .status
            .starts_with("all_jobs_failed:"),
        "{}",
        world.resource::<StreamingDiagnostics>().pending
    );
    let scene = world.resource::<SceneRenderQueue>();
    assert!(scene.visible_authored_ids.is_empty());
    assert!(scene.visible_cell_indices.is_empty());
    assert_eq!(scene.visible_cell_mask.len(), cell_count.div_ceil(32));
    assert!(scene.visible_cell_mask.iter().all(|&word| word == 0));
    world.run_system_once(cull_small_local_lights).unwrap();
    assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Hidden));
    assert_eq!(world.resource::<LocalLightCullStats>().static_id_culled, 1);
    assert!(
        world
            .resource::<LocalLightCullStats>()
            .static_id_binding_active
    );

    // Losing the unique camera has no initialized query descriptor. The
    // cleared mask must not masquerade as a fresh empty native result.
    world.despawn(camera_entity);
    world.run_system(stream).unwrap();
    assert_eq!(
        world.resource::<StreamingDiagnostics>().status,
        "camera_unavailable"
    );
    let scene = world.resource::<SceneRenderQueue>();
    assert!(scene.visible_authored_ids.is_empty());
    assert!(scene.visible_cell_indices.is_empty());
    assert!(scene.visible_cell_mask.is_empty());
    add_culling_test_camera(&mut world);
    world
        .get_mut::<LocalLightCull>(light)
        .unwrap()
        .clip_to_world = Some(Mat4::IDENTITY);
    world.run_system_once(cull_small_local_lights).unwrap();
    assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Visible));
    assert_eq!(world.resource::<LocalLightCullStats>().static_id_culled, 0);
    assert!(
        !world
            .resource::<LocalLightCullStats>()
            .static_id_binding_active
    );
}

#[test]
fn local_light_culling_system_applies_each_gate_and_counts_kinds() {
    use bevy::ecs::system::RunSystemOnce;

    let mut world = World::new();
    world.insert_resource(LocalLightCullEnabled(true));
    world.insert_resource(LocalLightCullStats::default());
    add_culling_test_camera(&mut world);
    let point = add_culling_test_light(
        &mut world,
        entities::LightType::Point,
        10.0,
        Some((Vec3::ZERO, LIGHT_FALLBACK_CULL_RADIUS)),
        Mat4::IDENTITY,
    );
    let half_point = add_culling_test_light(
        &mut world,
        entities::LightType::HalfPoint,
        10.0,
        Some((Vec3::ZERO, LIGHT_FALLBACK_CULL_RADIUS)),
        Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0)),
    );
    let spot = add_culling_test_light(
        &mut world,
        entities::LightType::Spot,
        10.0,
        Some((Vec3::new(50_000.0, 0.0, 0.0), 1.0)),
        Mat4::IDENTITY,
    );
    let area = add_culling_test_light(
        &mut world,
        entities::LightType::Area,
        0.001,
        Some((Vec3::ZERO, LIGHT_FALLBACK_CULL_RADIUS)),
        Mat4::IDENTITY,
    );
    assert_eq!(
        light_volume_intersects_camera(
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0)),
        ),
        Some(false)
    );
    let mut camera_query = world.query::<&Camera>();
    let camera = camera_query.iter(&world).next().unwrap();
    assert_eq!(camera.physical_viewport_size(), Some(UVec2::splat(128)));
    assert_eq!(camera.clip_from_view(), Mat4::IDENTITY);
    drop(camera_query);

    world.run_system_once(cull_small_local_lights).unwrap();
    assert_eq!(world.resource::<LocalLightCullStats>().clip_culled, 1);

    assert_eq!(world.get::<Visibility>(point), Some(&Visibility::Visible));
    assert_eq!(
        world.get::<Visibility>(half_point),
        Some(&Visibility::Hidden)
    );
    assert_eq!(world.get::<Visibility>(spot), Some(&Visibility::Hidden));
    assert_eq!(world.get::<Visibility>(area), Some(&Visibility::Hidden));
    let stats = world.resource::<LocalLightCullStats>();
    assert_eq!(stats.total, 4);
    assert_eq!(stats.total_by_kind, [1, 1, 1, 1]);
    assert_eq!(stats.proximity_culled, 1);
    assert_eq!(stats.clip_culled, 1);
    assert_eq!(stats.area_culled, 1);
    assert_eq!(stats.proximity_unverified, 0);
    assert_eq!(stats.source_class_counts, [4, 0]);
    assert_eq!(stats.static_name_hash_matches, 0);
}

#[test]
fn unknown_local_light_proximity_bounds_fail_open_and_are_counted() {
    use bevy::ecs::system::RunSystemOnce;

    let mut world = World::new();
    world.insert_resource(LocalLightCullEnabled(true));
    world.insert_resource(LocalLightCullStats::default());
    add_culling_test_camera(&mut world);
    let light = world
        .spawn((
            LocalLightCull {
                range: 10.0,
                kind: entities::LightType::Point,
                culling: entities::LightCulling::DynamicPosition,
                definition_name_hash: 0,
                linear_color: [1.0; 3],
                projection_bounds: None,
                proximity_bounds: None,
                initial_sphere: None,
                clip_to_world: None,
            },
            GlobalTransform::from(Transform::from_translation(Vec3::new(50_000.0, 0.0, 0.0))),
            Visibility::Inherited,
        ))
        .id();

    world.run_system_once(cull_small_local_lights).unwrap();

    assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Visible));
    let stats = world.resource::<LocalLightCullStats>();
    assert_eq!(stats.proximity_culled, 0);
    assert_eq!(stats.proximity_unverified, 1);
    assert_eq!(stats.projection_bounds_unverified, 1);
    assert_eq!(stats.source_class_counts, [0, 1]);
}

#[test]
fn local_light_culling_system_fails_open_without_camera_or_when_disabled() {
    use bevy::ecs::system::RunSystemOnce;

    for (enabled, add_camera) in [(true, false), (false, true)] {
        let mut world = World::new();
        world.insert_resource(LocalLightCullEnabled(enabled));
        world.insert_resource(LocalLightCullStats::default());
        if add_camera {
            add_culling_test_camera(&mut world);
        }
        let light = add_culling_test_light(
            &mut world,
            entities::LightType::Point,
            0.001,
            Some((Vec3::ZERO, LIGHT_FALLBACK_CULL_RADIUS)),
            Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0)),
        );

        world.run_system_once(cull_small_local_lights).unwrap();

        assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Visible));
        let stats = world.resource::<LocalLightCullStats>();
        assert_eq!(stats.total, 1);
        assert_eq!(stats.proximity_culled, 0);
        assert_eq!(stats.clip_culled, 0);
        assert_eq!(stats.area_culled, 0);
    }
}

#[test]
fn authored_light_angle_thresholds_match_cosine_transfer() {
    let (cutoff, falloff) = authored_light_cosines(70.0, 20.0);
    assert!((cutoff - 35.0_f32.to_radians().cos()).abs() < 0.000_001);
    assert!((falloff - 10.0_f32.to_radians().cos()).abs() < 0.000_001);

    let (cutoff, falloff) = authored_light_cosines(20.0, 70.0);
    assert!((cutoff - 10.0_f32.to_radians().cos()).abs() < 0.000_001);
    let expected_falloff = cutoff as f64 + 0.000_152_304_841_3;
    assert!((falloff as f64 - expected_falloff).abs() < 0.000_000_1);
}

#[test]
fn light_distance_uses_cutoff_range_and_falloff_start() {
    let (range, falloff_start) = light_distance(8.0, 15.0).unwrap();
    assert_eq!(range, 8.0);
    assert!((falloff_start - 7.92).abs() < 0.000_001);
    let (range, falloff_start) = light_distance(10.0, 20.0).unwrap();
    assert_eq!(range, 10.0);
    assert!((falloff_start - 9.9).abs() < 0.000_001);
    assert_eq!(light_distance(0.0, 0.0), Some((0.1, 0.0)));
    assert_eq!(light_distance(f32::NAN, 10.0), None);
}

#[test]
fn deferred_diffuse_reader_loads_installed_bc7_mip() {
    let base = crate::assets::installation::game_base().unwrap();
    if !base.is_dir() {
        return;
    }
    let catalog = archive::Catalog::open(&base).unwrap();
    let path = catalog
        .paths_containing("dirt_mud_01_dark_d2.bimage7_mip2")
        .into_iter()
        .find(|path| path.ends_with("dirt_mud_01_dark_d2.bimage7_mip2"))
        .expect("installed BC7 mip resource");
    let stem = path.strip_suffix("_mip2").unwrap();
    let inline = catalog.read(stem).unwrap().unwrap();
    let base = texture::compressed_inline_bc7_mip(&inline, 0).unwrap();
    let inline_chain = texture::compressed_inline_bc7_chain(&inline).unwrap();
    assert_eq!((base.width, base.height), (256, 256));
    let image = read_streamed_diffuse_image(&catalog, stem, 2)
        .unwrap()
        .expect("authored mip resource");
    assert_eq!(image.level.resource_mip, Some(2));
    assert_eq!(image.image.texture_descriptor.size.width, 512);
    assert_eq!(image.image.texture_descriptor.size.height, 512);
    assert_eq!(
        image.image.texture_descriptor.mip_level_count,
        1 + inline_chain.mip_count
    );
    assert!(image.image.data.is_none());
    assert_eq!(
        image
            .uploads
            .iter()
            .map(|mip| mip.payload.len())
            .sum::<usize>(),
        262_144 + inline_chain.payload.len()
    );
    let finer = read_streamed_diffuse_at_target(
        &catalog,
        stem,
        TextureReadTarget::Exact(2, texture::ImageFormat::Bc7Srgb),
    )
    .unwrap()
    .expect("single authored finer mip");
    assert_eq!(finer.uploads.len(), 1);
    assert_eq!(finer.uploads[0].payload.len(), 262_144);
    assert_eq!(finer.image.texture_descriptor.mip_level_count, 1);
}

#[test]
fn cutout_initial_residency_precedes_visibility_and_preserves_authored_chain() {
    let base = crate::assets::installation::game_base().unwrap();
    if !base.is_dir() {
        return;
    }
    let catalog = archive::Catalog::open(&base).unwrap();
    let path = catalog
        .paths_containing("dirt_mud_01_dark_d2.bimage7_mip2")
        .into_iter()
        .find(|p| p.ends_with("dirt_mud_01_dark_d2.bimage7_mip2"))
        .unwrap();
    let stem = path.strip_suffix("_mip2").unwrap().to_string();
    let image_bytes = catalog.read(&stem).unwrap().unwrap();
    let expected = texture::compressed_inline_chain(&image_bytes).unwrap();
    let mut queue = SceneRenderQueue {
        diffuse_image_paths: vec![stem],
        diffuse_image_handles: vec![None],
        diffuse_image_materials: vec![vec![]],
        diffuse_mip_states: vec![DiffuseMipState::default()],
        diffuse_image_attempted: vec![false],
        ..default()
    };
    let mut images = Assets::<Image>::default();
    let mut probes = Assets::<ProbeMaterial>::default();
    let mut decals = Assets::<DecalMaterial>::default();
    let bytes = super::texture_streaming::initialize_resident_image(
        &catalog,
        0,
        &mut queue,
        &mut images,
        &mut probes,
        &mut decals,
        &mut Assets::default(),
    )
    .unwrap();
    assert_eq!(bytes, expected.payload.len());
    let handle = queue.diffuse_image_handles[0].as_ref().unwrap().clone();
    let image = images.get(&handle).unwrap();
    assert_eq!(image.data.as_deref(), Some(expected.payload.as_slice()));
    assert_eq!(
        image.texture_descriptor.mip_level_count,
        expected.info.mip_count
    );
    assert_eq!(
        queue.diffuse_mip_states[0].minimum,
        queue.diffuse_mip_states[0].resident
    );
    assert_eq!(
        queue.diffuse_mip_states[0].resident.unwrap().resource_mip,
        None
    );
    assert!(queue.shadow_diffuse_slots.is_empty());
    assert_eq!(queue.active, 0);
    queue.request_shadow_diffuse(0);
    assert_eq!(
        super::texture_streaming::initialize_resident_image(
            &catalog,
            0,
            &mut queue,
            &mut images,
            &mut probes,
            &mut decals,
            &mut Assets::default()
        )
        .unwrap(),
        0
    );
    assert_eq!(
        queue.diffuse_image_handles[0].as_ref().unwrap().id(),
        handle.id()
    );
    assert_eq!(images.len(), 1);
}

#[test]
fn cutout_initial_residency_follows_compiled_shadow_banks() {
    use shader_cache::{CachedRigidShadowClass as Model, CachedWorldShadowClass as World};
    for class in [
        World::Opaque,
        World::AlphaTested,
        World::VistaAlpha,
        World::Vegetation,
        World::VegetationFixed,
    ] {
        let (_, mut materials) = world_shadow_test_material(class);
        let inputs = materials.0.values_mut().next().unwrap().as_mut().unwrap();
        // Ordinary alpha flags cannot replace the actual shadow fragment class.
        inputs.alpha_test = class == World::Opaque;
        assert_eq!(
            inputs.cutout_image_slot(),
            (class != World::Opaque).then_some(0)
        );
        let source = inputs.world_source.as_ref().unwrap().as_ref().unwrap();
        for class in [
            Model::Opaque,
            Model::AlphaTested,
            Model::VistaAlpha,
            Model::Vegetation,
        ] {
            let mut model = inputs.clone();
            model.world_source = Ok(None);
            model.diffuse_image_slot = Some(17);
            model.rigid_base_source = Ok(Some(Arc::new(shader_cache::CachedRigidShadowSource {
                class,
                skin_influences: 0,
                alpha_dither: false,
                program: source.program.clone(),
                variant_hash: source.variant_hash,
                definition_render_state: source.definition_render_state,
                variant: source.variant.clone(),
            })));
            assert_eq!(
                model.cutout_image_slot(),
                (class != Model::Opaque).then_some(17)
            );
        }
    }
}

#[test]
#[ignore = "requires a GPU and installed image; checks base-mip upload before first shadow demand"]
fn cutout_initial_residency_is_ready_for_late_caster_gpu() {
    use bevy::render::{
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        texture::GpuImage,
        RenderApp,
    };
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = archive::Catalog::open(&base).unwrap();
    let path = catalog
        .paths_containing("dirt_mud_01_dark_d2.bimage7_mip2")
        .into_iter()
        .find(|p| p.ends_with("dirt_mud_01_dark_d2.bimage7_mip2"))
        .unwrap();
    let stem = path.strip_suffix("_mip2").unwrap().to_string();
    let image_bytes = catalog.read(&stem).unwrap().unwrap();
    let expected = texture::compressed_inline_bc7_mip(&image_bytes, 0).unwrap();
    assert_eq!((expected.width, expected.height), (256, 256));
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(bevy::window::WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
    );
    app.finish();
    app.cleanup();
    let device = app
        .sub_app(RenderApp)
        .world()
        .resource::<RenderDevice>()
        .clone();
    let render_queue = app
        .sub_app(RenderApp)
        .world()
        .resource::<RenderQueue>()
        .clone();
    let mut queue = SceneRenderQueue {
        diffuse_image_paths: vec![stem],
        diffuse_image_handles: vec![None],
        diffuse_image_materials: vec![vec![]],
        diffuse_mip_states: vec![DiffuseMipState::default()],
        diffuse_image_attempted: vec![false],
        ..default()
    };
    super::texture_streaming::initialize_resident_image(
        &catalog,
        0,
        &mut queue,
        &mut app.world_mut().resource_mut::<Assets<Image>>(),
        &mut Assets::default(),
        &mut Assets::default(),
        &mut Assets::default(),
    )
    .unwrap();
    let handle = queue.diffuse_image_handles[0].as_ref().unwrap().clone();
    for _ in 0..4 {
        app.update();
        device.poll(PollType::wait_indefinitely()).unwrap();
    }
    let gpu = app
        .sub_app(RenderApp)
        .world()
        .resource::<RenderAssets<GpuImage>>()
        .get(handle.id())
        .unwrap();
    let texture_id = gpu.texture.id();
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 65536,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&default());
    encoder.copy_texture_to_buffer(
        gpu.texture.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &staging,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(1024),
                rows_per_image: None,
            },
        },
        Extent3d {
            width: 256,
            height: 256,
            depth_or_array_layers: 1,
        },
    );
    render_queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    staging
        .slice(..)
        .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
    device.poll(PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = staging.slice(..).get_mapped_range();
    assert_eq!(&*mapped, expected.payload);
    drop(mapped);
    staging.unmap();
    // First demand after the image was invisible for several frames reuses
    // resident data; it must not enqueue an initial read or lose its GPU view.
    queue.request_shadow_diffuse(0);
    queue.diffuse_image_pending = vec![None];
    queue.diffuse_image_demands = vec![texture::ProjectedAreaDemand::default()];
    queue.diffuse_image_last_visible = vec![0];
    app.insert_resource(queue);
    app.insert_resource(TextureReadWorker::new(catalog, 1));
    app.init_resource::<Assets<ProbeMaterial>>();
    app.init_resource::<Assets<DecalMaterial>>();
    app.add_systems(Update, stream_visible_textures);
    for _ in 0..4 {
        app.update();
        device.poll(PollType::wait_indefinitely()).unwrap();
    }
    assert_eq!(
        app.world().resource::<TextureReadWorker>().inflight_count,
        0
    );
    assert_eq!(
        app.sub_app(RenderApp)
            .world()
            .resource::<RenderAssets<GpuImage>>()
            .get(handle.id())
            .unwrap()
            .texture
            .id(),
        texture_id
    );
    assert!(app
        .world()
        .resource::<Assets<Image>>()
        .get(&handle)
        .is_some());
}

#[test]
fn visible_diffuse_is_uploaded_then_reduced_after_recorded_interval() {
    let base = crate::assets::installation::game_base().unwrap();
    if !base.is_dir() {
        return;
    }
    let catalog = archive::Catalog::open(&base).unwrap();
    let path = catalog
        .paths_containing("dirt_mud_01_dark_d2.bimage7_mip2")
        .into_iter()
        .find(|path| path.ends_with("dirt_mud_01_dark_d2.bimage7_mip2"))
        .unwrap();
    let stem = path.strip_suffix("_mip2").unwrap().to_string();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>>();
    app.insert_resource(Assets::<Image>::default());
    let mut materials = Assets::<ProbeMaterial>::default();
    let material = materials.add(ProbeMaterial {
        base: StandardMaterial::default(),
        extension: ProbeExtension {
            fog_parameters: default(),
            fog_objects: default(),
            fog_world: None,
            fog_canvas: None,
            fog_curve: None,
            fog_depth: None,
            reflections: None,
            reflection_brdf_lookup: None,
            reflection_planes: default(),
            material_exposure: Handle::default(),
            sh: Handle::default(),
            vertex_probes: Handle::default(),
            shadow: Handle::default(),
            local_shadow: Handle::default(),
            local_shadow_map: None,
            shadow_map: None,
            screen_map: None,
            cloud_map: None,
            surface: surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(&[])),
            diffuse0: None,
            diffuse1: None,
            diffuse2: None,
            bump0: None,
            bump1: None,
            bump2: None,
            gloss0: None,
            gloss1: None,
            gloss2: None,
            metallic0: None,
            metallic1: None,
            metallic2: None,
            mask0: None,
            mask1: None,
            mask2: None,
            specular0: None,
            specular1: None,
            specular2: None,
            detail_bump: None,
            emissive_map: None,
            emissive_mask: None,
            anisotropic_sampler_image: None,
            lightmap: None,
            filling_diffuse: None,
            filling_distortion1: None,
            filling_distortion2: None,
            filling_lookup: None,
            filling_normal: None,
            filling_fresnel: None,
            cloud_opacity: None,
            cloud_density: None,
            cloud_back_density: None,
            linear_clamp_image: None,
            vegetation_slots: default(),
            vegetation_instances: default(),
        },
    });
    app.insert_resource(materials);
    app.insert_resource(Assets::<DecalMaterial>::default());
    app.insert_resource(TextureReadWorker::new(catalog, 1));
    app.insert_resource(SceneRenderQueue {
        static_world_model_present: false,
        instances: vec![RenderInstance {
            source: None,
            mesh: Handle::default(),
            material: MaterialHandle::Probe(material.clone()),
            diffuse_image_slot: Some(0),
            probe_index: 0,
            transform: Transform::IDENTITY,
            bounds: render_bounds(&[Vec3::ZERO, Vec3::ONE]),
            pickup_id: None,
            entity: None,
            visible: true,
        }],
        active: 1,
        tome_render_instances: TomeRenderInstanceMap::default(),
        visible_authored_ids: Vec::new(),
        visible_cell_indices: Vec::new(),
        visible_cell_mask: Vec::new(),
        visibility_suppression_groups: Vec::new(),
        trace_bwm_lod: false,
        world_lods_by_render_instance: Vec::new(),
        last_bwm_lod_visibility_report: None,
        diffuse_image_demands: vec![texture::ProjectedAreaDemand::default()],
        diffuse_image_paths: vec![stem],
        light_query_state: None,
        shadow_ordinary_query: None,
        diffuse_image_materials: vec![vec![(
            MaterialHandle::Probe(material.clone()),
            surface_gpu::ImageRole(0),
        )]],
        material_images: HashMap::new(),
        material_stage_sorts: HashMap::new(),
        shadow_diffuse_slots: Vec::new(),
        shadow_diffuse_membership: Vec::new(),
        diffuse_image_handles: vec![None],
        diffuse_image_pending: vec![None],
        diffuse_mip_states: vec![DiffuseMipState::default()],
        diffuse_image_attempted: vec![false],
        diffuse_image_last_visible: vec![0],
        texture_stream_frame: 0,
        texture_viewport_pixels: 0,
        diffuse_mip_upgrades: 0,
        preferred_mip: None,
    });
    app.add_systems(Update, stream_visible_textures);
    for _ in 0..1000 {
        app.update();
        if app
            .world()
            .resource::<SceneRenderQueue>()
            .diffuse_image_handles[0]
            .is_some()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let image = app
        .world()
        .resource::<SceneRenderQueue>()
        .diffuse_image_handles[0]
        .clone()
        .expect("visible image staged");
    assert_eq!(
        app.world()
            .resource::<SceneRenderQueue>()
            .diffuse_mip_states[0]
            .resident
            .unwrap()
            .resource_mip,
        None
    );
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&image)
            .unwrap()
            .texture_descriptor
            .size
            .width,
        256
    );
    assert!(app
        .world()
        .resource::<Assets<Image>>()
        .get(&image)
        .is_some());
    assert!(app
        .world()
        .resource::<Assets<ProbeMaterial>>()
        .get(&material)
        .unwrap()
        .base
        .base_color_texture
        .is_some());

    {
        let mut queue = app.world_mut().resource_mut::<SceneRenderQueue>();
        assert_eq!(
            queue.diffuse_mip_states[0]
                .resident
                .unwrap()
                .next_separate_suffix,
            Some(2)
        );
        queue.texture_viewport_pixels = 1920 * 1080;
        queue.diffuse_image_demands[0].submit(1.0);
    }
    for _ in 0..1000 {
        app.update();
        if app
            .world()
            .resource::<SceneRenderQueue>()
            .diffuse_mip_states[0]
            .resident
            .is_some_and(|level| level.resource_mip == Some(2))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let finer = app
        .world()
        .resource::<SceneRenderQueue>()
        .diffuse_image_handles[0]
        .clone()
        .expect("finer authored mip staged");
    assert_eq!(
        app.world()
            .resource::<SceneRenderQueue>()
            .diffuse_mip_upgrades,
        1
    );
    assert_ne!(finer.id(), image.id());
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&finer)
            .unwrap()
            .texture_descriptor
            .size
            .width,
        512
    );
    assert!(app
        .world()
        .resource::<Assets<Image>>()
        .get(&image)
        .is_none());

    app.world_mut().resource_mut::<SceneRenderQueue>().instances[0].visible = false;
    for _ in 0..=TEXTURE_REDUCE_AFTER_FRAMES {
        app.update();
    }
    let reduced = app
        .world()
        .resource::<SceneRenderQueue>()
        .diffuse_image_handles[0]
        .clone()
        .expect("minimum authored mip remains resident");
    assert_eq!(
        app.world()
            .resource::<SceneRenderQueue>()
            .diffuse_mip_states[0]
            .resident
            .unwrap()
            .resource_mip,
        None
    );
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&reduced)
            .unwrap()
            .texture_descriptor
            .size
            .width,
        256
    );
    assert!(app
        .world()
        .resource::<Assets<Image>>()
        .get(&finer)
        .is_none());
    assert!(app
        .world()
        .resource::<Assets<ProbeMaterial>>()
        .get(&material)
        .unwrap()
        .base
        .base_color_texture
        .is_some());

    let late_image = Image::new(
        Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; 16],
        TextureFormat::Bc7RgbaUnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    {
        let mut worker = app.world_mut().resource_mut::<TextureReadWorker>();
        worker.inflight[0] = true;
        worker.inflight_count = 1;
        worker.ready.push_back((
            0,
            TextureReadTarget::Initial(Some(2)),
            Ok(Some(StreamedDiffuse {
                image: late_image,
                uploads: Vec::new(),
                level: DiffuseMipLevel {
                    resource_mip: Some(2),
                    next_separate_suffix: Some(1),
                    width: 4,
                    height: 4,
                    format: texture::ImageFormat::Bc7Srgb,
                },
            })),
        ));
    }
    app.world_mut()
        .resource_mut::<SceneRenderQueue>()
        .diffuse_image_attempted[0] = true;
    app.update();
    let queue = app.world().resource::<SceneRenderQueue>();
    assert!(queue.diffuse_image_handles[0].is_some());
    assert!(queue.diffuse_image_attempted[0]);
    assert_eq!(
        app.world().resource::<TextureReadWorker>().inflight_count,
        0
    );
}

#[test]
#[ignore = "requires local original-game EOTW Tome fixture"]
fn roof_point_diagnostic_reports_authored_tome_records() {
    let bytes =
        fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
    let tree = tome::SpatialTree::parse(&bytes).unwrap();
    let roof_bounds = tome::object_bounds(&bytes, 28684).unwrap();
    assert_eq!(
        roof_bounds,
        [
            [-53.679977, 132.2928, 42.5],
            [-32.959953, 145.30208, 47.52765],
        ]
    );
    let gate_states = tome::initial_gate_states(&bytes).unwrap();
    let query_data = tome::DirectTomeQueryData::parse(&bytes, &tree).unwrap();
    let tome_world = TomeWorld {
        sphere_graph: tome::SphereCellGraph::parse(&bytes),
        bytes,
        tree,
        gate_states,
        query_data,
        query_job_count: tome::DEFAULT_QUERY_JOB_COUNT,
        query_pool: None,
    };
    let empty_world = world_mesh::WorldMesh {
        meshes: Vec::new(),
        matrices: Vec::new(),
        visibility: Vec::new(),
        auxiliary_instance_groups: std::array::from_fn(|_| Vec::new()),
        visibility_groups: Vec::new(),
        tome_object_instances: Vec::new(),
        spatial_query_instances: Vec::new(),
        objects: Vec::new(),
    };
    let render_queue = SceneRenderQueue {
        static_world_model_present: false,
        light_query_state: None,
        shadow_ordinary_query: None,
        shadow_diffuse_slots: Vec::new(),
        shadow_diffuse_membership: Vec::new(),
        instances: Vec::new(),
        active: 0,
        tome_render_instances: TomeRenderInstanceMap::default(),
        visible_authored_ids: Vec::new(),
        visible_cell_indices: Vec::new(),
        visible_cell_mask: Vec::new(),
        visibility_suppression_groups: Vec::new(),
        trace_bwm_lod: false,
        world_lods_by_render_instance: Vec::new(),
        last_bwm_lod_visibility_report: None,
        diffuse_image_demands: Vec::new(),
        diffuse_image_paths: Vec::new(),
        diffuse_image_materials: Vec::new(),
        material_images: HashMap::new(),
        material_stage_sorts: HashMap::new(),
        diffuse_image_handles: Vec::new(),
        diffuse_image_pending: Vec::new(),
        diffuse_mip_states: Vec::new(),
        diffuse_image_attempted: Vec::new(),
        diffuse_image_last_visible: Vec::new(),
        texture_stream_frame: 0,
        texture_viewport_pixels: 0,
        diffuse_mip_upgrades: 0,
        preferred_mip: Some(3),
    };
    let point_leaves = tome_world
        .tree
        .record_leaves_containing_point([-43.0, 138.0, 44.0]);
    let point_text = tome_point_text(
        &tome_world,
        &empty_world,
        &render_queue,
        "eye",
        [-43.0, 138.0, 44.0],
    );
    assert!(point_text.contains(&format!(
        "point-containment prefilter touches {} authored spatial records",
        point_leaves.len()
    )));
    for leaf in point_leaves {
        assert!(point_text.contains(&format!("point-containing spatial record {leaf}")));
    }
}

#[test]
#[ignore = "requires local original-game EOTW Tome fixture"]
fn parallel_tome_dispatch_matches_serial_jobs() {
    let bytes =
        fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
    bevy::tasks::ComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
    let tree = tome::SpatialTree::parse(&bytes).unwrap();
    let query_data = tome::DirectTomeQueryData::parse(&bytes, &tree).unwrap();
    let gate_states = tome::initial_gate_states(&bytes).unwrap();
    let mut world = TomeWorld {
        sphere_graph: tome::SphereCellGraph::parse(&bytes),
        bytes,
        tree,
        gate_states,
        query_data,
        query_job_count: 1,
        query_pool: None,
    };
    // User capture frame 1164: dense street, ~5,000 mapped render instances.
    let pose = [
        [-0.72593063, 0.01375436, -0.6876305, -36.467716],
        [-0.68776804, -0.014517557, 0.7257855, 192.6837],
        [4.656613e-10, 0.9998, 0.019998547, 5.6344576],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let rows =
        tome::world_visibility_query_matrix(80.0, 1920, 1017, 1.0, pose, 0.01, 7000.0).unwrap();
    for (count, threads) in [
        (1, 0),
        (2, 0),
        (3, 0),
        (4, 0),
        (12, 0),
        (16, 0),
        (12, 4),
        (12, 6),
        (12, 12),
    ] {
        world.query_job_count = count;
        world.query_pool = (threads > 0).then(|| {
            bevy::tasks::TaskPoolBuilder::new()
                .num_threads(threads)
                .thread_name("Tome Query Test".into())
                .build()
        });
        let input = tome::OrdinaryDirectQueryInput {
            camera_point: [pose[0][3], pose[1][3], pose[2][3]],
            render_rows: rows,
            cropped_rows: rows,
            initial_tile_rect: [0, 0, 8, 16],
            camera_imprecision: [0.6; 3],
            object_imprecision: 0.6,
            minimum_area_metric: 25.0 / (1920.0 * 1017.0),
            global_distance_threshold: tome::DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
            object_distance_scale: 1.0,
            winding_flip: tome::query_matrix_winding_flip(rows),
            gate_states: Some(&world.gate_states),
            output_capacity: tome::WORLD_QUERY_OUTPUT_CAPACITY,
        };
        let serial: Vec<_> = (0..count)
            .map(|job| {
                tome::ordinary_direct_tome_render_job(
                    &world.tree,
                    &world.bytes,
                    &world.query_data,
                    &input,
                    job,
                    count,
                )
            })
            .collect();
        let started = Instant::now();
        let parallel = run_tome_render_jobs(&world, &input);
        let mut elapsed = started.elapsed();
        let samples = if count == tome::DEFAULT_QUERY_JOB_COUNT {
            20
        } else {
            1
        };
        for _ in 1..samples {
            let started = Instant::now();
            let sample = run_tome_render_jobs(&world, &input);
            elapsed += started.elapsed();
            for (sample, reference) in sample.iter().zip(&parallel) {
                let sample = sample.result.as_ref().unwrap().as_ref().unwrap();
                let reference = reference.result.as_ref().unwrap().as_ref().unwrap();
                assert_eq!(sample.indices(), reference.indices());
                assert_eq!(sample.area_metrics(), reference.area_metrics());
            }
        }
        println!(
            "headless dispatch jobs={count} threads={threads} samples={samples} mean_wall_ms={:.3}",
            elapsed.as_secs_f64() * 1000.0 / samples as f64
        );
        assert_eq!(parallel.len(), count as usize);
        for (parallel, serial) in parallel.iter().zip(&serial) {
            assert_eq!(parallel.diagnostics.job_index, serial.diagnostics.job_index);
            assert_eq!(parallel.diagnostics.viewport, serial.diagnostics.viewport);
            let parallel = parallel.result.as_ref().unwrap().as_ref().unwrap();
            let serial = serial.result.as_ref().unwrap().as_ref().unwrap();
            assert_eq!(parallel.indices(), serial.indices());
            assert_eq!(parallel.area_metrics(), serial.area_metrics());
        }
        let merged = tome::merge_render_query_outputs(&parallel)
            .unwrap()
            .unwrap();
        let serial = tome::merge_render_query_outputs(&serial).unwrap().unwrap();
        let diagnostics = parallel
            .into_iter()
            .map(|job| job.diagnostics)
            .collect::<Vec<_>>();
        let mut cache = TomeRenderQueryCache::default();
        cache.store(
            TomeRenderQueryKey::new(&input, count),
            &Ok(Some(merged.clone())),
            &diagnostics,
        );
        assert!(cache.matches(&TomeRenderQueryKey::new(&input, count), false));
        let cached = cache.output.as_ref().unwrap();
        assert_eq!(cached.indices(), merged.indices());
        assert_eq!(
            cached
                .area_metrics()
                .iter()
                .map(|value| value.map(f32::to_bits))
                .collect::<Vec<_>>(),
            merged
                .area_metrics()
                .iter()
                .map(|value| value.map(f32::to_bits))
                .collect::<Vec<_>>()
        );
        assert_eq!(cached.cell_indices(), merged.cell_indices());
        assert_eq!(cached.state, merged.state);
        assert_eq!(merged.indices(), serial.indices());
        assert_eq!(merged.area_metrics(), serial.area_metrics());
        assert_eq!(merged.state, serial.state);
        assert_eq!(merged.state.as_ref().unwrap().viewport, [0, 0, 64, 64]);
        assert_eq!(merged.state.as_ref().unwrap().cropped_rows, rows);
        assert!(!merged.indices().is_empty());
        // Exercise the downstream byte-style visibility map with the
        // duplicate indices preserved across jobs.
        let max_index = *merged.indices().iter().max().unwrap() as usize;
        let mapping: Vec<_> = (0..=max_index).map(|i| vec![i]).collect();
        let map = TomeRenderInstanceMap::from_authored_bwm_mapping(&mapping, &mapping);
        let mut visible = vec![false; mapping.len()];
        map.mark_visible_instances(merged.indices(), &mut visible)
            .unwrap();
        assert!(visible.iter().any(|visible| *visible));
    }
}

#[test]
#[ignore = "requires local original-game EOTW Tome and entities fixtures"]
fn eotw_player_start_direct_tome_query_accepts_authored_objects() {
    let tome_bytes =
        fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
    let entity_bytes = fs::read("artifacts/eotw_p_lowchaos.entities")
        .expect("missing local EOTW entities fixture");
    let document = entities::parse(&String::from_utf8_lossy(&entity_bytes)).unwrap();
    let start = entities::player_start(&document).unwrap();
    let camera = view_of(&FocusTarget {
        center: game_to_view(start.position),
        distance: 10.0,
        label: "player start".into(),
        spawn: Some(start),
    });
    let point = view_to_game(camera.translation);
    let rows = tome::world_visibility_query_matrix(
        DEFAULT_HORIZONTAL_FOV_DEGREES,
        1920,
        1080,
        1.0,
        game_pose_from_view_transform(&camera),
        DEFAULT_NEAR,
        DEFAULT_FAR,
    )
    .unwrap();
    let gates = tome::initial_gate_states(&tome_bytes).unwrap();
    let input = tome::OrdinaryDirectQueryInput {
        camera_point: point,
        render_rows: rows,
        cropped_rows: rows,
        initial_tile_rect: [0, 0, 8, 16],
        camera_imprecision: [0.6; 3],
        global_distance_threshold: tome::DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
        object_distance_scale: 1.0,
        object_imprecision: 0.6,
        minimum_area_metric: 25.0 / (1920.0 * 1080.0),
        winding_flip: tome::query_matrix_winding_flip(rows),
        gate_states: Some(&gates),
        output_capacity: 32768,
    };
    let tree = tome::SpatialTree::parse(&tome_bytes).unwrap();
    let output = tome::ordinary_direct_tome_visible_objects(&tree, &tome_bytes, &input)
        .unwrap()
        .expect("player start is inside the authored Tome");
    let mut counts = vec![output.count()];
    for degrees in [90.0_f32, 180.0, 270.0] {
        let mut rotated = camera.clone();
        rotated.rotation = Quat::from_rotation_y(degrees.to_radians()) * camera.rotation;
        let rows = tome::world_visibility_query_matrix(
            DEFAULT_HORIZONTAL_FOV_DEGREES,
            1920,
            1080,
            1.0,
            game_pose_from_view_transform(&rotated),
            DEFAULT_NEAR,
            DEFAULT_FAR,
        )
        .unwrap();
        let input = tome::OrdinaryDirectQueryInput {
            render_rows: rows,
            cropped_rows: rows,
            winding_flip: tome::query_matrix_winding_flip(rows),
            ..input
        };
        let result = tome::ordinary_direct_tome_visible_objects(&tree, &tome_bytes, &input);
        counts.push(result.unwrap().unwrap().count());
    }
    // Rewrite query regression counts after native outward pixel coverage.
    // The installed pixel/distance predicates have separate native fixtures.
    assert_eq!(counts, [1596, 1068, 235, 131]);
    assert_eq!(&output.indices()[..4], &[17564, 22092, 22095, 22098]);
}

#[test]
#[ignore = "requires local original-game EOTW Tome fixture"]
fn reported_street_position_tome_query_regression() {
    let bytes =
        fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
    let tree = tome::SpatialTree::parse(&bytes).unwrap();
    let gates = tome::initial_gate_states(&bytes).unwrap();
    let point = [-38.43, 175.76, 4.91];
    let eye = game_to_view(point);
    for direction in [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z] {
        let camera = Transform::from_translation(eye).looking_at(eye + direction, Vec3::Y);
        let rows = tome::world_visibility_query_matrix(
            DEFAULT_HORIZONTAL_FOV_DEGREES,
            1362,
            722,
            1.0,
            game_pose_from_view_transform(&camera),
            DEFAULT_NEAR,
            DEFAULT_FAR,
        )
        .unwrap();
        let output = tome::ordinary_direct_tome_visible_objects(
            &tree,
            &bytes,
            &tome::OrdinaryDirectQueryInput {
                camera_point: point,
                render_rows: rows,
                cropped_rows: rows,
                initial_tile_rect: [0, 0, 8, 16],
                camera_imprecision: [0.6; 3],
                global_distance_threshold: tome::DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
                object_distance_scale: 1.0,
                object_imprecision: 0.6,
                minimum_area_metric: 25.0 / (1362.0 * 722.0),
                winding_flip: tome::query_matrix_winding_flip(rows),
                gate_states: Some(&gates),
                output_capacity: 32768,
            },
        )
        .unwrap()
        .expect("reported street position is inside the Tome");
        assert!(
            output.count() > 0,
            "direction {direction:?} returned no world objects"
        );
        if direction == Vec3::X {
            // Installed FUN_140a82710 Tome-query captures include 22108
            // and 17564 but reject 22109 for this camera. This is a
            // visibility-set fixture, not evidence of an LOD-tier rule.
            for index in [22108, 17564] {
                assert!(
                    output.indices().contains(&index),
                    "nearby world object {index} missing"
                );
            }
            assert!(!output.indices().contains(&22109));
        }
    }
}

#[test]
#[ignore = "requires local original-game EOTW BWM/Tome fixtures and installed executable"]
fn dense_depth_grid_matches_bounds_after_camera_shifts() {
    bevy::tasks::ComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
    let bytes = fs::read("artifacts/eotw_p_lowchaos.tome").unwrap();
    let tree = tome::SpatialTree::parse(&bytes).unwrap();
    let query_data = tome::DirectTomeQueryData::parse(&bytes, &tree).unwrap();
    let world = TomeWorld {
        sphere_graph: tome::SphereCellGraph::parse(&bytes),
        gate_states: tome::initial_gate_states(&bytes).unwrap(),
        bytes,
        tree,
        query_data,
        query_job_count: tome::DEFAULT_QUERY_JOB_COUNT,
        query_pool: None,
    };
    let pose = [
        [-0.72593063, 0.01375436, -0.6876305, -36.467716],
        [-0.68776804, -0.014517557, 0.7257855, 192.6837],
        [4.656613e-10, 0.9998, 0.019998547, 5.6344576],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let base = Mat4::from_cols_array_2d(&std::array::from_fn(|column| {
        std::array::from_fn(|row| pose[row][column])
    }));
    let bwm = world_mesh::parse(&fs::read("artifacts/eotw_p_lowchaos.bwm").unwrap()).unwrap();
    let mut bwm_instances = vec![Vec::new(); bwm.visibility.len()];
    let mut triangles = Vec::new();
    for object in &bwm.objects {
        for instances in &mut bwm_instances[object.matrix_start..object.matrix_end] {
            instances.push(triangles.len());
            triangles.push(bwm.meshes[object.mesh_index].indices.len() / 3);
        }
    }
    let mapping = TomeRenderInstanceMap::from_world_mapping(&bwm, &bwm_instances);
    let groups: Vec<_> = bwm
        .visibility_groups
        .iter()
        .map(|group| RenderSuppressionGroup {
            parent: bwm_instances[group.parent].clone(),
            children: group
                .children
                .iter()
                .flat_map(|&index| bwm_instances[index].iter().copied())
                .collect(),
        })
        .collect();
    for yaw in [-0.0005, 0.0, 0.0005] {
        let columns = (base * Mat4::from_rotation_y(yaw)).to_cols_array_2d();
        let pose = std::array::from_fn(|row| std::array::from_fn(|column| columns[column][row]));
        let rows =
            tome::world_visibility_query_matrix(80.0, 1920, 1017, 1.0, pose, 0.01, 7000.0).unwrap();
        let input = tome::OrdinaryDirectQueryInput {
            camera_point: [pose[0][3], pose[1][3], pose[2][3]],
            render_rows: rows,
            cropped_rows: rows,
            initial_tile_rect: [0, 0, 8, 16],
            camera_imprecision: [0.6; 3],
            object_imprecision: 0.6,
            minimum_area_metric: 25.0 / (1920.0 * 1017.0),
            global_distance_threshold: tome::DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
            object_distance_scale: 1.0,
            winding_flip: tome::query_matrix_winding_flip(rows),
            gate_states: Some(&world.gate_states),
            output_capacity: tome::WORLD_QUERY_OUTPUT_CAPACITY,
        };
        let started = std::time::Instant::now();
        let jobs = run_tome_render_jobs(&world, &input);
        let output = tome::merge_render_query_outputs(&jobs).unwrap().unwrap();
        let query_ms = started.elapsed().as_secs_f64() * 1000.0;
        let state = output.state.as_ref().unwrap();
        let query = tome::RenderAabbQuery::from_state(state).unwrap();
        let mut old_state = state.clone();
        old_state.depth_grid = None;
        let old_query = tome::RenderAabbQuery::from_state(&old_state).unwrap();
        let mut workloads = Vec::new();
        for candidate in [&old_state, state] {
            let mut visible = vec![false; triangles.len()];
            mapping
                .mark_visible_instances(output.indices(), &mut visible)
                .unwrap();
            let accepted = mapping
                .apply_spatial_queries(Some(candidate), &mut visible)
                .unwrap();
            RenderSuppressionGroup::apply_all(&groups, &mut visible);
            let surfaces = visible.iter().filter(|&&value| value).count();
            let geometry: usize = visible
                .iter()
                .zip(&triangles)
                .filter_map(|(&value, &count)| value.then_some(count))
                .sum();
            workloads.push((accepted, surfaces, geometry));
        }
        println!(
            "dense yaw={yaw} query_ms={query_ms:.3} spatial/mapped_surfaces/mapped_triangles before={:?} after={:?}",
            workloads[0], workloads[1]
        );
        let mut old_count = 0;
        let mut depth_count = 0;
        // World object output and later spatial AABB output are different
        // native lists. Original764c0 also rejects world object7268 here;
        // compare every bound with native rather than forcing a superset.
        let mut raw = b"DH2DQ001".to_vec();
        for value in state.viewport {
            raw.extend_from_slice(&value.to_le_bytes());
        }
        let unused_bounds = [0.0_f32; 6];
        for value in state
            .render_rows
            .iter()
            .flatten()
            .chain(state.cropped_rows.iter().flatten())
            .chain(state.camera_imprecision.iter())
            .chain(std::iter::once(&state.maximum_depth))
            .chain(unused_bounds.iter())
            .chain(state.depth_grid.as_ref().unwrap().iter())
        {
            raw.extend_from_slice(&value.to_le_bytes());
        }
        raw.extend_from_slice(&123.0_f32.to_le_bytes());
        raw.extend_from_slice(&(world.query_data.object_count() as u32).to_le_bytes());
        for index in 0..world.query_data.object_count() as u32 {
            let bounds = tome::object_bounds(&world.bytes, index).unwrap();
            let old = old_query.visible(bounds).unwrap();
            let (depth, area) = query.visibility_and_area(bounds).unwrap();
            assert!(
                !depth || old,
                "depth admitted outside-frustum object{index} yaw={yaw}"
            );
            old_count += usize::from(old);
            depth_count += usize::from(depth);
            raw.extend_from_slice(&index.to_le_bytes());
            for value in bounds.iter().flatten() {
                raw.extend_from_slice(&value.to_le_bytes());
            }
            raw.extend_from_slice(&u32::from(depth).to_le_bytes());
            raw.extend_from_slice(&area.to_le_bytes());
        }
        let fixture = "artifacts/dense-depth-query-state.bin";
        fs::write(fixture, raw).unwrap();
        let game = std::process::Command::new("python")
            .args(["analysis/probe_original_depth_aabb.py", "--state", fixture])
            .output()
            .unwrap();
        assert!(
            game.status.success(),
            "AABB parity yaw={yaw}: {}{}",
            String::from_utf8_lossy(&game.stdout),
            String::from_utf8_lossy(&game.stderr)
        );
        print!("{}", String::from_utf8_lossy(&game.stdout));
        assert!(
            depth_count < old_count,
            "dense camera did not reject any extra bounds yaw={yaw}"
        );
        println!(
            "dense yaw={yaw} frustum_bounds={old_count} depth_bounds={depth_count} accepted_objects={} grid_max={}",
            output.indices().len(),
            state.maximum_depth
        );
    }
}

#[test]
#[ignore = "requires local original-game EOTW BWM and Tome fixtures"]
fn authored_distance_tiers_dispatch_far_bwm_surfaces() {
    let world_bytes =
        fs::read("artifacts/eotw_p_lowchaos.bwm").expect("missing local EOTW BWM fixture");
    let tome_bytes =
        fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
    let world = world_mesh::parse(&world_bytes).unwrap();
    let mut bwm_instances = vec![Vec::new(); world.visibility.len()];
    let mut lods = Vec::new();
    let mut triangles = Vec::new();
    for object in &world.objects {
        for instances in &mut bwm_instances[object.matrix_start..object.matrix_end] {
            instances.push(lods.len());
            lods.push(object.lod);
            triangles.push(world.meshes[object.mesh_index].indices.len() / 3);
        }
    }
    let mapping = TomeRenderInstanceMap::from_world_mapping(&world, &bwm_instances);
    // Optional capture from the previous executable and installed predicate;
    // these counts describe mapped geometry, not GPU time or draw calls.
    if let Ok(cases) = fs::read_to_string("artifacts/object-distance-render-work-inputs.txt") {
        let mut visible = vec![false; lods.len()];
        for line in cases.lines() {
            let (label, indices) = line.split_once('=').unwrap();
            let indices: Vec<u32> = indices
                .split(',')
                .map(|value| value.parse().unwrap())
                .collect();
            visible.fill(false);
            mapping
                .mark_visible_instances(&indices, &mut visible)
                .unwrap();
            let mut surface_count = 0;
            let mut triangle_count = 0;
            for (surface, &visible) in visible.iter().enumerate() {
                if visible && !(label.starts_with("before") && lods[surface] > 0) {
                    surface_count += 1;
                    triangle_count += triangles[surface];
                }
            }
            eprintln!(
                "DISTANCE_RENDER_WORK {label} mapped_surfaces={surface_count} mapped_triangles={triangle_count}"
            );
        }
    }
    let mut far_groups = 0;
    let mut mixed_groups = 0;
    for index in 0..world.tome_object_instances.len() {
        let surfaces = mapping.get(index);
        let has_far = surfaces.iter().any(|&surface| lods[surface] > 0);
        if has_far {
            assert!(tome::object_distance_bounds(&tome_bytes, index as u32)
                .unwrap()
                .is_some());
            far_groups += 1;
            mixed_groups += usize::from(surfaces.iter().any(|&surface| lods[surface] == 0));
        }
    }
    eprintln!("AUTHORED_BWM_DISTANCE_GROUPS far={far_groups} mixed={mixed_groups}");
    assert!(far_groups > 0);
    assert_eq!(mixed_groups, 0);
    let roof_bounds = tome::object_distance_bounds(&tome_bytes, 28684)
        .unwrap()
        .unwrap();
    for (point, index) in [
        ([-43.0, 138.0, 48.0], 28684),
        ([-43.0, 138.0, roof_bounds[1][2] + 20.0], 28685),
        ([-43.0, 138.0, 100.0], 28686),
    ] {
        let bounds = tome::object_distance_bounds(&tome_bytes, index)
            .unwrap()
            .unwrap();
        assert_eq!(
            tome::object_distance_rejection_mask(bounds, point, [1.0; 4]),
            0
        );
        let expected = mapping.get(index as usize);
        assert!(!expected.is_empty());
        let mut visible = vec![false; lods.len()];
        mapping
            .mark_visible_instances(&[index], &mut visible)
            .unwrap();
        let actual: Vec<usize> = visible
            .iter()
            .enumerate()
            .filter_map(|(surface, &visible)| visible.then_some(surface))
            .collect();
        let mut expected = expected.to_vec();
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(actual, expected);
        eprintln!(
            "AUTHORED_ROOF_DISTANCE_GROUP index={index} lods={:?}",
            expected
                .iter()
                .map(|&surface| lods[surface])
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn bwm_bounds_use_authored_game_coordinates() {
    let record = world_mesh::WorldVisibility {
        bounds: [[-3.0, 4.0, -2.0], [5.0, 9.0, 7.0]],
        umbra_id: 42,
        trailing_field: 0xffff,
    };
    let bounds = authored_world_bounds(&record);
    assert_eq!(Vec3::from(bounds.min()), Vec3::new(-3.0, -2.0, -9.0));
    assert_eq!(Vec3::from(bounds.max()), Vec3::new(5.0, 7.0, -4.0));
}

#[test]
fn unmapped_object_has_no_invented_frustum_visibility() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(PickupState {
        items: Vec::new(),
        world: runtime::WorldState::from_pickups(&[]).unwrap(),
    });
    app.insert_resource(SceneRenderQueue {
        static_world_model_present: false,
        instances: vec![RenderInstance {
            source: None,
            mesh: Handle::default(),
            material: MaterialHandle::Probe(Handle::default()),
            diffuse_image_slot: None,
            probe_index: 0,
            transform: Transform::IDENTITY,
            bounds: render_bounds(&[Vec3::new(-2.0, 0.0, -200.0), Vec3::new(2.0, 3.0, -200.0)]),
            pickup_id: None,
            entity: None,
            visible: false,
        }],
        active: 0,
        tome_render_instances: TomeRenderInstanceMap::default(),
        visible_authored_ids: Vec::new(),
        visible_cell_indices: Vec::new(),
        visible_cell_mask: Vec::new(),
        visibility_suppression_groups: Vec::new(),
        trace_bwm_lod: false,
        world_lods_by_render_instance: Vec::new(),
        last_bwm_lod_visibility_report: None,
        diffuse_image_demands: Vec::new(),
        diffuse_image_paths: Vec::new(),
        diffuse_image_materials: Vec::new(),
        material_images: HashMap::new(),
        material_stage_sorts: HashMap::new(),
        shadow_diffuse_slots: Vec::new(),
        shadow_diffuse_membership: Vec::new(),
        diffuse_image_handles: Vec::new(),
        diffuse_image_pending: Vec::new(),
        diffuse_mip_states: Vec::new(),
        diffuse_image_attempted: Vec::new(),
        diffuse_image_last_visible: Vec::new(),
        texture_stream_frame: 0,
        texture_viewport_pixels: 0,
        diffuse_mip_upgrades: 0,
        preferred_mip: Some(3),
        light_query_state: None,
        shadow_ordinary_query: None,
    });
    app.add_systems(Update, stream_visible_models);
    for camera_count in [0, 2, 1] {
        let cameras: Vec<_> = (0..camera_count)
            .map(|_| app.world_mut().spawn(Camera3d::default()).id())
            .collect();
        {
            let mut scene = app.world_mut().resource_mut::<SceneRenderQueue>();
            scene.visible_authored_ids = vec![123];
            scene.visible_cell_indices = vec![0];
            scene.visible_cell_mask = vec![1];
            scene.light_query_state = Some(light_test_query_state());
        }
        app.update();
        let scene = app.world().resource::<SceneRenderQueue>();
        assert_eq!(scene.active, 0);
        assert!(scene.visible_authored_ids.is_empty());
        assert!(scene.visible_cell_indices.is_empty());
        assert!(scene.visible_cell_mask.is_empty());
        assert!(scene.light_query_state.is_none());
        for camera in cameras {
            app.world_mut().despawn(camera);
        }
    }
}

#[test]
fn tome_object_mapping_resolves_authored_bwm_indices_to_render_instances() {
    let authored = vec![vec![2, 0, 2], vec![1]];
    let rendered = vec![vec![4, 5], vec![6], vec![7, 8, 9]];
    let map = TomeRenderInstanceMap::from_authored_bwm_mapping(&authored, &rendered);
    assert_eq!(map.get(0), &[7, 8, 9, 4, 5, 7, 8, 9]);
    assert_eq!(map.get(1), &[6]);
    assert!(map.get(2).is_empty());
}

#[test]
fn parent_visibility_suppresses_authored_child_instances() {
    let groups = [RenderSuppressionGroup {
        parent: vec![1],
        children: vec![2, 3],
    }];
    let mut visible = [true, false, true, true];
    RenderSuppressionGroup::apply_all(&groups, &mut visible);
    assert_eq!(visible, [true, false, true, true]);
    visible[1] = true;
    RenderSuppressionGroup::apply_all(&groups, &mut visible);
    assert_eq!(visible, [true, true, false, false]);
}

#[test]
fn successful_tome_results_mark_each_authored_bwm_render_instance() {
    let map = TomeRenderInstanceMap::from_authored_bwm_mapping(
        &[vec![2, 0, 2], vec![], vec![1]],
        &[vec![4], vec![5, 6], vec![7, 8]],
    );
    let mut visible = vec![false; 9];
    map.mark_visible_instances(&[0, 1, 2], &mut visible)
        .unwrap();
    assert_eq!(
        visible,
        [false, false, false, false, true, true, true, true, true]
    );
    assert!(map.mark_visible_instances(&[99], &mut visible).is_err());
    assert!(map.mark_visible_instances(&[0], &mut visible[..8]).is_err());
}

#[test]
#[ignore = "requires installed original image indices/resources"]
fn installed_image_source_aliases_load_lod_and_constant_maps() {
    let catalog =
        archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let cases = [
        (
            "models/environment/vs_map/nature/roots_01/roots_lod2_green_d$nomips",
            "generated/image/models/environment/vs_map/nature/roots_01/roots_lod2_green_d$nomips.tga.bimage7",
        ),
        (
            "models/environment/vs_map/nature/roots_01/roots_lod2_n.tga$nomips",
            "generated/image/models/environment/vs_map/nature/roots_01/roots_lod2_n.tga$nomips.bimage7",
        ),
        (
            "models/environment/vs_map/nature/roots_01/roots_lod2_g.tga$nomips",
            "generated/image/models/environment/vs_map/nature/roots_01/roots_lod2_g.tga$nomips.bimage7",
        ),
        (
            "ipr_constantcolor(0.0745098, 0.0980392, 0.12549, 1)",
            "generated/image/ipr_constantcolor__0.0745098__0.0980392__0.12549__1_.bimage7",
        ),
        (
            "ipr_constantcolor(0.501961, 0.501961, 0, 1)",
            "generated/image/ipr_constantcolor__0.501961__0.501961__0__1_.bimage7",
        ),
        (
            "ipr_constantcolor(0, 0, 0, 1)",
            "generated/image/ipr_constantcolor__0__0__0__1_.bimage7",
        ),
        (
            "ipr_constantcolor(0.6, 0.6, 0.6, 1)",
            "generated/image/ipr_constantcolor__0.6__0.6__0.6__1_.bimage7",
        ),
    ];
    for (source, expected) in cases {
        let stem = material::compiled_image_stem(&catalog, source).unwrap();
        assert_eq!(stem, expected, "{source}");
        let bytes = catalog.read(&stem).unwrap().unwrap();
        let chain = texture::compressed_inline_chain(&bytes).unwrap();
        assert!(!chain.payload.is_empty(), "{source}");
    }
}

#[test]
fn accepted_tome_area_metrics_reach_mapped_diffuse_images() {
    let map = TomeRenderInstanceMap::from_authored_bwm_mapping(
        &[vec![0, 1], vec![1]],
        &[vec![0], vec![1]],
    );
    let instances = [0, 1].map(|diffuse_image_slot| RenderInstance {
        source: None,
        mesh: Handle::default(),
        material: MaterialHandle::Probe(Handle::default()),
        diffuse_image_slot: Some(diffuse_image_slot),
        probe_index: 0,
        transform: Transform::IDENTITY,
        bounds: render_bounds(&[Vec3::ZERO, Vec3::ONE]),
        pickup_id: None,
        entity: None,
        visible: false,
    });
    let demands = [
        texture::ProjectedAreaDemand::default(),
        texture::ProjectedAreaDemand::default(),
    ];
    assert_eq!(
        map.submit_texture_demands(&[0, 1], &[Some(0.25), Some(0.5)], &instances, &demands,)
            .unwrap(),
        3
    );
    assert_eq!(demands[0].take(), 0.25);
    assert_eq!(demands[1].take(), 0.5);
    assert!(map
        .submit_texture_demands(&[2], &[Some(1.0)], &instances, &demands)
        .is_err());
    // A bad later object must retain the earlier object's submissions.
    assert!(map
        .submit_texture_demands(&[0, 2], &[Some(0.75), Some(1.0)], &instances, &demands,)
        .is_err());
    assert_eq!(demands[0].take(), 0.75);
    assert_eq!(demands[1].take(), 0.75);

    // All shader image roles share the same maximum, including repeated
    // associations and unordered metrics; retain logical submission counts.
    let mut bindings = HashMap::new();
    bindings.insert(instances[0].material.clone(), vec![0, 1, 0]);
    let metrics = [
        None,
        Some(f32::NAN),
        Some(-1.0),
        Some(-0.0),
        Some(0.125),
        Some(0.5),
        Some(0.25),
        Some(f32::INFINITY),
    ];
    let indices = vec![0; metrics.len()];
    assert_eq!(
        map.submit_material_texture_demands(&indices, &metrics, &instances, &demands, &bindings,)
            .unwrap(),
        (metrics.len() - 1) * 2 * 3
    );
    assert_eq!(demands[0].take(), f32::INFINITY);
    assert_eq!(demands[1].take(), f32::INFINITY);

    // Failure partway through one material also publishes the valid prefix.
    bindings.insert(instances[0].material.clone(), vec![0, 9, 1]);
    assert!(map
        .submit_material_texture_demands(&[0], &[Some(0.375)], &instances, &demands, &bindings,)
        .is_err());
    assert_eq!(demands[0].take(), 0.375);
    assert_eq!(demands[1].take(), 0.0);
}

#[test]
fn post_merge_spatial_list_overwrites_visibility_and_clears_area_metrics() {
    let bounds = [
        [[20.0; 3], [21.0; 3]],
        [[0.0, 0.1, 0.2], [0.3, 0.4, 0.5]],
        [[8.0, 0.0, 0.0], [9.0, 1.0, 1.0]],
    ];
    let world = world_mesh::WorldMesh {
        meshes: Vec::new(),
        matrices: vec![[0.0; 12]; 3],
        visibility: bounds
            .map(|bounds| world_mesh::WorldVisibility {
                bounds,
                umbra_id: u32::MAX,
                trailing_field: 0,
            })
            .into(),
        auxiliary_instance_groups: Default::default(),
        visibility_groups: Vec::new(),
        tome_object_instances: vec![vec![0], vec![1], vec![2]],
        spatial_query_instances: vec![2, 1],
        objects: Vec::new(),
    };
    // Authored BWM indices are deliberately not render indices, and the
    // separate spatial-list order is also different from both tables.
    let map = TomeRenderInstanceMap::from_world_mapping(&world, &[vec![2], vec![0], vec![1]]);
    assert_eq!(map.spatial_query_instances[0].bounds, bounds[2]);
    assert_eq!(map.spatial_query_instances[0].render_indices, [1]);
    assert_eq!(map.spatial_query_instances[1].bounds, bounds[1]);
    assert_eq!(map.spatial_query_instances[1].render_indices, [0]);
    assert_eq!(map.zero_metric_instances, HashSet::from([0, 1]));
    let rows = std::array::from_fn(|row| {
        std::array::from_fn(|column| if row == column { 1.0 } else { 0.0 })
    });
    let state = tome::RenderQueryState {
        render_rows: rows,
        cropped_rows: rows,
        camera_point: [0.0; 3],
        camera_imprecision: [0.6; 3],
        winding_flip: false,
        viewport: [0, 0, 64, 64],
        maximum_depth: 0.0,
        depth_grid: None,
    };
    let mut visible = [false, true, true];
    assert_eq!(
        map.apply_spatial_queries(Some(&state), &mut visible)
            .unwrap(),
        1
    );
    assert_eq!(visible, [true, false, true]);
    assert_eq!(map.apply_spatial_queries(None, &mut visible).unwrap(), 2);
    assert_eq!(visible, [true; 3]);
    assert!(map
        .apply_spatial_queries(Some(&state), &mut visible[..1])
        .is_err());
    let instances = [0, 1, 2].map(|slot| RenderInstance {
        source: None,
        mesh: Handle::default(),
        material: MaterialHandle::Probe(Handle::default()),
        diffuse_image_slot: Some(slot),
        probe_index: 0,
        transform: Transform::IDENTITY,
        bounds: render_bounds(&[Vec3::ZERO, Vec3::ONE]),
        pickup_id: None,
        entity: None,
        visible: false,
    });
    let demands = std::array::from_fn::<_, 3, _>(|_| texture::ProjectedAreaDemand::default());
    assert_eq!(
        map.submit_texture_demands(&[0, 1, 2], &[Some(0.5); 3], &instances, &demands)
            .unwrap(),
        3
    );
    assert_eq!(demands.map(|demand| demand.take()), [0.0, 0.0, 0.5]);
}

#[test]
fn authored_id_dispatch_uses_group_emptiness_and_unsigned_sort() {
    let map = TomeRenderInstanceMap::from_authored_bwm_mapping(
        &[vec![0], vec![], vec![1], vec![]],
        &[vec![0], vec![]],
    );
    // Group 2 has authored BWM instances but no instantiated surfaces:
    // this must not be mistaken for the empty-group authored-ID path.
    let mut bytes = vec![0u8; 0x90];
    bytes[0x40..0x44].copy_from_slice(&4u32.to_le_bytes());
    bytes[0x50..0x54].copy_from_slice(&0x80u32.to_le_bytes());
    for (index, id) in [11u32, 0x80000001, 22, 1].into_iter().enumerate() {
        bytes[0x80 + index * 4..0x84 + index * 4].copy_from_slice(&id.to_le_bytes());
    }
    assert_eq!(
        map.visible_authored_ids(&bytes, &[3, 2, 1, 0, 3]).unwrap(),
        [1, 1, 0x80000001],
    );
    assert!(map.visible_authored_ids(&bytes, &[4]).is_err());
}

#[test]
fn sightline_identifies_bwm_surface_and_instance() {
    let world = world_mesh::WorldMesh {
        meshes: vec![world_mesh::Mesh {
            positions: vec![[-1.0, 2.0, -1.0], [1.0, 2.0, -1.0], [0.0, 2.0, 1.0]],
            uvs: vec![],
            packed_channels: vec![],
            indices: vec![0, 1, 2],
        }],
        matrices: vec![[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]],
        visibility: vec![world_mesh::WorldVisibility {
            bounds: [[-1.0, 2.0, -1.0], [1.0, 2.0, 1.0]],
            umbra_id: 0,
            trailing_field: 0,
        }],
        tome_object_instances: vec![vec![0]],
        auxiliary_instance_groups: std::array::from_fn(|_| Vec::new()),
        visibility_groups: vec![],
        spatial_query_instances: vec![],
        objects: vec![world_mesh::WorldObject {
            mesh_index: 0,
            material_path: "sample_roof".into(),
            matrix_start: 0,
            matrix_end: 1,
            flipped: false,
            lod: 0,
        }],
    };
    let (distance, mesh, matrix, material) =
        sightline_world_surface(&world, Vec3::ZERO, Vec3::NEG_Z, 10.0).unwrap();
    assert!((distance - 2.0).abs() < 0.001);
    assert_eq!((mesh, matrix, material.as_str()), (0, 0, "sample_roof"));
}

#[test]
#[ignore = "requires local original-game EOTW Tome fixture"]
fn reported_street_query_accepts_architecture_railing_and_sky() {
    let bytes =
        fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
    let tree = tome::SpatialTree::parse(&bytes).unwrap();
    let gates = tome::initial_gate_states(&bytes).unwrap();
    let point = [-38.43, 175.76, 4.91];
    let camera = Transform::from_translation(game_to_view(point))
        .looking_at(game_to_view([-40.9, 169.7, 3.8]), Vec3::Y);
    let rows = tome::world_visibility_query_matrix(
        DEFAULT_HORIZONTAL_FOV_DEGREES,
        1362,
        722,
        1.0,
        game_pose_from_view_transform(&camera),
        DEFAULT_NEAR,
        DEFAULT_FAR,
    )
    .unwrap();
    assert!(tome::query_matrix_winding_flip(rows));
    let output = tome::ordinary_direct_tome_visible_objects(
        &tree,
        &bytes,
        &tome::OrdinaryDirectQueryInput {
            camera_point: point,
            render_rows: rows,
            cropped_rows: rows,
            initial_tile_rect: [0, 0, 8, 16],
            camera_imprecision: [0.6; 3],
            global_distance_threshold: tome::DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
            object_distance_scale: 1.0,
            object_imprecision: 0.6,
            minimum_area_metric: 25.0 / (1362.0 * 722.0),
            winding_flip: tome::query_matrix_winding_flip(rows),
            gate_states: Some(&gates),
            output_capacity: 32768,
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(output.count(), 255);
    for index in [10, 17113, 17184] {
        assert!(
            output.indices().contains(&index),
            "missing authored Tome group {index}"
        );
    }
    // The installed distance predicate rejects this alternate tier here.
    assert!(!output.indices().contains(&17581));
}

#[test]
fn shared_mesh_transform_matches_baked_world_vertex() {
    let matrix = [0.0, -1.0, 0.0, 3.0, 1.0, 0.0, 0.0, 4.0, 0.0, 0.0, 1.0, 5.0];
    let vertex = [2.0, 1.0, 3.0];
    let actual = game_affine_to_view(&matrix)
        .to_matrix()
        .transform_point3(game_to_view(vertex));
    let expected = transformed(&matrix, vertex);
    assert!(
        actual.distance(expected) < 1e-5,
        "{actual:?} vs {expected:?}"
    );
}

#[test]
fn placed_model_queries_select_separate_instances_without_bwm_ids() {
    let rows = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.01, 1.0],
        [0.0, 0.0, -0.101, 0.0],
    ];
    let state = tome::RenderQueryState {
        render_rows: rows,
        cropped_rows: rows,
        camera_point: [0.0; 3],
        camera_imprecision: [0.6; 3],
        winding_flip: false,
        viewport: [0, 0, 64, 64],
        maximum_depth: 0.0,
        depth_grid: None,
    };
    // An accepted matrix1/init1/imprecision0.6 box from light_aabb_cases.txt.
    let bounds = [[-5.0, -4.0, -3.5], [1.0, 2.0, 2.5]];
    let group = PlacedModelQueryGroup {
        indices: 2..4,
        input: render_model::ModelQueryInput {
            frame_flags: 5,
            shape_flags: 0,
            bounds,
            region: render_model::range_region(bounds, 40.0),
        },
    };
    let mut visible = vec![true, false, false, false];
    assert_eq!(
        apply_placed_model_queries(&mut visible, &[group], &state, [0.0; 3], [1280, 720]).unwrap(),
        1
    );
    assert_eq!(visible, [true, false, true, true]);
    let outside = PlacedModelQueryGroup {
        indices: 2..4,
        input: render_model::ModelQueryInput {
            frame_flags: 5,
            shape_flags: 0,
            bounds,
            region: [100.0, 0.0, 0.0, 1.0],
        },
    };
    let mut visible = vec![true, false, false, false];
    assert_eq!(
        apply_placed_model_queries(&mut visible, &[outside], &state, [0.0; 3], [1280, 720])
            .unwrap(),
        0
    );
    assert_eq!(visible, [true, false, false, false]);
    let invalid = PlacedModelQueryGroup {
        indices: 2..5,
        input: render_model::ModelQueryInput {
            frame_flags: 5,
            shape_flags: 0,
            bounds,
            region: [0.0, 0.0, 0.0, 32768.0],
        },
    };
    assert!(
        apply_placed_model_queries(&mut visible, &[invalid], &state, [0.0; 3], [1280, 720])
            .is_err()
    );
    assert_eq!(visible, [true, false, false, false]);
}

#[test]
fn initial_model_query_preserves_transform_and_constructor_range() {
    let mut placement = entities::PlacedModel {
        id: "range".into(),
        resource_path: String::new(),
        position: [3.0, 4.0, 5.0],
        scale: [-2.0, 3.0, 1.0],
        rotation: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        custom_materials: Vec::new(),
        max_visible_range: None,
    };
    let bounds = [[-1.0; 3], [1.0; 3]];
    let default_input = initial_placed_model_query(&placement, bounds).unwrap();
    assert_eq!(default_input.bounds, [[0.0, 2.0, 4.0], [6.0, 6.0, 6.0]]);
    assert_eq!(default_input.region, [0.0, 0.0, 0.0, 32768.0]);
    assert!(render_model::model_eligible(
        default_input.frame_flags,
        u8::MAX
    ));
    placement.max_visible_range = Some(40.0);
    let input = initial_placed_model_query(&placement, bounds).unwrap();
    assert_eq!(input.bounds, [[0.0, 2.0, 4.0], [6.0, 6.0, 6.0]]);
    assert_eq!(input.region, [3.0, 4.0, 5.0, 43.0]);
    assert!(render_model::model_eligible(input.frame_flags, u8::MAX));
}

#[test]
fn placed_model_orientation_uses_authored_axes_not_rows() {
    let placement = entities::PlacedModel {
        id: "rotated".into(),
        resource_path: String::new(),
        position: [3.0, 4.0, 5.0],
        scale: [2.0, 3.0, 1.0],
        // Local X points along the authored first axis (+Y), while
        // local Y points along the second axis (-X).
        rotation: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        custom_materials: Vec::new(),
        max_visible_range: None,
    };
    let local_x = [1.0, 0.0, 0.0];
    let expected = game_to_view([3.0, 6.0, 5.0]);
    assert!(placed_vertex(&placement, local_x).distance(expected) < 1e-6);
    assert!(
        placed_transform(&placement)
            .to_matrix()
            .transform_point3(game_to_view(local_x))
            .distance(expected)
            < 1e-5
    );
}

#[test]
fn sheared_world_instance_keeps_exact_vertices() {
    let matrix = [1.0, 0.3, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let source = [[1.0, 1.0, 0.0], [2.0, 0.0, 0.0]];
    let exact: Vec<_> = source
        .iter()
        .copied()
        .map(|point| transformed(&matrix, point))
        .collect();
    assert!(needs_exact_world_mesh(
        &game_affine_to_view(&matrix),
        &source,
        &exact
    ));
    assert_eq!(exact[0], game_to_view([1.3, 1.0, 0.0]));
}

#[test]
fn placed_havok_uses_baked_scale_and_instance_rotation() {
    let placement = entities::PlacedModel {
        id: "fence".into(),
        resource_path: "generated/model/models/fence.bmodel".into(),
        position: [2.0, 3.0, 4.0],
        scale: [1.2, 1.0, -1.0],
        rotation: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        custom_materials: Vec::new(),
        max_visible_range: None,
    };
    assert_eq!(
        model_havok_path(&placement, "hkt", false).as_deref(),
        Some("generated/models/fence.lwo.hkt._120_100_-100.10.hkx")
    );
    assert_eq!(
        placed_collision_vertex(&placement, [1.2, 0.0, 0.0]),
        game_to_view([2.0, 4.2, 4.0])
    );
}

#[test]
fn blueprint_havok_recipe_keeps_root_source_and_declared_kind() {
    let placement = entities::PlacedModel {
        id: "root".into(),
        resource_path: "generated/decls/blueprint/models/fence.blueprint.decl".into(),
        position: [0.0; 3],
        scale: [1.5, -0.015, 1.0],
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        custom_materials: Vec::new(),
        max_visible_range: None,
    };
    assert_eq!(
        model_havok_path(&placement, "hkt", false).as_deref(),
        Some("generated/models/fence.blueprint.hkt._150_-1_100.10.hkx")
    );
    assert_eq!(
        model_havok_path(&placement, "box", false).as_deref(),
        Some("generated/models/fence.blueprint.box._150_-1_100.10.hkx")
    );
    assert_eq!(
        model_havok_path(&placement, "cvd", false).as_deref(),
        Some("generated/models/fence.blueprint.cvd._150_-1_100.10.hkx")
    );
    assert_eq!(
        model_havok_path(&placement, "hkt", true).as_deref(),
        Some("generated/models/fence_hd.blueprint.hkt._150_-1_100.10.hkx")
    );
    assert_eq!(model_havok_path(&placement, "unknown", false), None);
}

#[test]
#[ignore = "requires installed catalog and artifacts/eotw_p_lowchaos.entities"]
fn blueprint_root_collision_uses_component_not_render_children() {
    let document =
        entities::parse(&fs::read_to_string("artifacts/eotw_p_lowchaos.entities").unwrap())
            .unwrap();
    let bodies = entities::rigid_body_settings(&document);
    let parent = entities::render_placements(&document)
        .into_iter()
        .find(|placement| placement.id == "env_bp_fence_metal_200cm_02")
        .unwrap();
    let catalog =
        archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    assert_eq!(
        model_havok_path(&parent, "hkt", false).as_deref(),
        Some(
            "generated/models/environment/props/karnaca/fence_metal_set_01/bp_fence_metal_200cm_02.blueprint.hkt._100_100_100.10.hkx"
        )
    );
    let placements = expanded_model_placements(&[parent], &catalog);
    assert_eq!(placements.len(), 10);
    let (triangles, owners, matched, failed) =
        static_model_collision(&catalog, &placements, &bodies);
    assert_eq!((matched, failed, triangles.len()), (1, 0, 312));
    assert_eq!(owners, ["env_bp_fence_metal_200cm_02"]);
    assert!(triangles
        .iter()
        .all(|(vertices, owner)| *owner == 0 && vertices.iter().all(|point| point.is_finite())));
    assert_eq!(
        static_model_collision(&catalog, &placements, &[]).0.len(),
        0
    );
    let mut unbound_defaults = bodies.clone();
    for body in &mut unbound_defaults {
        body.physics_behavior = None;
    }
    assert_eq!(
        static_model_collision(&catalog, &placements, &unbound_defaults)
            .0
            .len(),
        0
    );
    let world = navigation::CollisionWorld::with_owned(triangles, owners);
    let eye = game_to_view([-33.679825, 83.83279, 18.319]);
    let direction = game_to_view([0.5484812, 0.29665253, -0.78177094]).normalize();
    let (distance, _, owner, _) = world
        .first_hit_details(eye, eye + direction * 40.0)
        .unwrap();
    assert_eq!(world.owner_name(owner), Some("env_bp_fence_metal_200cm_02"));
    assert!((distance - 0.28008184).abs() < 0.00001);
}

#[test]
fn pickup_selection_rejects_wall_occlusion() {
    let definitions = [entities::Pickup {
        entity_id: "test_item".into(),
        position: [0.0, 0.0, 0.0],
        item_path: "test/inventoryitem".into(),
        quantity: 1,
    }];
    let state = PickupState {
        items: vec![PickupInstance {
            entity_id: "test_item".into(),
            position: Vec3::new(0.0, 1.0, -2.0),
        }],
        world: runtime::WorldState::from_pickups(&definitions).unwrap(),
    };
    let camera = Transform::from_xyz(0.0, 1.0, 0.0);
    let clear = navigation::CollisionWorld::new(std::iter::empty());
    assert_eq!(pickup_in_view(&camera, &state, Some(&clear)), Some(0));
    let wall = navigation::CollisionWorld::new([
        [
            Vec3::new(-1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(-1.0, 2.0, -1.0),
        ],
        [
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(1.0, 2.0, -1.0),
            Vec3::new(-1.0, 2.0, -1.0),
        ],
    ]);
    assert_eq!(pickup_in_view(&camera, &state, Some(&wall)), None);
}

#[test]
fn mouse_capture_and_debug_free_camera_look_reach_camera_system() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(ButtonInput::<KeyCode>::default());
    app.insert_resource(ButtonInput::<MouseButton>::default());
    app.insert_resource(AccumulatedMouseMotion {
        delta: Vec2::new(100.0, 0.0),
    });
    app.insert_resource(FocusTargets {
        targets: vec![FocusTarget {
            center: Vec3::ZERO,
            distance: 10.0,
            label: "test".into(),
            spawn: None,
        }],
        active: 0,
    });
    app.insert_resource(super::player::PlayerState { player: None, debug_free_fly: true, difficulty: None });
    app.insert_resource(super::player::PlayerInput::default());
    let cursor = app.world_mut().spawn(CursorOptions::default()).id();
    let camera = app
        .world_mut()
        .spawn((Camera3d::default(), Transform::default()))
        .id();
    app.add_systems(Update, camera_controls);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    app.update();
    let camera = app.world().entity(camera).get::<Transform>().unwrap();
    assert!(
        camera.forward().x > 0.1,
        "mouse delta did not rotate camera: {camera:?}"
    );
    let cursor = app.world().entity(cursor).get::<CursorOptions>().unwrap();
    assert_eq!(cursor.grab_mode, CursorGrabMode::Locked);
    assert!(!cursor.visible);
}

fn vista_lighting_matches_reference(app: &mut App) {
    use bevy::render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        RenderApp,
    };
    let render = app.sub_app(RenderApp).world();
    let device = render.resource::<RenderDevice>();
    let queue = render.resource::<RenderQueue>();
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/shaders/dh2_vista.wgsl"
    ))
    .replace("#define_import_path dh2_vista", "")
        + r#"
@group(0) @binding(0) var<storage,read_write> output:array<vec4<f32>>;
fn view_vector(v:vec3<f32>)->vec3<f32> {return vec3(v.x,v.z,-v.y);}
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) tid:vec3<u32>) {
    let id=tid.x;
    let albedo=vec3(f32(id%17u),f32((id*3u)%17u),f32((id*7u)%17u))/16.0;
    let baked=vec4(.2,.1,.04,f32(id%5u)/4.0);
    let p=array<vec4<f32>,4>(vec4(10.0,0.0,0.0,0.0),
        vec4(-1.0,1.0,0.0,select(0.0,1.0,(id&1u)!=0u)),
        vec4(0.0,select(0.0,1.0,(id&2u)!=0u),.85,0.0),vec4(1.0,.13,.01,.5));
    let N=vec3(0.0,1.0,0.0);let T=vec3(1.0,0.0,0.0);let B=vec3(0.0,0.0,-1.0);
    let V=view_vector(normalize(vec3(.3,.2,1.0)));
    let L=view_vector(normalize(vec3(.4,.1,select(1.0,-.7,(id&4u)!=0u))));
    output[id]=vec4(vista_lighting(albedo,baked,f32(id%9u)/10.0,vec3(.04,.02,.08),
        vec3(.01,.02,.03),f32((id/3u)%5u)/4.0,p,V,L,vec3(2.0,1.5,1.0),N,T,B,N,
        .5*vec3(.03,.02,.01)),1.0);
}
"#;
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("vista lighting parity"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
    let size = 64 * 16;
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[BindGroupEntry {
                binding: 0,
                resource: output.as_entire_binding(),
            }],
        });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(64, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(20)),
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    let expected = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/vista_lighting_cases.txt"
    ))
    .split_whitespace()
    .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
    .collect::<Vec<_>>();
    for (i, (chunk, e)) in bytes.chunks_exact(4).zip(expected).enumerate() {
        let actual = f32::from_le_bytes(chunk.try_into().unwrap());
        assert!(
            (actual - e).abs() < 0.00003 * e.abs().max(1.0),
            "vista lighting case{} field{}: {actual} != {e}",
            i / 4,
            i % 4
        );
    }
    drop(bytes);
    readback.unmap();
    eprintln!("GPU vista lighting:64 installed-HLSL gloss/metallic/baked-shadow cases passed");
}

fn filling_matches_reference(app: &mut App) {
    use bevy::render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        RenderApp,
    };
    let render = app.sub_app(RenderApp).world();
    let device = render.resource::<RenderDevice>();
    let queue = render.resource::<RenderQueue>();
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/shaders/dh2_filling.wgsl"
    ))
    .replace("#define_import_path dh2_filling", "")
        + r#"
@group(0) @binding(0) var<storage,read_write> output:array<vec4<f32>>;
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) tid:vec3<u32>) {
 let id=tid.x;let y=f32(id%17u)/16.0;let level=f32(id%5u)/4.0;let lerp_colour=f32(id%3u)/2.0;
 let time=f32(id/7u)*0.25;
 let a=filling_pan_uv(vec2(.23,y),.61*.37+time,.13,vec2(.01,.025),vec2(.5,1.0));
 let b=filling_pan_uv(-vec2(.23,y),-.61*.37+time,-.17,vec2(.005,.05),vec2(1.0,.7));
 output[2u*id]=vec4(a,b);
 let coord=filling_lookup_coordinate(y,f32(id%7u)/6.0,1.0-level,vec4(.12,.7,.18,0.0));
 output[2u*id+1u]=filling_transfer_color(vec4(.2,.4,.8,.6),vec4(.7,.1,.9,.3),coord,y,1.0-level,
     lerp_colour,vec4(5.0,.015,.01,1.0),vec4(1.2,.03,2.1,.8),1.0);
 let d=f32(id%17u)/8.0-.25;let start=f32(id%5u)/8.0;let end=f32(id%7u)/8.0;
 output[128u+id]=vec4(filling_view_fade(d,start,end),filling_view_fade(d,end,start),
     filling_view_fade(start,start,start),filling_view_fade(start+.25,start,start));
}
"#;
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("filling parity"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
    let size = 192 * 16;
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[BindGroupEntry {
                binding: 0,
                resource: output.as_entire_binding(),
            }],
        });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(64, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(20)),
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    let mut expected = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/filling_cases.txt"
    ))
    .split_whitespace()
    .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
    .collect::<Vec<_>>();
    let particles = include_str!("../../tests/fixtures/particle_math_cases.txt")
        .split_whitespace()
        .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(particles.len(), 64 * 16);
    expected.extend(
        particles
            .chunks_exact(16)
            .flat_map(|row| row[..4].iter().copied()),
    );
    assert_eq!(bytes.len(), expected.len() * 4);
    for (i, (chunk, e)) in bytes.chunks_exact(4).zip(expected).enumerate() {
        let actual = f32::from_le_bytes(chunk.try_into().unwrap());
        assert!(
            (actual - e).abs() < 0.00003 * e.abs().max(1.0),
            "filling case{} field{}: {actual} != {e}",
            i / 4,
            i % 4
        );
    }
    drop(bytes);
    readback.unmap();
    eprintln!(
        "GPU filling:64 installed-HLSL UV/fill/colour cases and 256 distance-fade values passed"
    );
}

#[test]
fn immutable_shadow_vectors_keep_raw_bits_and_model_fallbacks_live() {
    use crate::render_program::{
        RenderParameterKind, SerializedParameter, SerializedParameterBlock,
        SerializedParameterValue,
    };
    let words = [0x7fc01234, 0x80000000, 1, 0x7f800000];
    let parameters = ShadowMaterialParameters::new(
        SerializedParameterBlock {
            flags: 0,
            parameters: vec![
                SerializedParameter {
                    name: b"layer0_scalebias".to_vec(),
                    kind: RenderParameterKind::Uniform,
                    value: SerializedParameterValue::Words(words),
                },
                SerializedParameter {
                    name: b"bad".to_vec(),
                    kind: RenderParameterKind::Sampler,
                    value: SerializedParameterValue::Name(b"texture".to_vec()),
                },
            ],
        },
        [
            (b"layer0_scalebias".to_vec(), [0; 4]),
            (b"modeldepthhack".to_vec(), [1.0f32.to_bits(), 0, 0, 0]),
            (b"bad".to_vec(), [0; 4]),
        ]
        .into_iter()
        .collect(),
    );
    assert_eq!(
        parameters
            .vector(b"layer0_scalebias", None)
            .unwrap()
            .map(f32::to_bits),
        words
    );
    assert_eq!(parameters.vector(b"modeldepthhack", None).unwrap()[0], 1.0);
    let mut fallback = SerializedParameterBlock {
        flags: 0,
        parameters: vec![SerializedParameter {
            name: b"modeldepthhack".to_vec(),
            kind: RenderParameterKind::Uniform,
            value: SerializedParameterValue::Words([2.0f32.to_bits(), 0, 0, 0]),
        }],
    };
    assert_eq!(
        parameters
            .vector(b"modeldepthhack", Some(&fallback))
            .unwrap()[0],
        2.0
    );
    fallback.parameters[0].value = SerializedParameterValue::Words([3.0f32.to_bits(), 0, 0, 0]);
    assert_eq!(
        parameters
            .vector(b"modeldepthhack", Some(&fallback))
            .unwrap()[0],
        3.0
    );
    assert_eq!(parameters.vector(b"modeldepthhack", None).unwrap()[0], 1.0);
    assert_eq!(
        parameters.vector(b"bad", None).unwrap_err(),
        "numeric render parameter has a nonnumeric payload"
    );
    assert_eq!(
        parameters.vector(b"missing", None).unwrap_err(),
        "unresolved native shadow Uniform missing"
    );
}
