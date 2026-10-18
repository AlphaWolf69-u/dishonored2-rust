//! Game-thread ownership of the native parallel prepare/receiver/gather sequence.
//! Rendering consumes these masks, never the color-visible instance list.
use super::{
    build_native_world_layer, camera_projection, coarse_depth, frame_profile, game_clock,
    game_clock_runtime, game_pose_from_view_transform, material, native_material_option_overrides,
    native_view_distance_scale, placed_direction, prepare_native_world_shadow_masks,
    read_streamed_diffuse_at_target, render_model, render_program, render_shadow,
    screen_shadow_pass, screen_shadow_pipeline, shader_cache, shadow_pass, shadow_pipeline,
    shadow_receiver, shadow_sampling, tome, view_to_game, Arc, Extent3d, NativeParallelShadowOwner,
    NativeProgramCacheSource, NativeShadowCasterWorld, NativeShadowMaterials,
    NativeShadowModelBoundsSources, NativeVegetationMeshInputs, NativeViewSettings,
    NativeWorldShadowMeshSources, PickupState, Projection, RenderModelSurface,
    RenderSourcePlacements, SceneData, SceneRenderQueue, ShaderBuffer, TextureDimension,
    TextureFormat, TextureReadTarget, TomeWorld,
};
use bevy::prelude::*;
use tome::shadow_caster::{
    NativeCasterDirection, NativeGlobalShadowCasterInput, NativeGlobalShadowCasterScratch,
    NativeReceiverDepthProjection, NativeShadowDistanceDescriptor, NativeShadowFrustumQuery,
    NativeShadowInstanceMasks, NativeShadowObjectDistanceQuery, VerifiedShadowReceiverState,
};

#[derive(Resource, Default)]
pub(super) struct NativeModelShadowSources(pub(super) Vec<NativeModelShadowSource>);

pub(super) struct NativeModelShadowSource {
    pub placement: usize,
    pub bounds: [[f32; 3]; 2],
    pub skinned: bool,
    pub surfaces: Vec<(usize, i32, Result<u8, String>)>,
    pub instances: std::ops::Range<usize>,
    pub pickup_id: Option<String>,
    pub flags_1c30: u8,
    pub flags_18bc: u8,
    pub flags_18bd: u8,
    pub skinning: Option<Result<Arc<NativeRestSkinning>, String>>,
}

pub(super) struct NativeRestSkinning {
    pub palette: Handle<ShaderBuffer>,
    pub matrices: Vec<[[f32; 4]; 3]>,
}

#[derive(Clone)]
pub(super) struct NativeModelShadowCaster {
    pub source: usize,
    pub instances: Vec<usize>,
    pub flags: u8,
    pub sort_distance: f32,
    pub camera_clip_mask: u8,
    pub dynamic: bool,
}

#[derive(Resource, Default)]
pub(super) struct NativeModelCasterFrame {
    pub current: Option<(u32, Arc<Vec<NativeModelShadowCaster>>)>,
    pub error: Option<String>,
    reported: bool,
}

pub(super) fn collect_native_model_casters(
    sources: Res<NativeModelShadowSources>,
    scene: Res<SceneData>,
    queue: Res<SceneRenderQueue>,
    pickups: Res<PickupState>,
    cameras: Query<(&Transform, &Projection), With<Camera3d>>,
    frame_count: Res<bevy::diagnostic::FrameCount>,
    mut output: ResMut<NativeModelCasterFrame>,
    profile: Option<Res<frame_profile::CpuFrameProfile>>,
) {
    let _span = profile.as_deref().map(|p| p.measure(26));
    output.current = None;
    let result = (|| -> Result<Vec<NativeModelShadowCaster>, String> {
        let (transform, projection) = cameras.single().map_err(|error| error.to_string())?;
        let camera_view = tome::render_query_base_rows(
            Mat4::IDENTITY.to_cols_array_2d(),
            game_pose_from_view_transform(transform),
        );
        let mut inverse_view = [[0.0; 4]; 4];
        if !render_shadow::invert_rows(&camera_view, &mut inverse_view) {
            return Err("native caster camera view is singular".into());
        }
        let Projection::Custom(custom) = projection else {
            return Err("native model caster camera is unavailable".into());
        };
        let projection = custom
            .get::<camera_projection::NativeGameProjection>()
            .ok_or("native model caster projection type is unavailable")?;
        let clip = render_shadow::multiply_rows(projection.rows(), camera_view);
        let clip: [f32; 16] = std::array::from_fn(|i| clip[i / 4][i % 4]);
        let cull_distance = scene
            .shadow_quality
            .as_ref()
            .ok_or("native caster quality is unavailable")?
            .cull_distance;
        let mut collected = Vec::new();
        for (source_index, source) in sources.0.iter().enumerate() {
            if source
                .pickup_id
                .as_deref()
                .is_some_and(|id| !pickups.world.pickup_available(id))
            {
                continue;
            }
            let surfaces = source
                .surfaces
                .iter()
                .map(|&(_, vertices, ref flags)| {
                    if vertices <= 0 {
                        Ok((vertices, 0))
                    } else {
                        Ok((vertices, flags.clone()?))
                    }
                })
                .collect::<Result<Vec<_>, String>>()?;
            let info = render_model::prepare_model_caster_info(
                source.bounds,
                source.flags_1c30,
                source.flags_18bc,
                source.flags_18bd,
                source.skinned,
                surfaces.iter().copied(),
            );
            let Some(sphere) = info.sphere else {
                continue;
            };
            let Some(sort_distance) = render_model::model_caster_sort_distance(
                info.flags,
                sphere,
                &inverse_view,
                cull_distance,
            ) else {
                continue;
            };
            let mut instances = Vec::new();
            for ((surface, _, _), &(vertices, flags)) in source.surfaces.iter().zip(&surfaces) {
                if vertices <= 0 || flags & 1 == 0 {
                    continue;
                }
                let instance=source.instances.clone().find(|&index|queue.instances.get(index)
                    .and_then(|instance|instance.source).is_some_and(|instance|match instance.surface {
                        RenderModelSurface::Bmodel{placement,surface:actual}
                        |RenderModelSurface::Bmd6{placement,surface:actual,..}=>placement==source.placement && actual==*surface,
                        _=>false,
                    })).ok_or_else(||format!("native model caster placement {} surface {surface} lacks an independently retained draw mesh",source.placement))?;
                instances.push(instance);
            }
            if !instances.is_empty() {
                collected.push(NativeModelShadowCaster {
                    source: source_index,
                    instances,
                    flags: info.flags,
                    sort_distance,
                    camera_clip_mask: render_model::native_transformed_aabb_clip_cull(
                        &clip,
                        source.bounds[0],
                        source.bounds[1],
                    )
                    .output_byte,
                    dynamic: render_model::model_is_dynamic_caster(source.flags_18bc),
                });
            }
        }
        Ok(collected)
    })();
    match result {
        Ok(casters) => {
            if output.error.take().is_some() || !output.reported {
                println!(
                    "Native model caster collection online: {} owners, {} surfaces",
                    casters.len(),
                    casters
                        .iter()
                        .map(|caster| caster.instances.len())
                        .sum::<usize>()
                );
                output.reported = true;
            }
            output.current = Some((frame_count.0, Arc::new(casters)));
        }
        Err(error) => {
            if output.error.as_ref() != Some(&error) {
                eprintln!("Native model caster collection pending: {error}");
                output.error = Some(error);
            }
        }
    }
}

#[derive(Resource)]
pub(super) struct NativeParallelShadowCpu {
    pub prepared: render_shadow::ParallelShadowPrepareState,
    pub current: Option<NativeParallelCasterFrame>,
    reset_frame: Option<u32>,
    initialized: [bool; 4],
    gather: Option<NativeGlobalShadowCasterScratch>,
    masks: Option<NativeShadowInstanceMasks>,
    gather_count: usize,
    world_query: Option<crate::local_caster_query::NativeLocalWorldQuery>,
    shadow_extent: crate::rendering::environment::EnvironmentScalar,
    static_frames: Vec<Option<Arc<NativeStaticShadowFrame>>>,
    last_error: Option<String>,
}

pub(super) struct NativeParallelCasterFrame {
    pub frame: u32,
    pub masks: Vec<Vec<u8>>,
    pub receiver: shadow_receiver::NativeReceiverOutput,
    pub slice_plans: Vec<render_shadow::ParallelShadowSlicePlan>,
    pub camera_view: render_shadow::ShadowMatrix,
    pub initialized: [bool; 4],
    pub static_frames: Vec<Option<Arc<NativeStaticShadowFrame>>>,
    pub local_time: [f32; 2],
    pub models: Option<Arc<Vec<NativeModelShadowCaster>>>,
    pub clocks: game_clock::GameplayViewClocks,
}

pub(super) struct NativeStaticShadowFrame {
    pub frame: u32,
    pub mask: Vec<u8>,
    pub clip: render_shadow::ShadowMatrix,
    pub local_time: [f32; 2],
}

impl Default for NativeParallelShadowCpu {
    fn default() -> Self {
        Self {
            prepared: render_shadow::ParallelShadowPrepareState::new(),
            current: None,
            reset_frame: None,
            initialized: [false; 4],
            gather: None,
            masks: None,
            gather_count: 0,
            world_query: None,
            // Registered env/shadow/parallelLightExtend default at +0x50,
            // measured from the original registry; area targets remain authored.
            shadow_extent: crate::rendering::environment::EnvironmentScalar::new(70.0),
            static_frames: Vec::new(),
            last_error: None,
        }
    }
}

/// Resolved BWM submissions, kept separate from model submissions. These
/// alone cannot authorize receiver sampling: the model collector must finish
/// before the combined scene frame can be published.
#[derive(Resource, Default)]
pub(super) struct NativeParallelWorldDraws {
    pub current: Option<(u32, Vec<shadow_pass::NativeShadowLayer>)>,
    matrices: Option<Handle<ShaderBuffer>>,
    static_layers: Vec<Option<(u32, shadow_pass::NativeShadowLayer)>>,
    last_error: Option<String>,
}

#[derive(Resource, Default)]
pub(super) struct NativeParallelModelDraws {
    pub current: Option<(u32, Vec<Vec<shadow_pass::NativeShadowInstance>>)>,
    last_error: Option<String>,
    reported: bool,
}

#[derive(Resource)]
pub(super) struct NativeParallelReceiverSource(Result<NativeParallelReceiverCompile, String>);

struct NativeParallelReceiverCompile {
    filtering_quality: i32,
    slope_scale: bool,
    slope_bias: bool,
    slope_post_projection: bool,
}

#[derive(Resource)]
pub(super) struct NativeScreenShadowSource(Result<NativeScreenShadowParameters, String>);
struct NativeScreenShadowParameters {
    producer: [u32; 9],
    blur: [u32; 9],
    step: f32,
    jitter: bool,
    cloud_motion: Vec4,
    cloud_density: Vec4,
    cloud_enabled: bool,
}

pub(super) fn initialize_native_screen_shadow_source(
    mut commands: Commands,
    scene: Res<SceneData>,
    mut programs: ResMut<NativeProgramCacheSource>,
    mut images: ResMut<Assets<Image>>,
) {
    let result = (|| -> Result<(NativeScreenShadowParameters, Handle<Image>), String> {
        let library = programs.0.as_mut().map_err(|error| error.clone())?;
        let catalog = scene
            .catalog
            .as_ref()
            .ok_or("native screen catalog is unavailable")?;
        let environment = scene
            .environment_render_parameters
            .as_ref()
            .ok_or("native screen environment is unavailable")?;
        let empty = render_program::SerializedParameterBlock {
            flags: 0,
            parameters: Vec::new(),
        };
        let mut scalar = |name: &str| -> Result<f32, String> {
            if let Some(value) = environment.scalar_parameter(name)? {
                return Ok(value);
            }
            Ok(f32::from_bits(
                library.registered_float_vector(catalog, &empty, name.as_bytes(), None)?[0],
            ))
        };
        let cloud_motion = Vec4::new(
            scalar("env/atm/sky/cloudAltitude")?,
            scalar("env/atm/sky/cloudRotation")?,
            scalar("env/atm/sky/cloudSpeedX")?,
            scalar("env/atm/sky/cloudSpeedY")?,
        );
        let cloud_density = Vec4::new(
            scalar("env/atm/sky/cloudTilingX")?,
            scalar("env/atm/sky/cloudTilingY")?,
            scalar("env/atm/sky/cloudDensity")?,
            0.0,
        );
        let step = scalar("screenShadows/stepSize")?;
        let cloud_name = environment
            .parameter("env/atm/sky/cloudShadow")
            .ok_or("native cloud texture owner is unavailable")?
            .trim()
            .trim_matches('"');
        let cloud = if cloud_name.eq_ignore_ascii_case("_black") {
            // Actual inherited env/default assigns engine _black. Red=0 makes
            // exp(-sample*density) exactly 1 for every position and clock.
            images.add(Image::new_fill(
                Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                &[0, 0, 0, 0],
                TextureFormat::Rgba8Unorm,
                Default::default(),
            ))
        } else {
            let stem = material::compiled_image_stem(catalog, cloud_name)
                .ok_or_else(|| format!("native cloud image source unresolved: {cloud_name}"))?;
            let mut loaded = read_streamed_diffuse_at_target(
                catalog,
                &stem,
                TextureReadTarget::Initial(Some(0)),
            )?
            .ok_or("native cloud authored mip0 is unavailable")?;
            loaded.image.data = Some(
                loaded
                    .uploads
                    .into_iter()
                    .flat_map(|upload| upload.payload)
                    .collect(),
            );
            images.add(loaded.image)
        };
        images
            .get_mut(&cloud)
            .ok_or("native cloud image allocation failed")?
            .sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
            address_mode_u: bevy::image::ImageAddressMode::Repeat,
            address_mode_v: bevy::image::ImageAddressMode::Repeat,
            mag_filter: bevy::image::ImageFilterMode::Linear,
            min_filter: bevy::image::ImageFilterMode::Linear,
            ..Default::default()
        });
        let material = catalog
            .read("generated/decls/material/screenshadows.material.decl")
            .map_err(|e| e.to_string())?
            .ok_or("native screen material is unavailable")?;
        let overrides = native_material_option_overrides(library, &material)?;
        let jitter =
            library.option_value(b"screenshadows", &overrides, b"screenshadows/useJitter")? != 0;
        let producer = library
            .definition(b"screenshadows")?
            .ok_or("native screen RPD is unavailable")?
            .native_render_state();
        let blur = library
            .definition(b"postfx_dwaoblur")?
            .ok_or("native DWAO blur RPD is unavailable")?
            .native_render_state();
        Ok((
            NativeScreenShadowParameters {
                producer,
                blur,
                step,
                jitter,
                cloud_motion,
                cloud_density,
                cloud_enabled: !cloud_name.eq_ignore_ascii_case("_black"),
            },
            cloud,
        ))
    })();
    match result {
        Ok((parameters, cloud)) => {
            commands.insert_resource(screen_shadow_pass::NativeScreenShadowFrame {
                output: images.add(screen_shadow_pass::visibility_image(1, 1)),
                cloud,
                current: None,
                produced: Default::default(),
            });
            commands.insert_resource(NativeScreenShadowSource(Ok(parameters)));
        }
        Err(error) => {
            eprintln!("Native screen source unavailable: {error}");
            commands.insert_resource(NativeScreenShadowSource(Err(error)));
        }
    }
}

pub(super) fn prepare_native_screen_shadow_frame(
    source: Res<NativeScreenShadowSource>,
    cpu: Res<NativeParallelShadowCpu>,
    frame: Res<shadow_pass::NativeShadowSceneFrame>,
    mut screen: Option<ResMut<screen_shadow_pass::NativeScreenShadowFrame>>,
    cameras: Query<(&Transform, &Projection, &Camera), With<Camera3d>>,
    mut images: ResMut<Assets<Image>>,
    mut last_error: Local<Option<String>>,
) {
    let Some(ref mut screen) = screen else {
        return;
    };
    screen.current = None;
    let result = (|| -> Result<screen_shadow_pass::NativeScreenShadowView, String> {
        let source = source.0.as_ref().map_err(Clone::clone)?;
        let shadow_pass::NativeShadowSceneState::Ready { .. } = &frame.state else {
            return Err("native directional atlas pending".into());
        };
        let cpu = cpu
            .current
            .as_ref()
            .ok_or("native current view is absent")?;
        let (transform, projection, camera) = cameras.single().map_err(|e| e.to_string())?;
        let Projection::Custom(custom) = projection else {
            return Err("native screen projection is unavailable".into());
        };
        let projection = custom
            .get::<camera_projection::NativeGameProjection>()
            .ok_or("native screen projection type is unavailable")?
            .rows();
        let mut inverse = [[0.0; 4]; 4];
        let mut inverse_projection = [[0.0; 4]; 4];
        if !render_shadow::invert_rows(
            &render_shadow::multiply_rows(projection, cpu.camera_view),
            &mut inverse,
        ) || !render_shadow::invert_rows(&projection, &mut inverse_projection)
        {
            return Err("native screen view is singular".into());
        }
        let extent = camera
            .physical_viewport_size()
            .ok_or("native screen extent is unavailable")?;
        let size = Extent3d {
            width: extent.x,
            height: extent.y,
            depth_or_array_layers: 1,
        };
        if images
            .get(&screen.output)
            .ok_or("native screen image is unavailable")?
            .texture_descriptor
            .size
            != size
        {
            images
                .insert(
                    screen.output.id(),
                    screen_shadow_pass::visibility_image(extent.x, extent.y),
                )
                .map_err(|e| e.to_string())?;
        }
        let position = view_to_game(transform.translation);
        let world_time = cpu.clocks.view_shader_times(1).world_time[0];
        let parameters = screen_shadow_pipeline::NativeScreenShadowUniform {
            inverse_view_projection: inverse.map(Vec4::from_array),
            unproject: Vec4::new(
                -inverse_projection[2][3],
                inverse_projection[3][2],
                inverse_projection[3][3],
                0.0,
            ),
            camera_time: Vec4::new(position[0], position[1], position[2], world_time),
            depth_scale_step_hack: Vec4::new(1.0, 1.0, source.step, 1.0),
            target: UVec4::new(extent.x, extent.y, 0, 0),
            blur: Vec4::ZERO,
            cloud_motion: source.cloud_motion,
            cloud_density: source.cloud_density,
            options: UVec4::new(
                u32::from(source.jitter),
                1,
                u32::from(source.cloud_enabled),
                0,
            ),
        };
        parameters.validate()?;
        Ok(screen_shadow_pass::NativeScreenShadowView {
            revision: frame.revision,
            parameters,
            producer_state: source.producer,
            blur_state: source.blur,
        })
    })();
    match result {
        Ok(value) => {
            screen.current = Some(value);
            *last_error = None;
        }
        Err(error) => {
            if last_error.as_ref() != Some(&error) {
                eprintln!("Native screen frame pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}

pub(super) fn initialize_native_parallel_receiver_source(
    mut commands: Commands,
    mut programs: ResMut<NativeProgramCacheSource>,
) {
    let result = (|| -> Result<NativeParallelReceiverCompile, String> {
        let library = programs.0.as_mut().map_err(|error| error.clone())?;
        let empty = render_program::SerializedParameterBlock {
            flags: 0,
            parameters: Vec::new(),
        };
        let mut option = |name: &[u8]| library.option_value(b"screenshadows", &empty, name);
        Ok(NativeParallelReceiverCompile {
            filtering_quality: option(b"shadowFilteringQuality")?,
            slope_scale: option(b"shadow/useSlopeScale")? != 0,
            slope_bias: option(b"shadow/useSlopeScaleBias")? != 0,
            slope_post_projection: option(b"shadow/slopeScaleIsPostProjection")? != 0,
        })
    })();
    commands.insert_resource(NativeParallelReceiverSource(result));
}

pub(super) fn publish_native_parallel_shadow_frame(
    owner: Option<Res<NativeParallelShadowOwner>>,
    cpu: Res<NativeParallelShadowCpu>,
    world: Res<NativeParallelWorldDraws>,
    models: Res<NativeParallelModelDraws>,
    receiver_source: Res<NativeParallelReceiverSource>,
    scene: Res<SceneData>,
    lights: Query<&DirectionalLight>,
    completion: Res<shadow_pass::NativeShadowSceneCompletion>,
    mut output: ResMut<shadow_pass::NativeShadowSceneFrame>,
    mut last_error: Local<Option<String>>,
) {
    let Some(owner) = owner else {
        return;
    };
    // Preserve the original independent whole-view stage until its real GPU
    // draw/copy completed. The receiver may not authorize an undefined layer.
    if !completion.whole_view_completed(&owner.target_image, owner.dimensions) {
        return;
    }
    let result=(||->Result<(shadow_pass::NativeParallelReceiverVars,Vec<shadow_pass::NativeShadowLayer>),String>{
        let frame=cpu.current.as_ref().ok_or("native CPU frame is unavailable")?;
        let source=receiver_source.0.as_ref().map_err(Clone::clone)?;
        let (world_frame,world)=world.current.as_ref().ok_or("native world submissions are unavailable")?;
        let (model_frame,models)=models.current.as_ref().ok_or("native model submissions are unavailable")?;
        if *world_frame!=frame.frame || *model_frame!=frame.frame || world.len()!=models.len() {
            return Err("native directional submissions belong to different view frames".into());
        }
        // One directional light has a proven GPU-array index of zero. A
        // multiple-sun view must bind the renderer's actual chosen index.
        if lights.iter().count()!=1 {return Err("native sun requires the actual directional GPU-array index in a multiple-light view".into());}
        let environment=scene.environment_lighting.ok_or("native receiver environment is unavailable")?;
        let direction=render_shadow::environment_sun_direction(
            environment.sun_theta_degrees.ok_or("native sun theta is unavailable")?,
            environment.sun_phi_degrees.ok_or("native sun phi is unavailable")?);
        let count=world.len();
        let mut matrices=cpu.prepared.cascades[..count].iter().map(|cascade|cascade.atlas).collect::<Vec<_>>();
        matrices.push(owner.whole_view.atlas);
        let mut fars=[f32::MAX;4];
        fars[..count].copy_from_slice(&cpu.prepared.fars[..count]);
        let receiver=shadow_pass::NativeParallelReceiverVars {
            // This publication owns the screenShadows ComputeParallelShadow
            // call. Its explicit zero-normal path must not enable the direct
            // forward hook before the screen producer finishes.
            path:render_shadow::ParallelShadowReceiverPath::ScreenSpace,
            world_to_shadow:matrices,view_row2:frame.camera_view[2],cascade_fars:fars,
            shadow_size:[owner.dimensions.width as f32,owner.dimensions.height as f32],
            light_quality:2, // HIGH_QUALITY in installed global.decl/screenshadows.
            filtering_quality:source.filtering_quality,directional_light_index:0,
            offset:shadow_sampling::NativeShadowOffsetParameters {
                sun_direction:direction,use_normal_offset:false,use_slope_scale:source.slope_scale,
                use_slope_bias:source.slope_bias,slope_post_projection:source.slope_post_projection,
                use_uv_normal_offset:false,normal_offset:owner.shader_bias[1],
                bias:owner.shader_bias[0],slope_bias:owner.shader_bias[2],
            },
        };
        shadow_sampling::NativeParallelSamplingUniform::from_owner_receiver(&receiver)?;
        if std::env::var_os("DH2_TRACE_SHADOW_GEOMETRY").is_some() && frame.frame%300==0 {
            eprintln!("Native directional geometry frame={} fars={fars:?} view={:?} sun={direction:?} matrices={:?} counts={:?}",
                frame.frame,frame.camera_view,receiver.world_to_shadow,
                frame.masks.iter().map(|mask|mask.iter().filter(|&&v|v!=0).count()).collect::<Vec<_>>());
        }
        let mut layers=world.clone();
        for (layer,models) in layers.iter_mut().zip(models) {layer.instances.extend(models.iter().cloned());}
        layers.push(shadow_pass::NativeShadowLayer {
            ops:shadow_pass::NativeShadowLayerOps {
                depth:shadow_pass::NativeShadowLoad::Load,stencil:shadow_pass::NativeStencilLoad::Load,
            },cache:shadow_pass::NativeShadowCacheOp::None,instances:Vec::new(),
        });
        Ok((receiver,layers))
    })();
    match result {
        Ok((receiver, layers)) => {
            let Some(revision) = output.revision.checked_add(1) else {
                output.state = shadow_pass::NativeShadowSceneState::Unavailable(
                    "native frame revision overflow".into(),
                );
                return;
            };
            if last_error.take().is_some()
                || !matches!(
                    output.state,
                    shadow_pass::NativeShadowSceneState::Ready { .. }
                )
            {
                println!(
                    "Native directional atlas frame online: {} layers, {} current draws",
                    layers.len(),
                    layers
                        .iter()
                        .map(|layer| layer.instances.len())
                        .sum::<usize>()
                );
            }
            output.revision = revision;
            output.state = shadow_pass::NativeShadowSceneState::Ready {
                target_image: owner.target_image.clone(),
                dimensions: owner.dimensions,
                receiver,
                layers,
            };
        }
        Err(error) => {
            if last_error.as_ref() != Some(&error) {
                eprintln!("Native directional atlas frame pending: {error}");
                *last_error = Some(error.clone());
            }
            output.state = shadow_pass::NativeShadowSceneState::Unavailable(error);
        }
    }
}

pub(super) fn prepare_native_model_shadow_instances(
    records: &[(usize, usize)],
    clip: render_shadow::ShadowMatrix,
    hardware_bias: [f32; 3],
    margin: f32,
    dimensions: [u32; 2],
    clocks: &game_clock::GameplayViewClocks,
    sources: &NativeModelShadowSources,
    placements: &RenderSourcePlacements,
    materials: &NativeShadowMaterials,
    images: &Assets<Image>,
    vegetation: &NativeVegetationMeshInputs,
    queue: &mut SceneRenderQueue,
) -> Result<Vec<shadow_pass::NativeShadowInstance>, String> {
    let mut instances = Vec::with_capacity(records.len());
    for &(source_id, index) in records {
        let source = &sources.0[source_id];
        let instance = &queue.instances[index];
        let provenance = instance
            .source
            .ok_or("native model draw lost source provenance")?;
        let inputs = materials
            .0
            .get(&instance.material)
            .ok_or("native model material owner is absent")?
            .as_ref()
            .map_err(Clone::clone)?;
        // CPU declarations9/10 contain skin lanes even when the
        // selected material program does not skin. Bytecode class,
        // not presence of those lanes, owns deformation admission.
        let program = inputs
            .rigid_base_source
            .as_ref()
            .map_err(|error| {
                format!(
                    "native model placement {} ({}): {error}",
                    source.placement,
                    placements
                        .0
                        .get(source.placement)
                        .map(|p| p.resource_path.as_str())
                        .unwrap_or("missing source")
                )
            })?
            .as_ref()
            .ok_or("native admitted model has no resolved default slot10")?;
        let parameters = inputs
            .parameters
            .as_ref()
            .map_err(Clone::clone)?
            .as_ref()
            .ok_or("native model parameter block is absent")?;
        let multiplier = parameters.vector(b"modeldepthhack", None)?[0];
        if !multiplier.is_finite() {
            return Err("native model depth multiplier is non-finite".into());
        }
        let placement = placements
            .0
            .get(source.placement)
            .ok_or("native model placement owner is absent")?;
        let affine =
            render_model::affine_rows(placement.position, placement.rotation, placement.scale);
        let model = [affine[0], affine[1], affine[2], [0.0, 0.0, 0.0, 1.0]];
        let mvp = render_shadow::multiply_rows(clip, model);
        let state = render_model::selected_shadow_model_draw_state(
            inputs.source.shadow_draw_flags49(),
            inputs
                .source
                .main_render_state
                .ok_or("native model MAIN draw state is absent")?,
            hardware_bias,
            margin,
            render_shadow::model_mvp_winding_flipped(&mvp),
        );
        let flipped = if provenance.world_baked {
            placed_direction(placement, Vec3::X)
                .cross(placed_direction(placement, Vec3::Y))
                .dot(placed_direction(placement, Vec3::Z))
                < 0.0
        } else {
            instance.transform.to_matrix().determinant() < 0.0
        };
        let (front_face, cull_mode) = shadow_pipeline::native_shadow_mesh_raster(&state, !flipped);
        let class = program.class;
        let alpha = if class != shader_cache::CachedRigidShadowClass::Opaque {
            let slot = inputs
                .diffuse_image_slot
                .ok_or("native model cutout has no original diffuse image")?;
            if !queue.native_shadow_diffuse_slots.contains(&slot) {
                queue.native_shadow_diffuse_slots.push(slot);
            }
            let diffuse = queue
                .diffuse_image_handles
                .get(slot)
                .and_then(Option::as_ref)
                .ok_or_else(|| {
                    format!("native model shadow diffuse slot {slot} awaits texture streaming")
                })?;
            if images.get(diffuse).is_none() {
                return Err("native model diffuse asset is not resident".into());
            }
            Some(shadow_pass::NativeShadowAlphaMaterial {
                diffuse: diffuse.clone(),
                uv_set: if class == shader_cache::CachedRigidShadowClass::VistaAlpha {
                    0
                } else {
                    inputs
                        .world_uv_set
                        .ok_or("native model Layer0 UV selector is absent")?
                },
                uv_scale_bias: if class == shader_cache::CachedRigidShadowClass::VistaAlpha {
                    [1.0, 1.0, 0.0, 0.0]
                } else {
                    parameters.vector(b"layer0_scalebias", None)?
                },
            })
        } else {
            None
        };
        let (program, constants) = if program.skin_influences != 0 {
            let RenderModelSurface::Bmd6 {
                matrix_offset,
                skin_influences,
                ..
            } = provenance.surface
            else {
                return Err("native skin shader has no MD6 vertex source".into());
            };
            if skin_influences != program.skin_influences {
                return Err("native skin stream and selected shader weight counts disagree".into());
            }
            let skinning = source
                .skinning
                .as_ref()
                .ok_or("native skin owner has no palette source")?
                .as_ref()
                .map_err(Clone::clone)?;
            (
                shadow_pass::NativeShadowProgram::Skinned {
                    influences: program.skin_influences,
                    alpha: alpha.is_some(),
                    dither: program.alpha_dither,
                },
                shadow_pass::NativeShadowInstanceConstants::Skinned {
                    constants: shadow_pass::native_skin_shadow_instance_cb(
                        &mvp,
                        matrix_offset,
                        multiplier,
                        alpha.as_ref().map_or([0.0; 4], |alpha| alpha.uv_scale_bias),
                    ),
                    palette: skinning.palette.clone(),
                },
            )
        } else {
            match class {
                shader_cache::CachedRigidShadowClass::Opaque => (
                    shadow_pass::NativeShadowProgram::RigidOpaque,
                    shadow_pass::NativeShadowInstanceConstants::Rigid(
                        shadow_pass::native_shadow_instance_cb(&mvp, multiplier),
                    ),
                ),
                shader_cache::CachedRigidShadowClass::AlphaTested => (
                    shadow_pass::NativeShadowProgram::RigidAlphaTested,
                    shadow_pass::NativeShadowInstanceConstants::Rigid(
                        shadow_pass::native_shadow_instance_cb(&mvp, multiplier),
                    ),
                ),
                shader_cache::CachedRigidShadowClass::VistaAlpha => (
                    shadow_pass::NativeShadowProgram::RigidVistaAlpha,
                    shadow_pass::NativeShadowInstanceConstants::Rigid(
                        shadow_pass::native_shadow_instance_cb(&mvp, multiplier),
                    ),
                ),
                shader_cache::CachedRigidShadowClass::Vegetation => {
                    vegetation
                        .0
                        .get(&instance.mesh.id())
                        .ok_or("native model wind channels are absent")?
                        .as_ref()
                        .map_err(Clone::clone)?;
                    // Initial model constructor selects group0, with
                    // zero model-time offset. Later setter events must
                    // update the actual owner before changing this path.
                    let time = clocks
                        .model_local_time(0.0, 4)
                        .ok_or("native initial model clock is unavailable")?;
                    (
                        shadow_pass::NativeShadowProgram::Vegetation,
                        shadow_pass::NativeShadowInstanceConstants::Vegetation(
                            shadow_pass::native_vegetation_shadow_instance_cb(
                                alpha.as_ref().unwrap().uv_scale_bias,
                                &parameters.vegetation(None)?,
                                &mvp,
                                time,
                                multiplier,
                            ),
                        ),
                    )
                }
            }
        };
        instances.push(shadow_pass::NativeShadowInstance {
            mesh: instance.mesh.clone(),
            per_instance_cb: constants,
            program,
            alpha_material: alpha,
            front_face,
            cull_mode,
            draw_state: state,
            stencil_reference: 0,
            scissor: shadow_pipeline::NativeShadowScissor {
                x: 0,
                y: 0,
                width: dimensions[0],
                height: dimensions[1],
            },
        });
    }
    Ok(instances)
}

pub(super) fn prepare_native_parallel_model_draws(
    cpu: Res<NativeParallelShadowCpu>,
    owner: Option<Res<NativeParallelShadowOwner>>,
    sources: Res<NativeModelShadowSources>,
    placements: Res<RenderSourcePlacements>,
    materials: Res<NativeShadowMaterials>,
    images: Res<Assets<Image>>,
    vegetation: Res<NativeVegetationMeshInputs>,
    mut queue: ResMut<SceneRenderQueue>,
    mut output: ResMut<NativeParallelModelDraws>,
    profile: Option<Res<frame_profile::CpuFrameProfile>>,
) {
    let _span = profile.as_deref().map(|p| p.measure(23));
    output.current = None;
    let (Some(owner), Some(frame)) = (owner, cpu.current.as_ref()) else {
        return;
    };
    let result = (|| -> Result<Vec<Vec<shadow_pass::NativeShadowInstance>>, String> {
        let casters = frame
            .models
            .as_ref()
            .ok_or("native model collector has no matching frame")?;
        let count = frame.masks.len();
        if frame.initialized[..count].iter().any(|&ready| !ready) {
            return Err("native model draws await cascade initialization".into());
        }
        let partial = owner.cascade_controls.partial_dynamic_update;
        let queue = &mut *queue;
        let preceding = if count >= 2 {
            cpu.prepared.fars[count - 2]
        } else {
            0.0
        };
        let authored = if count >= 2 {
            cpu.prepared.far_state.authored_fars[count - 2]
        } else {
            0.0
        };
        // Native count1 compares BSS zero to a positive subnormal pointer
        // word, so its limited-far predicate is false. For the cutoff, zero
        // is equivalent unless a sort distance is itself positive subnormal.
        if partial
            && count == 1
            && casters.iter().any(|caster| {
                caster.sort_distance > 0.0 && caster.sort_distance < f32::MIN_POSITIVE
            })
        {
            return Err(
                "single-cascade model cutoff is outside the verified normal-distance domain".into(),
            );
        }
        let limited = count > 1 && partial && preceding >= authored;
        let maximum = if partial { authored } else { f32::MAX };
        let mut layers = Vec::with_capacity(count);
        for layer in 0..count {
            let mut instances = Vec::new();
            if frame.slice_plans[layer].skip
                || (layer == count - 1 && limited && queue.static_world_model_present)
            {
                layers.push(instances);
                continue;
            }
            let cascade = &cpu.prepared.cascades[layer];
            let margin = cascade.depth_offset;
            if !margin.is_finite() {
                return Err("native directional model clip margin is non-finite".into());
            }
            let clip: [f32; 16] = std::array::from_fn(|i| cascade.clip[i / 4][i % 4]);
            // FUN_140328260 runs separately for both model buckets. Both
            // follow static WORLD cache restoration in the preserve pass;
            // neither model bucket may be saved into the world static cache.
            for dynamic in [false, true] {
                let mut records = Vec::new();
                let mut keys = Vec::new();
                for caster in casters
                    .iter()
                    .filter(|caster| caster.dynamic == dynamic && caster.sort_distance <= maximum)
                {
                    let source = &sources.0[caster.source];
                    let culled = if margin < 0.0 {
                        render_model::native_transformed_aabb_clip_cull_at_depth(
                            &clip,
                            source.bounds[0],
                            source.bounds[1],
                            margin,
                        )
                    } else {
                        render_model::native_transformed_aabb_clip_cull(
                            &clip,
                            source.bounds[0],
                            source.bounds[1],
                        )
                    };
                    if culled.culled {
                        continue;
                    }
                    for &index in &caster.instances {
                        records.push((caster.source, index));
                        // Parallel collection passes a null light: the native
                        // fallback DAT_1431af658 is the measured zero vector.
                        keys.push(render_model::model_shadow_radial_sort_key(
                            source.bounds,
                            [0.0; 3],
                        ));
                    }
                }
                render_model::sort_model_shadow_records(&mut records, &mut keys)?;
                instances.extend(prepare_native_model_shadow_instances(
                    &records,
                    cascade.clip,
                    owner.hardware_bias,
                    margin,
                    [owner.dimensions.width, owner.dimensions.height],
                    &frame.clocks,
                    &sources,
                    &placements,
                    &materials,
                    &images,
                    &vegetation,
                    queue,
                )?);
            }
            layers.push(instances);
        }
        Ok(layers)
    })();
    match result {
        Ok(layers) => {
            if output.last_error.take().is_some() || !output.reported {
                println!(
                    "Native model shadow submissions online: {} draws",
                    layers.iter().map(Vec::len).sum::<usize>()
                );
                output.reported = true;
            }
            output.current = Some((frame.frame, layers));
        }
        Err(error) => {
            if output.last_error.as_ref() != Some(&error) {
                eprintln!("Native model shadow submissions pending: {error}");
                output.last_error = Some(error);
            }
        }
    }
}

pub(super) fn prepare_native_parallel_world_draws(
    cpu: Res<NativeParallelShadowCpu>,
    owner: Option<Res<NativeParallelShadowOwner>>,
    scene: Res<SceneData>,
    sources: Res<NativeWorldShadowMeshSources>,
    materials: Res<NativeShadowMaterials>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    mut queue: ResMut<SceneRenderQueue>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut draws: ResMut<NativeParallelWorldDraws>,
    profile: Option<Res<frame_profile::CpuFrameProfile>>,
) {
    let _span = profile.as_deref().map(|p| p.measure(22));
    draws.current = None;
    let (Some(owner), Some(frame)) = (owner, cpu.current.as_ref()) else {
        return;
    };
    let result = (|| -> Result<Vec<shadow_pass::NativeShadowLayer>, String> {
        let count = frame.masks.len();
        if frame.initialized[..count].iter().any(|&ready| !ready) {
            return Err("native world draws await initialized cascade projections".into());
        }
        if draws.static_layers.len() != count {
            draws.static_layers = (0..count).map(|_| None).collect();
        }
        let mut plans = Vec::with_capacity(count);
        for layer in 0..count {
            let plan = frame.slice_plans[layer];
            if plan.skip {
                plans.push(None);
                continue;
            }
            let cached = plan.write_static_cache || plan.reuse_static_cache;
            let (mask, clip, time, generation) = if cached {
                let retained = frame.static_frames[layer].as_ref().ok_or_else(|| {
                    format!("native layer {layer} has no retained static source frame")
                })?;
                (
                    &retained.mask,
                    &retained.clip,
                    retained.local_time,
                    Some(retained.frame),
                )
            } else {
                (
                    &frame.masks[layer],
                    &cpu.prepared.cascades[layer].clip,
                    frame.local_time,
                    None,
                )
            };
            if let Some(generation) = generation {
                if draws.static_layers[layer]
                    .as_ref()
                    .is_some_and(|(old, _)| *old == generation)
                {
                    plans.push(Some((generation, None)));
                    continue;
                }
            }
            let prepared = prepare_native_world_shadow_masks(
                &owner,
                &scene.world,
                &sources,
                &materials,
                &meshes,
                &images,
                &mut queue,
                std::slice::from_ref(mask),
                clip,
                Some(time),
                None,
            )?;
            plans.push(Some((generation.unwrap_or(frame.frame), Some(prepared))));
        }
        // Resolve every selected program/image first; failed preparation owns
        // no transient GPU buffers. The immutable BWM matrix bank is shared.
        if draws.matrices.is_none() {
            draws.matrices = Some(buffers.add(ShaderBuffer::from(scene.world.matrices.clone())));
        }
        let mut layers = Vec::with_capacity(count);
        for (layer, prepared) in plans.into_iter().enumerate() {
            let Some((generation, prepared)) = prepared else {
                layers.push(shadow_pass::NativeShadowLayer {
                    ops: shadow_pass::NativeShadowLayerOps {
                        depth: shadow_pass::NativeShadowLoad::Load,
                        stencil: shadow_pass::NativeStencilLoad::Load,
                    },
                    cache: shadow_pass::NativeShadowCacheOp::None,
                    instances: Vec::new(),
                });
                continue;
            };
            let cached = frame.slice_plans[layer].write_static_cache
                || frame.slice_plans[layer].reuse_static_cache;
            let mut built = match prepared {
                Some(prepared) => build_native_world_layer(
                    prepared,
                    draws.matrices.as_ref(),
                    owner.dimensions,
                    &mut buffers,
                ),
                None => draws.static_layers[layer].as_ref().unwrap().1.clone(),
            };
            if cached {
                built.cache = shadow_pass::NativeShadowCacheOp::Retained {
                    generation: u64::from(generation),
                    static_instances: built.instances.len(),
                };
                draws.static_layers[layer] = Some((generation, built.clone()));
            }
            layers.push(built);
        }
        Ok(layers)
    })();
    match result {
        Ok(layers) => {
            if draws.last_error.take().is_some() {
                println!(
                    "Native parallel BWM submissions online: {} cascades, {} draws",
                    layers.len(),
                    layers
                        .iter()
                        .map(|layer| layer.instances.len())
                        .sum::<usize>()
                );
            }
            draws.current = Some((frame.frame, layers));
        }
        Err(error) => {
            if draws.last_error.as_ref() != Some(&error) {
                eprintln!("Native parallel BWM submissions pending: {error}");
                draws.last_error = Some(error);
            }
        }
    }
}

pub(super) fn prepare_native_parallel_shadow_cpu(
    owner: Option<Res<NativeParallelShadowOwner>>,
    scene: Res<SceneData>,
    queue: Res<SceneRenderQueue>,
    world: Option<Res<TomeWorld>>,
    caster_world: Option<Res<NativeShadowCasterWorld>>,
    model_bounds: Res<NativeShadowModelBoundsSources>,
    pickups: Res<PickupState>,
    cameras: Query<(&Camera, &Transform, &Projection), With<Camera3d>>,
    coarse: Res<coarse_depth::NativeCoarseDepthMailbox>,
    frame_count: Res<bevy::diagnostic::FrameCount>,
    settings: Res<NativeViewSettings>,
    clocks: Res<game_clock_runtime::NativeGameplayViewClock>,
    model_frame: Res<NativeModelCasterFrame>,
    mut cpu: ResMut<NativeParallelShadowCpu>,
    mut receiver_dumped: Local<bool>,
    profile: Option<Res<frame_profile::CpuFrameProfile>>,
) {
    let _span = profile.as_deref().map(|p| p.measure(21));
    cpu.current = None;
    let Some(owner) = owner else {
        return;
    };
    let result = (|| -> Result<NativeParallelCasterFrame, String> {
        let world = world
            .as_deref()
            .ok_or("native caster gather has no Tome owner")?;
        let caster_world = caster_world
            .as_deref()
            .ok_or("native caster Tome cache is unavailable")?;
        let (camera, transform, projection) = cameras.single().map_err(|e| e.to_string())?;
        let host_projection = projection;
        let Projection::Custom(custom) = projection else {
            return Err(
                "native cascade fitting requires the original finite camera projection".into(),
            );
        };
        let projection = custom
            .get::<camera_projection::NativeGameProjection>()
            .ok_or("native gameplay projection type is unavailable")?;
        let camera_projection = projection.rows();
        let mut inverse_projection = [[0.0; 4]; 4];
        if !render_shadow::invert_rows(&camera_projection, &mut inverse_projection) {
            return Err("native gameplay projection is singular".into());
        }
        let pose = game_pose_from_view_transform(transform);
        let authored_shadow_near = cpu.shadow_extent.update(
            scene.environment_render_parameters.as_ref().ok_or("shadow environment is unavailable")?,
            scene.environment_blend_duration,
            &scene.area_environments,
            view_to_game(transform.translation),
            "env/shadow/parallelLightExtend",
            clocks.view_shader_times(owner.source_flag_04)?.world_time[1],
        )?;
        let shadow_near = authored_shadow_near
            .max(scene.rewrite_shadow_settings.minimum_sun_caster_extent);
        let camera_view = tome::render_query_base_rows(Mat4::IDENTITY.to_cols_array_2d(), pose);
        let count = owner
            .dimensions
            .layers
            .checked_sub(1)
            .filter(|count| (1..=4).contains(count))
            .ok_or("native atlas has no cascade layers")? as usize;
        let controls = owner.cascade_controls;
        let effective_far = if controls.tighten_parallel_shadows {
            let snapshot = coarse
                .latest()?
                .ok_or("native adaptive GPU depth has not mapped yet")?;
            let viewport = camera
                .physical_viewport_size()
                .ok_or("native camera has no viewport")?;
            if snapshot.metadata.source_extent != [viewport.x, viewport.y] {
                return Err("native adaptive GPU depth awaits the resized camera target".into());
            }
            // For count1 the preceding relocated pointer's high word is a
            // positive subnormal. Its projected threshold exceeds every
            // finite D24 sample in [0,1]; zero projects to +infinity and
            // selects exactly the same points. Native probe cases cover both
            // Win64 address-word extremes. Reject inputs outside that domain.
            let previous_reference = if count == 1 {
                if snapshot
                    .pixels
                    .iter()
                    .any(|pixel| !(0.0..=1.0).contains(&pixel[1]))
                {
                    return Err(
                        "single-cascade adaptive depth is outside the verified D24 domain".into(),
                    );
                }
                0.0
            } else {
                cpu.prepared.far_state.authored_fars[count - 2]
            };
            let depths = Box::new(std::array::from_fn(|i| snapshot.pixels[i][1]));
            render_shadow::adaptive_directional_far(
                true,
                &camera_projection,
                &inverse_projection,
                [
                    previous_reference,
                    cpu.prepared.far_state.authored_fars[count - 1],
                ],
                Some(&depths),
            )?
        } else {
            -1.0
        };
        if std::env::var_os("DH2_TRACE_SHADOW_GEOMETRY").is_some() && frame_count.0 % 300 == 0 {
            eprintln!(
                "Native adaptive candidate frame={} far={effective_far}",
                frame_count.0
            );
        }
        let reset_frame = *cpu.reset_frame.get_or_insert(frame_count.0);
        let far_input = render_shadow::CascadeFarInput {
            parsed_authored_fars: Some(owner.cascade_fars),
            camera_near: projection.near,
            camera_far: projection.far,
            view_translation: [camera_view[0][3], camera_view[1][3], camera_view[2][3]],
            view_flag_3a69: render_shadow::prepared_shadow_view_flag(
                render_shadow::RESET_VIEW_SHADOW_FLAG,
                frame_count.0,
                reset_frame,
            ),
            slice_nears: cpu.prepared.fars,
            count,
            effective_far,
            adaptive: controls.partial_dynamic_update,
            partial_slice_updates: controls.partial_slice_update,
            partial_pass: queue.static_world_model_present,
            frame: frame_count.0,
        };
        let environment = scene
            .environment_lighting
            .ok_or("native shadow sun is unavailable")?;
        let toward_sun = render_shadow::environment_sun_direction(
            environment
                .sun_theta_degrees
                .ok_or("native sun theta is unavailable")?,
            environment
                .sun_phi_degrees
                .ok_or("native sun phi is unavailable")?,
        );
        let previous = cpu.prepared.fars;
        let refreshed = cpu.prepared.prepare(
            &far_input,
            &render_shadow::DirectionalProjectionInput {
                camera_projection,
                camera_view,
                renderer_sun_direction: toward_sun,
                camera_near: projection.near,
                shadow_near,
                fars: previous,
                authored_fars: owner.cascade_fars,
                count,
                resolution: owner.dimensions.width as i32,
                static_world_model_present: queue.static_world_model_present,
                alternating: controls.partial_slice_update,
                frame_counter: frame_count.0 as i32,
                force_first: controls.tighten_parallel_shadows,
            },
        )?;
        for (initialized, refreshed) in cpu.initialized.iter_mut().zip(refreshed) {
            *initialized |= refreshed;
        }
        let local_time = clocks.view_shader_times(owner.source_flag_04)?.local_time;
        let model_clocks = *clocks.clocks().map_err(str::to_owned)?;
        let models = model_frame
            .current
            .as_ref()
            .filter(|(frame, _)| *frame == frame_count.0)
            .map(|(_, models)| models.clone());
        let ordinary = queue
            .shadow_ordinary_query
            .as_ref()
            .and_then(|output| output.state.as_ref())
            .ok_or("native receiver has no valid merged ordinary query")?;
        let depth = ordinary
            .depth_grid
            .as_deref()
            .ok_or("native ordinary query has no depth grid")?;
        let viewport = camera
            .physical_viewport_size()
            .ok_or("native camera has no viewport")?;
        let scale = native_view_distance_scale(host_projection, viewport, Some(&settings))?;
        let distance = (scale != 1.0).then_some(NativeShadowDistanceDescriptor {
            center: [0.0; 3],
            radius: scale,
            override_center: false,
        });
        let dynamic_bounds = model_bounds.active_bounds(&queue, &pickups)?;
        let ray = toward_sun.map(|value| f32::from_bits(value.to_bits() ^ 0x8000_0000));
        let mut depth_projection = NativeReceiverDepthProjection {
            receiver_rows: [[0.0; 4]; 4],
            transposed_rows: [[0.0; 4]; 4],
            mask: Box::new([0; 128 * 128]),
        };
        let receiver = shadow_receiver::initialize_native_receiver(
            shadow_receiver::NativeReceiverInput {
                flags: 0,
                camera_clip_rows: ordinary.render_rows,
                camera_position: ordinary.camera_point,
                ray_direction: ray,
                maximum_depth: Some(ordinary.maximum_depth),
                maximum_distance: cpu.prepared.receiver_maximum_distance(count)?,
                world_bounds: tome::global_bounds(&world.bytes)?,
                dynamic_bounds: &dynamic_bounds,
                ordinary_depth: depth,
            },
            &mut depth_projection,
        )?;
        // Optional numerical replay evidence; no framebuffer or image capture.
        if !*receiver_dumped && frame_count.0 >= 300 {
            if let Some(path) = std::env::var_os("DH2_SHADOW_RECEIVER_DUMP") {
                let mut bytes = b"DHR2".to_vec();
                bytes.extend_from_slice(&(dynamic_bounds.len() as u32).to_le_bytes());
                for value in ordinary.render_rows.iter().flatten()
                    .chain(ordinary.camera_point.iter()).chain(ray.iter())
                    .chain([cpu.prepared.receiver_maximum_distance(count)?, ordinary.maximum_depth].iter())
                    .chain(tome::global_bounds(&world.bytes)?.iter().flatten())
                    .chain(dynamic_bounds.iter().flatten().flatten())
                    .chain(depth.iter())
                    .chain([receiver.distance, receiver.cutoff].iter())
                    .chain(receiver.bounds.iter())
                    .chain(depth_projection.receiver_rows.iter().flatten()) {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                for value in depth_projection.mask.iter() {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                std::fs::write(path, bytes).map_err(|error| error.to_string())?;
                *receiver_dumped = true;
            }
        }
        if cpu.masks.is_none() {
            cpu.masks = Some(NativeShadowInstanceMasks::new(
                scene.world.visibility.len(),
                count,
            ));
            cpu.static_frames = (0..count).map(|_| None).collect();
        }
        cpu.masks.as_mut().unwrap().clear();
        if receiver.status == shadow_receiver::NativeReceiverStatus::Initialized {
            // An uninitialized, skipped slice cannot contribute a submission.
            // Prune only those independent tests, then restore physical bits.
            let active: Vec<usize> = (0..count).filter(|&index| cpu.initialized[index]).collect();
            if active.is_empty() {
                return Err("native cascade initialization is pending".into());
            }
            let cascades = active
                .iter()
                .map(|&index| {
                    NativeShadowFrustumQuery::from_projection_rows(
                        cpu.prepared.cascades[index].caster_clip,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            if cpu.gather_count != active.len() {
                cpu.gather = Some(NativeGlobalShadowCasterScratch::new(
                    caster_world.0.object_count(),
                    active.len(),
                ));
                cpu.gather_count = active.len();
            }
            let cascades = cascades
                .iter()
                .map(NativeShadowFrustumQuery::as_receiver)
                .collect::<Vec<_>>();
            let planes: Vec<usize> = (0..receiver.plane_count).collect();
            let primary = VerifiedShadowReceiverState::from_native_planes(
                &receiver.planes[..receiver.plane_count],
                &planes,
            );
            let direction = NativeCasterDirection::from_receiver_direction(ray)?;
            for job in 0..8 {
                let mut hits = cpu
                    .gather
                    .as_mut()
                    .unwrap()
                    .gather(
                        &caster_world.0,
                        &NativeGlobalShadowCasterInput {
                            source: &world.bytes,
                            primary,
                            cascades: &cascades,
                            distance: NativeShadowObjectDistanceQuery::from_descriptor(
                                ordinary.camera_point,
                                distance,
                            ),
                            direction,
                            depth: Some(&depth_projection),
                            job,
                            jobs: 8,
                        },
                    )?
                    .to_vec();
                for hit in &mut hits {
                    hit.cascade_mask =
                        active
                            .iter()
                            .enumerate()
                            .fold(0, |mask, (index, &physical)| {
                                mask | if hit.cascade_mask & (1u32 << index) != 0 {
                                    1u32 << physical
                                } else {
                                    0
                                }
                            });
                }
                cpu.masks
                    .as_mut()
                    .unwrap()
                    .publish(&hits, &scene.world.tome_object_instances)?;
            }
            cpu.masks.as_mut().unwrap().suppress_parents(
                scene
                    .world
                    .visibility_groups
                    .iter()
                    .map(|group| (group.parent, group.children.as_slice())),
            )?;
        }
        if cpu.world_query.is_none() {
            cpu.world_query = Some(crate::local_caster_query::NativeLocalWorldQuery::new(&scene.world));
        }
        for layer in 0..count {
            if cpu.initialized[layer] {
                let cascade = &cpu.prepared.cascades[layer];
                let broad = cpu.world_query.as_ref().unwrap()
                    .query_projection(&cascade.clip, cascade.depth_offset);
                cpu.masks.as_mut().unwrap().intersect_world_query(layer, &broad)?;
            }
        }
        let slice_plans: Vec<_> = (0..count)
            .map(|layer| {
                render_shadow::parallel_shadow_slice_plan(
                    &render_shadow::ParallelShadowSliceInput {
                        layer,
                        count,
                        camera_near: projection.near,
                        fars: cpu.prepared.fars,
                        // The one-cascade planner never consumes a count-2 prefix:
                        // layer0 always updates and has no static-cache entry.
                        previous_penultimate: if count >= 2 {
                            cpu.prepared.fars[count - 2]
                        } else {
                            0.0
                        },
                        authored_penultimate: if count >= 2 {
                            cpu.prepared.far_state.authored_fars[count - 2]
                        } else {
                            0.0
                        },
                        partial_dynamic: controls.partial_dynamic_update,
                        partial_slices: controls.partial_slice_update,
                        static_world_model_present: queue.static_world_model_present,
                        frame: frame_count.0,
                        preserve_depth_pass: false,
                    },
                )
            })
            .collect();
        for (layer, plan) in slice_plans.iter().enumerate() {
            if plan.write_static_cache && cpu.initialized[layer] {
                cpu.static_frames[layer] = Some(Arc::new(NativeStaticShadowFrame {
                    frame: frame_count.0,
                    mask: cpu.masks.as_ref().unwrap().masks()[layer].clone(),
                    clip: cpu.prepared.cascades[layer].clip,
                    local_time,
                }));
            }
        }
        Ok(NativeParallelCasterFrame {
            frame: frame_count.0,
            masks: cpu.masks.as_ref().unwrap().masks().to_vec(),
            receiver,
            slice_plans,
            camera_view,
            initialized: cpu.initialized,
            static_frames: cpu.static_frames.clone(),
            local_time,
            models,
            clocks: model_clocks,
        })
    })();
    match result {
        Ok(frame) => {
            if cpu.last_error.take().is_some() {
                println!(
                    "Native parallel CPU frame online: {} cascades, {} world caster selections",
                    frame.masks.len(),
                    frame
                        .masks
                        .iter()
                        .flatten()
                        .filter(|&&mask| mask != 0)
                        .count()
                );
            }
            cpu.current = Some(frame);
        }
        Err(error) => {
            if cpu.last_error.as_ref() != Some(&error) {
                eprintln!("Native parallel CPU frame pending: {error}");
                cpu.last_error = Some(error);
            }
        }
    }
}
