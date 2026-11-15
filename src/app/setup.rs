//! Setup.
use super::{
    apply_rest_pose, authored_area_light_transform, authored_light_cosines,
    authored_world_bounds, auto_exposure, bmd6, bmodel, camera_projection, coarse_depth, entities,
    env, environment_sun_ray, f8_camera_pose, f8_reference_fov, fs,
    game_affine_to_view, game_to_view, initial_placed_model_query, light_backend_intensity,
    light_distance, light_preset, light_projection_bounds, light_proximity_from_cached_bounds,
    local_light_clip_to_world, local_shadow_updates, material, mean_game_position,
    needs_exact_world_mesh, placed_direction, placed_transform, placed_vertex,
    preserved_half_point_angular_scale, render_bounds, render_light, render_model,
    rest_skinning, retain_model_shadow_position, retain_skin_inputs, shadow_pipeline,
    shadow_sampling, shadow_updates, surface_sampler_image, texture, tome,
    transformed, transformed_direction, view_of, view_to_game, Arc, BTreeMap, Bounds,
    CascadeShadowConfigBuilder, DecalMaterial, DepthPrepass, DiffuseMipState,
    FocusTarget, FocusTargets, HashMap, HashSet, LocalLightCull,
    MaterialResolver, NavigationWorld, PathBuf,
    PlacedModelQueries, PlacedModelQueryGroup, ProbeMaterial, ProgramCacheSource, Projection,
    RectLight, RenderDevice, RenderInstance, RenderInstanceSource, RenderModelSurface,
    RenderSourcePlacements, RenderSuppressionGroup, SceneData, SceneRenderQueue, ShaderBuffer,
    ShadowMaterials, ShadowModelBoundsSource, ShadowModelBoundsSources, TextureFormat,
    TextureReadWorker, TomeRenderInstanceMap, VegetationMeshInputs, ViewSettings,
    WorldShadowMeshSources, DEFAULT_FAR, DEFAULT_HORIZONTAL_FOV_DEGREES, DEFAULT_NEAR,
};
use super::view_geometry::{authored_mesh_from_uvs, source_mesh_from_uvs, MeshUvs};
use bevy::prelude::*;
use crate::rendering::bevy_backend::asset_preparation::AssetWrite;

enum SceneSkeletalModel {
    Prepared(Arc<bmd6::ModelInfo>),
    Shared(Arc<bmd6::ModelInfo>),
}

struct StaticModelSlot {
    last_placement: usize,
    source: Option<Option<bmodel::ModelSource>>,
    level_bytes: usize,
}

fn uses_static_model(path: &str) -> bool {
    !path.ends_with(".blueprint.decl") && !path.ends_with(".bmd6model")
        && !path.contains("/collisions/") && !path.contains("/templates/occlusion/")
        && !path.contains("/skiff_01_noswim")
}

fn capacity_bytes<T>(values: &Vec<T>) -> usize {
    values.capacity() * std::mem::size_of::<T>()
}

struct TransformScratch {
    points: Vec<Vec3>,
    separate: bool,
    observation: Option<TransformScratchObservation>,
}

#[derive(Default)]
struct TransformScratchObservation {
    fills: usize,
    growths: usize,
    vertices: usize,
    peak_capacity: usize,
    cycles: u64,
}

impl TransformScratch {
    fn new(separate: bool, observe: bool) -> Self {
        Self { points: Vec::new(), separate, observation: observe.then(TransformScratchObservation::default) }
    }

    fn fill(&mut self, positions: &[[f32; 3]], transform: impl Fn([f32; 3]) -> Vec3) {
        let started = self.observation.as_ref().and_then(|_| crate::loading::thread_cycles());
        let capacity = if self.separate { 0 } else { self.points.capacity() };
        let transformed = positions.iter().copied().map(transform);
        if self.separate {
            self.points = Vec::new();
            self.points = transformed.collect();
        } else {
            self.points.clear();
            self.points.reserve_exact(positions.len());
            self.points.extend(transformed);
        }
        if let Some(observation) = &mut self.observation {
            observation.fills += 1;
            observation.growths += usize::from(self.points.capacity() > capacity);
            observation.vertices += positions.len();
            observation.peak_capacity = observation.peak_capacity.max(self.points.capacity());
            if let Some(started) = started {
                observation.cycles += crate::loading::thread_cycles().unwrap_or(started).saturating_sub(started);
            }
        }
    }

    fn report(self, phase: &str) {
        if let Some(observation) = self.observation {
            eprintln!("SCENE_TRANSFORM_SCRATCH phase={phase} separate={} fills={} growths={} vertices={} peak_bytes={} cycles={}",
                self.separate, observation.fills, observation.growths, observation.vertices,
                observation.peak_capacity * std::mem::size_of::<Vec3>(), observation.cycles);
        }
    }
}

#[derive(Default)]
struct ModelMeshObservation {
    enabled: bool,
    repeated: bool,
    initializations: usize,
    skinned_vertices: usize,
    cycles: u64,
}

impl ModelMeshObservation {
    fn prepare(&mut self, skinned_vertices: usize, initialize: impl FnOnce()) {
        let start = self.enabled.then(crate::loading::thread_cycles).flatten();
        initialize();
        if self.enabled {
            self.initializations += 1;
            self.skinned_vertices += skinned_vertices;
            if let Some(start) = start {
                self.cycles += crate::loading::thread_cycles().unwrap_or(start).saturating_sub(start);
            }
        }
    }

    fn report(&self, marker: &str, meshes: &dyn AssetWrite<Mesh>, instances: &[RenderInstance],
        common: Option<&super::common_model_meshes::Prepared>) {
        if !self.enabled { return; }
        let mut known = HashMap::new();
        let mut digest = blake3::Hasher::new();
        for instance in instances {
            let ordinal = known.len();
            let index = known.entry(instance.mesh.id()).or_insert_with(|| {
                let mesh = meshes.get(&instance.mesh)
                    .or_else(|| common.and_then(|common| common.debug_mesh(instance.mesh.id())))
                    .expect("prepared model mesh or enabled common-mesh observation");
                digest.update(&mesh.count_vertices().to_le_bytes());
                for (attribute, values) in mesh.attributes() {
                    digest.update(format!("{:?}", attribute).as_bytes());
                    let bytes = values.get_bytes();
                    digest.update(&bytes.len().to_le_bytes());
                    digest.update(bytes);
                }
                match mesh.indices() {
                    Some(bevy::mesh::Indices::U16(indices)) => {
                        digest.update(&[2]);
                        digest.update(&indices.len().to_le_bytes());
                        for index in indices { digest.update(&index.to_le_bytes()); }
                    }
                    Some(bevy::mesh::Indices::U32(indices)) => {
                        digest.update(&[4]);
                        digest.update(&indices.len().to_le_bytes());
                        for index in indices { digest.update(&index.to_le_bytes()); }
                    }
                    None => { digest.update(&[0]); }
                }
                ordinal
            });
            digest.update(&index.to_le_bytes());
        }
        eprintln!("{marker} repeated={} instances={} meshes={} initializations={} skinned_vertices={} cycles={} signature={}",
            self.repeated, instances.len(), known.len(), self.initializations, self.skinned_vertices,
            self.cycles, digest.finalize());
    }
}
impl std::ops::Deref for SceneSkeletalModel {
    type Target = bmd6::ModelInfo;
    fn deref(&self) -> &Self::Target {
        match self { Self::Prepared(model) => model, Self::Shared(model) => model }
    }
}

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

fn report_placement_storage(
    source: &Vec<entities::PlacedModel>,
    expanded: &Vec<entities::PlacedModel>,
) {
    fn capacity_bytes(placements: &Vec<entities::PlacedModel>) -> usize {
        placements.capacity() * std::mem::size_of::<entities::PlacedModel>()
            + placements.iter().map(|placement| {
                placement.id.capacity() + placement.resource_path.capacity()
                    + placement.custom_materials.capacity() * std::mem::size_of::<Option<String>>()
                    + placement.custom_materials.iter().flatten().map(String::capacity).sum::<usize>()
            }).sum::<usize>()
    }
    let mut signature = flate2::Crc::new();
    for placement in expanded {
        signature.update(format!("{placement:?}").as_bytes());
    }
    eprintln!("MEMORY model placement storage: staging={} render={} count={} signature={:08x}",
        capacity_bytes(source), capacity_bytes(expanded), expanded.len(), signature.sum());
}

pub(super) fn setup(
    mut commands: Commands,
    mut scene: ResMut<SceneData>,
    programs: (Option<ResMut<ProgramCacheSource>>, Option<Res<super::scene_preparation::SetupPrepared>>),
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
    moving: (Option<Res<super::machinery::AmbientMachinery>>, Option<Res<super::interactions::Interactions>>),
    environment_volumes: Option<Res<super::state::EnvironmentVolumes>>,
    effects_and_images: (Option<Res<crate::rendering::bevy_backend::world_effects::WorldEffectDefinitions>>, Res<super::resident_images::ResidentImages>, Option<Res<crate::gameplay::entity_lifetime::LiveEntities>>),
) {
    let (mut programs, prepared) = programs;
    let (effect_graphs, resident_images, live_entities) = effects_and_images;
    let (machinery, interactions) = moving;
    if prepared.is_some() { return; }
    let mut commands = super::setup_commands::SetupCommands::Direct(commands);
    let _ = prepare(&mut commands, &mut scene, programs.as_deref_mut(), navigation.as_deref(),
        &mut meshes, None, &mut images, &mut probe_materials, &mut sky_materials,
        &mut probe_sh_buffers, &render_device, &mut decal_materials, &view_settings,
        &mut world_matrices, machinery.as_deref(), interactions.as_deref(), environment_volumes.as_deref(),
        effect_graphs.as_deref(), live_entities.as_deref(), None, &resident_images, &|| Ok(()))
        .unwrap_or_else(|error| panic!("Scene preparation failed: {error}"));
}

pub(super) fn prepare(
    commands: &mut super::setup_commands::SetupCommands,
    scene: &mut SceneData,
    mut programs: Option<&mut ProgramCacheSource>,
    navigation: Option<&NavigationWorld>,
    meshes: &mut dyn AssetWrite<Mesh>,
    mut common_meshes: Option<&mut super::common_model_meshes::Prepared>,
    images: &mut dyn AssetWrite<Image>,
    probe_materials: &mut dyn AssetWrite<ProbeMaterial>,
    sky_materials: &mut dyn AssetWrite<crate::rendering::bevy_backend::sky_mask::SkyMaterial>,
    probe_sh_buffers: &mut dyn AssetWrite<ShaderBuffer>,
    render_device: &RenderDevice,
    decal_materials: &mut dyn AssetWrite<DecalMaterial>,
    view_settings: &ViewSettings,
    world_matrices: &mut super::world_instances::WorldMatrices,
    machinery: Option<&super::machinery::AmbientMachinery>,
    interactions: Option<&super::interactions::Interactions>,
    environment_volumes: Option<&super::state::EnvironmentVolumes>,
    effect_graphs: Option<&crate::rendering::bevy_backend::world_effects::WorldEffectDefinitions>,
    live_entities: Option<&crate::gameplay::entity_lifetime::LiveEntities>,
    shared_images: Option<(&super::texture_streaming::SharedImages, Option<usize>)>,
    resident_images: &super::resident_images::ResidentImages,
    check_cancelled: &impl Fn() -> Result<(), String>,
) -> Result<Option<super::texture_streaming::BaseImageReader>, String> {
    check_cancelled()?;
    // Setup mutates the parsed world. Borrow through a separate lease while
    // the published scene keeps its catalog for runtime FX and diagnostics.
    let catalog_storage = scene.catalog.clone();
    let catalog = catalog_storage.as_deref();
    let fog_material = crate::rendering::bevy_backend::fog_material::FogMaterialResources::new(
        images,
        probe_sh_buffers,
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
        let cache = programs
            .as_deref_mut()
            .expect("atmosphere requires the installed program cache")
            .0.clone();
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        let program_library = cache.as_mut()
            .unwrap_or_else(|error| panic!("atmosphere program cache unavailable: {error}"));
        let mut fog = super::fog_scene::FogScene::load(
            catalog,
            program_library,
            parameters,
            &scene.area_environments,
            sun,
            DEFAULT_FAR,
            images,
            resident_images,
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
            images,
            resident_images,
        )
        .unwrap_or_else(|error| panic!("display scene preparation failed: {error}"));
        fog.upload_volumes(std::mem::take(&mut scene.area_fog), meshes);
        commands.insert_resource(fog);
        commands.insert_resource(display);
    }
    if super::performance::map_scoped_overlay() {
        super::performance::spawn_overlay(commands);
    }
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
    super::load_timing::mark("setup: start");
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
            scene.expanded_placement_roots.unwrap_or(scene.placed_models.len())
        );
    }
    // Preserve ordered primary probe IDs in binding100; per-object records
    // are appended after them. World geometry uses its PNW instance offset.
    let mut records = scene
        .environment_probes
        .as_deref()
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
    // The CPU references build only this GPU buffer. Write the host WGSL
    // layout straight into its upload backing, then retire setup-only arrays.
    let trace_probe = env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1");
    let copy_probe = env::var_os("DH2_DEBUG_COPY_VERTEX_PROBE_BUFFER").is_some_and(|v| v == "1");
    let copy_indices = env::var_os("DH2_DEBUG_COPY_PROBE_INDICES").is_some_and(|v| v == "1");
    let started = trace_probe.then(crate::loading::thread_cycles).flatten();
    let mut word_capacity_bytes = 0;
    let mut reuse_index_backing = false;
    let mut vertex_probe_buffer = if let Some(set) = scene.environment_probes.as_mut() {
        let buffer = if copy_probe {
            let words = set.vertex_shader_words();
            word_capacity_bytes = words.capacity() * size_of::<u32>();
            ShaderBuffer::from(words)
        } else if let Some(set) = Arc::get_mut(set).filter(|_| !copy_indices) {
            let (bytes, direct) = set.take_vertex_shader_bytes();
            reuse_index_backing = direct;
            ShaderBuffer { data: Some(bytes), ..default() }
        } else {
            ShaderBuffer {
                data: Some(set.vertex_shader_bytes()),
                ..default()
            }
        };
        if let Some(set) = std::sync::Arc::get_mut(set) {
            set.vertex_references.instance_offsets = Vec::new();
            set.vertex_references.tetrahedron_indices = Default::default();
            let retain_source = env::var_os("DH2_DEBUG_RETAIN_VERTEX_PROBE_SOURCES").is_some_and(|v| v == "1");
            if trace_probe {
                let position_bytes = set.spatial.vertex_positions.capacity() * size_of::<[f32; 3]>();
                let table_bytes = set.vertex_references.cell_probe_indices.capacity() * size_of::<[u32; 4]>();
                eprintln!("MEMORY vertex probe source retirement: retain={retain_source} position_bytes={position_bytes} table_bytes={table_bytes}");
            }
            if !retain_source {
                set.spatial.vertex_positions = Vec::new();
                set.vertex_references.cell_probe_indices = Vec::new();
            }
        }
        buffer
    } else {
        ShaderBuffer {
            data: Some(vec![0; 8 * size_of::<u32>()]),
            ..default()
        }
    };
    let data = vertex_probe_buffer.data.as_ref().expect("prepared PNW GPU buffer");
    let bytes = data.len() as u64;
    if trace_probe {
        let cycles = started.zip(crate::loading::thread_cycles())
            .map_or(0, |(start, end)| end.saturating_sub(start));
        let mut crc = flate2::Crc::new();
        crc.update(data);
        eprintln!(
            "MEMORY vertex probe buffer: copy={copy_probe} bytes={bytes} capacity={} word_capacity_bytes={word_capacity_bytes} cycles={cycles} signature={:08x} reuse_index_backing={reuse_index_backing}",
            data.capacity(), crc.sum(),
        );
    }
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
    vertex_probe_buffer.asset_usage = bevy::asset::RenderAssetUsages::RENDER_WORLD;
    let vertex_probe_buffer = probe_sh_buffers.add(vertex_probe_buffer);
    // Only the handle is consumed before the completed probe data is uploaded.
    let probe_sh_buffer = probe_sh_buffers.add(ShaderBuffer::default());
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
    commands.insert_resource(crate::rendering::bevy_backend::particle_pass::ParticleMaterialSampler(
        anisotropic_sampler_image.clone()));
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
    let vegetation = super::vegetation_wind::VegetationMaterials::new(probe_sh_buffers);
    let loaded_reflections =
        super::environment_reflection::load(
            catalog,
            scene.environment_probes.as_deref(),
            images,
            probe_sh_buffers,
            check_cancelled,
        )?;
    let water_probes = loaded_reflections.water_probes(
        scene.environment_probes.as_deref(),
        vertex_probe_buffer.clone(),
        probe_sh_buffers,
    );
    let secondary_probe_count = loaded_reflections.cube_count;
    if let Some(probes) = scene.environment_probes.as_mut().and_then(Arc::get_mut) {
        probes.release_reflection_preparation();
    }
    commands.insert_resource(crate::rendering::bevy_backend::water_scene::WaterLighting(
        water_probes,
    ));
    super::load_timing::mark("setup: reflections and lighting inputs");
    let reflections = loaded_reflections.image;
    let reflection_planes = loaded_reflections.planes;
    let reflection_brdf_lookup = loaded_reflections.brdf_lookup;
    // Scene preparation runs before runtime FX requests and owns the same
    // program library that those requests later acquire on their worker.
    let mut program_guard = programs.as_ref().map(|source| source.0.lock().unwrap_or_else(|e| e.into_inner()));
    let mut resolver = MaterialResolver {
        blended_materials: Default::default(),
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
        programs: program_guard.as_deref_mut(),
        probe_materials: probe_materials,
        sky_materials: sky_materials,
        decal_materials: decal_materials,
        probe_sh_buffers: probe_sh_buffers,
        probe_sh_buffer,
        vertex_probe_buffer,
        shadow_buffer,
        local_shadow_buffer,
        shadow_map,
        probe_sh_records: records,
        probes: scene.environment_probes.as_deref(),
        cache: default(),
        shadow_materials: ShadowMaterials::default(),
        vegetation,
        water_materials: default(),
        water_model_colours: default(),
        water_stencil_materials: default(),
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
    let runtime_material_bindings = super::runtime_materials::MaterialBindings::capture(&resolver);
    let mut whole_world = Bounds::empty();
    let mut regions = BTreeMap::<i32, Bounds>::new();
    let mut world_shadow_mesh_sources = WorldShadowMeshSources::prepare(&scene.world, meshes);
    super::load_timing::mark("setup: world shadow mesh sources");
    let mut world_admissions = HashMap::<String, Result<u8, String>>::new();
    let mut vegetation_meshes = VegetationMeshInputs::default();
    let mut baked_shear_instances = 0usize;
    let mut authored_lod_instances = BTreeMap::<usize, usize>::new();
    let mut render_instances = Vec::new();
    // 88e220 can select a volume's surface material even when no placed mesh
    // names it. Load it through the same resolver before preparing water's
    // immutable program/image registries; ordinary area blending excludes these
    // profiles and therefore cannot discover these dependencies itself.
    if let Some(volumes)=environment_volumes.as_ref() {
        for volume in volumes.0.iter().filter(|v|v.is_water_volume) {
            if let Some(material)=volume.surface_material.as_deref() {
                let resolved=resolver.get(material,0,volume.position);
                if !resolved.is_some_and(|m|matches!(m.handle,super::MaterialHandle::Water(_))) {
                    warn!("Water volume {} surface material is not ready for water rendering: {material}",volume.id);
                }
            }
        }
    }
    let separate_points = env::var_os("DH2_DEBUG_SEPARATE_TRANSFORM_SCRATCH").is_some_and(|value| value == "1");
    let observe_points = env::var_os("DH2_DEBUG_TRANSFORM_SCRATCH").is_some_and(|value| value == "1");
    let mut world_points = TransformScratch::new(separate_points, observe_points);
    let world_mesh_start = render_instances.len();
    let mut bwm_visibility_render_instances = vec![Vec::new(); scene.world.visibility.len()];
    for (object_number, object) in scene.world.objects.iter().enumerate() {
        if object_number % 32 == 0 {
            check_cancelled()?;
        }
        if object_number == scene.world.objects.len() / 2 {
            super::load_timing::mark("setup: half of the world objects");
        }
        let _growth = super::load_timing::Growth::new(format!("BWM object {object_number} material {}", object.material_path));
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
            world_points.fill(source.positions(), |position| transformed(matrix, position));
            let _tag = crate::platform::allocation_tags::tag("world material resolution");
            resolver.get_world(
                &object.material_path,
                object_number,
                mean_game_position(&world_points.points),
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
            if matrix_index != object.matrix_start || separate_points {
                world_points.fill(source.positions(), |position| transformed(matrix, position));
            }
            let points = world_points.points.as_slice();
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
                    mean_game_position(points),
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
            for &point in points {
                whole_world.add(point);
                regions
                    .entry((point.x / 35.0).floor() as i32)
                    .or_insert_with(Bounds::empty)
                    .add(point);
            }
            let instance = game_affine_to_view(matrix);
            let bounds = authored_world_bounds(visibility_record);
            let render_instance = if needs_exact_world_mesh(&instance, source.positions(), points) {
                // Bevy's Transform cannot express shear. Preserve the exact
                // authored affine transform in this instance's vertex data.
                let baked = meshes.add(super::view_geometry::authored_mesh_from_uvs(
                    points,
                    super::view_geometry::MeshUvs::Packed(source.uvs()),
                    source.indices(),
                    source.packed_channels(),
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

                    entity: None,
                    visible: false,
                }
            };
            vegetation_meshes.prepare(
                resolver.needs_vegetation_channels(&render_instance.material),
                &render_instance.mesh,
                meshes,
                source.positions(),
                source.packed_channels(),
            );
            let render_index = render_instances.len();
            bwm_visibility_render_instances[matrix_index].push(render_index);
            render_instances.push(render_instance);
        }
    }
    world_points.report("world");
    if observe_points {
        ModelMeshObservation { enabled: true, ..Default::default() }
            .report("WORLD_MESH_PREPARATION", meshes, &render_instances[world_mesh_start..], common_meshes.as_deref());
    }
    super::load_timing::mark("setup: world geometry and materials");
    // Only the later placed-model and arm writers can update their records.
    let writable_probe_start = resolver.probe_sh_records.len();

    if trace_startup {
        eprintln!(
            "Startup trace: BWM setup finished ({} render instances, {} diffuse paths discovered); beginning placed-model setup",
            render_instances.len(),
            resolver.diffuse_image_paths.len()
        );
    }
    let prepared_root_count = scene.expanded_placement_roots.take();
    let root_placement_count = prepared_root_count.unwrap_or(scene.placed_models.len());
    let mut expanded_placements =
        if env::var_os("DH2_DEBUG_RETAIN_PLACEMENT_STAGING").is_some_and(|value| value == "1") {
            scene.placed_models.clone()
        } else {
            // RenderSourcePlacements is the only live owner after setup.
            std::mem::take(&mut scene.placed_models)
        };
    if let Some(catalog) = catalog.filter(|_| prepared_root_count.is_none()) {
        super::view_geometry::expand_model_placements(&mut expanded_placements, catalog);
    }
    if env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
        report_placement_storage(&scene.placed_models, &expanded_placements);
    }
    let trace_model_staging = env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1");
    let retain_static_sources = env::var_os("DH2_DEBUG_RETAIN_STATIC_MODEL_STAGING").is_some_and(|value| value == "1");
    let placement_start = trace_model_staging.then(crate::loading::thread_cycles).flatten();
    let mut model_cache = HashMap::new();
    if !retain_static_sources {
        for (number, placement) in expanded_placements.iter().enumerate() {
            let path = placement.resource_path.as_str();
            if uses_static_model(path) {
                model_cache.insert(path, StaticModelSlot { last_placement: number, source: None, level_bytes: 0 });
            }
        }
    }
    let mut static_live_bytes = 0;
    let mut static_peak_bytes = 0;
    let mut static_decoded_bytes = 0;
    let mut packed_uv_pairs = 0;
    let mut packed_uv_mismatches = 0;
    let common_models = catalog.and_then(bmodel::CommonModels::shared);
    let mut md6_cache = HashMap::<String, Option<SceneSkeletalModel>>::new();
    let mut model_sources = catalog.map(bmd6::ModelSources::new);
    let mut md6_rest_pose_instances = 0usize;
    let mut model_mesh_handles = HashMap::<(String, usize, bool, u8), Handle<Mesh>>::new();
    let placed_mesh_start = render_instances.len();
    let mut mesh_observation = ModelMeshObservation {
        enabled: env::var_os("DH2_DEBUG_MODEL_MESH_PREPARATION").is_some_and(|value| value == "1"),
        repeated: env::var_os("DH2_DEBUG_REPEAT_MODEL_MESH_PREPARATION").is_some_and(|value| value == "1"),
        ..Default::default()
    };
    let mut model_instances = 0usize;
    let mut model_surfaces = 0usize;
    let mut placed_model_queries = Vec::new();
    let mut shadow_model_bounds = Vec::new();
    let mut model_shadow_sources = Vec::new();
    let mut skin_palettes =
        HashMap::<String, Result<Arc<shadow_updates::RestSkinning>, String>>::new();
    let mut placed_model_default_ranges = 0usize;
    let mut deformed = machinery.map(|m| m.deformed_placements()).unwrap_or_default();
    if let Some(interactions) = interactions {
        for (index, placement) in expanded_placements.iter().enumerate() {
            if crate::bmd6::declaration_path(&placement.resource_path).is_some()
                && interactions.actor_by_entity.get(&placement.id).is_some_and(|&actor| {
                    interactions.actors[actor].definition.steps.iter().any(|step| step.animation.is_some() || step.transition_animation.is_some())
                }) {
                deformed.insert(index);
            }
        }
    }
    // Scene preparation can borrow already acquired physics sources. Editable
    // layers use fresh reads so changes between preparation phases remain visible.
    let physics_models: HashMap<&str, &Arc<bmd6::ModelInfo>> = if catalog.is_some_and(|c| !c.has_overlays()) {
        machinery.into_iter().flat_map(|m| m.skin_models()).filter_map(|(index, model)| {
            expanded_placements.get(index).map(|placement| (placement.resource_path.as_str(), model))
        }).collect()
    } else { HashMap::new() };
    let mut placed_points = TransformScratch::new(separate_points, observe_points);
    for (number, placement) in expanded_placements.iter().enumerate() {
        if number % 32 == 0 {
            check_cancelled()?;
        }
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
                    if let Some(model) = physics_models.get(placement.resource_path.as_str()) {
                        return model.has_decoded_geometry.then(|| SceneSkeletalModel::Shared(Arc::clone(model)));
                    }
                    let catalog = catalog?;
                    match model_sources.as_mut()?.acquire(catalog, &placement.resource_path) {
                        Ok(model) if model.has_decoded_geometry => Some(SceneSkeletalModel::Prepared(model)),
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
                    placed_points.fill(&surface.positions, |vertex| placed_vertex(placement, vertex));
                    let points = placed_points.points.as_slice();
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
                    let bounds = render_bounds(points);
                    let instance = placed_transform(placement);
                    let deforms = deformed.contains(&number);
                    let world_baked = !deforms
                        && needs_exact_world_mesh(&instance, &surface.positions, points);
                    let skin_influences = resolver.shadow_materials.skin_influences(&mat.handle);
                    let flipped = instance.to_matrix().determinant() < 0.0;
                    let repeated = mesh_observation.repeated;
                    let pose_vertices = if skin_influences != 0 && skinning.is_ok() {
                        surface.positions.len()
                    } else { 0 };
                    let mut initialize_mesh = |mesh: &mut Mesh| {
                        mesh_observation.prepare(pose_vertices, || {
                            retain_model_shadow_position(mesh, &surface.positions);
                            // Physics-system skinning re-poses these every frame.
                            if deforms {
                                mesh.asset_usage |= bevy::asset::RenderAssetUsages::MAIN_WORLD;
                            }
                            retain_skin_inputs(mesh, surface);
                            if skin_influences != 0 {
                                if let Err(error) = skinning.as_ref().map_err(Clone::clone).and_then(|skin| {
                                    apply_rest_pose(mesh, surface, &skin.matrices, skin_influences, placement, world_baked)
                                }) {
                                    eprintln!("Model rest pose pending {}: {error}", placement.resource_path);
                                }
                            }
                        });
                    };
                    let mut prepare_mesh = |mut mesh: Mesh| {
                        if !repeated { initialize_mesh(&mut mesh); }
                        meshes.add(mesh)
                    };
                    let uvs = MeshUvs::Packed(&surface.uvs);
                    let (mesh, transform) = if world_baked {
                        (
                            prepare_mesh(authored_mesh_from_uvs(
                                points,
                                uvs,
                                &surface.indices,
                                &surface.packed_channels,
                                |v| placed_direction(placement, v),
                            )),
                            Transform::IDENTITY,
                        )
                    } else if deforms {
                        (
                            prepare_mesh(source_mesh_from_uvs(
                                &surface.positions,
                                uvs,
                                &surface.indices,
                                &surface.packed_channels,
                                flipped,
                            )),
                            instance,
                        )
                    } else {
                        (
                            model_mesh_handles
                                .entry((
                                    placement.resource_path.clone(),
                                    surface_number,
                                    flipped,
                                    skin_influences,
                                ))
                                .or_insert_with(|| {
                                    prepare_mesh(source_mesh_from_uvs(
                                        &surface.positions,
                                        uvs,
                                        &surface.indices,
                                        &surface.packed_channels,
                                        flipped,
                                    ))
                                })
                                .clone(),
                            instance,
                        )
                    };
                    if repeated {
                        initialize_mesh(meshes.get_mut(&mesh).expect("prepared skeletal mesh"));
                    }
                    vegetation_meshes.prepare(
                        resolver.needs_vegetation_channels(&mat.handle),
                        &mesh,
                        meshes,
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
                    // arkSkinnedRenderModel shares idRenderModelStatic's
                    // vtable +0xc0 update (140235680, bounds bit 4), so
                    // 14031ad30 selects MD6 owners the same way as BML.
                    if let Some(input) = initial_placed_model_query(placement, model.authored_bounds) {
                        placed_model_queries.push(PlacedModelQueryGroup {
                            indices: model_render_start..render_instances.len(),
                            input,
                        });
                    }
                }
            }
            continue;
        }
        // Authoring helpers have no slot because they are not visible objects.
        let path = placement.resource_path.as_str();
        let slot = if retain_static_sources && uses_static_model(path) {
            model_cache.entry(path).or_insert(StaticModelSlot {
                last_placement: number, source: None, level_bytes: 0,
            })
        } else {
            let Some(slot) = model_cache.get_mut(path) else { continue; };
            slot
        };
        let StaticModelSlot { last_placement, source, level_bytes } = slot;
        let model = source.get_or_insert_with(|| {
            let catalog = catalog.as_ref()?;
            match bmodel::acquire(catalog, path, common_models) {
                Ok(Some(model)) => {
                    if trace_model_staging && matches!(&model, bmodel::ModelSource::Level(_)) {
                        *level_bytes = model.surfaces.iter().map(|s| capacity_bytes(&s.positions)
                            + capacity_bytes(&s.uvs) + capacity_bytes(&s.packed_channels)
                            + capacity_bytes(&s.indices)).sum();
                        static_live_bytes += *level_bytes;
                        static_decoded_bytes += *level_bytes;
                        static_peak_bytes = static_peak_bytes.max(static_live_bytes);
                    }
                    Some(model)
                }
                Ok(None) => {
                    eprintln!("Model not indexed: {path}");
                    None
                }
                Err(error) => {
                    eprintln!("Model acquisition failed {path}: {error}");
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
            placed_points.fill(&surface.positions, |vertex| placed_vertex(placement, vertex));
            let points = placed_points.points.as_slice();
            for &point in points {
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
            let bounds = render_bounds(points);
            let instance = placed_transform(placement);
            let world_baked = needs_exact_world_mesh(&instance, &surface.positions, points);
            let repeated = mesh_observation.repeated;
            let mut initialize_mesh = |mesh: &mut Mesh| mesh_observation.prepare(0, || {
                retain_model_shadow_position(mesh, &surface.positions);
            });
            let needs_vegetation = resolver.needs_vegetation_channels(&mat.handle);
            let cached_common = if !world_baked && matches!(&*model, bmodel::ModelSource::Common(_)) {
                common_meshes.as_deref_mut().filter(|common| common.enabled()).map(|common|
                    common.acquire(path, surface_number, instance.to_matrix().determinant() < 0.0,
                        needs_vegetation, surface, meshes, |mesh| initialize_mesh(mesh)))
            } else { None };
            let mut prepare_mesh = |mut mesh: Mesh| {
                if !repeated { initialize_mesh(&mut mesh); }
                meshes.add(mesh)
            };
            let (mesh, transform) = if let Some(common) = &cached_common {
                (common.handle.clone(), instance)
            } else if world_baked {
                baked_shear_instances += 1;
                (
                    prepare_mesh(super::view_geometry::authored_mesh_from_uvs(
                        points,
                        super::view_geometry::MeshUvs::Packed(&surface.uvs),
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
                            let mesh = super::view_geometry::source_mesh_from_uvs(
                                &surface.positions,
                                super::view_geometry::MeshUvs::Packed(&surface.uvs),
                                &surface.indices,
                                &surface.packed_channels,
                                instance.to_matrix().determinant() < 0.0,
                            );
                            if trace_model_staging {
                                if let Some(bevy::mesh::VertexAttributeValues::Float16x2(uvs)) =
                                    mesh.attribute(super::view_geometry::PACKED_UV_0)
                                {
                                    packed_uv_pairs += uvs.len();
                                    packed_uv_mismatches += surface.uvs.len().abs_diff(uvs.len())
                                        + surface.uvs.iter().zip(uvs).filter(|(a, b)| a.map(half::f16::to_bits) != b.map(half::f16::to_bits)).count();
                                }
                            }
                            prepare_mesh(mesh)
                        })
                        .clone(),
                    instance,
                )
            };
            if repeated {
                initialize_mesh(meshes.get_mut(&mesh).expect("prepared static mesh"));
            }
            if let Some(common) = &cached_common {
                if let Some(result) = &common.vegetation {
                    vegetation_meshes.0.insert(mesh.id(), result.clone());
                }
            } else {
                vegetation_meshes.prepare(needs_vegetation, &mesh, meshes,
                    &surface.positions, &surface.packed_channels);
            }
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
        if !retain_static_sources && *last_placement == number {
            *source = None;
            static_live_bytes -= *level_bytes;
        }
    }
    let static_resource_count = model_cache.len();
    drop(model_cache);
    if trace_model_staging {
        let cycles = placement_start.zip(crate::loading::thread_cycles())
            .map_or(0, |(start, end)| end.saturating_sub(start));
        eprintln!("STATIC_MODEL_STAGING retained={retain_static_sources} sources={static_resource_count} decoded_bytes={static_decoded_bytes} peak_bytes={static_peak_bytes} live_bytes={static_live_bytes} cycles={cycles}");
    }
    placed_points.report("placed-models");
    println!(
        "Initial BML/MD6 query groups: {}; constructor-default range models: {}; blueprint and surface bit40 producers remain unsupported",
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
    let mut player_declarations = crate::entities::player::DeclarationSource::default();
    if let Some(catalog) = catalog {
        match super::player_arms::load_player_arms(catalog, &mut resolver, meshes, commands, &mut player_declarations) {
            Ok(arms) => commands.insert_resource(arms),
            Err(error) => eprintln!("First-person arms unavailable: {error}"),
        }
    }
    let probe_sh_record_count = resolver.probe_sh_records.len().saturating_sub(1);
    commands.insert_resource(crate::rendering::bevy_backend::water_scene::WaterModelLighting {
        records: resolver.probe_sh_buffer.clone(),
        indices: render_instances.iter().map(|instance| instance.probe_index).collect(),
    });
    check_cancelled()?;
    super::load_timing::mark("setup: placed models");
    if let Some(lighting) = resolver.upload_probe_records(writable_probe_start) {
        commands.insert_resource(lighting);
    }
    println!(
        "Shared geometry: {} world meshes and {} placed-model surfaces; {} world instances baked for shear",
        world_shadow_mesh_sources.shared_mesh_count,
        model_mesh_handles.len(),
        baked_shear_instances
    );
    println!("BWM authored instance counts by LOD tag: {authored_lod_instances:?}");
    println!(
        "Placed models: {model_instances}/{} instances, {} unique resources, {model_surfaces} surfaces",
        root_placement_count,
        static_resource_count
    );
    println!(
        "MD6 skeletal instances shown in compiled rest pose: {md6_rest_pose_instances}; joint animation is not implemented"
    );
    // Host decoding scratch ends here, before later water/FX acquisition.
    // Runtime meshes/query/skinning owners already hold their required data;
    // as with input retirement at 14025aa00's exit, staging is not residency.
    mesh_observation.report("MODEL_MESH_PREPARATION", meshes, &render_instances[placed_mesh_start..], common_meshes.as_deref());
    if trace_model_staging {
        eprintln!("MEMORY BML1 packed UV preparation: pairs={packed_uv_pairs} mismatches={packed_uv_mismatches}");
        let skeletal_bytes: usize = md6_cache.values().flatten()
            .filter(|model| matches!(model, SceneSkeletalModel::Prepared(_)))
            .flat_map(|model| &model.surfaces)
            .map(|s| capacity_bytes(&s.positions) + capacity_bytes(&s.uvs)
                + capacity_bytes(&s.packed_channels) + capacity_bytes(&s.indices)
                + capacity_bytes(&s.skin_weights) + capacity_bytes(&s.skin_indices)).sum();
        eprintln!("MEMORY model staging retirement: static_resources={} skeletal_resources={} vertex_index_capacity_bytes={}",
            static_resource_count, md6_cache.len(), skeletal_bytes);
    }
    drop(md6_cache);
    drop(model_sources);
    drop(physics_models);
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
    let player = match (navigation.as_ref(), scene.player_start, catalog) {
        (Some(_), Some(start), Some(catalog)) if env::var_os("DH2_REPLAY_F8").is_none() => {
            match super::player::load_player_setup_with_source_cancellable(catalog, super::pause_menu::debug_character(), &mut player_declarations, Some(&|| check_cancelled().is_err())) {
                Ok(setup) => {
                    let yaw = start.forward_xy[1].atan2(start.forward_xy[0]);
                    crate::gameplay::player::Player::new(setup, Vec3::from(start.position), yaw, reference_horizontal_fov)
                        .map_err(|error| eprintln!("Player unavailable, debug free camera only: {error}"))
                        .ok()
                }
                Err(error) => {
                    eprintln!("Player unavailable, debug free camera only: {error}");
                    None
                }
            }
        }
        _ => None,
    };
    player_declarations.keep_screen_inputs();
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
                secondary_probe_count,
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
            // COPY_SRC: the "Depth buffer hack" pass copies view depth/stencil.
            depth_texture_usages: (bevy::render::render_resource::TextureUsages::RENDER_ATTACHMENT
                | bevy::render::render_resource::TextureUsages::TEXTURE_BINDING
                | bevy::render::render_resource::TextureUsages::COPY_SRC)
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
    commands.insert_resource(super::player::PlayerState { debug_free_fly: player.is_none(), player, difficulty: catalog.and_then(super::player::load_difficulty) });
    commands.insert_resource(super::player::PlayerInput::default());
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
                        // Bevy's spot-light implementation requires a sub-90°
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
                        // The rect carries the stable scene-record index; the
                        // forward evaluator uses its authored area profile and pool.
                        let Some(transform) =
                            authored_area_light_transform(position, light.rotation)
                        else {
                            unsupported += 1;
                            continue;
                        };
                        commands.spawn((
                            RectLight {
                                source_index: Some(light_index as u32),
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
        "Controls: mouse/arrows look (captured after loading), click to recapture mouse, Esc release; G toggles walk/free-fly; WASD move, Shift sprint, Space jump in walk; F8 save collision report and screenshot; Q/E vertical in free-fly; PageUp/PageDown region, Home reset, End overview"
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
    commands.insert_resource(resolver.alpha_depth_materials());
    tome_render_instances.bind_texture_demands(&render_instances, &material_images);
    if env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
        let bytes: usize = expanded_placements.iter().map(|placement| {
            placement.custom_materials.capacity() * std::mem::size_of::<Option<String>>()
                + placement.custom_materials.iter().flatten().map(String::capacity).sum::<usize>()
        }).sum();
        eprintln!("MEMORY placement material overrides retired: bytes={bytes}");
    }
    for placement in &mut expanded_placements {
        placement.custom_materials = Vec::new();
    }
    commands.insert_resource(RenderSourcePlacements(expanded_placements));
    commands.insert_resource(PlacedModelQueries(placed_model_queries));
    commands.insert_resource(ShadowModelBoundsSources(shadow_model_bounds));
    commands.insert_resource(shadow_updates::ModelShadowSources(model_shadow_sources));
    let mut ocean_image_slots = Vec::new();
    let mut ocean_inline_slots = std::collections::HashSet::new();
    let mut particle_resources = crate::rendering::particles::definition::ParticleResources::default();
    if std::env::var("DH2_DEBUG_SEPARATE_PARTICLE_IMAGES").as_deref() != Ok("1") {
        particle_resources.use_scene_images();
    }
    if let (Some(catalog),Some(parameters))=(catalog,scene.environment_render_parameters.as_ref()) {
        let (water,sources)=super::water_scene::WaterScene::load_with_model_colours(catalog,parameters,&scene.area_environments,&resolver.water_materials,&resolver.water_model_colours, resident_images, images)
            .unwrap_or_else(|error|panic!("water scene preparation failed: {error}"));
        for path in sources.ocean.keys() {
            // 1402e9ff0 registers images whose authored finer mips exist.
            if !(0..=15).any(|mip| catalog.contains(&format!("{path}_mip{mip}"))) { continue; }
            let previous_count = resolver.diffuse_image_paths.len();
            let slot = resolver.register_image(path).expect("prepared ocean image must resolve");
            if slot == previous_count { ocean_inline_slots.insert(slot); }
            ocean_image_slots.push((path.clone(), slot));
        }
        commands.insert_resource(water);
        commands.insert_resource(sources);
        let programs=resolver.programs.as_deref_mut().expect("ocean program library").as_mut()
            .unwrap_or_else(|error|panic!("ocean program library: {error}"));
        commands.insert_resource(crate::rendering::bevy_backend::ocean_scene::OceanSources::load(catalog,programs)
            .unwrap_or_else(|error|panic!("ocean programs: {error}")));
        commands.insert_resource(crate::rendering::bevy_backend::screen_effects::ScreenEffectMaterials(Some(Arc::new(
            crate::rendering::bevy_backend::screen_effects::ScreenEffectSources::load_shared_with_player(catalog,programs,
                super::player::character_name(super::pause_menu::debug_character()), &mut particle_resources, &mut player_declarations)
                .unwrap_or_else(|error|panic!("player screen effects: {error}"))
        ))));
        commands.insert_resource(crate::rendering::bevy_backend::camera_scene::FxSimulation::default());
    }
    // World FX load after the water and ocean programs, as before. Particle
    // surface proxies may register further images: extend the tables.
    let mut particle_surfaces = Vec::new();
    let mut particle_world_images = std::collections::HashMap::new();
    let mut dynamic_particle_materials = HashSet::new();
    if let (Some(catalog), Some(graphs)) = (catalog, effect_graphs) {
        let programs = resolver.programs.as_deref_mut().expect("world FX program library").as_mut()
            .unwrap_or_else(|error| panic!("world FX program library: {error}"));
        let sources = Arc::new(
            crate::rendering::particles::world_sources::WorldFxSources::load_shared(&graphs.graphs,&graphs.actors,catalog,programs,&mut particle_resources)
        );
        let surface_failures = prepare_particle_surfaces(sources.world_meshes(), &mut resolver, meshes,
            &mut particle_resources, &mut particle_world_images, &mut dynamic_particle_materials,
            |surface| particle_surfaces.push(super::particle_surface::publish(commands, surface)));
        for error in surface_failures { eprintln!("Particle surface: {error}"); }
        particle_surfaces.extend(super::projected_decals::prepare(&sources, &scene.world, &render_instances,
            &mut resolver, meshes, commands));
        let mut simulation = crate::rendering::bevy_backend::camera_scene::FxSimulation::default();
        simulation.prepare_entity_graphs(graphs, &sources, live_entities)?;
        commands.insert_resource(simulation);
        commands.insert_resource(crate::rendering::bevy_backend::world_effects::WorldEffectSources(sources));
    }
    commands.insert_resource(super::texture_streaming::environment_images::OceanImageSlots(ocean_image_slots));
    particle_resources.retire_unused();
    let mut particle_image_slots = Vec::new();
    for image in particle_resources.images().filter(|image| image.scene_owned) {
        let slot = resolver.register_image(&image.path)
            .unwrap_or_else(|| panic!("particle scene image unresolved: {}", image.path));
        resolver.cache.raise_image_priority(slot,
            image.sampling_priority.load(std::sync::atomic::Ordering::Relaxed));
        particle_image_slots.push((slot, image.clone()));
    }
    commands.insert_resource(crate::rendering::bevy_backend::particle_images::SceneParticleImages {
        slots: Arc::new(particle_image_slots), world_materials: Arc::new(particle_world_images), ..Default::default()
    });
    particle_resources.report_scene_images(&resolver.diffuse_image_paths);
    commands.insert_resource(super::usable_effects::ParticleResourceOwner::new(particle_resources));
    diffuse_image_materials.resize(resolver.diffuse_image_paths.len(), Vec::new());
    let diffuse_image_count = resolver.diffuse_image_paths.len();
    // A proxy streams the images of the material it was cloned from.
    let mut particle_surface_images = Vec::new();
    for (proxy, images) in particle_surfaces {
        material_images.insert(proxy.clone(), images.iter().map(|&(slot, _)| slot).collect());
        for (slot, role) in images {
            diffuse_image_materials[slot].push((proxy.clone(), role));
            if !dynamic_particle_materials.contains(&proxy) {
                particle_surface_images.push(slot);
            }
        }
    }
    particle_surface_images.sort_unstable();
    particle_surface_images.dedup();
    commands.insert_resource(std::mem::take(&mut resolver.water_materials));
    commands.insert_resource(std::mem::take(&mut resolver.water_model_colours));
    commands.insert_resource(std::mem::take(&mut resolver.water_stencil_materials));
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
        &scene.world, &world_shadow_mesh_sources, meshes,
        world_matrices, resolver.probe_sh_buffers,
    ));
    world_shadow_mesh_sources.release_cpu_copies(meshes);
    commands.insert_resource(world_shadow_mesh_sources);
    // Water was the last CPU consumer of the installed geometry sections.
    // The prepared meshes own their vertex and index data independently.
    scene.world.meshes = Vec::new();
    super::load_timing::mark("world geometry source released");
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
    let render_queue = SceneRenderQueue {
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
        diffuse_image_demands: resolver.diffuse_image_paths.iter().enumerate()
            .map(|(slot, path)| super::texture_streaming::image_demand(path, resolver.cache.image_priority(slot)))
            .collect(),
        diffuse_image_paths: std::mem::take(&mut resolver.diffuse_image_paths),
        diffuse_image_materials,
        material_images,
        particle_surface_images,
        material_stage_sorts,
        diffuse_image_handles: vec![None; diffuse_image_count],
        shadow_diffuse_slots: Vec::new(),
        shadow_diffuse_membership: Vec::new(),
        diffuse_image_pending: vec![None; diffuse_image_count],
        diffuse_mip_states: (0..diffuse_image_count)
            .map(|slot| DiffuseMipState { sampling_priority: resolver.cache.image_priority(slot), ..Default::default() })
            .collect(),
        diffuse_image_attempted: vec![false; diffuse_image_count],
        diffuse_image_last_visible: vec![0; diffuse_image_count],
        texture_stream_frame: 0,
        texture_viewport_pixels: 0,
        diffuse_mip_upgrades: 0,
        preferred_mip,
    };
    let base_reader = if let (Some(catalog), Some((shared, budget))) = (&catalog_storage, shared_images) {
        let (prepared, reader) = super::texture_streaming::prepare_base_images(
            catalog, &render_queue, shared, resident_images, &ocean_inline_slots, budget, check_cancelled,
        )?;
        commands.insert_resource(prepared);
        reader
    } else { None };
    resolver.cache.retire_staged_source();
    resolver.cache.report_source_handoff();
    resolver.cache.report_runtime_retention();
    // Host handoff follows every material consumer, including particle and
    // decal preparation (1fbad0/31ad30). Taking these owners earlier replaces
    // their shared bindings with defaults while the resolver still uses them.
    commands.insert_resource(std::mem::take(&mut resolver.shadow_materials));
    commands.insert_resource(std::mem::take(&mut resolver.vegetation));
    commands.insert_resource(super::runtime_materials::RuntimeMaterialOwner {
        bindings: runtime_material_bindings,
        cache: std::mem::take(&mut resolver.cache),
    });
    let image_catalog = catalog_storage.clone();
    commands.insert_resource(render_queue);
    if let Some(catalog) = catalog_storage {
        commands.insert_resource(TextureReadWorker::new(catalog, diffuse_image_count));
    }
    commands.queue(move |world: &mut World| {
        let Some(catalog) = image_catalog else { return };
        world.init_resource::<super::texture_streaming::SharedImages>();
        super::texture_streaming::reserve_scene_images(world);
        // Standalone renderer diagnostics have no loading coordinator. They
        // acquire through the same owner immediately, as setup did before.
        if !super::loading_screen::image_uploads_deferred(world) {
            for common in [true, false] {
                if let Err(error) = super::texture_streaming::initialize_scene_images(world, &catalog, common) {
                    error!("Scene image acquisition failed: {error}");
                    world.write_message(AppExit::error());
                    break;
                }
            }
        }
    });
    super::load_timing::mark("setup: end");
    Ok(base_reader)
}

/// 1401fbad0 makes a particle mesh renderer an ordinary model surface of its
/// world material; one proxy per (PMODEL, material) pair carries that
/// material's own pipelines, and the FX update supplies its instances.
pub(super) fn prepare_particle_surfaces(
    world_meshes: Vec<(String, String, Arc<crate::assets::particle_model::ParticleModel>)>,
    resolver: &mut MaterialResolver,
    meshes: &mut dyn AssetWrite<Mesh>,
    resources: &mut crate::rendering::particles::definition::ParticleResources,
    world_images: &mut std::collections::HashMap<String, Vec<Arc<crate::rendering::particles::image_resources::ParticleImage>>>,
    dynamic_materials: &mut HashSet<super::MaterialHandle>,
    mut prepared: impl FnMut(super::particle_surface::PreparedSurface),
) -> Vec<String> {
    let mut failures = Vec::new();
    for (source, material, model) in world_meshes {
        let Some(resolved) = resolver.get(&material, 0, [0.0; 3]) else {
            failures.push(format!("material unresolved {source} {material}"));
            continue;
        };
        let super::MaterialHandle::Probe(handle) = &resolved.handle else {
            failures.push(format!("{source} {material} is not a forward model material"));
            continue;
        };
        let Some(mut instanced) = resolver.probe_materials.get(handle).cloned() else {
            failures.push(format!("{source} {material} has no prepared material"));
            continue;
        };
        match resolver.caster_admission(&material) {
            Ok(admission) if admission & 1 != 0 => eprintln!(
                "Particle surface {source} {material}: shadow casting of particle surfaces is unported"
            ),
            Ok(_) => {}
            Err(error) => eprintln!("Particle surface {source} {material}: {error}"),
        }
        let grid = source.strip_prefix("sprite-grid:").map(str::parse::<u32>);
        let prepared_mesh = match grid {
            Some(Ok(level)) => super::particle_surface::prepare_sprite_grid(level),
            Some(Err(error)) => { failures.push(format!("{source} {error}")); continue; }
            None => match super::particle_surface::prepare_mesh(&model) {
                Ok(mesh) => mesh,
                Err(error) => { failures.push(format!("{source} {error}")); continue; }
            },
        };
        instanced.extension.surface.particle_instances.x = 1;
        let images = resolver.model_images(&material);
        let mut bindings = Vec::new();
        let mut image_failure = None;
        if let Some(catalog) = resolver.catalog {
            for &(slot, _) in &images {
                match resources.scene_image(catalog, &resolver.diffuse_image_paths[slot]) {
                    Ok(Some(image)) => bindings.push(image),
                    Ok(None) => {}
                    Err(error) => { image_failure = Some(error); break; }
                }
            }
        }
        if let Some(error) = image_failure {
            failures.push(format!("{source} {material} image acquisition: {error}"));
            continue;
        }
        let mesh = meshes.add(prepared_mesh);
        let instanced = resolver.probe_materials.add(instanced);
        if !bindings.is_empty() && bindings.len() == images.len() {
            world_images.entry(material.clone()).or_insert(bindings);
            dynamic_materials.insert(super::MaterialHandle::Probe(instanced.clone()));
        }
        prepared(super::particle_surface::PreparedSurface {
            source, material, mesh, handle: instanced, images,
        });
    }
    failures
}
