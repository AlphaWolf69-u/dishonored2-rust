//! Setup.
use super::{
    apply_rest_pose, authored_area_light_transform, authored_light_cosines, authored_mesh,
    authored_world_bounds, auto_exposure, bmd6, bmodel, camera_projection, coarse_depth, entities,
    env, environment_sun_ray, expanded_model_placements, f8_camera_pose, f8_reference_fov, fs,
    game_affine_to_view, game_to_view, initial_placed_model_query, light_backend_intensity,
    light_distance, light_preset, light_projection_bounds, light_proximity_from_cached_bounds,
    local_light_clip_to_world, local_shadow_updates, material, mean_game_position, navigation,
    needs_exact_world_mesh, placed_direction, placed_transform, placed_vertex,
    preserved_half_point_angular_scale, probe_lighting, render_bounds, render_light, render_model,
    rest_skinning, retain_model_shadow_position, retain_skin_inputs, runtime, shadow_pipeline,
    shadow_sampling, shadow_updates, source_mesh, surface_sampler_image, texture, tome,
    transformed, transformed_direction, view_of, view_to_game, Arc, BTreeMap, Bounds,
    CascadeShadowConfigBuilder, CoordinateOverlay, DecalMaterial, DepthPrepass, DiffuseMipState,
    FocusTarget, FocusTargets, FrameRateOverlay, HashMap, HashSet, LocalLightCull,
    MaterialResolver, NavigationState, NavigationWorld, PathBuf, PickupInstance, PickupState,
    PlacedModelQueries, PlacedModelQueryGroup, ProbeMaterial, ProgramCacheSource, Projection,
    RectLight, RenderDevice, RenderInstance, RenderInstanceSource, RenderModelSurface,
    RenderSourcePlacements, RenderSuppressionGroup, SceneData, SceneRenderQueue, ShaderBuffer,
    ShadowMaterials, ShadowModelBoundsSource, ShadowModelBoundsSources, TextureFormat,
    TextureReadWorker, TomeRenderInstanceMap, VegetationMeshInputs, ViewSettings,
    WorldShadowMeshSources, DEFAULT_FAR, DEFAULT_HORIZONTAL_FOV_DEGREES, DEFAULT_NEAR,
};
use bevy::prelude::*;

// Model vtable+88 (1bd8b0): the center of the current world bounds,
// shared by every material surface, rather than its vertex-density centroid.
fn model_probe_position(placement: &entities::PlacedModel, local: [[f32; 3]; 2]) -> [f32; 3] {
    let bounds = render_model::transformed_bounds(
        local,
        placement.position,
        placement.rotation,
        placement.scale,
    );
    std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5)
}

// Shared by ordinary parallel lights and the vista global-sun transport.
// Vista sun visibility comes from its baked lightmap, independent of shadow
// atlas readiness and the clustered renderer's current directional-light slot.
fn parallel_light_inputs(
    light: &entities::PlacedLight,
    environment: Option<entities::EnvironmentLighting>,
    preset_color: [f32; 3],
) -> ([f32; 3], Vec3) {
    let is_sun = light.preset.as_deref() == Some("LIGHT_PRESET_SUN");
    let color = if is_sun {
        render_light::sun_ground_color(
            environment
                .and_then(|e| e.sun_color)
                .unwrap_or([0.215861; 3]),
        )
    } else {
        preset_color
    };
    let authored = game_to_view(light.rotation[0]).normalize_or_zero();
    let direction = if is_sun {
        environment
            .and_then(environment_sun_ray)
            .filter(|d| *d != Vec3::ZERO)
            .unwrap_or(authored)
    } else {
        authored
    };
    (color, direction)
}

pub(super) fn setup(
    mut commands: Commands,
    mut scene: ResMut<SceneData>,
    mut programs: Option<ResMut<ProgramCacheSource>>,
    navigation: Option<Res<NavigationWorld>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut probe_materials: ResMut<Assets<ProbeMaterial>>,
    mut sky_materials: ResMut<Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>>,
    mut probe_sh_buffers: ResMut<Assets<ShaderBuffer>>,
    render_device: Res<RenderDevice>,
    mut decal_materials: ResMut<Assets<DecalMaterial>>,
    view_settings: Res<ViewSettings>,
    mut world_matrices: ResMut<super::world_instances::WorldMatrices>,
) {
    // Take the catalog to avoid borrowing the scene while setup mutates its
    // parsed world, then transfer ownership to the texture worker below.
    let catalog_storage = scene.catalog.take();
    let catalog = catalog_storage.as_ref();
    let fog_material = crate::rendering::bevy_backend::fog_material::FogMaterialResources::new(
        &mut images,
        &mut probe_sh_buffers,
    );
    commands.insert_resource(fog_material.clone());
    if let (Some(catalog), Some(parameters)) =
        (catalog, scene.environment_render_parameters.as_ref())
    {
        let sun = scene
            .environment_lighting
            .and_then(|lighting| {
                lighting
                    .sun_theta_degrees
                    .zip(lighting.sun_phi_degrees)
                    .zip(lighting.sun_color)
            })
            .map(|((theta, phi), colour)| {
                (
                    Vec3::from_array(crate::render_shadow::environment_sun_direction(theta, phi)),
                    Vec3::from_array(render_light::sun_ground_color(colour)),
                )
            });
        let program_library = programs
            .as_deref_mut()
            .expect("atmosphere requires the installed program cache")
            .0
            .as_mut()
            .unwrap_or_else(|error| panic!("atmosphere program cache unavailable: {error}"));
        let mut fog = super::fog_scene::FogScene::load(
            catalog,
            program_library,
            parameters,
            &scene.area_environments,
            sun,
            DEFAULT_FAR,
            &mut images,
        )
        .unwrap_or_else(|error| panic!("fog scene preparation failed: {error}"));
        let screen_fx = super::pause_fx::load(catalog)
            .inspect_err(|e| warn!("Pause screen FX unavailable: {e}"))
            .ok();
        let screen_fx_environment = screen_fx
            .as_ref()
            .and_then(|fx| {
                super::pause_fx::environment_parameters(catalog, &fx.environment)
                    .inspect_err(|e| warn!("Pause screen FX environment: {e}"))
                    .ok()
            });
        commands.insert_resource(super::pause_fx::PauseScreenFx::new(screen_fx));
        let display = super::display_scene::DisplayScene::load(
            catalog,
            parameters,
            &scene.area_environments,
            screen_fx_environment.as_ref(),
            &mut images,
        )
        .unwrap_or_else(|error| panic!("display scene preparation failed: {error}"));
        fog.upload_volumes(std::mem::take(&mut scene.area_fog), &mut meshes);
        commands.insert_resource(fog);
        commands.insert_resource(display);
    }
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(12),
                left: px(12),
                padding: UiRect::all(px(8)),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            ZIndex(100),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("FPS: measuring\nRAM: measuring"),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                FrameRateOverlay,
            ));
            parent.spawn((
                Text::new("Game XYZ: loading"),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                CoordinateOverlay,
            ));
        });
    let preferred_mip = env::var("DH2_TEXTURE_MIP")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|&value| value <= 15);
    let lit = env::var("DH2_LIT").as_deref() != Ok("0");
    // The game-derived atlas owners render shadows and honor m_NoShadows.
    // Backend light components transport radiance only: enabling Bevy maps
    // creates a second set of shadow views and multiplies local attenuation.
    // Profiling switch: leave probe/environment lighting on while isolating scene lights.
    let scene_lights_enabled = env::var("DH2_SCENE_LIGHTS").as_deref() != Ok("0");
    let trace_startup = env::var("DH2_TRACE_STARTUP").as_deref() == Ok("1");
    let trace_bwm_lod = env::var("DH2_TRACE_BWM_LOD").as_deref() == Ok("1");
    if trace_bwm_lod {
        eprintln!(
            "BWM LOD visibility trace enabled; reports appear when authored-tag visibility counts change"
        );
    }
    if trace_startup {
        eprintln!(
            "Startup trace: beginning world setup ({} BWM objects, {} world meshes, {} placed models)",
            scene.world.objects.len(),
            scene.world.meshes.len(),
            scene.placed_models.len()
        );
    }
    // Preserve ordered primary probe IDs in binding100; per-object records
    // are appended after them. World geometry uses its PNW instance offset.
    let mut records = scene
        .environment_probes
        .as_ref()
        .map(|set| {
            set.probes
                .iter()
                .map(|probe| {
                    probe.render_lighting_sh.map(|v| {
                        if lit {
                            Vec4::new(v[0], v[1], v[2], 0.0)
                        } else {
                            Vec4::ZERO
                        }
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if records.is_empty() {
        records.push([Vec4::ZERO; 9]);
    }
    let words = scene
        .environment_probes
        .as_ref()
        .map(probe_lighting::ProbeLightingSet::vertex_shader_words)
        .unwrap_or_else(|| vec![0u32; 8]);
    let bytes = words.len() as u64 * 4;
    let limits = render_device.limits();
    assert!(
        bytes <= u64::from(limits.max_storage_buffer_binding_size)
            && bytes <= limits.max_buffer_size,
        "PNW vertex lighting requires a {bytes}-byte buffer; device storage limit={}, buffer limit={}",
        limits.max_storage_buffer_binding_size,
        limits.max_buffer_size
    );
    println!(
        "Vertex-probe GPU data: {} primary records, {bytes} bytes (device storage limit {})",
        records.len(),
        limits.max_storage_buffer_binding_size
    );
    let vertex_probe_buffer = probe_sh_buffers.add(ShaderBuffer::from(words));
    let probe_sh_buffer = probe_sh_buffers.add(ShaderBuffer::from(records.clone()));
    let shadow_buffer = probe_sh_buffers.add(ShaderBuffer::from(
        shadow_sampling::ParallelSamplingUniform::default(),
    ));
    let shadow_map = images.add(
        shadow_pipeline::shadow_depth_image(
            bevy::render::render_resource::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureFormat::Depth24PlusStencil8,
        )
        .expect("unavailable depth binding descriptor"),
    );
    commands.insert_resource(shadow_sampling::ParallelSamplingBuffer {
        buffer: shadow_buffer.clone(),
        unavailable_depth: shadow_map.clone(),
    });
    let local_capacity = scene.placed_lights.len().max(1).next_power_of_two() as u32;
    let local_shadow_buffer = probe_sh_buffers.add(ShaderBuffer::from(
        vec![0u32; shadow_sampling::local_sampling_word_count(local_capacity)],
    ));
    commands.insert_resource(shadow_sampling::LocalSamplingBuffer {
        buffer: local_shadow_buffer.clone(),
        light_capacity: local_capacity,
    });
    let anisotropic_sampler_image =
        images.add(surface_sampler_image(catalog).expect("invalid original material sampler"));
    // The original publishes parallelLightDir/Color from the environment
    // even in levels with no placed LIGHT_PRESET_SUN entity (Dust District).
    let vista_sun = scene
        .environment_lighting
        .and_then(|environment| {
            let ray = environment_sun_ray(environment)?;
            let color =
                render_light::sun_ground_color(environment.sun_color.unwrap_or([0.215861; 3]));
            Some([Vec3::from_array(color).extend(0.), (-ray).extend(0.)])
        })
        .unwrap_or([Vec4::ZERO; 2]);
    let mut linear_clamp = surface_sampler_image(catalog).expect("sampler source unavailable");
    let bevy::image::ImageSampler::Descriptor(sampler) = &mut linear_clamp.sampler else {
        unreachable!()
    };
    sampler.set_address_mode(bevy::image::ImageAddressMode::ClampToEdge);
    sampler.anisotropy_clamp = 1;
    let linear_clamp_image = images.add(linear_clamp);
    let material_exposure = probe_sh_buffers.add(auto_exposure::initial_material_exposure());
    let vegetation = super::vegetation_wind::VegetationMaterials::new(&mut probe_sh_buffers);
    let loaded_reflections =
        super::environment_reflection::load(
            catalog,
            scene.environment_probes.as_ref(),
            &mut images,
            &mut probe_sh_buffers,
        )
        .expect("installed environment reflections");
    let water_probes = loaded_reflections.water_probes(
        scene.environment_probes.as_ref(),
        vertex_probe_buffer.clone(),
        &mut probe_sh_buffers,
    );
    commands.insert_resource(crate::rendering::bevy_backend::water_scene::WaterLighting(
        water_probes,
    ));
    let reflections = loaded_reflections.image;
    let reflection_planes = loaded_reflections.planes;
    let reflection_brdf_lookup = loaded_reflections.brdf_lookup;
    let mut resolver = MaterialResolver {
        fog_material: fog_material.bindings(),
        reflections: Some(reflections),
        reflection_brdf_lookup,
        reflection_planes,
        material_exposure: material_exposure.clone(),
        environment: scene.environment_render_parameters.as_ref(),
        linear_clamp_image,
        vista_sun,
        anisotropic_sampler_image,
        catalog,
        programs: programs.as_deref_mut().map(|source| &mut source.0),
        probe_materials: &mut probe_materials,
        sky_materials: &mut sky_materials,
        decal_materials: &mut decal_materials,
        probe_sh_buffers: &mut probe_sh_buffers,
        probe_sh_buffer,
        vertex_probe_buffer,
        shadow_buffer,
        local_shadow_buffer,
        shadow_map,
        probe_sh_records: records,
        probes: scene.environment_probes.as_ref(),
        cache: default(),
        shadow_materials: ShadowMaterials::default(),
        vegetation,
        water_materials: default(),
        boat_materials: default(),
        diffuse_image_slots: HashMap::new(),
        diffuse_image_paths: Vec::new(),
        misses: 0,
        suppressed_transparent: 0,
        cutout: 0,
        blended: 0,
        probe_samples: 0,
        probe_sample_misses: 0,
        lit,
    };
    let mut whole_world = Bounds::empty();
    let mut regions = BTreeMap::<i32, Bounds>::new();
    let mut world_shadow_mesh_sources = WorldShadowMeshSources::prepare(&scene.world, &mut meshes);
    let mut world_admissions = HashMap::<String, Result<u8, String>>::new();
    let mut vegetation_meshes = VegetationMeshInputs::default();
    let mut baked_shear_instances = 0usize;
    let mut authored_lod_instances = BTreeMap::<usize, usize>::new();
    let mut render_instances = Vec::new();
    let mut bwm_visibility_render_instances = vec![Vec::new(); scene.world.visibility.len()];
    for (object_number, object) in scene.world.objects.iter().enumerate() {
        *authored_lod_instances.entry(object.lod).or_default() +=
            object.matrix_end - object.matrix_start;
        if trace_startup {
            eprintln!(
                "Startup trace: BWM object {}/{} mesh={} lod={} matrices={}..{} material={}",
                object_number + 1,
                scene.world.objects.len(),
                object.mesh_index,
                object.lod,
                object.matrix_start,
                object.matrix_end,
                object.material_path
            );
        }
        // Preserve every serialized BWM representation until the native
        // model/world draw selector is traced. FUN_140a82710 is the Tome
        // object-visibility query implemented in tome.rs, not an LOD selector;
        // filtering to LOD 0 here would discard authored representations.
        let null_material = material::is_null_material(&object.material_path);
        let source = &scene.world.meshes[object.mesh_index];
        // The auxiliary whole-view pass owns native +0x18 material pointers
        // independently of the color-visible render entities. Resolve this
        // object's actual material independently of colour admission; never turn
        // that absence into a no-caster flag.
        let admission = world_admissions
            .entry(object.material_path.clone())
            .or_insert_with(|| resolver.caster_admission(&object.material_path))
            .clone();
        world_shadow_mesh_sources.objects[object_number].gather_flags48 = Some(admission);
        let first_material = if let Some(matrix) = scene.world.matrices.get(object.matrix_start) {
            let points: Vec<Vec3> = source
                .positions
                .iter()
                .map(|&position| transformed(matrix, position))
                .collect();
            resolver.get_world(
                &object.material_path,
                object_number,
                mean_game_position(&points),
            )
        } else {
            None
        };
        if let Some(material) = &first_material {
            world_shadow_mesh_sources
                .bind_material(object_number, &material.handle)
                .expect("resolved native BWM object/material ownership");
        } else {
            world_shadow_mesh_sources
                .record_material_error(
                    object_number,
                    format!(
                        "BWM object {object_number} material {} could not be resolved",
                        object.material_path
                    ),
                )
                .expect("prepared native BWM object association");
        }
        // Only the serialized NULL sentinel is absent. All real material paths
        // are admitted through their resolved program and visibility flags.
        if null_material {
            continue;
        }
        // layer0_vtxcolormaskchannel blends material layers, not final opacity.
        // Using it as vertex alpha made 1,059 authored world objects translucent.
        // Reuse the unbaked shared forward mesh, which also retains explicit
        // raw local attributes for native world-shadow vertex declarations.
        let mesh = world_shadow_mesh_sources
            .object(object_number)
            .expect("prepared native BWM object association")
            .mesh
            .clone();
        for matrix_index in object.matrix_start..object.matrix_end {
            let matrix = &scene.world.matrices[matrix_index];
            let visibility_record = &scene.world.visibility[matrix_index];
            let points: Vec<Vec3> = source
                .positions
                .iter()
                .map(|&position| transformed(matrix, position))
                .collect();
            if trace_startup {
                eprintln!(
                    "Startup trace: resolving BWM material object={} matrix={} path={}",
                    object_number, matrix_index, object.material_path
                );
            }
            let mat = if matrix_index == object.matrix_start {
                first_material.clone()
            } else {
                resolver.get_world(
                    &object.material_path,
                    object_number,
                    mean_game_position(&points),
                )
            };
            let Some(mat) = mat else {
                world_shadow_mesh_sources
                    .record_material_error(
                        object_number,
                        format!(
                            "BWM object {object_number} material {} could not be resolved",
                            object.material_path
                        ),
                    )
                    .expect("prepared native BWM object association");
                continue;
            };
            world_shadow_mesh_sources
                .bind_material(object_number, &mat.handle)
                .expect("resolved BWM object/material ownership");
            for &point in &points {
                whole_world.add(point);
                regions
                    .entry((point.x / 35.0).floor() as i32)
                    .or_insert_with(Bounds::empty)
                    .add(point);
            }
            let instance = game_affine_to_view(matrix);
            let bounds = authored_world_bounds(visibility_record);
            let render_instance = if needs_exact_world_mesh(&instance, &source.positions, &points) {
                // Bevy's Transform cannot express shear. Preserve the exact
                // authored affine transform in this instance's vertex data.
                let baked = meshes.add(authored_mesh(
                    &points,
                    &source.uvs,
                    &source.indices,
                    &source.packed_channels,
                    |v| transformed_direction(matrix, v),
                ));
                let render_instance = RenderInstance {
                    source: Some(RenderInstanceSource {
                        surface: RenderModelSurface::World {
                            object: object_number,
                            matrix: matrix_index,
                        },
                        world_baked: true,
                    }),
                    mesh: baked,
                    material: mat.handle.clone(),
                    diffuse_image_slot: mat.diffuse_image_slot,
                    probe_index: if mat.vertex_lighting && scene.environment_probes.is_some() {
                        0x80000000 | matrix_index as u32
                    } else {
                        mat.probe_index
                    },
                    transform: Transform::IDENTITY,
                    bounds,
                    pickup_id: None,
                    entity: None,
                    visible: false,
                };
                baked_shear_instances += 1;
                render_instance
            } else {
                RenderInstance {
                    source: Some(RenderInstanceSource {
                        surface: RenderModelSurface::World {
                            object: object_number,
                            matrix: matrix_index,
                        },
                        world_baked: false,
                    }),
                    mesh: mesh.clone(),
                    material: mat.handle.clone(),
                    diffuse_image_slot: mat.diffuse_image_slot,
                    probe_index: if mat.vertex_lighting && scene.environment_probes.is_some() {
                        0x80000000 | matrix_index as u32
                    } else {
                        mat.probe_index
                    },
                    transform: instance,
                    bounds,
                    pickup_id: None,
                    entity: None,
                    visible: false,
                }
            };
            vegetation_meshes.prepare(
                resolver.needs_vegetation_channels(&render_instance.material),
                &render_instance.mesh,
                &mut meshes,
                &source.positions,
                &source.packed_channels,
            );
            let render_index = render_instances.len();
            bwm_visibility_render_instances[matrix_index].push(render_index);
            render_instances.push(render_instance);
        }
    }
    if trace_startup {
        eprintln!(
            "Startup trace: BWM setup finished ({} render instances, {} diffuse paths discovered); beginning placed-model setup",
            render_instances.len(),
            resolver.diffuse_image_paths.len()
        );
    }
    let expanded_placements = catalog
        .as_ref()
        .map(|catalog| expanded_model_placements(&scene.placed_models, catalog))
        .unwrap_or_else(|| scene.placed_models.clone());
    let mut model_cache = HashMap::<String, Option<bmodel::Model>>::new();
    let mut md6_cache = HashMap::<String, Option<bmd6::ModelInfo>>::new();
    let mut md6_rest_pose_instances = 0usize;
    let mut model_mesh_handles = HashMap::<(String, usize, bool, u8), Handle<Mesh>>::new();
    let pickup_ids: HashSet<&str> = scene
        .pickups
        .iter()
        .map(|pickup| pickup.entity_id.as_str())
        .collect();
    let mut model_instances = 0usize;
    let mut model_surfaces = 0usize;
    let mut placed_model_queries = Vec::new();
    let mut shadow_model_bounds = Vec::new();
    let mut model_shadow_sources = Vec::new();
    let mut skin_palettes =
        HashMap::<String, Result<Arc<shadow_updates::RestSkinning>, String>>::new();
    let mut placed_model_default_ranges = 0usize;
    for (number, placement) in expanded_placements.iter().enumerate() {
        if trace_startup {
            eprintln!(
                "Startup trace: placed model {}/{} id={} resource={}",
                number + 1,
                expanded_placements.len(),
                placement.id,
                placement.resource_path
            );
        }
        if placement.resource_path.ends_with(".blueprint.decl") {
            continue;
        }
        if placement.resource_path.ends_with(".bmd6model") {
            let model = md6_cache
                .entry(placement.resource_path.clone())
                .or_insert_with(|| {
                    let catalog = catalog.as_ref()?;
                    let mesh_path =
                        match bmd6::resolve_compiled_mesh(&placement.resource_path, |decl| {
                            catalog
                                .read(decl)
                                .ok()
                                .flatten()
                                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                        }) {
                            Ok(path) => path,
                            Err(error) => {
                                eprintln!(
                                    "MD6 declaration failed {}: {error}",
                                    placement.resource_path
                                );
                                return None;
                            }
                        };
                    let bytes = catalog.read(&mesh_path).ok()??;
                    match bmd6::inspect(&bytes) {
                        Ok(model) if model.has_decoded_geometry => Some(model),
                        Ok(_) => None,
                        Err(error) => {
                            eprintln!("MD6 parse failed {}: {error}", placement.resource_path);
                            None
                        }
                    }
                });
            if let Some(model) = model {
                let skinning = skin_palettes
                    .entry(placement.resource_path.clone())
                    .or_insert_with(|| {
                        rest_skinning(
                            catalog.as_ref().ok_or("skin catalog is unavailable")?,
                            &placement.resource_path,
                            model,
                            resolver.probe_sh_buffers,
                        )
                    })
                    .clone();
                model_instances += 1;
                if model.has_skeleton {
                    md6_rest_pose_instances += 1;
                }
                let model_render_start = render_instances.len();
                let caster_surfaces = model
                    .surfaces
                    .iter()
                    .enumerate()
                    .map(|(surface_number, surface)| {
                        let path = placement
                            .custom_materials
                            .get(surface_number)
                            .and_then(Option::as_deref)
                            .unwrap_or(&surface.material_path);
                        let flags = world_admissions
                            .entry(path.to_owned())
                            .or_insert_with(|| resolver.caster_admission(path))
                            .clone();
                        (surface_number, surface.vertex_count as i32, flags)
                    })
                    .collect();
                for (surface_number, surface) in model.surfaces.iter().enumerate() {
                    let material_path = placement
                        .custom_materials
                        .get(surface_number)
                        .and_then(Option::as_deref)
                        .unwrap_or(&surface.material_path);
                    if material::is_null_material(material_path) {
                        continue;
                    }
                    let points: Vec<Vec3> = surface
                        .positions
                        .iter()
                        .map(|&vertex| placed_vertex(placement, vertex))
                        .collect();
                    if points.is_empty() || surface.indices.is_empty() {
                        continue;
                    }
                    let Some(mat) = resolver.get(
                        material_path,
                        number + scene.world.objects.len(),
                        model_probe_position(placement, model.authored_bounds),
                    ) else {
                        continue;
                    };
                    let bounds = render_bounds(&points);
                    let instance = placed_transform(placement);
                    let world_baked =
                        needs_exact_world_mesh(&instance, &surface.positions, &points);
                    let skin_influences = resolver.shadow_materials.skin_influences(&mat.handle);
                    let (mesh, transform) = if world_baked {
                        (
                            meshes.add(authored_mesh(
                                &points,
                                &surface.uvs,
                                &surface.indices,
                                &surface.packed_channels,
                                |v| placed_direction(placement, v),
                            )),
                            Transform::IDENTITY,
                        )
                    } else {
                        (
                            model_mesh_handles
                                .entry((
                                    placement.resource_path.clone(),
                                    surface_number,
                                    instance.to_matrix().determinant() < 0.0,
                                    skin_influences,
                                ))
                                .or_insert_with(|| {
                                    meshes.add(source_mesh(
                                        &surface.positions,
                                        &surface.uvs,
                                        &surface.indices,
                                        &surface.packed_channels,
                                        instance.to_matrix().determinant() < 0.0,
                                    ))
                                })
                                .clone(),
                            instance,
                        )
                    };
                    retain_model_shadow_position(&mut meshes, &mesh, &surface.positions);
                    if let Some(mut mesh) = meshes.get_mut(&mesh) {
                        retain_skin_inputs(&mut mesh, surface);
                        if skin_influences != 0 {
                            match skinning.as_ref().map_err(Clone::clone).and_then(|skin| {
                                apply_rest_pose(
                                    &mut mesh,
                                    surface,
                                    &skin.matrices,
                                    skin_influences,
                                    placement,
                                    world_baked,
                                )
                            }) {
                                Ok(()) => {}
                                Err(error) => eprintln!(
                                    "Model rest pose pending {}: {error}",
                                    placement.resource_path
                                ),
                            }
                        }
                    }
                    vegetation_meshes.prepare(
                        resolver.needs_vegetation_channels(&mat.handle),
                        &mesh,
                        &mut meshes,
                        &surface.positions,
                        &surface.packed_channels,
                    );
                    render_instances.push(RenderInstance {
                        source: Some(RenderInstanceSource {
                            surface: RenderModelSurface::Bmd6 {
                                placement: number,
                                surface: surface_number,
                                cpu_vertex_declaration: surface.cpu_vertex_declaration,
                                gpu_vertex_declaration: surface.gpu_vertex_declaration,
                                skin_influences: surface.skin_influences,
                                matrix_offset: surface.matrix_offset,
                            },
                            world_baked,
                        }),
                        mesh,
                        material: mat.handle,
                        diffuse_image_slot: mat.diffuse_image_slot,
                        probe_index: mat.probe_index,
                        transform,
                        bounds,
                        pickup_id: None,
                        entity: None,
                        visible: false,
                    });
                    model_surfaces += 1;
                }
                let caster_bounds = render_model::transformed_bounds(
                    model.authored_bounds,
                    placement.position,
                    placement.rotation,
                    placement.scale,
                );
                model_shadow_sources.push(shadow_updates::ModelShadowSource {
                    placement: number,
                    local_bounds: model.authored_bounds,
                    bounds: caster_bounds,
                    skinned: true,
                    surfaces: caster_surfaces,
                    skinning: Some(skinning),
                    instances: model_render_start..render_instances.len(),
                    pickup_id: pickup_ids
                        .contains(placement.id.as_str())
                        .then(|| placement.id.clone()),
                    flags_1c30: render_model::model_owner_bounds_flags(
                        0,
                        [
                            caster_bounds[0][0],
                            caster_bounds[0][1],
                            caster_bounds[0][2],
                            0.0,
                        ],
                        [
                            caster_bounds[1][0],
                            caster_bounds[1][1],
                            caster_bounds[1][2],
                            0.0,
                        ],
                    ),
                    flags_18bc: render_model::INITIAL_MODEL_CONTEXT_FLAGS_18BC,
                    flags_18bd: 0,
                });
                if model_render_start < render_instances.len() {
                    shadow_model_bounds.push(ShadowModelBoundsSource {
                        placement: number,
                        instances: model_render_start..render_instances.len(),
                        bounds: render_model::transformed_bounds(
                            model.authored_bounds,
                            placement.position,
                            placement.rotation,
                            placement.scale,
                        ),
                    });
                }
            }
            continue;
        }
        // Authoring helpers carry debug colours but are not visible objects.
        if placement.resource_path.contains("/collisions/")
            || placement.resource_path.contains("/templates/occlusion/")
            || placement.resource_path.contains("/skiff_01_noswim")
        {
            continue;
        }
        let model = model_cache
            .entry(placement.resource_path.clone())
            .or_insert_with(|| {
                let catalog = catalog.as_ref()?;
                let bytes = match catalog.read(&placement.resource_path) {
                    Ok(Some(bytes)) => bytes,
                    Ok(None) => {
                        eprintln!("Model not indexed: {}", placement.resource_path);
                        return None;
                    }
                    Err(error) => {
                        eprintln!("Model read failed {}: {error}", placement.resource_path);
                        return None;
                    }
                };
                match bmodel::parse(&bytes) {
                    Ok(model) => Some(model),
                    Err(error) => {
                        eprintln!("Model parse failed {}: {error}", placement.resource_path);
                        None
                    }
                }
            });
        let Some(model) = model else {
            continue;
        };
        model_instances += 1;
        let query_input = initial_placed_model_query(placement, model.authored_bounds);
        placed_model_default_ranges += usize::from(placement.max_visible_range.is_none());
        let model_render_start = render_instances.len();
        let caster_surfaces = model
            .surfaces
            .iter()
            .enumerate()
            .map(|(surface_number, surface)| {
                let path = placement
                    .custom_materials
                    .get(surface_number)
                    .and_then(Option::as_deref)
                    .unwrap_or(&surface.material_path);
                let flags = world_admissions
                    .entry(path.to_owned())
                    .or_insert_with(|| resolver.caster_admission(path))
                    .clone();
                (surface_number, surface.positions.len() as i32, flags)
            })
            .collect();
        for (surface_number, surface) in model.surfaces.iter().enumerate() {
            let material_path = placement
                .custom_materials
                .get(surface_number)
                .and_then(Option::as_deref)
                .unwrap_or(&surface.material_path);
            if material::is_null_material(material_path) {
                continue;
            }
            let points: Vec<Vec3> = surface
                .positions
                .iter()
                .map(|&vertex| placed_vertex(placement, vertex))
                .collect();
            for &point in &points {
                whole_world.add(point);
                regions
                    .entry((point.x / 35.0).floor() as i32)
                    .or_insert_with(Bounds::empty)
                    .add(point);
            }
            let Some(mat) = resolver.get(
                material_path,
                number + scene.world.objects.len(),
                model_probe_position(placement, model.authored_bounds),
            ) else {
                continue;
            };
            let bounds = render_bounds(&points);
            let instance = placed_transform(placement);
            let world_baked = needs_exact_world_mesh(&instance, &surface.positions, &points);
            let (mesh, transform) = if world_baked {
                baked_shear_instances += 1;
                (
                    meshes.add(authored_mesh(
                        &points,
                        &surface.uvs,
                        &surface.indices,
                        &surface.packed_channels,
                        |v| placed_direction(placement, v),
                    )),
                    Transform::IDENTITY,
                )
            } else {
                (
                    model_mesh_handles
                        .entry((
                            placement.resource_path.clone(),
                            surface_number,
                            instance.to_matrix().determinant() < 0.0,
                            0,
                        ))
                        .or_insert_with(|| {
                            meshes.add(source_mesh(
                                &surface.positions,
                                &surface.uvs,
                                &surface.indices,
                                &surface.packed_channels,
                                instance.to_matrix().determinant() < 0.0,
                            ))
                        })
                        .clone(),
                    instance,
                )
            };
            retain_model_shadow_position(&mut meshes, &mesh, &surface.positions);
            vegetation_meshes.prepare(
                resolver.needs_vegetation_channels(&mat.handle),
                &mesh,
                &mut meshes,
                &surface.positions,
                &surface.packed_channels,
            );
            render_instances.push(RenderInstance {
                source: Some(RenderInstanceSource {
                    surface: RenderModelSurface::Bmodel {
                        placement: number,
                        surface: surface_number,
                    },
                    world_baked,
                }),
                mesh,
                material: mat.handle,
                diffuse_image_slot: mat.diffuse_image_slot,
                probe_index: mat.probe_index,
                transform,
                bounds,
                pickup_id: pickup_ids
                    .contains(placement.id.as_str())
                    .then(|| placement.id.clone()),
                entity: None,
                visible: false,
            });
            model_surfaces += 1;
        }
        let caster_bounds = render_model::transformed_bounds(
            model.authored_bounds,
            placement.position,
            placement.rotation,
            placement.scale,
        );
        model_shadow_sources.push(shadow_updates::ModelShadowSource {
            placement: number,
            local_bounds: model.authored_bounds,
            bounds: caster_bounds,
            skinned: false,
            surfaces: caster_surfaces,
            skinning: None,
            instances: model_render_start..render_instances.len(),
            pickup_id: pickup_ids
                .contains(placement.id.as_str())
                .then(|| placement.id.clone()),
            flags_1c30: render_model::model_owner_bounds_flags(
                0,
                [
                    caster_bounds[0][0],
                    caster_bounds[0][1],
                    caster_bounds[0][2],
                    0.0,
                ],
                [
                    caster_bounds[1][0],
                    caster_bounds[1][1],
                    caster_bounds[1][2],
                    0.0,
                ],
            ),
            flags_18bc: render_model::INITIAL_MODEL_CONTEXT_FLAGS_18BC,
            flags_18bd: 0,
        });
        if let Some(input) = query_input {
            if model_render_start < render_instances.len() {
                shadow_model_bounds.push(ShadowModelBoundsSource {
                    placement: number,
                    instances: model_render_start..render_instances.len(),
                    bounds: input.bounds,
                });
                placed_model_queries.push(PlacedModelQueryGroup {
                    indices: model_render_start..render_instances.len(),
                    input,
                });
            }
        }
    }
    println!(
        "Initial BML query groups: {}; constructor-default range models: {}; blueprint/MD6 and surface bit40 producers remain unsupported",
        placed_model_queries.len(),
        placed_model_default_ranges
    );
    if trace_startup {
        eprintln!(
            "Startup trace: placed-model setup finished ({} render instances, {} diffuse paths discovered); beginning scene finalization",
            render_instances.len(),
            resolver.diffuse_image_paths.len()
        );
    }
    let probe_sh_record_count = resolver.probe_sh_records.len().saturating_sub(1);
    commands.insert_resource(super::model_motion::ModelProbeLighting(
        resolver.probe_sh_buffer.clone(),
    ));
    resolver.upload_probe_records();
    println!(
        "Shared geometry: {} world meshes and {} placed-model surfaces; {} world instances baked for shear",
        world_shadow_mesh_sources.shared_mesh_count,
        model_mesh_handles.len(),
        baked_shear_instances
    );
    println!("BWM authored instance counts by LOD tag: {authored_lod_instances:?}");
    println!(
        "Placed models: {model_instances}/{} instances, {} unique resources, {model_surfaces} surfaces",
        scene.placed_models.len(),
        model_cache.len()
    );
    println!(
        "MD6 skeletal instances shown in compiled rest pose: {md6_rest_pose_instances}; joint animation is not implemented"
    );
    println!(
        "Materials: {} unique, {} registered texture resources, {} without diffuse ({} transparent surfaces suppressed); initial mip {}",
        resolver.cache.len(),
        resolver.diffuse_image_paths.len(),
        resolver.misses,
        resolver.suppressed_transparent,
        preferred_mip.map_or_else(
            || "authored inline base".to_owned(),
            |mip| format!("separate <= {mip} override")
        )
    );
    println!(
        "Static probe SH storage: {probe_sh_record_count} per-instance records, {} interpolated, {} outside the PNW tetrahedra",
        resolver.probe_samples, resolver.probe_sample_misses
    );
    println!(
        "Transparency: {} cutout materials, {} blended materials",
        resolver.cutout, resolver.blended
    );
    let mut targets = vec![whole_world.focus("whole world".to_string())];
    let mut best = (0usize, 0usize);
    for (region, bounds) in regions {
        if bounds.points < 200 {
            continue;
        }
        let index = targets.len();
        if bounds.points > best.1 {
            best = (index, bounds.points);
        }
        targets.push(bounds.focus(format!("region {region}")));
    }
    let active = if let Some(start) = scene.player_start {
        targets.insert(
            0,
            FocusTarget {
                center: game_to_view(start.position),
                distance: 10.0,
                label: "player start".to_string(),
                spawn: Some(start),
            },
        );
        0
    } else {
        best.0
    };
    let mut camera = view_of(&targets[active]);
    let mut reference_horizontal_fov = view_settings
        .0
        .as_ref()
        .map(|settings| settings.horizontal_fov_degrees)
        .unwrap_or(DEFAULT_HORIZONTAL_FOV_DEGREES);
    if let Some(path) = env::var_os("DH2_REPLAY_F8") {
        let report = fs::read_to_string(&path).expect("cannot read DH2_REPLAY_F8 capture");
        camera = f8_camera_pose(&report).expect("DH2_REPLAY_F8 has an invalid camera pose");
        reference_horizontal_fov = f8_reference_fov(&report)
            .expect("DH2_REPLAY_F8 has an invalid reference horizontal FOV");
        println!(
            "Replaying saved F8 camera from {} at reference FOV {reference_horizontal_fov}",
            PathBuf::from(path).display()
        );
    }
    // Replayed F8 poses are benchmark inputs, not spawn points. Gravity must
    // not move the saved camera while comparing rendering at that same pose.
    let walking = navigation.is_some()
        && scene.player_start.is_some()
        && env::var_os("DH2_REPLAY_F8").is_none();
    let feet = camera.translation - Vec3::Y * 1.7;
    let walker = navigation
        .as_ref()
        .map(|world| navigation::Walker::new(feet, &world.0))
        .unwrap_or(navigation::Walker {
            feet,
            vertical_speed: 0.0,
            supported: false,
        });
    if walking {
        camera.translation = walker.feet + Vec3::Y * 1.7;
    }
    if let Some(probes) = &scene.environment_probes {
        let camera_game = view_to_game(camera.translation);
        if let Some(interpolated) = probes.interpolate_static_sh(camera_game) {
            println!(
                "Authored probe data at camera: tetrahedron {}, weights {:?}, static-light illuminance {:?}, sky/sun factors {}/{}, estimated illuminance {:?}, static/sun/sky-input SH L0 {:?}/{:?}/{:?}; material samples use each surface center",
                interpolated.tetrahedron_index,
                interpolated.probe_weights,
                interpolated.static_lights_illuminance,
                interpolated.sky_occlusion_factor,
                interpolated.sun_irradiance_factor,
                interpolated.illuminance_estimate,
                interpolated.static_lighting_sh[0],
                interpolated.sun_factor_sh[0],
                interpolated.sky_occlusion_sh[0],
            );
        } else {
            println!("No authored probe tetrahedron contains camera position {camera_game:?}");
        }
        if let Some((distance, probe)) = probes
            .probes
            .iter()
            .map(|probe| {
                let distance = (0..3)
                    .map(|axis| (probe.position[axis] - camera_game[axis]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                (distance, probe)
            })
            .min_by(|left, right| left.0.total_cmp(&right.0))
        {
            println!(
                "Nearest authored probe: {} at {:?}, {:.2} m; individual static SH L0 {:?}; secondary network entries {}",
                probe.resource,
                probe.position,
                distance,
                probe.static_lighting_sh[0],
                probes.secondary_resources.len(),
            );
        }
    }
    // These are the installed executable's registered perspective defaults:
    // ark_fieldOfView=80 degrees at its 16:9 reference, r_znear=0.01, and
    // r_zfar=7000. The vertical FOV is derived by FUN_140890b50; Bevy updates
    // the projection aspect when the actual camera viewport changes.
    let _game_default_vertical_fov = tome::render_vertical_fov_radians(reference_horizontal_fov)
        .expect("the registered game FOV default is valid");
    // The original registers r_multiSamples=0 and clamps physical count to1.
    // Override only in explicit profiling runs for same-source A/B testing.
    let multi_samples = env::var("DH2_PERF_MULTI_SAMPLES")
        .ok()
        .filter(|_| env::var("DH2_PERF_PROFILE").as_deref() == Ok("1"))
        .map(|value| {
            value
                .parse::<i32>()
                .expect("invalid DH2_PERF_MULTI_SAMPLES")
        })
        .unwrap_or(render_model::DEFAULT_MULTI_SAMPLES);
    let physical_samples = render_model::renderer_sample_count(multi_samples);
    assert!(
        [1, 2, 4, 8].contains(&physical_samples),
        "DH2_PERF_MULTI_SAMPLES={multi_samples} requests {physical_samples} samples; this renderer supports 1, 2, 4, or 8"
    );
    let renderer_msaa = Msaa::from_samples(physical_samples);
    let coarse_depth_source = coarse_depth::diagnostics_enabled()
        || scene
            .shadow_cascade_controls
            .is_some_and(|controls| controls.tighten_parallel_shadows);
    let camera_3d = {
        Camera3d {
            depth_texture_usages: (bevy::render::render_resource::TextureUsages::RENDER_ATTACHMENT
                | bevy::render::render_resource::TextureUsages::TEXTURE_BINDING)
                .into(),
            ..default()
        }
    };
    eprintln!(
        "[renderer] r_multiSamples={multi_samples} physical_samples={}",
        renderer_msaa.samples()
    );
    let mut camera_entity = commands.spawn((
        camera_3d,
        Projection::custom(camera_projection::GameProjection::new(
            reference_horizontal_fov,
            DEFAULT_NEAR,
            DEFAULT_FAR,
        )),
        camera,
        renderer_msaa,
    ));
    camera_entity.insert(crate::rendering::bevy_backend::fog::FogCamera);
    if coarse_depth_source {
        camera_entity.insert(coarse_depth::CoarseDepthSource);
    }
    // Direct submission avoids Bevy's per-bin indirect-buffer uploads/unpacking.
    // Mesh transforms, shaders and CPU/Tome visibility stay unchanged.
    // Set only at camera creation; DH2_DIRECT_MESH_DRAWS=0 retains the A/B control.
    if env::var("DH2_DIRECT_MESH_DRAWS").as_deref() != Ok("0") {
        camera_entity.insert(bevy::render::view::NoIndirectDrawing);
    }
    // Standalone depth profiling remains explicit. The native screen-shadow
    // owner separately enables its required same-view depth source.
    if env::var("DH2_ZPREPASS").as_deref() == Ok("1") {
        camera_entity.insert(DepthPrepass);
    }
    if lit {
        if let Some(parameters) = &scene.environment_render_parameters {
            let exposure = auto_exposure::ExposureParameters::from_parameters(parameters)
                .expect("invalid exposure environment");
            println!("Exposure parameters: {exposure:?}");
            camera_entity.insert((exposure, auto_exposure::MaterialExposure(material_exposure)));
        }
    }
    commands.insert_resource(NavigationState { walking, walker });
    commands.insert_resource(PickupState {
        items: scene
            .pickups
            .iter()
            .map(|pickup| PickupInstance {
                entity_id: pickup.entity_id.clone(),
                position: game_to_view(pickup.position),
            })
            .collect(),
        world: runtime::WorldState::from_pickups(&scene.pickups)
            .expect("level contains invalid or duplicate pickup definitions"),
    });
    if lit {
        // lightcontext HQ/AQ/LQ_ComputeAmbientLighting explicitly disable
        // the old env/ambient high/low term. Probe irradiance owns ambient;
        // adding Bevy's global ambient would illuminate every shadow again.
        commands.insert_resource(GlobalAmbientLight::NONE);
        let mut spawned = 0;
        let mut unsupported = 0;
        let mut energy_shortened_ranges = 0;
        if scene_lights_enabled {
            // FUN_140017d30 publishes the environment sun independently of
            // placed light actors. A directional backend light transports its
            // radiance to all ordinary materials, including maps with no sun actor.
            let environment_sun = scene
                .environment_lighting
                .and_then(environment_sun_ray)
                .filter(|ray| *ray != Vec3::ZERO)
                .map(|ray| {
                    let color = render_light::sun_ground_color(
                        scene
                            .environment_lighting
                            .and_then(|environment| environment.sun_color)
                            .unwrap_or([0.215861; 3]),
                    );
                    commands.spawn((
                        DirectionalLight {
                            color: Color::linear_rgb(color[0], color[1], color[2]),
                            illuminance: render_light::ARKANE_TO_WORLD_LUMINANCE,
                            shadow_maps_enabled: false,
                            ..default()
                        },
                        Transform::from_rotation(Quat::from_rotation_arc(Vec3::NEG_Z, ray)),
                    ));
                    spawned += 1;
                })
                .is_some();
            for (light_index, light) in scene.placed_lights.iter().enumerate() {
                // The sun preset represents the same environment owner, rather
                // than an additional source of solar illumination.
                if environment_sun
                    && light.kind == entities::LightType::Parallel
                    && light.preset.as_deref() == Some("LIGHT_PRESET_SUN")
                {
                    continue;
                }

                if !light.visible {
                    continue;
                }
                let position = game_to_view(light.position);
                // FUN_140881640 applies FUN_1401644a0 to idLight.m_Color before
                // preset conversion. Its installed constants (0.04045, 0.055,
                // 1.055, 2.4, 12.92) exactly match Bevy's sRGB decode.
                let color = Color::srgb(
                    light.color[0].max(0.0),
                    light.color[1].max(0.0),
                    light.color[2].max(0.0),
                );
                let linear_color = color.to_linear();
                let Some(preset) = light_preset(
                    light,
                    [linear_color.red, linear_color.green, linear_color.blue],
                ) else {
                    unsupported += 1;
                    continue;
                };
                if !preset.normalized_intensity.is_finite()
                    || !preset.color.iter().all(|value| value.is_finite())
                    || !preset.range.is_finite()
                    || !preset.falloff.is_finite()
                {
                    unsupported += 1;
                    continue;
                }
                let range = preset.range;
                energy_shortened_ranges += usize::from(
                    light_distance(light.distance_cutoff, light.distance_falloff)
                        .is_some_and(|(cutoff, _)| range < cutoff),
                );
                let falloff_start = preset.falloff;
                // Native normalized RGB is clamped before scaling. Point/spot
                // and parallel paths convert native HDR with the original
                // world-luminance factor; area profile photometry is separate.
                let color = Color::linear_rgb(
                    linear_color.red.clamp(0.0, 1.0),
                    linear_color.green.clamp(0.0, 1.0),
                    linear_color.blue.clamp(0.0, 1.0),
                );
                // Native area SizeX/Y are half extents (arealight's clipping
                // corners are +/-SizeX/Y); RectLight takes full dimensions.
                let rect_size = light.area_size.map(|size| 2.0 * size.max(0.01));
                let Some(intensity) =
                    light_backend_intensity(preset.normalized_intensity, light.kind, rect_size)
                else {
                    unsupported += 1;
                    continue;
                };
                let spot_cosines = (light.kind == entities::LightType::Spot)
                    .then(|| authored_light_cosines(light.angular_cutoff, light.angular_falloff));
                let cull = (light.kind != entities::LightType::Parallel).then(|| {
                    let clip_to_world = local_light_clip_to_world(
                        position,
                        light.rotation,
                        light.kind,
                        range,
                        spot_cosines.map(|(cutoff, _)| cutoff),
                    );
                    let projection_bounds = clip_to_world.and_then(light_projection_bounds);
                    LocalLightCull {
                        range,
                        kind: light.kind,
                        culling: light.culling,
                        definition_name_hash: light.definition_name_hash,
                        linear_color: preset.color,
                        // +0x28 actor origin plus +0x4c preset offset. Area
                        // proxies share this source, not their Bevy positions.
                        initial_sphere: Some([
                            light.position[0] + preset.center_offset[0],
                            light.position[1] + preset.center_offset[1],
                            light.position[2] + preset.center_offset[2],
                            range,
                        ]),
                        projection_bounds,
                        proximity_bounds: light_proximity_from_cached_bounds(
                            light.max_visible_range,
                            projection_bounds,
                        ),
                        clip_to_world,
                    }
                });
                // tlf/cull_lights excludes LIGHT_SCATTER_ONLY from surface
                // lists. Keep the source for fog admission and volumetric
                // shadows, without adding a host surface-light component.
                if light.scatter_only && light.kind != entities::LightType::Parallel {
                    commands.spawn((
                        Transform::from_translation(position),
                        Visibility::Inherited,
                        cull.expect("local scattering light cull data"),
                        local_shadow_updates::LocalLightSource(light_index),
                    ));
                    spawned += 1;
                    continue;
                }
                match light.kind {
                    entities::LightType::Point => {
                        commands.spawn((
                            PointLight {
                                color,
                                intensity,
                                decay: light.decay,
                                specular_attenuation: light.specular_attenuation,
                                falloff_start,
                                range,
                                shadow_maps_enabled: false,
                                shadow_map_near_z: light
                                    .shadow_near_plane
                                    .unwrap_or(PointLight::DEFAULT_SHADOW_MAP_NEAR_Z),
                                ..default()
                            },
                            Transform::from_translation(position),
                            cull.expect("local light cull data"),
                            local_shadow_updates::LocalLightSource(light_index),
                        ));
                        spawned += 1;
                    }
                    entities::LightType::HalfPoint => {
                        // The original renderer keeps these omnidirectional for
                        // shadowing, then applies a one-sided angular ramp in the
                        // shared point-light shader. Its signed reciprocal is
                        // carried through the vendored Bevy point-light shader.
                        let direction = game_to_view(light.rotation[0]).normalize_or_zero();
                        if direction == Vec3::ZERO {
                            unsupported += 1;
                            continue;
                        }
                        let angular_scale = preserved_half_point_angular_scale(
                            light.angular_cutoff,
                            light.angular_falloff,
                        );
                        commands.spawn((
                            PointLight {
                                color,
                                intensity,
                                decay: light.decay,
                                specular_attenuation: light.specular_attenuation,
                                falloff_start,
                                range,
                                half_point_data: direction.extend(angular_scale),
                                shadow_maps_enabled: false,
                                shadow_map_near_z: light
                                    .shadow_near_plane
                                    .unwrap_or(PointLight::DEFAULT_SHADOW_MAP_NEAR_Z),
                                ..default()
                            },
                            Transform::from_translation(position),
                            cull.expect("local light cull data"),
                            local_shadow_updates::LocalLightSource(light_index),
                        ));
                        spawned += 1;
                    }
                    entities::LightType::Spot => {
                        let direction = game_to_view(light.rotation[0]).normalize_or_zero();
                        if direction == Vec3::ZERO {
                            unsupported += 1;
                            continue;
                        }
                        let up = game_to_view(light.rotation[1]).normalize_or_zero();
                        if up == Vec3::ZERO || up.cross(direction).length_squared() < 0.0001 {
                            unsupported += 1;
                            continue;
                        }
                        let (cutoff_cosine, falloff_cosine) =
                            spot_cosines.expect("spot light cosine thresholds");
                        // The game uses m_AngularCutoff for the projection/outer cone
                        // and m_AngularFalloff for its inner transition boundary.
                        let outer_angle = cutoff_cosine.acos();
                        let inner_angle = falloff_cosine.acos();
                        // Bevy's spot-light implementation requires a sub-90В°
                        // outer cone; the authored mission cutoffs are within
                        // that range after the game's half-angle conversion.
                        let outer_angle =
                            outer_angle.clamp(0.01, std::f32::consts::FRAC_PI_2 - 0.01);
                        let inner_angle = inner_angle.min(outer_angle);
                        commands.spawn((
                            SpotLight {
                                color,
                                intensity,
                                decay: light.decay,
                                specular_attenuation: light.specular_attenuation,
                                falloff_start,
                                range,
                                inner_angle,
                                outer_angle,
                                shadow_maps_enabled: false,
                                shadow_map_near_z: light
                                    .shadow_near_plane
                                    .unwrap_or(SpotLight::DEFAULT_SHADOW_MAP_NEAR_Z),
                                ..default()
                            },
                            Transform::from_translation(position).looking_to(direction, up),
                            cull.expect("local light cull data"),
                            local_shadow_updates::LocalLightSource(light_index),
                        ));
                        spawned += 1;
                    }
                    entities::LightType::Area => {
                        // LIGHT_AREA uses the separate paraboloid path in
                        // renderprog/lighting.decl; Bevy RectLight has no matching
                        // shadow path. The authored area profile/projection and
                        // area-specific shadow controls remain unported.
                        let Some(transform) =
                            authored_area_light_transform(position, light.rotation)
                        else {
                            unsupported += 1;
                            continue;
                        };
                        commands.spawn((
                            RectLight {
                                color,
                                intensity,
                                range,
                                width: rect_size[0],
                                height: rect_size[1],
                            },
                            transform,
                            cull.expect("local light cull data"),
                            local_shadow_updates::LocalLightSource(light_index),
                        ));
                        spawned += 1;
                    }
                    entities::LightType::Parallel => {
                        let (parallel_color, direction) =
                            parallel_light_inputs(light, scene.environment_lighting, preset.color);
                        let sun_color = Color::linear_rgb(
                            parallel_color[0],
                            parallel_color[1],
                            parallel_color[2],
                        );
                        let up = game_to_view(light.rotation[1]).normalize_or_zero();
                        if direction == Vec3::ZERO
                            || up == Vec3::ZERO
                            || up.cross(direction).length_squared() < 0.0001
                        {
                            unsupported += 1;
                            continue;
                        }
                        let mut cascade_shadow_config = CascadeShadowConfigBuilder::default();
                        if let Some(maximum_distance) = scene
                            .environment_lighting
                            .and_then(|environment| environment.parallel_light_extend)
                            .filter(|distance| {
                                distance.is_finite()
                                    && *distance > cascade_shadow_config.first_cascade_far_bound
                            })
                        {
                            // Mapping env/shadow/parallellightextend to Bevy's
                            // directional cascade extent is inferred from the
                            // authored field name and Bevy's distance control.
                            cascade_shadow_config.maximum_distance = maximum_distance;
                        }
                        commands.spawn((
                            DirectionalLight {
                                color: sun_color,
                                specular_attenuation: light.specular_attenuation,
                                // Native lighting uses normalized HDR radiance;
                                // PostFX/Luminance/common supplies world units.
                                illuminance: render_light::ARKANE_TO_WORLD_LUMINANCE,
                                shadow_maps_enabled: false,
                                ..default()
                            },
                            Transform::from_translation(position).looking_to(direction, up),
                            cascade_shadow_config.build(),
                        ));
                        spawned += 1;
                    }
                }
            }
        }
        println!(
            "Resolved authored lights spawned: {spawned}; unsupported shapes, presets, or invalid ranges: {unsupported}; native energy-shortened ranges: {energy_shortened_ranges}; scene lights {}; game-derived shadow atlas owners active",
            if scene_lights_enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
    }
    println!(
        "World bounds {:?} .. {:?}; {} camera regions; starting at {}",
        whole_world.low,
        whole_world.high,
        targets.len() - 1,
        targets[active].label
    );
    println!(
        "Controls: click to capture mouse, mouse/arrows look, Esc release; G toggles walk/free-fly; WASD move, Shift sprint, Space jump in walk; F pick up mapped items; F8 save collision report and screenshot; Q/E vertical in free-fly; PageUp/PageDown region, Home reset, End overview"
    );
    println!(
        "Mapped pickups with explicit item and quantity: {}",
        scene.pickups.len()
    );
    commands.insert_resource(FocusTargets { targets, active });
    let mut tome_render_instances =
        TomeRenderInstanceMap::from_world_mapping(&scene.world, &bwm_visibility_render_instances);
    let visibility_suppression_groups = scene
        .world
        .visibility_groups
        .iter()
        .map(|group| RenderSuppressionGroup {
            parent: bwm_visibility_render_instances[group.parent].clone(),
            children: group
                .children
                .iter()
                .flat_map(|&index| bwm_visibility_render_instances[index].iter().copied())
                .collect(),
        })
        .collect();
    let mut diffuse_image_materials = vec![Vec::new(); resolver.diffuse_image_paths.len()];
    let mut material_images = HashMap::new();
    let mut material_stage_sorts = HashMap::new();
    for cached in resolver.cache.values().flatten() {
        material_stage_sorts.insert(cached.handle.clone(), cached.stage_sort.clone());
        material_images.insert(
            cached.handle.clone(),
            cached.images.iter().map(|&(slot, _)| slot).collect(),
        );
        for &(slot, role) in &cached.images {
            diffuse_image_materials[slot].push((cached.handle.clone(), role));
        }
    }
    let diffuse_image_count = resolver.diffuse_image_paths.len();
    tome_render_instances.bind_texture_demands(&render_instances, &material_images);
    commands.insert_resource(RenderSourcePlacements(expanded_placements));
    commands.insert_resource(PlacedModelQueries(placed_model_queries));
    commands.insert_resource(ShadowModelBoundsSources(shadow_model_bounds));
    commands.insert_resource(shadow_updates::ModelShadowSources(model_shadow_sources));
    let mut shadow_image_slots: Vec<_> = resolver
        .shadow_materials
        .0
        .values()
        .filter_map(|inputs| inputs.as_ref().ok())
        .filter_map(super::shadow_preparation::ShadowMaterialInputs::cutout_image_slot)
        .collect();
    shadow_image_slots.sort_unstable();
    shadow_image_slots.dedup();
    commands.insert_resource(std::mem::take(&mut resolver.shadow_materials));
    commands.insert_resource(std::mem::take(&mut resolver.vegetation));
    if let (Some(catalog),Some(parameters))=(catalog,scene.environment_render_parameters.as_ref()) {
        let (water,sources)=super::water_scene::WaterScene::load(catalog,parameters,&scene.area_environments,&resolver.water_materials)
            .unwrap_or_else(|error|panic!("water scene preparation failed: {error}"));
        commands.insert_resource(water);
        commands.insert_resource(sources);
    }
    commands.insert_resource(std::mem::take(&mut resolver.water_materials));
    commands.insert_resource(std::mem::take(&mut resolver.boat_materials));
    let vegetation_vertices: usize = vegetation_meshes
        .0
        .values()
        .filter_map(|result| result.as_ref().ok())
        .sum();
    let vegetation_mesh_count = vegetation_meshes
        .0
        .values()
        .filter(|result| result.is_ok())
        .count();
    println!(
        "Vegetation inputs: {vegetation_mesh_count} shared meshes, {vegetation_vertices} vertices, {} decoded attribute bytes; {} unavailable",
        vegetation_vertices * std::mem::size_of::<[f32; 8]>(),
        vegetation_meshes.0.len() - vegetation_mesh_count
    );
    commands.insert_resource(vegetation_meshes);
    commands.insert_resource(super::water_objects::prepare_world_geometry(
        &scene.world, &world_shadow_mesh_sources, &mut meshes,
        &mut world_matrices, resolver.probe_sh_buffers,
    ));
    commands.insert_resource(world_shadow_mesh_sources);
    let world_lods_by_render_instance = if trace_bwm_lod {
        render_instances
            .iter()
            .map(|instance| {
                instance.source.and_then(|source| match source.surface {
                    RenderModelSurface::World { object, .. } => scene
                        .world
                        .objects
                        .get(object)
                        .map(|world_object| world_object.lod),
                    RenderModelSurface::Bmodel { .. } | RenderModelSurface::Bmd6 { .. } => None,
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut render_queue = SceneRenderQueue {
        instances: render_instances,
        active: 0,
        static_world_model_present: true,
        tome_render_instances,
        visible_authored_ids: Vec::new(),
        visible_cell_indices: Vec::new(),
        visible_cell_mask: Vec::new(),
        light_query_state: None,
        shadow_ordinary_query: None,
        visibility_suppression_groups,
        trace_bwm_lod,
        world_lods_by_render_instance,
        last_bwm_lod_visibility_report: None,
        diffuse_image_demands: (0..diffuse_image_count)
            .map(|_| texture::ProjectedAreaDemand::default())
            .collect(),
        diffuse_image_paths: std::mem::take(&mut resolver.diffuse_image_paths),
        diffuse_image_materials,
        material_images,
        material_stage_sorts,
        diffuse_image_handles: vec![None; diffuse_image_count],
        shadow_diffuse_slots: Vec::new(),
        shadow_diffuse_membership: Vec::new(),
        diffuse_image_pending: vec![None; diffuse_image_count],
        diffuse_mip_states: (0..diffuse_image_count)
            .map(|_| DiffuseMipState::default())
            .collect(),
        diffuse_image_attempted: vec![false; diffuse_image_count],
        diffuse_image_last_visible: vec![0; diffuse_image_count],
        texture_stream_frame: 0,
        texture_viewport_pixels: 0,
        diffuse_mip_upgrades: 0,
        preferred_mip,
    };
    if let Some(catalog) = catalog {
        let mut resident_images = 0;
        let mut resident_bytes = 0;
        // Sky colour and fog depth are global view inputs. As with cutout
        // images, publish their actual base mip chains before GPU submission;
        // finer levels still use the ordinary texture streaming owner.
        let sky_slots = render_queue
            .diffuse_image_materials
            .iter()
            .enumerate()
            .filter_map(|(slot, bindings)| {
                bindings
                    .iter()
                    .any(|(material, _)| matches!(material, super::MaterialHandle::Sky(_)))
                    .then_some(slot)
            });
        let resident_slots = shadow_image_slots
            .into_iter()
            .chain(sky_slots)
            .collect::<std::collections::BTreeSet<_>>();
        for slot in resident_slots {
            match super::texture_streaming::initialize_resident_image(
                catalog,
                slot,
                &mut render_queue,
                &mut images,
                &mut probe_materials,
                &mut decal_materials,
                &mut sky_materials,
            ) {
                Ok(bytes) => {
                    resident_images += usize::from(bytes != 0);
                    resident_bytes += bytes;
                }
                Err(error) => {
                    error!(target: "dh2::textures", slot, "Scene base image initialization failed: {error}")
                }
            }
        }
        info!(target: "dh2::textures", resident_images, resident_bytes, "Shadow cutout and sky base mips initialized");
    }
    commands.insert_resource(render_queue);
    if let Some(catalog) = catalog_storage {
        commands.insert_resource(TextureReadWorker::new(catalog, diffuse_image_count));
    }
}
