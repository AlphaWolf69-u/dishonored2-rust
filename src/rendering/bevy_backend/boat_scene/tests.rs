use super::*;
use crate::{archive, render_program, shader_cache};
use bevy::{
    asset::RenderAssetUsages,
    camera::RenderTarget,
    render::{texture::CachedTexture, view::ViewDepthTexture, RenderApp},
};

#[test]
#[ignore = "requires installed boat programs and graphics adapter; actual multi-target mask lifecycle"]
fn boats_mask_main_and_reduced_depth_without_masking_full_alpha() {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(bevy::window::WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            })
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
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
    );
    app.finish();
    app.cleanup();
    let target = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::new_target_texture(
            32,
            32,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
    app.world_mut().spawn((
        Camera3d::default(),
        Camera::default(),
        RenderTarget::Image(target.into()),
        Transform::IDENTITY,
    ));
    // Source state uses clockwise front faces. The near cap faces the camera;
    // the far cap faces away. Import reverses their indices below.
    let raw = vec![
        [-1f32, -1., 0.7],
        [-1., 3., 0.7],
        [3., -1., 0.7],
        [-1., -1., 0.05],
        [3., -1., 0.05],
        [-1., 3., 0.05],
    ];
    let boat_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            raw.iter().map(|p| [p[0], p[2], -p[1]]).collect::<Vec<_>>(),
        )
        .with_inserted_attribute(super::super::shadow_pass::VEGETATION_POSITION, raw)
        .with_inserted_indices(bevy::mesh::Indices::U32(vec![0, 2, 1, 3, 5, 4])),
    );
    let water_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[99f32; 3]; 3])
        .with_inserted_attribute(
            super::super::shadow_pass::VEGETATION_POSITION,
            vec![[-1f32, -1., 0.5], [3., -1., 0.5], [-1., 3., 0.5]],
        )
        .with_inserted_indices(bevy::mesh::Indices::U32(vec![0, 2, 1])),
    );
    let matrices = app
        .world_mut()
        .resource_mut::<Assets<bevy::render::storage::ShaderBuffer>>()
        .add(bevy::render::storage::ShaderBuffer::from(vec![[
            1f32, 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.,
        ]]));
    for _ in 0..4 {
        app.update();
    }
    let render = app.sub_app_mut(RenderApp).world_mut();
    render.init_resource::<super::super::tiled_lighting_gpu::TlfPipelines>();
    let device = render.resource::<RenderDevice>();
    let queue = render.resource::<RenderQueue>();
    let errors = Arc::new(std::sync::Mutex::new(Vec::new()));
    let caught = errors.clone();
    device.wgpu_device().on_uncaptured_error(Arc::new(move |e| {
        caught.lock().unwrap().push(e.to_string())
    }));
    let pipelines = render.resource::<super::super::tiled_lighting_gpu::TlfPipelines>();
    let mut owner = super::super::water_depth_scene::WaterDepthScene::new(
        device,
        pipelines,
        super::super::fog_targets::FogExtents::new(UVec2::splat(32), 2).unwrap(),
        TextureFormat::Depth32FloatStencil8,
    )
    .unwrap();
    let world = WaterWorldMeshes {
        objects: Arc::new(HashMap::from([(
            0,
            Ok(super::super::water_scene::WaterWorldMesh {
                mesh: boat_mesh.clone(),
                surface_flags: 0,
                indices_reversed: true,
            }),
        )])),
        matrices: Some(matrices),
    };
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = archive::Catalog::open(&base).unwrap();
    let mut library =
        shader_cache::ProgramLibrary::open(&base, shader_cache::ShaderCacheBackend::Pc, &catalog)
            .unwrap();
    let options = render_program::SerializedParameterBlock {
        flags: 0,
        parameters: Vec::new(),
    };
    let model_state = library
        .boat_volume_program_state(b"arkBoatVolume", &options, false)
        .unwrap();
    let world_state = library
        .boat_volume_program_state(b"arkBoatVolume", &options, true)
        .unwrap();
    let legacy = catalog
        .read("generated/decls/material/models/campaign/boat_shared/boat_gabarit_01.material.decl")
        .unwrap()
        .unwrap();
    let values = crate::material::material_option_parameters(&legacy, |name| {
        library.parameter_kinds.get(name.as_bytes())
            == Some(&render_program::RenderParameterKind::Option)
    })
    .unwrap();
    let legacy_options = render_program::SerializedParameterBlock {
        flags: 0,
        parameters: values
            .into_iter()
            .map(|(name, value)| render_program::SerializedParameter {
                name: name.into_bytes(),
                kind: render_program::RenderParameterKind::Option,
                value: render_program::SerializedParameterValue::Words([value as u32, 0, 0, 0]),
            })
            .collect(),
    };
    let legacy_state = library
        .boat_volume_program_state(b"arkDefault", &legacy_options, true)
        .unwrap();
    let legacy_state = crate::render_state::material_render_state(
        &crate::entities::declaration_text(&legacy),
        legacy_state,
    )
    .unwrap();
    let materials = BoatMaterials {
        model: Some(Ok(model_state)),
        world: Arc::new(HashMap::from([
            ("test/boat".into(), Ok(world_state)),
            ("test/legacy".into(), Ok(legacy_state)),
        ])),
    };
    let main = device.create_texture(&TextureDescriptor {
        label: Some("actual main camera stencil fixture"),
        size: Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Depth24PlusStencil8,
        usage: TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::TEXTURE_BINDING
            | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let attachment = main.create_view(&default());
    let sample = main.create_view(&TextureViewDescriptor {
        aspect: TextureAspect::DepthOnly,
        ..default()
    });
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 16,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 28928,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let shader=device.create_and_validate_shader_module(ShaderModuleDescriptor{label:None,source:ShaderSource::Wgsl(r#"
@group(0) @binding(0) var water:texture_depth_2d;
@group(0) @binding(1) var alpha:texture_depth_2d;
@group(0) @binding(2) var reduced:texture_depth_2d;
@group(0) @binding(3) var main:texture_depth_2d;
@group(0) @binding(4) var<storage,read_write> result:array<vec4<f32>>;
@compute @workgroup_size(1) fn read(){result[0]=vec4(textureLoad(water,vec2(16),0),textureLoad(alpha,vec2(16),0),textureLoad(reduced,vec2(8),0),textureLoad(main,vec2(16),0));}
"#.into())});
    let read = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &shader,
        entry_point: Some("read"),
        compilation_options: default(),
        cache: None,
    });
    let mut entries: Vec<_> = [
        &owner.water.sample,
        &owner.alpha.full_sample,
        &owner.alpha.reduced_sample,
        &sample,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BindGroupEntry {
        binding: i as u32,
        resource: BindingResource::TextureView(v),
    })
    .collect();
    entries.push(BindGroupEntry {
        binding: 4,
        resource: output.as_entire_binding(),
    });
    let binding = device.create_bind_group(None, &read.get_bind_group_layout(0).into(), &entries);
    let frame = crate::rendering::view_history::ViewMatrixFrame {
        frame: 1,
        current: default(),
        previous: default(),
    };
    for (geometry, water, expected_depth, expected_mask, legacy) in [
        (
            Some(BoatGeometry::World {
                object: 0,
                matrix: 0,
            }),
            false,
            0.1,
            2,
            false,
        ),
        (Some(BoatGeometry::RigidModel), false, 0.1, 2, false),
        (
            Some(BoatGeometry::World {
                object: 0,
                matrix: 0,
            }),
            true,
            0.1,
            2,
            false,
        ),
        (Some(BoatGeometry::RigidModel), true, 0.1, 2, false),
        (
            Some(BoatGeometry::World {
                object: 0,
                matrix: 0,
            }),
            false,
            0.1,
            2,
            true,
        ),
        (
            Some(BoatGeometry::World {
                object: 0,
                matrix: 0,
            }),
            true,
            0.1,
            2,
            true,
        ),
        (None, true, 0.1, 0, false),
    ] {
        let surfaces = water.then(|| DepthSurface {
            mesh: water_mesh.id(),
            matrix: None,
            world_from_local: Mat4::IDENTITY,
            order: 0,
            state: crate::render_state::water_depth_state(),
            reversed: true,
            motion: true,
            reference: 4,
            host_space: false,
        });
        owner
            .geometry
            .prepare(
                device,
                queue,
                surfaces,
                &world,
                render.resource::<RenderAssets<RenderMesh>>(),
                render.resource::<MeshAllocator>(),
                render.resource::<RenderAssets<GpuShaderBuffer>>(),
                frame,
                owner.water.texture.format(),
            )
            .unwrap();
        let instance = geometry.map(|geometry| BoatInstance {
            material: if legacy { "test/legacy" } else { "test/boat" }.into(),
            order: 0,
            geometry,
            mesh: boat_mesh.id(),
            world_from_local: Mat4::IDENTITY,
        });
        let pending = owner
            .boats
            .prepare(
                device,
                queue,
                instance.iter(),
                &materials,
                &world,
                render.resource::<RenderAssets<RenderMesh>>(),
                render.resource::<MeshAllocator>(),
                render.resource::<RenderAssets<GpuShaderBuffer>>(),
                frame,
                owner.water.texture.format(),
                main.format(),
            )
            .unwrap();
        assert!(pending.is_empty(), "{pending:?}");
        let view = ViewDepthTexture::new(
            CachedTexture {
                texture: main.clone(),
                default_view: attachment.clone(),
            },
            Some(0.1),
        );
        let mut encoder = device.create_command_encoder(&default());
        let initial = view.get_attachment(StoreOp::Store);
        assert!(
            matches!(
                initial.stencil_ops.as_ref().map(|o| o.load),
                Some(LoadOp::Clear(0))
            ),
            "camera must clear stencil with its first depth clear"
        );
        {
            let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                depth_stencil_attachment: Some(initial),
                ..default()
            });
        }
        owner
            .record_with_main(device, &mut encoder, &main, Some(view.view()))
            .unwrap();
        assert!(matches!(
            view.get_attachment(StoreOp::Store)
                .stencil_ops
                .unwrap()
                .load,
            LoadOp::Load
        ));
        {
            let mut pass = encoder.begin_compute_pass(&default());
            pass.set_pipeline(&read);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, 16);
        for (texture, offset) in [
            (&owner.water.texture, 256),
            (&owner.alpha.full, 8448),
            (&owner.alpha.reduced, 16640),
            (&main, 20736),
        ] {
            let mut source = texture.as_image_copy();
            source.aspect = TextureAspect::StencilOnly;
            encoder.copy_texture_to_buffer(
                source,
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(texture.height()),
                    },
                },
                texture.size(),
            );
        }
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = staging.slice(..).get_mapped_range();
        let depths: Vec<_> = bytes[..16]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let at = |offset: usize, size: usize| bytes[offset + (size / 2) * 256 + size / 2];
        println!("boat world={} present={} water={water} legacy={legacy} depth={depths:?} stencil={:?}",
            matches!(geometry, Some(BoatGeometry::World { .. })), geometry.is_some(),
            [at(256, 32), at(8448, 32), at(16640, 16), at(20736, 32)]);
        let water_depth = if geometry.is_none() { 0.5 } else { 0.1 };
        assert!(
            (depths[0] - water_depth).abs() < 1e-6 && (depths[1] - water_depth).abs() < 1e-6,
            "water/full alpha depth {depths:?}"
        );
        let reduced = if geometry.is_none() {
            0.5
        } else {
            expected_depth
        };
        assert!(
            (depths[2] - reduced).abs() < 1e-6 && (depths[3] - expected_depth).abs() < 1e-6,
            "world/model stencil depth {depths:?}"
        );
        assert_eq!(
            at(256, 32),
            if !water {
                0
            } else if geometry.is_some() {
                8
            } else {
                4
            },
            "water stencil/restoration"
        );
        assert_eq!(at(8448, 32), 0, "full alpha is not the main stencil target");
        assert_eq!(at(16640, 16), expected_mask, "reduced stencil");
        assert_eq!(at(20736, 32), expected_mask, "actual main stencil");
        drop(bytes);
        staging.unmap();
    }
    assert!(
        errors.lock().unwrap().is_empty(),
        "{:?}",
        errors.lock().unwrap()
    );
}
