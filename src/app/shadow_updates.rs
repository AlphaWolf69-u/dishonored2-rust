//! Game-thread ownership of the native parallel prepare/receiver/gather sequence.
//! Rendering consumes these masks, never the color-visible instance list.
use super::{
    camera_projection, coarse_depth, frame_profile, game_clock, game_clock_runtime,
    game_pose_from_view_transform, material, material_option_overrides,
    view_distance_scale, placed_direction, read_streamed_diffuse_at_target, render_model,
    render_program, render_shadow, screen_shadow_pass, screen_shadow_pipeline, shader_cache,
    shadow_pass, shadow_pipeline, shadow_receiver, shadow_sampling, tome, view_to_game, Arc,
    Extent3d, ParallelShadowOwner, ProgramCacheSource, ShadowCasterWorld,
    ShadowMaterials, ShadowModelBoundsSources, VegetationMeshInputs,
    ViewSettings, WorldShadowMeshSources, PickupState, Projection, RenderModelSurface,
    RenderSourcePlacements, SceneData, SceneRenderQueue, ShaderBuffer, TextureDimension,
    TextureFormat, TextureReadTarget, TomeWorld,
};
use bevy::prelude::*;
mod query_cache;

use tome::shadow_caster::{
    CasterDirection, GlobalShadowCasterInput, GlobalShadowCasterScratch,
    ReceiverDepthProjection, ShadowDistanceDescriptor, ShadowFrustumQuery,
    ShadowInstanceMasks, ShadowObjectDistanceQuery, ShadowReceiverState,
};

#[derive(Resource, Default)]
pub(super) struct ModelShadowSources(pub(super) Vec<ModelShadowSource>);

pub(super) struct ModelShadowSource {
    pub placement: usize,
    /// Resource-space bounds retained for physics pose publication.
    pub local_bounds: [[f32; 3]; 2],
    pub bounds: [[f32; 3]; 2],
    pub skinned: bool,
    pub surfaces: Vec<(usize, i32, Result<u8, String>)>,
    pub instances: std::ops::Range<usize>,
    pub pickup_id: Option<String>,
    pub flags_1c30: u8,
    pub flags_18bc: u8,
    pub flags_18bd: u8,
    pub skinning: Option<Result<Arc<RestSkinning>, String>>,
}

pub(super) struct RestSkinning {
    pub palette: Handle<ShaderBuffer>,
    pub matrices: Vec<[[f32; 4]; 3]>,
}

#[derive(Clone)]
pub(super) struct ModelShadowCaster {
    pub source: usize,
    pub instances: Vec<usize>,
    pub flags: u8,
    pub sort_distance: f32,
    pub camera_clip_mask: u8,
    pub dynamic: bool,
}

#[derive(Resource, Default)]
pub(super) struct ModelCasterFrame {
    pub current: Option<(u32, Arc<Vec<ModelShadowCaster>>)>,
    pub error: Option<String>,
    reported: bool,
}

pub(super) fn collect_model_casters(
    sources: Res<ModelShadowSources>,
    scene: Res<SceneData>,
    queue: Res<SceneRenderQueue>,
    pickups: Res<PickupState>,
    cameras: Query<(&Transform, &Projection), With<Camera3d>>,
    frame_count: Res<bevy::diagnostic::FrameCount>,
    mut output: ResMut<ModelCasterFrame>,
    profile: Option<Res<frame_profile::CpuFrameProfile>>,
) {
    let _span = profile.as_deref().map(|p| p.measure(26));
    output.current = None;
    let result = (|| -> Result<Vec<ModelShadowCaster>, String> {
        let (transform, projection) = cameras.single().map_err(|error| error.to_string())?;
        let camera_view = tome::render_query_base_rows(
            Mat4::IDENTITY.to_cols_array_2d(),
            game_pose_from_view_transform(transform),
        );
        let mut inverse_view = [[0.0; 4]; 4];
        if !render_shadow::invert_rows(&camera_view, &mut inverse_view) {
            return Err("caster camera view is singular".into());
        }
        let Projection::Custom(custom) = projection else {
            return Err("model caster camera is unavailable".into());
        };
        let projection = custom
            .get::<camera_projection::GameProjection>()
            .ok_or("model caster projection type is unavailable")?;
        let clip = render_shadow::multiply_rows(projection.rows(), camera_view);
        let clip: [f32; 16] = std::array::from_fn(|i| clip[i / 4][i % 4]);
        let cull_distance = scene
            .shadow_quality
            .as_ref()
            .ok_or("caster quality is unavailable")?
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
                    })).ok_or_else(||format!("model caster placement {} surface {surface} lacks an independently retained draw mesh",source.placement))?;
                instances.push(instance);
            }
            if !instances.is_empty() {
                collected.push(ModelShadowCaster {
                    source: source_index,
                    instances,
                    flags: info.flags,
                    sort_distance,
                    camera_clip_mask: render_model::transformed_aabb_clip_cull(
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
                    "Model caster collection online: {} owners, {} surfaces",
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
                warn!(target: "dh2::shadows", frame = frame_count.0, "Model caster collection pending: {error}");
                output.error = Some(error);
            }
        }
    }
}

#[derive(Resource)]
pub(super) struct ParallelShadowCpu {
    pub prepared: render_shadow::ParallelShadowPrepareState,
    pub current: Option<ParallelCasterFrame>,
    pub adaptive_input: Option<AdaptiveShadowInput>,
    reset_frame: Option<u32>,
    initialized: [bool; 4],
    gather: Vec<GlobalShadowCasterScratch>,
    gather_verified: bool,
    gather_cache: query_cache::CasterQueryCache,
    projection_cache: Vec<query_cache::ProjectionMaskCache>,
    masks: Option<ShadowInstanceMasks>,
    gather_count: usize,
    world_query: Option<crate::local_caster_query::LocalWorldQuery>,
    shadow_extent: crate::rendering::environment::EnvironmentScalar,
    last_shadow_extent: Option<f32>,
    static_frames: Vec<Option<Arc<StaticShadowFrame>>>,
    last_error: Option<String>,
}

/// Retain the exact mapped input selected by preparation, not a later mailbox
/// observation. F8 formats it on demand; no extra readback or pixel copy.
pub(super) struct AdaptiveShadowInput {
    pub frame: u32,
    pub snapshot: Arc<coarse_depth::CoarseDepthSnapshot>,
    pub projection: render_shadow::ShadowMatrix,
    pub inverse_projection: render_shadow::ShadowMatrix,
    pub reference_interval: [f32; 2],
    pub effective_far: f32,
}

pub(super) struct ParallelCasterFrame {
    pub frame: u32,
    pub masks: Vec<Vec<u8>>,
    pub receiver: shadow_receiver::ReceiverOutput,
    pub slice_plans: Vec<render_shadow::ParallelShadowSlicePlan>,
    pub camera_view: render_shadow::ShadowMatrix,
    pub initialized: [bool; 4],
    pub static_frames: Vec<Option<Arc<StaticShadowFrame>>>,
    pub local_time: [f32; 2],
    pub models: Option<Arc<Vec<ModelShadowCaster>>>,
    pub clocks: game_clock::GameplayViewClocks,
}

pub(super) struct StaticShadowFrame {
    pub frame: u32,
    pub mask: Vec<u8>,
    pub clip: render_shadow::ShadowMatrix,
    pub local_time: [f32; 2],
}

impl Default for ParallelShadowCpu {
    fn default() -> Self {
        Self {
            prepared: render_shadow::ParallelShadowPrepareState::new(),
            current: None,
            adaptive_input: None,
            reset_frame: None,
            initialized: [false; 4],
            gather: Vec::new(),
            gather_verified: false,
            gather_cache: Default::default(),
            projection_cache: Vec::new(),
            masks: None,
            gather_count: 0,
            world_query: None,
            // Registered env/shadow/parallelLightExtend default at +0x50,
            // measured from the original registry; area targets remain authored.
            shadow_extent: crate::rendering::environment::EnvironmentScalar::new(70.0),
            last_shadow_extent: None,
            static_frames: Vec::new(),
            last_error: None,
        }
    }
}

/// Resolved BWM submissions, kept separate from model submissions. These
/// alone cannot authorize receiver sampling: the model collector must finish
/// before the combined scene frame can be published.
#[derive(Resource, Default)]
pub(super) struct ParallelWorldDraws {
    pub current: Option<(u32, Vec<shadow_pass::ShadowLayer>)>,
    matrices: Option<Handle<ShaderBuffer>>,
    static_layers: Vec<Option<(u32, shadow_pass::ShadowLayer)>>,
    uploads: Vec<Vec<super::shadow_preparation::WorldShadowUpload>>,
    static_uploads: Vec<Vec<super::shadow_preparation::WorldShadowUpload>>,
    last_error: Option<String>,
}

#[derive(Resource, Default)]
pub(super) struct ParallelModelDraws {
    pub current: Option<(u32, Vec<Vec<shadow_pass::ShadowInstance>>)>,
    last_error: Option<String>,
    reported: bool,
}

#[derive(Resource)]
pub(super) struct ParallelReceiverSource(Result<ParallelReceiverCompile, String>);

struct ParallelReceiverCompile {
    filtering_quality: i32,
    slope_scale: bool,
    slope_bias: bool,
    slope_post_projection: bool,
}

#[derive(Resource)]
pub(super) struct ScreenShadowSource(Result<ScreenShadowParameters, String>);
struct ScreenShadowParameters {
    ambient_occlusion: crate::platform::game_settings::AmbientOcclusion,
    producer: [u32; 9],
    blur: [u32; 9],
    step: f32,
    jitter: bool,
    cloud_motion: Vec4,
    cloud_density: Vec4,
    cloud_enabled: bool,
}

pub(super) fn initialize_screen_shadow_source(
    mut commands: Commands,
    scene: Res<SceneData>,
    mut programs: ResMut<ProgramCacheSource>,
    mut images: ResMut<Assets<Image>>,
) {
    let result = (|| -> Result<(ScreenShadowParameters, Handle<Image>, Option<Handle<Image>>), String> {
        let library = programs.0.as_mut().map_err(|error| error.clone())?;
        let catalog = scene
            .catalog
            .as_ref()
            .ok_or("screen catalog is unavailable")?;
        let environment = scene
            .environment_render_parameters
            .as_ref()
            .ok_or("screen environment is unavailable")?;
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
            .ok_or("cloud texture owner is unavailable")?
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
                .ok_or_else(|| format!("game cloud image source unresolved: {cloud_name}"))?;
            let mut loaded = read_streamed_diffuse_at_target(
                catalog,
                &stem,
                TextureReadTarget::Initial(Some(0)),
            )?
            .ok_or("game cloud authored mip0 is unavailable")?;
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
            .ok_or("game cloud image allocation failed")?
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
            .ok_or("game screen material is unavailable")?;
        let overrides = material_option_overrides(library, &material)?;
        let jitter =
            library.option_value(b"screenshadows", &overrides, b"screenshadows/useJitter")? != 0;
        let producer = library
            .definition(b"screenshadows")?
            .ok_or("game screen RPD is unavailable")?
            .render_state();
        let blur = library
            .definition(b"postfx_dwaoblur")?
            .ok_or("game DWAO blur RPD is unavailable")?
            .render_state();
        let ambient_occlusion = crate::platform::game_settings::read_ambient_occlusion()?;
        let depth_default = if ambient_occlusion == crate::platform::game_settings::AmbientOcclusion::Hbao {
            let empty = crate::entities::EnvironmentRenderParameters {layers:Vec::new()};
            let defaults = crate::assets::environment::ParameterReader {catalog,parameters:&empty};
            let source = defaults.image_source("SSAO/Zbuffer")?;
            Some(super::texture_streaming::load_resident_2d(catalog,&source,&mut images,&mut Default::default())?)
        } else {None};
        Ok((
            ScreenShadowParameters {
                ambient_occlusion,
                producer,
                blur,
                step,
                jitter,
                cloud_motion,
                cloud_density,
                cloud_enabled: !cloud_name.eq_ignore_ascii_case("_black"),
            },
            cloud,
            depth_default,
        ))
    })();
    match result {
        Ok((parameters, cloud, depth_default)) => {
            commands.insert_resource(screen_shadow_pass::ScreenShadowFrame {
                output: images.add(screen_shadow_pass::visibility_image(1, 1)),
                cloud,
                depth_default,
                current: None,
                ambient_occlusion: None,
                player_depth_hack: 0.0,
                produced: Default::default(),
                directional_produced: Default::default(),
            });
            commands.insert_resource(ScreenShadowSource(Ok(parameters)));
        }
        Err(error) => {
            eprintln!("Screen source unavailable: {error}");
            commands.insert_resource(ScreenShadowSource(Err(error)));
        }
    }
}

pub(super) fn prepare_screen_shadow_frame(
    source: Res<ScreenShadowSource>,
    cpu: Res<ParallelShadowCpu>,
    frame: Res<shadow_pass::ShadowSceneFrame>,
    mut screen: Option<ResMut<screen_shadow_pass::ScreenShadowFrame>>,
    cameras: Query<(&Transform, &Projection, &Camera), With<Camera3d>>,
    mut images: ResMut<Assets<Image>>,
    player_view: Option<Res<super::player_arms::PlayerArms>>,
    mut last_error: Local<Option<String>>,
) {
    let Some(ref mut screen) = screen else {
        return;
    };
    screen.current = None;
    screen.ambient_occlusion = None;
    // r_playerDepthHack keeps its cvar default 0 without the body component.
    let player_depth_hack = player_view.map_or(0.0, |view| view.player_depth_hack);
    screen.player_depth_hack = player_depth_hack;
    let result = (|| -> Result<screen_shadow_pass::ScreenShadowView, String> {
        let source = source.0.as_ref().map_err(Clone::clone)?;
        let (transform, projection, camera) = cameras.single().map_err(|e| e.to_string())?;
        let Projection::Custom(custom) = projection else {
            return Err("game screen projection is unavailable".into());
        };
        let projection = custom
            .get::<camera_projection::GameProjection>()
            .ok_or("game screen projection type is unavailable")?
            .rows();
        let mut inverse_projection = [[0.0; 4]; 4];
        if !render_shadow::invert_rows(&projection, &mut inverse_projection) {
            return Err("game screen projection is singular".into());
        }
        let extent = camera
            .physical_viewport_size()
            .filter(|extent| extent.min_element() > 0)
            .ok_or("game screen extent is unavailable")?;
        let size = Extent3d {
            width: extent.x,
            height: extent.y,
            depth_or_array_layers: 1,
        };
        if images
            .get(&screen.output)
            .ok_or("game screen image is unavailable")?
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
        use crate::platform::game_settings::AmbientOcclusion;
        use crate::rendering::ambient_occlusion::{AmbientOcclusionParameters, HbaoParameters};
        let render_scale = [1.0, 1.0];
        screen.ambient_occlusion = Some(screen_shadow_pass::AmbientOcclusionView {
            extent: [extent.x, extent.y],
            unproject: Vec4::new(-inverse_projection[2][3], inverse_projection[3][2], inverse_projection[3][3], 0.0),
            render_scale,
            blur_state: source.blur,
            parameters: match source.ambient_occlusion {
                AmbientOcclusion::Hbao => AmbientOcclusionParameters::Hbao(
                    HbaoParameters::new(
                        [extent.x, extent.y],
                        std::array::from_fn(|i| projection[i % 4][i / 4]),
                        render_scale,
                    )?,
                ),
                AmbientOcclusion::Disabled => AmbientOcclusionParameters::Disabled,
                AmbientOcclusion::Dwao => AmbientOcclusionParameters::Dwao {
                    extent: [extent.x, extent.y],
                    inverse_projection,
                    render_scale,
                },
            },
        });
        let shadow_pass::ShadowSceneState::Ready { .. } = &frame.state else {
            return Err("game directional atlas pending".into());
        };
        let cpu = cpu
            .current
            .as_ref()
            .ok_or("game current view is absent")?;
        let mut inverse = [[0.0; 4]; 4];
        if !render_shadow::invert_rows(&render_shadow::multiply_rows(projection, cpu.camera_view), &mut inverse) {
            return Err("game screen view is singular".into());
        }
        let position = view_to_game(transform.translation);
        let world_time = cpu.clocks.view_shader_times(1).world_time[0];
        let parameters = screen_shadow_pipeline::ScreenShadowUniform {
            inverse_view_projection: inverse.map(Vec4::from_array),
            unproject: Vec4::new(
                -inverse_projection[2][3],
                inverse_projection[3][2],
                inverse_projection[3][3],
                0.0,
            ),
            camera_time: Vec4::new(position[0], position[1], position[2], world_time),
            // FUN_14032c7f0's stencil-one draw: r_playerDepthHack, or 1.
            depth_scale_step_hack: Vec4::new(
                1.0,
                1.0,
                source.step,
                if player_depth_hack > 0.0 { player_depth_hack } else { 1.0 },
            ),
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
        Ok(screen_shadow_pass::ScreenShadowView {
            revision: frame.revision,
            parameters,
            producer_state: source.producer,
        })
    })();
    match result {
        Ok(value) => {
            screen.current = Some(value);
            *last_error = None;
        }
        Err(error) => {
            if last_error.as_ref() != Some(&error) {
                warn!(target: "dh2::shadows", revision = frame.revision, "Screen frame pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}

pub(super) fn initialize_parallel_receiver_source(
    mut commands: Commands,
    mut programs: ResMut<ProgramCacheSource>,
) {
    let result = (|| -> Result<ParallelReceiverCompile, String> {
        let library = programs.0.as_mut().map_err(|error| error.clone())?;
        let empty = render_program::SerializedParameterBlock {
            flags: 0,
            parameters: Vec::new(),
        };
        let mut option = |name: &[u8]| library.option_value(b"screenshadows", &empty, name);
        Ok(ParallelReceiverCompile {
            filtering_quality: option(b"shadowFilteringQuality")?,
            slope_scale: option(b"shadow/useSlopeScale")? != 0,
            slope_bias: option(b"shadow/useSlopeScaleBias")? != 0,
            slope_post_projection: option(b"shadow/slopeScaleIsPostProjection")? != 0,
        })
    })();
    commands.insert_resource(ParallelReceiverSource(result));
}

pub(super) fn publish_parallel_shadow_frame(
    owner: Option<Res<ParallelShadowOwner>>,
    cpu: Res<ParallelShadowCpu>,
    mut world: ResMut<ParallelWorldDraws>,
    mut models: ResMut<ParallelModelDraws>,
    receiver_source: Res<ParallelReceiverSource>,
    scene: Res<SceneData>,
    lights: Query<&DirectionalLight>,
    completion: Res<shadow_pass::ShadowSceneCompletion>,
    mut output: ResMut<shadow_pass::ShadowSceneFrame>,
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
    let result=(||->Result<shadow_pass::ParallelReceiverVars,String>{
        let frame=cpu.current.as_ref().ok_or("game CPU frame is unavailable")?;
        let source=receiver_source.0.as_ref().map_err(Clone::clone)?;
        let (world_frame,world)=world.current.as_ref().ok_or("game world submissions are unavailable")?;
        let (model_frame,models)=models.current.as_ref().ok_or("game model submissions are unavailable")?;
        if *world_frame!=frame.frame || *model_frame!=frame.frame || world.len()!=models.len() {
            return Err("game directional submissions belong to different view frames".into());
        }
        // One directional light has a proven GPU-array index of zero. A
        // multiple-sun view must bind the renderer's actual chosen index.
        if lights.iter().count()!=1 {return Err("game sun requires the actual directional GPU-array index in a multiple-light view".into());}
        let environment=scene.environment_lighting.ok_or("game receiver environment is unavailable")?;
        let direction=render_shadow::environment_sun_direction(
            environment.sun_theta_degrees.ok_or("game sun theta is unavailable")?,
            environment.sun_phi_degrees.ok_or("game sun phi is unavailable")?);
        let count=world.len();
        let mut matrices=cpu.prepared.cascades[..count].iter().map(|cascade|cascade.atlas).collect::<Vec<_>>();
        matrices.push(owner.whole_view.atlas);
        let mut fars=[f32::MAX;4];
        fars[..count].copy_from_slice(&cpu.prepared.fars[..count]);
        let receiver=shadow_pass::ParallelReceiverVars {
            // This publication owns the screenShadows ComputeParallelShadow
            // call. Its explicit zero-normal path must not enable the direct
            // forward hook before the screen producer finishes.
            path:render_shadow::ParallelShadowReceiverPath::ScreenSpace,
            world_to_shadow:matrices,view_row2:frame.camera_view[2],cascade_fars:fars,
            shadow_size:[owner.dimensions.width as f32,owner.dimensions.height as f32],
            light_quality:2, // HIGH_QUALITY in installed global.decl/screenshadows.
            filtering_quality:source.filtering_quality,directional_light_index:0,
            offset:shadow_sampling::ShadowOffsetParameters {
                sun_direction:direction,use_normal_offset:false,use_slope_scale:source.slope_scale,
                use_slope_bias:source.slope_bias,slope_post_projection:source.slope_post_projection,
                use_uv_normal_offset:false,normal_offset:owner.shader_bias[1],
                bias:owner.shader_bias[0],slope_bias:owner.shader_bias[2],
            },
        };
        shadow_sampling::ParallelSamplingUniform::from_owner_receiver(&receiver)?;
        if std::env::var_os("DH2_TRACE_SHADOW_GEOMETRY").is_some() && frame.frame%300==0 {
            eprintln!("Directional geometry frame={} fars={fars:?} view={:?} sun={direction:?} matrices={:?} counts={:?}",
                frame.frame,frame.camera_view,receiver.world_to_shadow,
                frame.masks.iter().map(|mask|mask.iter().filter(|&&v|v!=0).count()).collect::<Vec<_>>());
        }
        Ok(receiver)
    })();
    match result {
        Ok(receiver) => {
            let Some(revision) = output.revision.checked_add(1) else {
                output.state = shadow_pass::ShadowSceneState::Unavailable(
                    "game frame revision overflow".into(),
                );
                return;
            };
            // Both draw builders publish once before this sole consumer. Move
            // their completed lists only after every fallible check succeeds.
            let mut layers = world.current.take().unwrap().1;
            for (layer, models) in layers.iter_mut().zip(models.current.take().unwrap().1) {
                layer.instances.extend(models);
            }
            layers.push(shadow_pass::ShadowLayer {
                ops: shadow_pass::ShadowLayerOps {
                    depth: shadow_pass::ShadowLoad::Load,
                    stencil: shadow_pass::StencilLoad::Load,
                },
                cache: shadow_pass::ShadowCacheOp::None,
                instances: Vec::new(),
            });
            if last_error.take().is_some()
                || !matches!(
                    output.state,
                    shadow_pass::ShadowSceneState::Ready { .. }
                )
            {
                println!(
                    "Directional atlas frame online: {} layers, {} current draws",
                    layers.len(),
                    layers
                        .iter()
                        .map(|layer| layer.instances.len())
                        .sum::<usize>()
                );
            }
            output.revision = revision;
            output.state = shadow_pass::ShadowSceneState::Ready {
                target_image: owner.target_image.clone(),
                dimensions: owner.dimensions,
                receiver,
                layers: layers.into(),
            };
        }
        Err(error) => {
            if last_error.as_ref() != Some(&error) {
                warn!(target: "dh2::shadows", frame = ?cpu.current.as_ref().map(|frame| frame.frame), "Directional atlas frame pending: {error}");
                *last_error = Some(error.clone());
            }
            output.state = shadow_pass::ShadowSceneState::Unavailable(error);
        }
    }
}

pub(super) fn prepare_model_shadow_instances(
    records: &[(usize, usize)],
    clip: render_shadow::ShadowMatrix,
    hardware_bias: [f32; 3],
    margin: f32,
    dimensions: [u32; 2],
    clocks: &game_clock::GameplayViewClocks,
    sources: &ModelShadowSources,
    placements: &RenderSourcePlacements,
    materials: &ShadowMaterials,
    images: &Assets<Image>,
    vegetation: &VegetationMeshInputs,
    queue: &mut super::scene_queue::ShadowTextureRequests<'_>,
) -> Result<Vec<shadow_pass::ShadowInstance>, String> {
    let mut instances = Vec::with_capacity(records.len());
    for &(source_id, index) in records {
        let source = &sources.0[source_id];
        let instance = &queue.instances[index];
        let mesh = instance.mesh.clone();
        let provenance = instance
            .source
            .ok_or("game model draw lost source provenance")?;
        let inputs = materials
            .0
            .get(&instance.material)
            .ok_or("game model material owner is absent")?
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
                    "game model placement {} ({}): {error}",
                    source.placement,
                    placements
                        .0
                        .get(source.placement)
                        .map(|p| p.resource_path.as_str())
                        .unwrap_or("missing source")
                )
            })?
            .as_ref()
            .ok_or("game admitted model has no resolved default slot10")?;
        let parameters = inputs
            .parameters
            .as_ref()
            .map_err(Clone::clone)?
            .as_ref()
            .ok_or("game model parameter block is absent")?;
        let multiplier = parameters.vector(b"modeldepthhack", None)?[0];
        if !multiplier.is_finite() {
            return Err("game model depth multiplier is non-finite".into());
        }
        let placement = placements
            .0
            .get(source.placement)
            .ok_or("game model placement owner is absent")?;
        let affine =
            render_model::affine_rows(placement.position, placement.rotation, placement.scale);
        let model = [affine[0], affine[1], affine[2], [0.0, 0.0, 0.0, 1.0]];
        let mvp = render_shadow::multiply_rows(clip, model);
        let state = render_model::selected_shadow_model_draw_state(
            inputs.source.shadow_draw_flags49(),
            inputs
                .source
                .main_render_state
                .ok_or("game model MAIN draw state is absent")?,
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
        let (front_face, cull_mode) = shadow_pipeline::shadow_mesh_raster(&state, !flipped);
        let class = program.class;
        let alpha = if class != shader_cache::CachedRigidShadowClass::Opaque {
            let slot = inputs
                .diffuse_image_slot
                .ok_or("game model cutout has no reference diffuse image")?;
            queue.request_shadow_diffuse(slot);
            let diffuse = queue
                .diffuse_image_handles
                .get(slot)
                .and_then(Option::as_ref)
                .ok_or_else(|| {
                    format!("game model shadow diffuse slot {slot} awaits texture streaming")
                })?;
            if images.get(diffuse).is_none() {
                return Err("game model diffuse asset is not resident".into());
            }
            Some(shadow_pass::ShadowAlphaMaterial {
                diffuse: diffuse.clone(),
                uv_set: if class == shader_cache::CachedRigidShadowClass::VistaAlpha {
                    0
                } else {
                    inputs
                        .world_uv_set
                        .ok_or("game model Layer0 UV selector is absent")?
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
                return Err("game skin shader has no MD6 vertex source".into());
            };
            if skin_influences != program.skin_influences {
                return Err("game skin stream and selected shader weight counts disagree".into());
            }
            let skinning = source
                .skinning
                .as_ref()
                .ok_or("game skin owner has no palette source")?
                .as_ref()
                .map_err(Clone::clone)?;
            (
                shadow_pass::ShadowProgram::Skinned {
                    influences: program.skin_influences,
                    alpha: alpha.is_some(),
                    dither: program.alpha_dither,
                },
                shadow_pass::ShadowInstanceConstants::Skinned {
                    constants: shadow_pass::skin_shadow_instance_cb(
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
                    shadow_pass::ShadowProgram::RigidOpaque,
                    shadow_pass::ShadowInstanceConstants::Rigid(
                        shadow_pass::shadow_instance_cb(&mvp, multiplier),
                    ),
                ),
                shader_cache::CachedRigidShadowClass::AlphaTested => (
                    shadow_pass::ShadowProgram::RigidAlphaTested,
                    shadow_pass::ShadowInstanceConstants::Rigid(
                        shadow_pass::shadow_instance_cb(&mvp, multiplier),
                    ),
                ),
                shader_cache::CachedRigidShadowClass::VistaAlpha => (
                    shadow_pass::ShadowProgram::RigidVistaAlpha,
                    shadow_pass::ShadowInstanceConstants::Rigid(
                        shadow_pass::shadow_instance_cb(&mvp, multiplier),
                    ),
                ),
                shader_cache::CachedRigidShadowClass::Vegetation => {
                    vegetation
                        .0
                        .get(&mesh.id())
                        .ok_or("game model wind channels are absent")?
                        .as_ref()
                        .map_err(Clone::clone)?;
                    // Initial model constructor selects group0, with
                    // zero model-time offset. Later setter events must
                    // update the actual owner before changing this path.
                    let time = clocks
                        .model_local_time(0.0, 4)
                        .ok_or("game initial model clock is unavailable")?;
                    (
                        shadow_pass::ShadowProgram::Vegetation,
                        shadow_pass::ShadowInstanceConstants::Vegetation(
                            shadow_pass::vegetation_shadow_instance_cb(
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
        instances.push(shadow_pass::ShadowInstance {
            mesh,
            per_instance_cb: constants,
            program,
            alpha_material: alpha,
            front_face,
            cull_mode,
            draw_state: state,
            stencil_reference: 0,
            scissor: shadow_pipeline::ShadowScissor {
                x: 0,
                y: 0,
                width: dimensions[0],
                height: dimensions[1],
            },
        });
    }
    Ok(instances)
}

/// Preparation reads one immutable frame. Results and texture requests are
/// committed in layer order, retaining the serial first-error boundary.
fn prepare_shadow_layers<T: Send + std::fmt::Debug + PartialEq + 'static>(
    count: usize,
    prepare: impl Fn(usize) -> (Result<T, String>, Vec<usize>) + Sync,
) -> Vec<(Result<T, String>, Vec<usize>)> {
    let parallel =
        count > 1 && std::env::var("DH2_PARALLEL_SHADOW_PREPARATION").as_deref() != Ok("0");
    if !parallel {
        return (0..count).map(prepare).collect();
    }
    let mut jobs = bevy::tasks::ComputeTaskPool::get().scope(|scope| {
        for layer in 0..count {
            let prepare = &prepare;
            scope.spawn(async move { (layer, prepare(layer)) });
        }
    });
    jobs.sort_unstable_by_key(|job| job.0);
    let results: Vec<_> = jobs.into_iter().map(|(_, result)| result).collect();
    if std::env::var("DH2_VERIFY_SHADOW_PREPARATION").as_deref() == Ok("1") {
        let serial: Vec<_> = (0..count).map(prepare).collect();
        assert_eq!(
            results, serial,
            "parallel shadow preparation changed ordered draw inputs or texture requests"
        );
    }
    results
}

fn commit_shadow_layers<T>(
    results: Vec<(Result<T, String>, Vec<usize>)>,
    requests: &mut super::scene_queue::ShadowTextureRequests<'_>,
) -> Result<Vec<T>, String> {
    let mut layers = Vec::with_capacity(results.len());
    for (result, slots) in results {
        requests.append_slots(slots);
        layers.push(result?);
    }
    Ok(layers)
}

#[cfg(test)]
mod layer_preparation_tests {
    use super::*;

    #[test]
    fn ordered_commit_keeps_requests_through_first_error_only() {
        let scene = SceneRenderQueue::default();
        let mut requests = super::super::scene_queue::ShadowTextureRequests::new(&scene);
        let result = commit_shadow_layers(
            vec![
                (Ok(7u32), vec![4, 2]),
                (Err("pending alpha image".into()), vec![5, 2]),
                (Ok(9), vec![8]),
            ],
            &mut requests,
        );
        assert_eq!(result, Err("pending alpha image".into()));
        assert_eq!(requests.into_slots(), vec![4, 2, 5, 2]);
        let mut requests = super::super::scene_queue::ShadowTextureRequests::new(&scene);
        let result = commit_shadow_layers(
            vec![(Ok(7u32), vec![4]), (Ok(3), vec![2, 4])],
            &mut requests,
        );
        assert_eq!(result, Ok(vec![7, 3]));
        assert_eq!(requests.into_slots(), vec![4, 2, 4]);
    }
}

pub(super) fn prepare_parallel_model_draws(
    cpu: Res<ParallelShadowCpu>,
    owner: Option<Res<ParallelShadowOwner>>,
    sources: Res<ModelShadowSources>,
    placements: Res<RenderSourcePlacements>,
    materials: Res<ShadowMaterials>,
    images: Res<Assets<Image>>,
    vegetation: Res<VegetationMeshInputs>,
    queue: Res<SceneRenderQueue>,
    mut commands: Commands,
    mut output: ResMut<ParallelModelDraws>,
    profile: Option<Res<frame_profile::CpuFrameProfile>>,
) {
    let _span = profile.as_deref().map(|p| p.measure(23));
    output.current = None;
    let (Some(owner), Some(frame)) = (owner, cpu.current.as_ref()) else {
        return;
    };
    let mut shadow_textures = super::scene_queue::ShadowTextureRequests::new(&queue);
    let result = (|| -> Result<Vec<Vec<shadow_pass::ShadowInstance>>, String> {
        let casters = frame
            .models
            .as_ref()
            .ok_or("game model collector has no matching frame")?;
        let count = frame.masks.len();
        if frame.initialized[..count].iter().any(|&ready| !ready) {
            return Err("game model draws await cascade initialization".into());
        }
        let partial = owner.cascade_controls.partial_dynamic_update;
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
        let results = prepare_shadow_layers(count, |layer| {
            let mut requests = super::scene_queue::ShadowTextureRequests::new(&queue);
            let result = (|| -> Result<Vec<shadow_pass::ShadowInstance>, String> {
                let mut instances = Vec::new();
                if frame.slice_plans[layer].skip
                    || (layer == count - 1 && limited && queue.static_world_model_present)
                {
                    return Ok(instances);
                }
                let cascade = &cpu.prepared.cascades[layer];
                let margin = cascade.depth_offset;
                if !margin.is_finite() {
                    return Err("game directional model clip margin is non-finite".into());
                }
                let clip: [f32; 16] = std::array::from_fn(|i| cascade.clip[i / 4][i % 4]);
                // FUN_140328260 runs separately for both model buckets. Both
                // follow static WORLD cache restoration in the preserve pass;
                // neither model bucket may be saved into the world static cache.
                for dynamic in [false, true] {
                    let mut records = Vec::new();
                    let mut keys = Vec::new();
                    for caster in casters.iter().filter(|caster| {
                        caster.dynamic == dynamic && caster.sort_distance <= maximum
                    }) {
                        let source = &sources.0[caster.source];
                        let culled = if margin < 0.0 {
                            render_model::transformed_aabb_clip_cull_at_depth(
                                &clip,
                                source.bounds[0],
                                source.bounds[1],
                                margin,
                            )
                        } else {
                            render_model::transformed_aabb_clip_cull(
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
                    instances.extend(prepare_model_shadow_instances(
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
                        &mut requests,
                    )?);
                }
                Ok(instances)
            })();
            (result, requests.into_slots())
        });
        commit_shadow_layers(results, &mut shadow_textures)
    })();
    shadow_textures.submit(&mut commands);
    match result {
        Ok(layers) => {
            if output.last_error.take().is_some() || !output.reported {
                info!(target: "dh2::shadows", frame = frame.frame,
                    "Model shadow submissions online: {} draws",
                    layers.iter().map(Vec::len).sum::<usize>()
                );
                output.reported = true;
            }
            output.current = Some((frame.frame, layers));
        }
        Err(error) => {
            if output.last_error.as_ref() != Some(&error) {
                warn!(target: "dh2::shadows", frame = frame.frame, "Model shadow submissions pending: {error}");
                output.last_error = Some(error);
            }
        }
    }
}

pub(super) fn prepare_parallel_world_draws(
    cpu: Res<ParallelShadowCpu>,
    owner: Option<Res<ParallelShadowOwner>>,
    scene: Res<SceneData>,
    sources: Res<WorldShadowMeshSources>,
    materials: Res<ShadowMaterials>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    queue: Res<SceneRenderQueue>,
    mut commands: Commands,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut draws: ResMut<ParallelWorldDraws>,
    profile: Option<Res<frame_profile::CpuFrameProfile>>,
    mut matrix_bank: ResMut<super::world_instances::WorldMatrices>,
) {
    let _span = profile.as_deref().map(|p| p.measure(22));
    draws.current = None;
    let (Some(owner), Some(frame)) = (owner, cpu.current.as_ref()) else {
        return;
    };
    let mut shadow_textures = super::scene_queue::ShadowTextureRequests::new(&queue);
    let admission = super::shadow_preparation::WorldShadowAdmission::new(
        &scene.world,
        &sources,
        &materials,
    );

    let result = (|| -> Result<Vec<shadow_pass::ShadowLayer>, String> {
        let count = frame.masks.len();
        if frame.initialized[..count].iter().any(|&ready| !ready) {
            return Err("game world draws await initialized cascade projections".into());
        }
        if draws.static_layers.len() != count {
            draws.static_layers = (0..count).map(|_| None).collect();
        }
        draws.uploads.resize_with(count, Vec::new);
        draws.static_uploads.resize_with(count, Vec::new);
        let static_layers = &draws.static_layers;
        let results = prepare_shadow_layers(count, |layer| {
            let mut requests = super::scene_queue::ShadowTextureRequests::new(&queue);
            let result = (|| {
                let plan = frame.slice_plans[layer];
                if plan.skip {
                    return Ok(None);
                }
                let cached = plan.write_static_cache || plan.reuse_static_cache;
                let (mask, clip, time, generation) = if cached {
                    let retained = frame.static_frames[layer].as_ref().ok_or_else(|| {
                        format!("game layer {layer} has no retained static source frame")
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
                    if static_layers[layer]
                        .as_ref()
                        .is_some_and(|(old, _)| *old == generation)
                    {
                        return Ok(Some((generation, None)));
                    }
                }
                let prepared =
                    super::shadow_preparation::prepare_world_shadow_masks_with_admission(
                        &owner.world_draw_settings(),
                        &admission,
                        &meshes,
                        &images,
                        &mut requests,
                        std::slice::from_ref(mask),
                        clip,
                        Some(time),
                        None,
                    )?;
                Ok(Some((generation.unwrap_or(frame.frame), Some(prepared))))
            })();
            (result, requests.into_slots())
        });
        let plans = commit_shadow_layers(results, &mut shadow_textures)?;
        // Resolve every selected program/image first; failed preparation owns
        // no transient GPU buffers. The immutable BWM matrix bank is shared.
        if draws.matrices.is_none() {
            draws.matrices = matrix_bank.get_or_upload(&scene.world.matrices, &mut buffers);
        }
        let mut layers = Vec::with_capacity(count);
        for (layer, prepared) in plans.into_iter().enumerate() {
            let Some((generation, prepared)) = prepared else {
                layers.push(shadow_pass::ShadowLayer {
                    ops: shadow_pass::ShadowLayerOps {
                        depth: shadow_pass::ShadowLoad::Load,
                        stencil: shadow_pass::StencilLoad::Load,
                    },
                    cache: shadow_pass::ShadowCacheOp::None,
                    instances: Vec::new(),
                });
                continue;
            };
            let cached = frame.slice_plans[layer].write_static_cache
                || frame.slice_plans[layer].reuse_static_cache;
            let mut built = match prepared {
                Some(prepared) => {
                    let matrices = draws.matrices.clone();
                    super::shadow_preparation::build_world_shadow_layer(
                        prepared,
                        matrices.as_ref(),
                        owner.dimensions,
                        &mut buffers,
                        Some(if cached {
                            &mut draws.static_uploads[layer]
                        } else {
                            &mut draws.uploads[layer]
                        }),
                        cached,
                    )
                }
                None => draws.static_layers[layer].as_ref().unwrap().1.clone(),
            };
            if cached {
                built.cache = shadow_pass::ShadowCacheOp::Retained {
                    generation: u64::from(generation),
                    static_instances: built.instances.len(),
                };
                draws.static_layers[layer] = Some((generation, built.clone()));
            }
            layers.push(built);
        }
        Ok(layers)
    })();
    shadow_textures.submit(&mut commands);
    match result {
        Ok(layers) => {
            if draws.last_error.take().is_some() {
                info!(target: "dh2::shadows", frame = frame.frame,
                    "Parallel BWM submissions online: {} cascades, {} draws",
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
                warn!(target: "dh2::shadows", frame = frame.frame, "Parallel BWM submissions pending: {error}");
                draws.last_error = Some(error);
            }
        }
    }
}

pub(super) fn prepare_parallel_shadow_cpu(
    owner: Option<Res<ParallelShadowOwner>>,
    scene: Res<SceneData>,
    queue: Res<SceneRenderQueue>,
    world: Option<Res<TomeWorld>>,
    caster_world: Option<Res<ShadowCasterWorld>>,
    model_bounds: Res<ShadowModelBoundsSources>,
    pickups: Res<PickupState>,
    cameras: Query<(&Camera, &Transform, &Projection), With<Camera3d>>,
    coarse: Res<coarse_depth::CoarseDepthMailbox>,
    frame_count: Res<bevy::diagnostic::FrameCount>,
    settings: Res<ViewSettings>,
    clocks: Res<game_clock_runtime::GameplayViewClock>,
    model_frame: Res<ModelCasterFrame>,
    mut cpu: ResMut<ParallelShadowCpu>,
    mut receiver_dumped: Local<bool>,
    profile: Option<Res<frame_profile::CpuFrameProfile>>,
) {
    let _span = profile.as_deref().map(|p| p.measure(21));
    cpu.current = None;
    cpu.adaptive_input = None;
    let Some(owner) = owner else {
        return;
    };
    let source_versions = (
        world.as_ref().map_or(0, |world| world.last_changed().get()),
        caster_world
            .as_ref()
            .map_or(0, |world| world.last_changed().get()),
    );
    let cache_queries = std::env::var("DH2_CACHE_SHADOW_QUERIES").as_deref() != Ok("0");
    let verify_queries = std::env::var("DH2_VERIFY_SHADOW_QUERY_CACHE").as_deref() == Ok("1");
    let result = (|| -> Result<ParallelCasterFrame, String> {
        let world = world
            .as_deref()
            .ok_or("game caster gather has no Tome owner")?;
        let caster_world = caster_world
            .as_deref()
            .ok_or("game caster Tome cache is unavailable")?;
        let (camera, transform, projection) = cameras.single().map_err(|e| e.to_string())?;
        let host_projection = projection;
        let Projection::Custom(custom) = projection else {
            return Err(
                "game cascade fitting requires the reference finite camera projection".into(),
            );
        };
        let projection = custom
            .get::<camera_projection::GameProjection>()
            .ok_or("game gameplay projection type is unavailable")?;
        let camera_projection = projection.rows();
        let mut inverse_projection = [[0.0; 4]; 4];
        if !render_shadow::invert_rows(&camera_projection, &mut inverse_projection) {
            return Err("game gameplay projection is singular".into());
        }
        let pose = game_pose_from_view_transform(transform);
        let authored_shadow_near = cpu.shadow_extent.update(
            scene
                .environment_render_parameters
                .as_ref()
                .ok_or("shadow environment is unavailable")?,
            scene.environment_blend_duration,
            &scene.area_environments,
            view_to_game(transform.translation),
            "env/shadow/parallelLightExtend",
            clocks.view_shader_times(owner.source_flag_04)?.world_time[1],
        )?;
        let shadow_near =
            authored_shadow_near.max(scene.rewrite_shadow_settings.minimum_sun_caster_extent);
        if cpu.last_shadow_extent.map(f32::to_bits) != Some(authored_shadow_near.to_bits()) {
            let position = view_to_game(transform.translation);
            let areas: Vec<_> = scene.area_environments.iter()
                .filter(|area| area.contains(position))
                .map(|area| area.id.as_str())
                .collect();
            info!(target: "dh2::shadows", frame = frame_count.0, ?position, ?areas,
                previous = ?cpu.last_shadow_extent, authored = authored_shadow_near,
                applied = shadow_near, "Caster extent changed");
            cpu.last_shadow_extent = Some(authored_shadow_near);
        }
        let camera_view = tome::render_query_base_rows(Mat4::IDENTITY.to_cols_array_2d(), pose);
        let count = owner
            .dimensions
            .layers
            .checked_sub(1)
            .filter(|count| (1..=4).contains(count))
            .ok_or("game atlas has no cascade layers")? as usize;
        let controls = owner.cascade_controls;
        let effective_far = if controls.tighten_parallel_shadows {
            let snapshot = coarse
                .latest()?
                .ok_or("game adaptive GPU depth has not mapped yet")?;
            let viewport = camera
                .physical_viewport_size()
                .ok_or("game camera has no viewport")?;
            if snapshot.metadata.source_extent != [viewport.x, viewport.y] {
                return Err("game adaptive GPU depth awaits the resized camera target".into());
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
            let reference_interval = [previous_reference, cpu.prepared.far_state.authored_fars[count - 1]];
            let effective_far = render_shadow::adaptive_directional_far(
                true,
                &camera_projection,
                &inverse_projection,
                reference_interval,
                Some(&depths),
            )?;
            cpu.adaptive_input = Some(AdaptiveShadowInput {
                frame: frame_count.0,
                snapshot,
                projection: camera_projection,
                inverse_projection,
                reference_interval,
                effective_far,
            });
            effective_far
        } else {
            -1.0
        };
        if std::env::var_os("DH2_TRACE_SHADOW_GEOMETRY").is_some() && frame_count.0 % 300 == 0 {
            eprintln!(
                "Adaptive candidate frame={} far={effective_far}",
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
            .ok_or("game shadow sun is unavailable")?;
        let toward_sun = render_shadow::environment_sun_direction(
            environment
                .sun_theta_degrees
                .ok_or("game sun theta is unavailable")?,
            environment
                .sun_phi_degrees
                .ok_or("game sun phi is unavailable")?,
        );
        let previous = cpu.prepared.fars;
        let fit_span = profile.as_deref().map(|p| p.measure(38));
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
        drop(fit_span);
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
            .ok_or("game receiver has no valid merged ordinary query")?;
        let depth = ordinary
            .depth_grid
            .as_deref()
            .ok_or("game ordinary query has no depth grid")?;
        let viewport = camera
            .physical_viewport_size()
            .ok_or("game camera has no viewport")?;
        let scale = view_distance_scale(host_projection, viewport, Some(&settings))?;
        let distance = (scale != 1.0).then_some(ShadowDistanceDescriptor {
            center: [0.0; 3],
            radius: scale,
            override_center: false,
        });
        let dynamic_bounds = model_bounds.active_bounds(&queue, &pickups)?;
        let ray = toward_sun.map(|value| f32::from_bits(value.to_bits() ^ 0x8000_0000));
        let mut depth_projection = ReceiverDepthProjection {
            receiver_rows: [[0.0; 4]; 4],
            transposed_rows: [[0.0; 4]; 4],
            mask: Box::new([0; 128 * 128]),
        };
        let receiver_span = profile.as_deref().map(|p| p.measure(39));
        let receiver = shadow_receiver::initialize_receiver(
            shadow_receiver::ReceiverInput {
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
        drop(receiver_span);
        // Optional numerical replay evidence; no framebuffer or image capture.
        if !*receiver_dumped && frame_count.0 >= 300 {
            if let Some(path) = std::env::var_os("DH2_SHADOW_RECEIVER_DUMP") {
                let mut bytes = b"DHR2".to_vec();
                bytes.extend_from_slice(&(dynamic_bounds.len() as u32).to_le_bytes());
                for value in ordinary
                    .render_rows
                    .iter()
                    .flatten()
                    .chain(ordinary.camera_point.iter())
                    .chain(ray.iter())
                    .chain(
                        [
                            cpu.prepared.receiver_maximum_distance(count)?,
                            ordinary.maximum_depth,
                        ]
                        .iter(),
                    )
                    .chain(tome::global_bounds(&world.bytes)?.iter().flatten())
                    .chain(dynamic_bounds.iter().flatten().flatten())
                    .chain(depth.iter())
                    .chain([receiver.distance, receiver.cutoff].iter())
                    .chain(receiver.bounds.iter())
                    .chain(depth_projection.receiver_rows.iter().flatten())
                {
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
            cpu.masks = Some(ShadowInstanceMasks::new(
                scene.world.visibility.len(),
                count,
            ));
            cpu.static_frames = (0..count).map(|_| None).collect();
        }
        cpu.masks.as_mut().unwrap().clear();
        if receiver.status == shadow_receiver::ReceiverStatus::Initialized {
            // An uninitialized, skipped slice cannot contribute a submission.
            // Prune only those independent tests, then restore physical bits.
            let active: Vec<usize> = (0..count).filter(|&index| cpu.initialized[index]).collect();
            if active.is_empty() {
                return Err("game cascade initialization is pending".into());
            }
            let cascades = active
                .iter()
                .map(|&index| {
                    ShadowFrustumQuery::from_projection_rows(
                        cpu.prepared.cascades[index].caster_clip,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            if cpu.gather_count != active.len() {
                cpu.gather = (0..8)
                    .map(|_| {
                        GlobalShadowCasterScratch::new(
                            caster_world.0.object_count(),
                            active.len(),
                        )
                    })
                    .collect();
                cpu.gather_count = active.len();
            }
            let cascades = cascades
                .iter()
                .map(ShadowFrustumQuery::as_receiver)
                .collect::<Vec<_>>();
            let planes: Vec<usize> = (0..receiver.plane_count).collect();
            let primary = ShadowReceiverState::from_planes(
                &receiver.planes[..receiver.plane_count],
                &planes,
            );
            let direction = CasterDirection::from_receiver_direction(ray)?;
            // 330c10 dispatches eight gatherCaster jobs (32a690), each with
            // independent TLS scratch. Preserve their exact aligned ranges,
            // then merge in job order before the native parent suppression.
            let object_distance =
                ShadowObjectDistanceQuery::from_descriptor(ordinary.camera_point, distance);
            let gather_span = profile.as_deref().map(|p| p.measure(40));
            let common_input = GlobalShadowCasterInput {
                source: &world.bytes,
                primary,
                cascades: &cascades,
                distance: object_distance,
                direction,
                depth: Some(&depth_projection),
                job: 0,
                jobs: 8,
            };
            let key = cache_queries.then(|| {
                query_cache::CasterQueryKey::new(
                    common_input,
                    source_versions,
                    &caster_world.0 as *const _ as usize,
                    &active,
                )
            });
            let cached = key.as_ref().and_then(|key| cpu.gather_cache.get(key));
            let force_fresh_query = verify_queries
                || (!cpu.gather_verified
                    && std::env::var_os("DH2_VERIFY_PARALLEL_CASTER_JOBS").is_some());
            if let Some(cached) = cached.as_ref().filter(|_| !force_fresh_query) {
                for hits in cached.iter() {
                    cpu.masks
                        .as_mut()
                        .unwrap()
                        .publish(hits, &scene.world.tome_object_instances)?;
                }
            } else {
                let mut gathered = bevy::tasks::ComputeTaskPool::get().scope(|scope| {
                    for (job, scratch) in cpu.gather.iter_mut().enumerate() {
                        let caster_tome = &caster_world.0;
                        scope.spawn(async move {
                            (
                                job,
                                scratch
                                    .gather(
                                        caster_tome,
                                        &GlobalShadowCasterInput {
                                            job: job as u32,
                                            ..common_input
                                        },
                                    )
                                    .map(|hits| hits.to_vec()),
                            )
                        });
                    }
                });
                gathered.sort_by_key(|(job, _)| *job);
                if !cpu.gather_verified
                    && std::env::var_os("DH2_VERIFY_PARALLEL_CASTER_JOBS").is_some()
                {
                    let mut serial = GlobalShadowCasterScratch::new(
                        caster_world.0.object_count(),
                        active.len(),
                    );
                    for (job, result) in &gathered {
                        let expected = serial.gather(
                            &caster_world.0,
                            &GlobalShadowCasterInput {
                                job: *job as u32,
                                ..common_input
                            },
                        )?;
                        if result.as_ref().map_err(Clone::clone)?.as_slice() != expected {
                            return Err(format!(
                                "parallel caster job {job} differs from serial gather"
                            ));
                        }
                    }
                    cpu.gather_verified = true;
                    eprintln!(
                        "Parallel caster jobs verified: all eight index/mask lists match serial execution"
                    );
                }
                let mut physical_results = Vec::with_capacity(gathered.len());
                for (_, result) in gathered {
                    let mut hits = result?;
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
                    physical_results.push(hits);
                }
                if let Some(cached) = &cached {
                    assert_eq!(
                        cached.as_ref(),
                        &physical_results,
                        "cached caster query differs from reference eight-job gather"
                    );
                }
                if let Some(key) = key {
                    cpu.gather_cache.store(key, Arc::new(physical_results));
                }
            }
            drop(gather_span);
            if cache_queries
                && std::env::var_os("DH2_TRACE_SHADOW_QUERY_CACHE").is_some()
                && frame_count.0 % 300 == 0
            {
                eprintln!(
                    "CASTER_QUERY_CACHE hits={} misses={}",
                    cpu.gather_cache.hits, cpu.gather_cache.misses
                );
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
            cpu.world_query = Some(crate::local_caster_query::LocalWorldQuery::new(
                &scene.world,
            ));
            cpu.projection_cache.clear();
        }
        let broad_span = profile.as_deref().map(|p| p.measure(41));
        cpu.projection_cache.resize_with(count, Default::default);
        for layer in 0..count {
            if cpu.initialized[layer] {
                let cascade = &cpu.prepared.cascades[layer];
                let clip = cascade.clip;
                let margin = cascade.depth_offset;
                let broad = if cache_queries {
                    let ParallelShadowCpu {
                        world_query,
                        projection_cache,
                        ..
                    } = &mut *cpu;
                    projection_cache[layer].query(clip, margin, verify_queries, || {
                        world_query
                            .as_ref()
                            .unwrap()
                            .query_projection(&clip, margin)
                    })
                } else {
                    Arc::new(
                        cpu.world_query
                            .as_ref()
                            .unwrap()
                            .query_projection(&clip, margin),
                    )
                };
                cpu.masks
                    .as_mut()
                    .unwrap()
                    .intersect_world_query(layer, &broad)?;
            }
        }
        drop(broad_span);
        if cache_queries
            && std::env::var_os("DH2_TRACE_SHADOW_QUERY_CACHE").is_some()
            && frame_count.0 % 300 == 0
        {
            eprintln!(
                "WORLD_PROJECTION_CACHE hits={} misses={}",
                cpu.projection_cache
                    .iter()
                    .map(|cache| cache.hits)
                    .sum::<u64>(),
                cpu.projection_cache
                    .iter()
                    .map(|cache| cache.misses)
                    .sum::<u64>()
            );
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
                cpu.static_frames[layer] = Some(Arc::new(StaticShadowFrame {
                    frame: frame_count.0,
                    mask: cpu.masks.as_ref().unwrap().masks()[layer].clone(),
                    clip: cpu.prepared.cascades[layer].clip,
                    local_time,
                }));
            }
        }
        Ok(ParallelCasterFrame {
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
                info!(target: "dh2::shadows", frame = frame.frame,
                    "Parallel CPU frame online: {} cascades, {} world caster selections",
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
                warn!(target: "dh2::shadows", frame = frame_count.0,
                    extent = ?cpu.last_shadow_extent, "Parallel CPU frame pending: {error}");
                cpu.last_error = Some(error);
            }
        }
    }
}
