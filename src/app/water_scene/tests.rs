use super::*;
use crate::app::*;

#[test]
#[ignore = "requires installed Edge of the World and GPU; complete gameplay ocean submission"]
fn installed_ocean_submits_depth_and_colour() {
    use crate::rendering::bevy_backend::{
        ocean_scene::{OceanScene, OceanSources},
        water_colour_scene::{WaterColourResources, WaterColourView},
    };
    use bevy::render::{
        mesh::allocator::MeshAllocator,
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        RenderApp,
    };
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = archive::Catalog::open(&base).unwrap();
    let path = catalog
        .paths_with_suffix(".bwm")
        .into_iter()
        .find(|p| p.contains("edge_of_the_world") && p.contains("lowchaos"))
        .unwrap();
    let level = crate::assets::level::LevelResources::resolve(&catalog, &path).unwrap();
    let bytes = level.read_entities(&catalog).unwrap().unwrap();
    let document = entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
    let read = |name: &str| -> Result<Option<String>, String> {
        catalog
            .read(&format!("generated/decls/env/{name}.decl"))
            .map(|v| v.map(|v| String::from_utf8_lossy(&v).into_owned()))
            .map_err(|e| e.to_string())
    };
    let name = entities::world_environment(&document).unwrap();
    let parameters = entities::environment_render_parameters_with_inheritance(
        &name,
        &read(&name).unwrap().unwrap(),
        read,
    )
    .unwrap();
    let areas = crate::rendering::environment::load_area_environments(&document, &catalog).unwrap();
    let materials = WaterMaterials::default();
    let (mut scene, images) = WaterScene::load(&catalog, &parameters, &areas, &materials).unwrap();
    let mut library =
        shader_cache::ProgramLibrary::open(&base, shader_cache::ShaderCacheBackend::Pc, &catalog)
            .unwrap();
    let sources = OceanSources::load(&catalog, &mut library).unwrap();
    let eye = [0., -100., 15.];
    let projection =
        crate::rendering::bevy_backend::camera_projection::GameProjection::new(90., 0.01, 7000.);
    let h = std::f32::consts::FRAC_1_SQRT_2;
    let matrices = ViewMatrices::from_camera(
        eye,
        [[0., h, -h], [-1., 0., 0.], [0., h, h]],
        projection.rows(),
    );
    let frame = WaterFrame {
        camera: Entity::from_raw_u32(1).unwrap(),
        delta_seconds: 1. / 60.,
        near_far: [projection.near, projection.far],
        parallel: Some(([0., 0., -1.], [1.; 3])),
        ocean_render_settings: scene.ocean_render_settings,
        values: scene.update(&areas, eye, 1., 1. / 60.).unwrap(),
        ocean_environment: scene
            .ocean_environment
            .update(&areas, eye, 1., 1. / 60.)
            .unwrap(),
        matrices: ViewMatrixFrame {
            frame: 1,
            current: matrices,
            previous: matrices,
        },
        local_time: [1., 0.],
    };
    assert!(
        frame.ocean_environment.view.enabled,
        "selected installed level must enable ocean"
    );
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(bevy::render::RenderPlugin {
                render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(
                    bevy::render::settings::WgpuSettings {
                        features: bevy::render::settings::WgpuSettings::default().features
                            | bevy::render::settings::WgpuFeatures::DEPTH32FLOAT_STENCIL8,
                        ..default()
                    },
                )),
                ..default()
            })
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
    app.sub_app_mut(RenderApp)
        .init_resource::<crate::tiled_lighting_gpu::TlfPipelines>();
    // MeshAllocator is installed during the first render extraction, as in
    // gameplay; finish/cleanup alone does not initialize that schedule.
    app.update();
    let world = app.sub_app(RenderApp).world();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let pipelines = world.resource::<crate::tiled_lighting_gpu::TlfPipelines>();
    let extent = [64, 64];
    let texture = |label, format, layers| {
        device.create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let opaque = texture("ocean test opaque", TextureFormat::Depth32FloatStencil8, 1);
    let depth = texture(
        "ocean test water depth",
        TextureFormat::Depth32FloatStencil8,
        1,
    );
    let colour = texture("ocean test HDR", TextureFormat::Rgba16Float, 1);
    let motion = texture("ocean test motion", TextureFormat::Rg16Float, 1);
    let motion_view = motion.create_view(&default());
    let fog = texture("ocean test fog", TextureFormat::Rgba16Float, 1);
    let shadow = texture("ocean test shadow", TextureFormat::Depth32Float, 2);
    let depth_attachment = depth.create_view(&default());
    let depth_sample = depth.create_view(&TextureViewDescriptor {
        aspect: TextureAspect::DepthOnly,
        ..default()
    });
    let colour_view = colour.create_view(&default());
    let fog_view = fog.create_view(&default());
    let shadow_view = shadow.create_view(&TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    let exposure = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("ocean test adaptation"),
        contents: &[1f32, 0., 0., 0., 0., 0., 0., 0.]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>(),
        usage: BufferUsages::STORAGE,
    });
    let mut ocean = OceanScene::new(device, queue).unwrap();
    let mut encoder = device.create_command_encoder(&default());
    for texture in [&opaque, &depth] {
        let view = texture.create_view(&default());
        let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &view,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(0.),
                    store: StoreOp::Store,
                }),
                stencil_ops: Some(Operations {
                    load: LoadOp::Clear(0),
                    store: StoreOp::Store,
                }),
            }),
            ..default()
        });
    }
    for view in [&colour_view, &fog_view, &motion_view] {
        let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
            color_attachments: &[Some(RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Default::default()),
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
    }
    ocean
        .prepare(device, queue, &mut encoder, &frame, &images, extent)
        .unwrap();
    assert!(
        ocean.draw_count() > 0,
        "gameplay camera must submit ocean leaves"
    );
    ocean
        .record_depth(
            device,
            queue,
            &mut encoder,
            &frame,
            sources.0.as_deref().unwrap(),
            &depth_attachment,
            &motion_view,
            extent,
        )
        .unwrap();
    let pyramid =
        crate::rendering::bevy_backend::coarse_depth::DepthPyramid::new(device, pipelines, &opaque)
            .unwrap();
    let inverse = Mat4::from_cols_array_2d(&matrices.projection)
        .transpose()
        .inverse()
        .transpose()
        .to_cols_array_2d();
    pyramid
        .record(queue, &mut encoder, pipelines, extent, inverse)
        .unwrap();
    let water_meshes = crate::rendering::bevy_backend::water_scene::WaterWorldMeshes::default();
    let lighting = crate::rendering::bevy_backend::water_scene::WaterLighting::default();
    let resources = WaterColourResources {
        device,
        queue,
        materials: &materials,
        world: &water_meshes,
        lighting: &lighting,
        images: &images,
        image_assets: world.resource::<RenderAssets<bevy::render::texture::GpuImage>>(),
        meshes: world.resource(),
        buffers: world.resource(),
        allocator: world.resource::<MeshAllocator>(),
        depth_programs: pipelines,
    };
    let shadow_parameters =
        crate::rendering::bevy_backend::water_refraction::WaterShadowParameters {
            world_to_shadow: [[0.; 16]; 5],
            view_row: [0.; 4],
            slice_fars: [0.; 4],
            sun_direction: [0.; 4],
            shadow_size: [0.; 4],
            bias: [0.; 4],
            options: [0; 4],
        };
    let view = WaterColourView {
        frame: &frame,
        white_level: 1.,
        camera_exposure: 1.,
        adaptation: &exposure,
        shadow: shadow_parameters,
        atlas: &shadow_view,
        fog: &fog_view,
        hdr: &colour_view,
        opaque: &opaque,
        water_depth: &depth_attachment,
        pyramid: Some(&pyramid),
        shared_pyramid_expected: true,
    };
    ocean
        .record_colour(
            &resources,
            &view,
            sources.0.as_deref().unwrap(),
            extent,
            false,
            TextureFormat::Rgba16Float,
            &depth_sample,
            &mut encoder,
        )
        .unwrap();
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("ocean gameplay colour result"),
        size: 64 * 64 * 8,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        colour.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(512),
                rows_per_image: Some(64),
            },
        },
        Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
    );
    let submission = queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(MapMode::Read, move |result| sender.send(result).unwrap());
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    receiver
        .recv_timeout(std::time::Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let result = readback.slice(..).get_mapped_range();
    let mut nonzero = 0;
    for pixel in result.chunks_exact(8) {
        for channel in pixel[..6].chunks_exact(2) {
            let value =
                half::f16::from_bits(u16::from_le_bytes(channel.try_into().unwrap())).to_f32();
            assert!(value.is_finite(), "nonfinite ocean gameplay output");
            nonzero += usize::from(value > 0.);
        }
    }
    assert!(
        nonzero > 0,
        "ocean must write visible colour, not just issue draw calls"
    );
    println!(
        "Installed ocean gameplay submission: {} patches, depth + FFT + foam + colour + exposure",
        ocean.draw_count()
    );
    drop(result);
    readback.unmap();

    // Exercise the gameplay owner, not just the isolated stencil shader:
    // installed wave inputs -> retained FFT -> water line -> both alpha targets.
    let mut depths = crate::rendering::bevy_backend::water_depth_scene::WaterDepthScene::new(
        device, pipelines,
        crate::rendering::bevy_backend::fog_targets::FogExtents::new(UVec2::splat(64), 1).unwrap(),
        TextureFormat::Depth32FloatStencil8,
    ).unwrap();
    let stencil = device.create_buffer(&BufferDescriptor {
        label: Some("gameplay underwater classifications"),
        size: 256 * (64 + 32),
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    for (i, offset) in [100., -100.].into_iter().enumerate() {
        let mut frame = frame.clone();
        let mut eye = eye;
        eye[2] = frame.ocean_environment.view.ocean_water_level + offset;
        let matrices = ViewMatrices::from_camera(
            eye, [[0., h, -h], [-1., 0., 0.], [0., h, h]], projection.rows(),
        );
        frame.matrices = ViewMatrixFrame { frame: i as u32 + 2, current: matrices, previous: matrices };
        let mut encoder = device.create_command_encoder(&default());
        depths.request_ocean_depth(true);
        depths.record_with_main(device, &mut encoder, &opaque, None).unwrap();
        ocean.prepare(device, queue, &mut encoder, &frame, &images, extent).unwrap();
        // The opaque target stands in for the view's main depth/stencil.
        let main = bevy::render::view::ViewDepthTexture::new(
            bevy::render::texture::CachedTexture { texture: opaque.clone(), default_view: opaque.create_view(&default()) },
            Some(0.0),
        );
        ocean.record_underwater(device, queue, &mut encoder, &frame,
            sources.0.as_deref().unwrap(), &depths, &main).unwrap();
        for (texture, offset) in [(&depths.alpha.full, 0), (&depths.alpha.reduced, 64 * 256)] {
            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo { texture, mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::StencilOnly },
                TexelCopyBufferInfo { buffer: &stencil, layout: TexelCopyBufferLayout {
                    offset, bytes_per_row: Some(256), rows_per_image: Some(texture.height()),
                }}, texture.size(),
            );
        }
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        stencil.slice(..).map_async(MapMode::Read, move |r| tx.send(r).unwrap());
        device.poll(PollType::Wait { submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)) }).unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = stencil.slice(..).get_mapped_range();
        for (size, start) in [(64, 0), (32, 64 * 256)] {
            for y in 0..size {
                for x in 0..size {
                    assert_eq!(bytes[start + y * 256 + x], if i == 0 { 0 } else { 0x40 },
                        "underwater owner eye offset {offset}, target {size}, pixel {x},{y}");
                }
            }
        }
        drop(bytes);
        stencil.unmap();
    }
}

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
        water_model_colours: default(),
        water_stencil_materials: default(),
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
        vegetation: default(),
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
