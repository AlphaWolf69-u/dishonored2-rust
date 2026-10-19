//! Exact draw-descriptor reuse; uniform and storage contents remain live.
use super::*;
use bevy::platform::collections::HashSet;
use bevy::render::render_resource::{BufferId, TextureViewId};

pub(super) struct PreparedRecordCache {
    geometry_epoch: (u32, u32),
    resource_epoch: (u32, u32),
    ring: BufferId,
    stride: u64,
    depth_32_bits: bool,
    layers: Vec<Vec<DescriptorSnapshot>>,
    uniforms: Vec<ShadowUniformRecord>,
    buffers: Vec<(AssetId<ShaderBuffer>, BufferId, u64)>,
    images: Vec<(AssetId<Image>, TextureViewId)>,
    pub records: Arc<Vec<PreparedShadowRecord>>,
}

impl PreparedRecordCache {
    pub fn new(
        layers: &[NativeShadowLayer],
        geometry_epoch: (u32, u32),
        resource_epoch: (u32, u32),
        ring: BufferId,
        stride: u64,
        depth_32_bits: bool,
        uniforms: &[ShadowUniformRecord],
        records: Arc<Vec<PreparedShadowRecord>>,
        images: &RenderAssets<GpuImage>,
        buffers: &RenderAssets<GpuShaderBuffer>,
    ) -> Option<Self> {
        let mut buffer_ids: HashSet<AssetId<ShaderBuffer>> = HashSet::default();
        let mut image_ids: HashSet<AssetId<Image>> = HashSet::default();
        for instance in layers.iter().flat_map(|layer| &layer.instances) {
            if let Some(alpha) = &instance.alpha_material {
                image_ids.insert(alpha.diffuse.id());
            }
            match &instance.per_instance_cb {
                NativeShadowInstanceConstants::Skinned { palette, .. } => {
                    buffer_ids.insert(palette.id());
                }
                NativeShadowInstanceConstants::World {
                    visible_indices,
                    matrices,
                    ..
                } => {
                    buffer_ids.extend([visible_indices.id(), matrices.id()]);
                }
                NativeShadowInstanceConstants::WorldVegetation {
                    visible_indices,
                    matrices,
                    winds,
                    ..
                } => {
                    buffer_ids.extend([visible_indices.id(), matrices.id()]);
                    if let Some(winds) = winds {
                        buffer_ids.insert(winds.id());
                    }
                }
                NativeShadowInstanceConstants::Rigid(_)
                | NativeShadowInstanceConstants::Vegetation(_) => {}
            }
        }
        Some(Self {
            geometry_epoch,
            resource_epoch,
            ring,
            stride,
            depth_32_bits,
            layers: layers
                .iter()
                .map(|layer| {
                    layer
                        .instances
                        .iter()
                        .map(DescriptorSnapshot::new)
                        .collect()
                })
                .collect(),
            uniforms: uniforms.to_vec(),
            records,
            buffers: buffer_ids
                .into_iter()
                .map(|id| {
                    buffers
                        .get(id)
                        .map(|gpu| (id, gpu.buffer.id(), gpu.buffer.size()))
                })
                .collect::<Option<_>>()?,
            images: image_ids
                .into_iter()
                .map(|id| images.get(id).map(|gpu| (id, gpu.texture_view.id())))
                .collect::<Option<_>>()?,
        })
    }

    pub fn matches(
        &mut self,
        layers: &[NativeShadowLayer],
        geometry_epoch: (u32, u32),
        resource_epoch: (u32, u32),
        ring: BufferId,
        stride: u64,
        depth_32_bits: bool,
        uniforms: &[ShadowUniformRecord],
        images: &RenderAssets<GpuImage>,
        buffers: &RenderAssets<GpuShaderBuffer>,
    ) -> bool {
        if self.geometry_epoch != geometry_epoch
            || self.ring != ring
            || self.stride != stride
            || self.depth_32_bits != depth_32_bits
            || self.uniforms != uniforms
            || self.layers.len() != layers.len()
            || !self.layers.iter().zip(layers).all(|(a, b)| {
                a.len() == b.instances.len()
                    && a.iter().zip(&b.instances).all(|(a, b)| a.matches(b))
            })
        {
            return false;
        }
        if self.resource_epoch != resource_epoch {
            if !self.buffers.iter().all(|(id, buffer, size)| {
                buffers
                    .get(*id)
                    .is_some_and(|gpu| gpu.buffer.id() == *buffer && gpu.buffer.size() == *size)
            }) || !self.images.iter().all(|(id, view)| {
                images
                    .get(*id)
                    .is_some_and(|gpu| gpu.texture_view.id() == *view)
            }) {
                return false;
            }
            self.resource_epoch = resource_epoch;
        }
        true
    }
}

/// Keep strong asset ownership, but omit live CB bytes. Those bytes are uploaded
/// independently and never participated in descriptor equality. Only the remap
/// offsets below affect the CPU resource-range validation.
struct DescriptorSnapshot {
    mesh: Handle<Mesh>,
    constants: DescriptorConstants,
    program: NativeShadowProgram,
    alpha_material: Option<NativeShadowAlphaMaterial>,
    front_face: FrontFace,
    cull_mode: Option<Face>,
    draw_state: [u32; 9],
    stencil_reference: u32,
    scissor: NativeShadowScissor,
}

enum DescriptorConstants {
    Rigid,
    Vegetation,
    Skinned(Handle<ShaderBuffer>),
    World {
        visible_indices: Handle<ShaderBuffer>,
        matrices: Handle<ShaderBuffer>,
        winds: Option<Handle<ShaderBuffer>>,
        instance_count: u32,
        offsets: [u8; 8],
        vegetation: bool,
    },
}

impl DescriptorSnapshot {
    fn new(instance: &NativeShadowInstance) -> Self {
        use NativeShadowInstanceConstants::*;
        let constants = match &instance.per_instance_cb {
            Rigid(_) => DescriptorConstants::Rigid,
            Vegetation(_) => DescriptorConstants::Vegetation,
            Skinned { palette, .. } => DescriptorConstants::Skinned(palette.clone()),
            World {
                constants,
                visible_indices,
                matrices,
                instance_count,
            } => {
                let mut offsets = [0; 8];
                offsets[..4].copy_from_slice(&constants[64..68]);
                DescriptorConstants::World {
                    visible_indices: visible_indices.clone(),
                    matrices: matrices.clone(),
                    winds: None,
                    instance_count: *instance_count,
                    offsets,
                    vegetation: false,
                }
            }
            WorldVegetation {
                constants,
                visible_indices,
                matrices,
                winds,
                instance_count,
            } => DescriptorConstants::World {
                visible_indices: visible_indices.clone(),
                matrices: matrices.clone(),
                winds: winds.clone(),
                instance_count: *instance_count,
                offsets: constants[200..208].try_into().unwrap(),
                vegetation: true,
            },
        };
        Self {
            mesh: instance.mesh.clone(),
            constants,
            program: instance.program,
            alpha_material: instance.alpha_material.clone(),
            front_face: instance.front_face,
            cull_mode: instance.cull_mode,
            draw_state: instance.draw_state,
            stencil_reference: instance.stencil_reference,
            scissor: instance.scissor,
        }
    }

    fn matches(&self, instance: &NativeShadowInstance) -> bool {
        if self.mesh != instance.mesh
            || self.program != instance.program
            || self.alpha_material != instance.alpha_material
            || self.front_face != instance.front_face
            || self.cull_mode != instance.cull_mode
            || self.draw_state != instance.draw_state
            || self.stencil_reference != instance.stencil_reference
            || self.scissor != instance.scissor
        {
            return false;
        }
        use NativeShadowInstanceConstants::*;
        match (&self.constants, &instance.per_instance_cb) {
            (DescriptorConstants::Rigid, Rigid(_))
            | (DescriptorConstants::Vegetation, Vegetation(_)) => true,
            (DescriptorConstants::Skinned(a), Skinned { palette: b, .. }) => a == b,
            (
                DescriptorConstants::World {
                    visible_indices: ai,
                    matrices: am,
                    winds: None,
                    instance_count: ac,
                    offsets,
                    vegetation: false,
                },
                World {
                    constants,
                    visible_indices: bi,
                    matrices: bm,
                    instance_count: bc,
                },
            ) => ai == bi && am == bm && ac == bc && offsets[..4] == constants[64..68],
            (
                DescriptorConstants::World {
                    visible_indices: ai,
                    matrices: am,
                    winds: aw,
                    instance_count: ac,
                    offsets,
                    vegetation: true,
                },
                WorldVegetation {
                    constants,
                    visible_indices: bi,
                    matrices: bm,
                    winds: bw,
                    instance_count: bc,
                },
            ) => ai == bi && am == bm && aw == bw && ac == bc && offsets[..] == constants[200..208],
            _ => false,
        }
    }
}

/// CB bytes are uploaded on every frame independently of these descriptors.
/// Preserve the offsets/counts used by CPU bounds validation, and every field
/// which changes pipeline, resources, geometry, scissor or draw order.
#[cfg(test)]
fn same_descriptor(a: &NativeShadowInstance, b: &NativeShadowInstance) -> bool {
    let NativeShadowInstance {
        mesh,
        per_instance_cb,
        program,
        alpha_material,
        front_face,
        cull_mode,
        draw_state,
        stencil_reference,
        scissor,
    } = a;
    if mesh != &b.mesh
        || program != &b.program
        || alpha_material != &b.alpha_material
        || front_face != &b.front_face
        || cull_mode != &b.cull_mode
        || draw_state != &b.draw_state
        || stencil_reference != &b.stencil_reference
        || scissor != &b.scissor
    {
        return false;
    }
    use NativeShadowInstanceConstants::*;
    match (per_instance_cb, &b.per_instance_cb) {
        (Rigid(_), Rigid(_)) | (Vegetation(_), Vegetation(_)) => true,
        (Skinned { palette: a, .. }, Skinned { palette: b, .. }) => a == b,
        (
            World {
                constants: a,
                visible_indices: ai,
                matrices: am,
                instance_count: ac,
            },
            World {
                constants: b,
                visible_indices: bi,
                matrices: bm,
                instance_count: bc,
            },
        ) => ai == bi && am == bm && ac == bc && a[64..68] == b[64..68],
        (
            WorldVegetation {
                constants: a,
                visible_indices: ai,
                matrices: am,
                winds: aw,
                instance_count: ac,
            },
            WorldVegetation {
                constants: b,
                visible_indices: bi,
                matrices: bm,
                winds: bw,
                instance_count: bc,
            },
        ) => ai == bi && am == bm && aw == bw && ac == bc && a[200..208] == b[200..208],
        _ => false,
    }
}

pub(super) fn verify_records(a: &[PreparedShadowRecord], b: &[PreparedShadowRecord]) {
    assert_eq!(a.len(), b.len(), "prepared shadow record count changed");
    for (a, b) in a.iter().zip(b) {
        assert!(
            a.layer == b.layer
                && a.scissor == b.scissor
                && a.key == b.key
                && a.pipeline == b.pipeline
                && a.uniform_offset == b.uniform_offset
                && a.instances == b.instances
                && a.vertex.id() == b.vertex.id()
                && a.base_vertex == b.base_vertex
                && a.index.id() == b.index.id()
                && a.indices == b.indices
                && a.index_format == b.index_format
                && a.resources.as_ref().map(|v| v.id()) == b.resources.as_ref().map(|v| v.id()),
            "retained shadow descriptor differs from fresh preparation"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        ecs::system::RunSystemOnce,
        prelude::*,
        render::{render_resource::*, RenderApp},
    };

    #[test]
    fn compact_descriptors_preserve_full_snapshot_comparisons() {
        use NativeShadowInstanceConstants::*;
        let base = super::super::uniform_tests::world_instance(0, 1);
        let mut variants = vec![
            Rigid([0; 80]),
            Vegetation([0; 208]),
            Skinned {
                constants: [0; 112],
                palette: Handle::default(),
            },
            base.per_instance_cb.clone(),
        ];
        for winds in [None, Some(Handle::default())] {
            variants.push(WorldVegetation {
                constants: [0; 208],
                visible_indices: Handle::default(),
                matrices: Handle::default(),
                winds,
                instance_count: 1,
            });
        }
        for constants in &variants {
            let mut a = base.clone();
            a.per_instance_cb = constants.clone();
            let snapshot = DescriptorSnapshot::new(&a);
            let compare = |b: &NativeShadowInstance| {
                assert_eq!(snapshot.matches(b), same_descriptor(&a, b));
            };
            compare(&a);
            for other in &variants {
                let mut b = a.clone();
                b.per_instance_cb = other.clone();
                compare(&b);
            }
            for offset in 0..constants.as_bytes().len() {
                let mut b = a.clone();
                let bytes: &mut [u8] = match &mut b.per_instance_cb {
                    Rigid(bytes) => bytes,
                    Vegetation(bytes) => bytes,
                    Skinned { constants, .. } => constants,
                    World { constants, .. } => constants,
                    WorldVegetation { constants, .. } => constants,
                };
                bytes[offset] ^= 0x81;
                compare(&b);
            }
            let mut b = a.clone();
            if let World { instance_count, .. } | WorldVegetation { instance_count, .. } =
                &mut b.per_instance_cb
            {
                *instance_count += 1;
            }
            compare(&b);
            for word in 0..9 {
                let mut b = a.clone();
                b.draw_state[word] ^= 1;
                compare(&b);
            }
            let mut b = a.clone();
            b.stencil_reference ^= 1;
            compare(&b);
            let mut b = a.clone();
            b.scissor.width -= 1;
            compare(&b);
            let mut b = a.clone();
            b.program = NativeShadowProgram::Unsupported(123);
            compare(&b);
            let mut b = a.clone();
            b.cull_mode = Some(Face::Front);
            compare(&b);
        }
        assert!(
            std::mem::size_of::<DescriptorSnapshot>() < std::mem::size_of::<NativeShadowInstance>()
        );
    }

    #[test]
    fn wind_descriptor_reuse_keeps_validated_offsets_but_not_live_clock() {
        let mut a = super::super::uniform_tests::world_instance(0, 1);
        a.program = NativeShadowProgram::WorldVegetation;
        a.per_instance_cb = NativeShadowInstanceConstants::WorldVegetation {
            constants: [0; NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES],
            visible_indices: Handle::default(),
            matrices: Handle::default(),
            winds: None,
            instance_count: 1,
        };
        let mut b = a.clone();
        if let NativeShadowInstanceConstants::WorldVegetation { constants, .. } =
            &mut b.per_instance_cb
        {
            constants[192..196].copy_from_slice(&3.5f32.to_le_bytes());
        }
        assert!(
            same_descriptor(&a, &b),
            "live clock is read from the updated CB"
        );
        for offset in [200, 204] {
            let mut shifted = b.clone();
            if let NativeShadowInstanceConstants::WorldVegetation { constants, .. } =
                &mut shifted.per_instance_cb
            {
                constants[offset] = 1;
            }
            assert!(
                !same_descriptor(&a, &shifted),
                "CPU-validated offset {offset} must invalidate reuse"
            );
        }
        b.scissor.width -= 1;
        assert!(!same_descriptor(&a, &b));
    }

    #[derive(Resource)]
    struct Input(Vec<NativeShadowLayer>);

    fn draw(
        input: Res<Input>,
        mut gpu: ResMut<NativeShadowSceneGpu>,
        allocator: Res<MeshAllocator>,
        meshes: Res<RenderAssets<RenderMesh>>,
        images: Res<RenderAssets<GpuImage>>,
        buffers: Res<RenderAssets<GpuShaderBuffer>>,
        device: Res<RenderDevice>,
        queue: Res<RenderQueue>,
        adapter: Res<RenderAdapter>,
    ) -> Option<Vec<u16>> {
        upload_native_shadow_uniform_ring(&input.0, &device, &queue, &mut gpu);
        let ring = gpu.uniform_ring.as_ref()?.clone();
        let stride = gpu.uniform_stride;
        let records = prepare_native_shadow_records(
            &input.0, false, true, ring, stride, &mut gpu, &allocator, &meshes, &images, &buffers,
            &device, &adapter,
        )?;
        let depth = NativeShadowDepthArray::new_with_format(
            &device,
            Extent3d {
                width: 32,
                height: 32,
                depth_or_array_layers: 1,
            },
            TextureFormat::Depth16Unorm,
            "prepared descriptor lifetime parity",
        )
        .unwrap();
        let readback = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 8192,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        let draws = directional_layer_draws(&records, &gpu.pipelines, 1);
        draw_native_shadow_groups(&mut encoder, &depth, &gpu.pipelines, draws, input.0[1].ops)
            .unwrap();
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &depth.texture,
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
            depth.size,
        );
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
        let values = bytes
            .chunks_exact(256)
            .flat_map(|row| {
                row[..64]
                    .chunks_exact(2)
                    .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
            })
            .collect();
        drop(bytes);
        readback.unmap();
        Some(values)
    }

    struct RestoreEnv(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            for (name, value) in &self.0 {
                if let Some(value) = value {
                    std::env::set_var(name, value);
                } else {
                    std::env::remove_var(name);
                }
            }
        }
    }

    #[test]
    #[ignore = "requires GPU; validates prepared descriptors against fresh preparation and live changed resources"]
    fn prepared_shadow_descriptors_preserve_live_uniforms_and_resource_lifetimes_gpu() {
        let names = ["DH2_CACHE_PREPARED_SHADOWS", "DH2_VERIFY_PREPARED_SHADOWS"];
        let _restore = RestoreEnv(
            names
                .iter()
                .map(|&name| (name, std::env::var_os(name)))
                .collect(),
        );
        for name in names {
            std::env::set_var(name, "1");
        }
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
        let make_mesh = |shift: f32| {
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                bevy::asset::RenderAssetUsages::default(),
            );
            let vertices = vec![
                [-0.7 + shift, -0.7, 0.3],
                [0.7 + shift, -0.7, 0.3],
                [shift, 0.7, 0.3],
            ];
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertices.clone());
            mesh.insert_attribute(NATIVE_VEGETATION_POSITION, vertices);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.5, 0.5]; 3]);
            mesh.insert_indices(bevy::mesh::Indices::U16(vec![0, 1, 2]));
            mesh
        };
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(make_mesh(0.));
        let ids = app
            .world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .add(ShaderBuffer::from(vec![0u32; 2]));
        let matrices =
            app.world_mut()
                .resource_mut::<Assets<ShaderBuffer>>()
                .add(ShaderBuffer::from(vec![
                    1f32, 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.,
                ]));
        let make_image = |alpha: u8| {
            Image::new_fill(
                Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                &[255, 255, 255, alpha],
                TextureFormat::Rgba8Unorm,
                bevy::asset::RenderAssetUsages::default(),
            )
        };
        let image = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(make_image(255));
        for _ in 0..3 {
            app.update();
        }
        let world = app.sub_app_mut(RenderApp).world_mut();
        world.init_resource::<NativeShadowSceneGpu>();
        let mut instance = super::super::uniform_tests::world_instance(0, 1);
        instance.mesh = mesh.clone();
        crate::render_state::reset_native_render_state(&mut instance.draw_state);
        // Executable-verified LESS comparator, matching this fixture's clear 1.
        instance.draw_state[5] = (instance.draw_state[5] & !7) | 1;
        if let NativeShadowInstanceConstants::World {
            visible_indices,
            matrices: transforms,
            ..
        } = &mut instance.per_instance_cb
        {
            *visible_indices = ids.clone();
            *transforms = matrices.clone();
        }
        let empty = NativeShadowLayer {
            cache: NativeShadowCacheOp::None,
            ops: NativeShadowLayerOps {
                depth: NativeShadowLoad::Clear(1.),
                stencil: NativeStencilLoad::Load,
            },
            instances: Vec::new(),
        };
        world.insert_resource(Input(vec![
            empty.clone(),
            NativeShadowLayer {
                cache: NativeShadowCacheOp::Retained {
                    generation: 7,
                    static_instances: 0,
                },
                ops: empty.ops,
                instances: vec![instance],
            },
            empty,
        ]));
        let run = |app: &mut App| {
            app.sub_app_mut(RenderApp)
                .world_mut()
                .run_system_once(draw)
                .unwrap()
        };
        let counts = |app: &App| {
            let gpu = app
                .sub_app(RenderApp)
                .world()
                .resource::<NativeShadowSceneGpu>();
            (gpu.prepared_hits, gpu.prepared_misses)
        };
        let first = run(&mut app).unwrap();
        assert!(
            first.iter().filter(|&&v| v < u16::MAX).count() > 100,
            "fixture must draw geometry"
        );
        assert_eq!(run(&mut app).unwrap(), first);
        assert!(
            counts(&app).0 > 0,
            "must actually reuse prepared descriptors"
        );
        // Submission policy changes do not change the prepared descriptors.
        app.sub_app_mut(RenderApp)
            .world_mut()
            .resource_mut::<Input>()
            .0[1]
            .cache = NativeShadowCacheOp::Store {
            static_instances: 0,
        };
        let before = counts(&app).0;
        assert_eq!(run(&mut app).unwrap(), first);
        assert!(counts(&app).0 > before);
        // Projection bytes change, but the same bound ring must supply fresh depth.
        if let NativeShadowInstanceConstants::World { constants, .. } = &mut app
            .sub_app_mut(RenderApp)
            .world_mut()
            .resource_mut::<Input>()
            .0[1]
            .instances[0]
            .per_instance_cb
        {
            constants[28..32].copy_from_slice(&0.2f32.to_le_bytes());
        }
        let moved = run(&mut app).unwrap();
        assert_ne!(
            moved, first,
            "retained draws must consume the updated projection"
        );
        assert!(counts(&app).0 >= 2);
        // Remap offset is a validated draw input, not an ignored matrix byte.
        if let NativeShadowInstanceConstants::World { constants, .. } = &mut app
            .sub_app_mut(RenderApp)
            .world_mut()
            .resource_mut::<Input>()
            .0[1]
            .instances[0]
            .per_instance_cb
        {
            constants[64..68].copy_from_slice(&2u32.to_le_bytes());
        }
        assert!(
            run(&mut app).is_none(),
            "invalid remap range must reject cached preparation"
        );
        if let NativeShadowInstanceConstants::World { constants, .. } = &mut app
            .sub_app_mut(RenderApp)
            .world_mut()
            .resource_mut::<Input>()
            .0[1]
            .instances[0]
            .per_instance_cb
        {
            constants[64..68].fill(0);
        }
        assert_eq!(run(&mut app).unwrap(), moved);
        let before = counts(&app).1;
        app.world_mut()
            .resource_mut::<Assets<Mesh>>()
            .insert(mesh.id(), make_mesh(0.25))
            .unwrap();
        for _ in 0..3 {
            app.update();
        }
        let mut relocated = run(&mut app).unwrap();
        assert_ne!(relocated, moved);
        assert!(counts(&app).1 > before);
        // Same-size GPU storage writes preserve allocation identity but must
        // change the image through the retained bind group.
        let buffer_id = app
            .sub_app(RenderApp)
            .world()
            .resource::<RenderAssets<GpuShaderBuffer>>()
            .get(matrices.id())
            .unwrap()
            .buffer
            .id();
        let hits = counts(&app).0;
        *app.world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .get_mut(&matrices)
            .unwrap() = ShaderBuffer::from(vec![1f32, 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.1]);
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(
            buffer_id,
            app.sub_app(RenderApp)
                .world()
                .resource::<RenderAssets<GpuShaderBuffer>>()
                .get(matrices.id())
                .unwrap()
                .buffer
                .id(),
            "fixture must update the same allocation"
        );
        let updated = run(&mut app).unwrap();
        assert_ne!(
            updated, relocated,
            "retained bindings must read live storage contents"
        );
        assert!(counts(&app).0 > hits);
        relocated = updated;
        // A real storage-buffer allocation replacement invalidates resources.
        let before = counts(&app).1;
        *app.world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .get_mut(&ids)
            .unwrap() = ShaderBuffer::from(vec![0u32; 1024]);
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(run(&mut app).unwrap(), relocated);
        assert!(counts(&app).1 > before);
        {
            let mut input = app
                .sub_app_mut(RenderApp)
                .world_mut()
                .resource_mut::<Input>();
            let instance = &mut input.0[1].instances[0];
            instance.program = NativeShadowProgram::WorldAlphaTested;
            instance.alpha_material = Some(NativeShadowAlphaMaterial {
                diffuse: image.clone(),
                uv_set: 0,
                uv_scale_bias: [1., 1., 0., 0.],
            });
            if let NativeShadowInstanceConstants::World { constants, .. } =
                &mut instance.per_instance_cb
            {
                for (i, v) in [1f32, 1., 0., 0.].iter().enumerate() {
                    constants[80 + i * 4..84 + i * 4].copy_from_slice(&v.to_le_bytes());
                }
            }
        }
        let alpha = run(&mut app).unwrap();
        assert_eq!(alpha, relocated);
        assert_eq!(run(&mut app).unwrap(), alpha);
        app.world_mut()
            .resource_mut::<Assets<Image>>()
            .insert(image.id(), make_image(0))
            .unwrap();
        for _ in 0..3 {
            app.update();
        }
        assert!(
            run(&mut app).unwrap().iter().all(|&v| v == u16::MAX),
            "replacement alpha image must affect retained draws"
        );
        let before = counts(&app).1;
        app.sub_app_mut(RenderApp)
            .world_mut()
            .resource_mut::<NativeShadowSceneGpu>()
            .uniform_ring = None;
        assert!(run(&mut app).unwrap().iter().all(|&v| v == u16::MAX));
        assert!(
            counts(&app).1 > before,
            "new uniform ring must invalidate prepared bindings"
        );
        app.sub_app_mut(RenderApp)
            .world_mut()
            .resource_mut::<RenderAssets<RenderMesh>>()
            .remove(mesh.id());
        assert!(
            run(&mut app).is_none(),
            "missing GPU mesh must reject cached preparation"
        );
    }
}
