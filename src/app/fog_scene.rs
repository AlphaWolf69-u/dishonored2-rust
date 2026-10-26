//! Level-owned atmosphere inputs and resident images, loaded from archives.
use super::{
    archive,
    texture_streaming::{read_streamed_diffuse_at_target, TextureReadTarget},
};
use crate::rendering::{
    environment::AreaEnvironment,
    environment_parameters::{EnvironmentParameters, EnvironmentTracks, EnvironmentValue},
    environment_profiles::{EnvironmentProfiles, PreparedEnvironmentProfile},
    fog::settings::FogSettings,
};
use bevy::prelude::*;
use std::collections::HashMap;

/// 2ff520 consumes the game's forward/left/up camera axes at 3a9c/3aa8/3ab4.
/// Host +X points right; the native left axis is host -X, as in view history.
pub(super) fn canvas_camera_view(
    rotation: Quat,
    projection: [[f32; 4]; 4],
    position_delta: Vec3,
    inverse_frequency: f32,
    focus_distance: f32,
) -> crate::rendering::fog::canvas::CanvasView {
    let [forward, left, up] = [Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y]
        .map(|axis| Vec3::from_array(super::view_to_game(rotation * axis)));
    crate::rendering::fog::canvas::CanvasView {
        position_delta,
        forward,
        left,
        up,
        // 2ff520 reads render-view +3a70 (vertical FOV), whereas the
        // area-volume LOD calculation below uses +3a6c (horizontal FOV).
        fov_radians: 2. * (1. / projection[1][1]).atan(),
        inverse_frequency,
        focus_distance,
    }
}

fn prepare_image(
    catalog: &archive::Catalog,
    name: &str,
    images: &mut Assets<Image>,
    resident: &mut HashMap<String, Handle<Image>>,
    aliases: &mut HashMap<String, Handle<Image>>,
) -> Result<Handle<Image>, String> {
    if let Some(handle) = aliases.get(name) {
        return Ok(handle.clone());
    }
    // Preserve authored image option suffixes and accept compiled BIM paths.
    let path = crate::material::image_load_resource(catalog, name)
        .ok_or_else(|| format!("unresolved fog image source {name}"))?;
    if path == "_default" && !name.eq_ignore_ascii_case("_default") {
        warn!("Fog image {name}: resource missing; using the image loader's default texture");
    }
    let handle = if let Some(handle) = resident.get(&path) {
        handle.clone()
    } else {
        let mut loaded =
            read_streamed_diffuse_at_target(catalog, &path, TextureReadTarget::Initial(Some(0)))?
                .ok_or_else(|| format!("missing resident fog image {path}"))?;
        loaded.image.data = Some(loaded.uploads.into_iter().flat_map(|mip| mip.payload).collect());
        let handle = images.add(loaded.image);
        resident.insert(path, handle.clone());
        handle
    };
    aliases.insert(name.to_owned(), handle.clone());
    aliases.insert(name.trim_end_matches(".curvefloat").to_owned(), handle.clone());
    Ok(handle)
}

#[derive(Resource)]
pub(super) struct FogScene {
    shadow_options: [crate::rendering::bevy_backend::fog_shadow::FogShadowMaterialOptions; 2],
    default_shadow: Handle<Image>,
    sun_program: crate::render_state::RenderState,
    ray_program: crate::render_state::RenderState,
    world: EnvironmentParameters,
    areas: Vec<EnvironmentParameters>,
    tracks: EnvironmentTracks,
    lookup: crate::rendering::environment_tracks::FogLookupTrack<Option<String>>,
    world_lookup: Option<String>,
    lookup_initialized: bool,
    sun: Option<(Vec3, Vec3)>,
    images: HashMap<String, Handle<Image>>,
    resident: HashMap<String, Handle<Image>>,
    profiles: EnvironmentProfiles<Handle<Image>>,
    volumes: Vec<super::fog_volumes::AreaFog>,
    area_states: crate::rendering::bevy_backend::fog::AreaFogStates,
}

impl FogScene {
    pub(super) fn load(
        catalog: &archive::Catalog,
        programs: &mut crate::assets::shader_cache::ProgramLibrary,
        world: &crate::entities::EnvironmentRenderParameters,
        areas: &[AreaEnvironment],
        sun: Option<(Vec3, Vec3)>,
        far_clip: f32,
        images: &mut Assets<Image>,
    ) -> Result<Self, String> {
        let mut shadow_options = Vec::new();
        for path in [
            "generated/decls/material/atm/fogshadow/generate.material.decl",
            "generated/decls/material/atm/fogshadow/generate_shadowhmap.material.decl",
        ] {
            let bytes = catalog
                .read(path)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("missing fog shadow material {path}"))?;
            let program = crate::material::authored_program_override(&bytes, "mainProgram")
                .ok_or_else(|| format!("fog shadow material has no main program: {path}"))?;
            let overrides = super::material_loading::material_option_overrides(programs, &bytes)?;
            shadow_options.push(
                crate::rendering::bevy_backend::fog_shadow::FogShadowMaterialOptions {
                    projected: programs.option_value(
                        program.as_bytes(),
                        &overrides,
                        b"atm/distanceFog/useProjectedShadowHeightMap",
                    )? != 0,
                    temporal: programs.option_value(
                        program.as_bytes(),
                        &overrides,
                        b"atm/worldFog/useTemporal",
                    )? != 0,
                },
            );
        }
        let shadow_options: [_; 2] = shadow_options
            .try_into()
            .map_err(|_| "fog shadow material count")?;
        let sun_program = programs
            .definition(b"PostFX_Sun_Sprite")?
            .ok_or("missing PostFX_Sun_Sprite program definition")?
            .render_state();
        let ray_program = programs.definition(b"PostFX_GodRays_Scattering")?
            .ok_or("missing PostFX_GodRays_Scattering definition")?.render_state();
        let area_states = crate::rendering::bevy_backend::fog::AreaFogStates {
            interval: programs
                .definition(b"atm/distanceFog/renderMinMax")?
                .ok_or("missing area fog interval program")?
                .render_state(),
            radiance: programs
                .definition(b"atm/distanceFog/renderVolume")?
                .ok_or("missing area fog radiance program")?
                .render_state(),
        };
        let mut prepared = EnvironmentParameters::capture(catalog, world, |read| {
            FogSettings::resolve(read, sun, far_clip)
        })?;
        let masking = crate::assets::environment::ParameterReader {
            catalog,
            parameters: world,
        }
        .vector("env/atm/sky/maskingfactor")?;
        prepared.values.insert(
            "env/atm/sky/maskingfactor".into(),
            EnvironmentValue::Float(Vec4::from_array(masking)),
        );
        let world_lookup = world.parameter("atm/worlddistancefog/colorlut")
            .and_then(|_|match prepared.values.get("atm/worlddistancefog/colorlut") {
                Some(EnvironmentValue::Image(image))=>Some(image.clone()),_=>None,
            });
        let world = prepared;
        let defaults = EnvironmentParameters::defaults(catalog, &world)?;
        let areas = areas
            .iter()
            .map(|area| EnvironmentParameters::supplied(catalog, &area.parameters, &world))
            .collect::<Result<Vec<_>, _>>()?;
        let mut resident = HashMap::<String, Handle<Image>>::new();
        let mut aliases = HashMap::new();
        for source in std::iter::once(&world)
            .chain(std::iter::once(&defaults))
            .chain(areas.iter())
        {
            for value in source.values.values() {
                let EnvironmentValue::Image(name) = value else {
                    continue;
                };
                prepare_image(catalog,name,images,&mut resident,&mut aliases)?;
            }
        }
        let EnvironmentValue::Image(default_shadow) = &defaults.values["ocean/projectedshadowheightmap"]
        else {
            return Err("projected fog shadow default is not an image".into());
        };
        let default_shadow = aliases
            .get(default_shadow)
            .ok_or("unprepared projected fog shadow default")?
            .clone();
        let fallback=super::texture_streaming::load_resident_2d(catalog,"_default",images,&mut resident)?;
        aliases.insert("_default".into(),fallback);
        let registered_curve=match defaults.values.get("atm/worlddistancefog/colorlut") {
            Some(EnvironmentValue::Image(image))=>Some(image.clone()),
            _=>return Err("fog curve registered default is not an image".into()),
        };
        Ok(Self {
            shadow_options,
            default_shadow,
            volumes: Vec::new(),
            area_states,
            sun_program,
            ray_program,
            profiles: EnvironmentProfiles::new(&world),
            world,
            areas,
            tracks: EnvironmentTracks::new(&defaults),
            lookup:crate::rendering::environment_tracks::FogLookupTrack::new(registered_curve,None,0.),
            world_lookup,
            lookup_initialized:false,
            sun,
            images: aliases,
            resident,
        })
    }

    /// Called by effect preparation, never by the per-frame fog update. Image
    /// aliases and residency are shared with this level's ordinary atmosphere.
    pub(super) fn prepare_profile(
        &mut self,
        catalog: &archive::Catalog,
        name: &str,
        images: &mut Assets<Image>,
    ) -> Result<std::sync::Arc<PreparedEnvironmentProfile<Handle<Image>>>, String> {
        let resident = &mut self.resident;
        let aliases = &mut self.images;
        self.profiles.load(catalog,name,|_,source| prepare_image(catalog,source,images,resident,aliases))
    }

    pub(super) fn update(
        &mut self,
        areas: &[AreaEnvironment],
        world_duration: f32,
        position: [f32; 3],
        delta: f32,
        far_clip: f32,
    ) -> Result<FogSettings, String> {
        if areas.len() != self.areas.len() {
            return Err("fog area environments changed without reloading level inputs".into());
        }
        let active = areas
            .iter()
            .enumerate()
            .filter(|(_, area)| area.contains(position))
            .map(|(index, area)| (index, &self.areas[index], area.blend_duration))
            .collect::<Vec<_>>();
        let mut selection=self.world_lookup.clone().map(|image|
            (crate::rendering::environment::EnvironmentSource::World,Some(image)));
        let mut duration=world_duration;
        for (index,area,blend_duration) in &active {
            if let Some(EnvironmentValue::Image(image))=area.values.get("atm/worlddistancefog/colorlut") {
                selection=Some((crate::rendering::environment::EnvironmentSource::Area(*index),Some(image.clone())));
                duration=*blend_duration;
            }
        }
        self.lookup.update(selection,duration,delta,self.lookup_initialized);
        self.lookup_initialized=true;
        let frame = self
            .tracks
            .update(&self.world, world_duration, &active, delta)?;
        FogSettings::resolve(frame, self.sun, far_clip)
    }

    pub(super) fn image(&self, source: &str) -> Result<Handle<Image>, String> {
        self.images
            .get(source)
            .cloned()
            .ok_or_else(|| format!("fog image was not prepared: {source}"))
    }
    pub(super) fn lookup_frame(&self)->Result<(Handle<Image>,f32),String> {
        let target=self.lookup.target().as_deref().unwrap_or("_default");
        self.image(target).map(|image|(image,self.lookup.weight()))
    }

    pub(super) fn shadow_frame(
        &self,
        settings: &crate::rendering::fog::settings::FogShadowSettings,
    ) -> Result<crate::rendering::bevy_backend::fog::FogShadowFrame, String> {
        let image = self.image(&settings.image)?;
        let options = self.shadow_options[usize::from(image.id() != self.default_shadow.id())];
        Ok(crate::rendering::bevy_backend::fog::FogShadowFrame {
            projected: options.projected,
            image,
            extent: settings.extent,
            temporal: options.temporal,
        })
    }

    pub(super) fn upload_volumes(
        &mut self,
        sources: Vec<super::fog_volumes::AreaFogSource>,
        meshes: &mut Assets<Mesh>,
    ) {
        self.volumes = sources
            .into_iter()
            .map(|source| source.upload(meshes))
            .collect();
    }

    fn volume_frames(
        &self,
        view: &super::fog_volumes::AreaFogView,
        state: Option<&crate::tome::RenderQueryState>,
    ) -> Result<Vec<crate::rendering::bevy_backend::fog::AreaFogFrame>, String> {
        let eye = Vec3::from_array(view.eye);
        let query = state
            .map(crate::tome::RenderAabbQuery::from_state)
            .transpose()?;
        let mut frames = Vec::new();
        for volume in &self.volumes {
            if volume.visible(view, query.as_ref())? {
                frames.push(volume.frame(eye, self.sun)?);
            }
        }
        // 30b750 treats equal/unordered distances like its descending branch.
        crate::world_mesh::sort_shadow_records(&mut frames, |a, b| {
            Some(
                if a.origin.distance_squared(eye) < b.origin.distance_squared(eye) {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                },
            )
        });
        Ok(frames)
    }

    pub(super) fn masking_factor(&self) -> Result<f32, String> {
        use crate::assets::environment::ParameterSource;
        self.tracks.current().scalar("env/atm/sky/maskingfactor")
    }
}

/// Camera wind is registered only by the world entity's attached component.
#[derive(Resource)]
pub(super) struct WorldWind(pub(super) Option<crate::gameplay::wind::Wind>);
impl WorldWind {
    pub(super) fn load(
        catalog: &archive::Catalog,
        document: Option<&crate::entities::Document>,
    ) -> Result<Self, String> {
        let settings = document
            .map(|document| {
                crate::entities::wind::world_wind(document, |name| {
                    catalog
                        .read(name)
                        .map(|bytes| {
                            bytes
                                .map(|bytes| crate::entities::declaration_text(&bytes).into_owned())
                        })
                        .map_err(|error| error.to_string())
                })
            })
            .transpose()?
            .flatten();
        Ok(Self(settings.map(crate::gameplay::wind::Wind::new)))
    }
    pub(super) fn camera_velocity(&self) -> Vec3 {
        self.0
            .as_ref()
            .map(|wind| {
                let velocity = wind.velocity(Vec3::ZERO);
                // 8ac710 uses this unit-X vector when the registered wind returns
                // exactly zero. It does not substitute a zero displacement.
                if velocity == Vec3::ZERO {
                    Vec3::X
                } else {
                    velocity
                }
            })
            .unwrap_or(Vec3::X) // 374730 initializes renderView.m_worldWind to unit X.
    }
}

pub(super) fn update_world_wind(
    clock: Res<crate::gameplay::game_clock_runtime::ClockRuntime>,
    mut wind: ResMut<WorldWind>,
) {
    if !clock.advanced() {
        return;
    }
    if let (Ok(clock), Some(wind)) = (clock.clock(), wind.0.as_mut()) {
        wind.advance(clock.delta(3).expect("five clock groups"));
    }
}

#[derive(Resource)]
pub(super) struct FogMotion {
    canvas: crate::rendering::fog::canvas::DynamicCanvas,
    light_canvas: crate::rendering::fog::canvas::DynamicCanvas,
    previous_position: Option<Vec3>,
    revision: u64,
    settings: crate::platform::game_settings::FogRenderSettings,
}
impl FogMotion {
    pub(super) fn load() -> Result<Self, String> {
        let (_, settings) = crate::platform::game_settings::read_fog_render_settings()?;
        let counter = crate::platform::performance_counter::counter()? as u64;
        Ok(Self {
            canvas: crate::rendering::fog::canvas::DynamicCanvas::new(counter),
            light_canvas: crate::rendering::fog::canvas::DynamicCanvas::new(
                crate::platform::performance_counter::counter()? as u64,
            ),
            previous_position: None,
            revision: 0,
            settings,
        })
    }
}

pub(super) fn publish_fog_frame(
    scene: Res<super::SceneData>,
    mut object_sources: super::fog_objects::FogObjectSources,
    view_settings: Res<super::ViewSettings>,
    mut fog: Option<ResMut<FogScene>>,
    mut motion: ResMut<FogMotion>,
    wind: Res<WorldWind>,
    clock: Res<crate::gameplay::game_clock_runtime::ClockRuntime>,
    cameras: Query<(Entity, &Transform, &Projection, &Camera), With<Camera3d>>,
    mut material: ResMut<crate::rendering::bevy_backend::fog_material::FogMaterialResources>,
    mut images: ResMut<Assets<Image>>,
    mut probe_materials: ResMut<Assets<super::ProbeMaterial>>,
    mut decal_materials: ResMut<Assets<super::DecalMaterial>>,
    mut output: ResMut<crate::rendering::bevy_backend::fog::FogSceneFrame>,
    mut ray_sources: ResMut<crate::rendering::bevy_backend::sun_rays::SunRaySources>,
    mut last_error: Local<Option<String>>,
) {
    use crate::rendering::bevy_backend::fog::{FogCanvas, FogFrame, FogParameters};
    let Some(fog) = fog.as_mut() else {
        output.0 = None;
        ray_sources.0 = None;
        return;
    };
    ray_sources.0 = Some(fog.ray_program);
    if !clock.advanced() {
        return;
    }
    output.0 = None;
    material.object_frame = None;
    let result = (|| -> Result<FogFrame, String> {
        let clock = clock.clock().map_err(str::to_owned)?;
        let delta = clock.delta(3).ok_or("fog gameplay clock unavailable")?;
        let (camera, transform, projection, camera_view) =
            cameras.single().map_err(|error| error.to_string())?;
        let full = camera_view
            .physical_viewport_size()
            .ok_or("fog camera viewport unavailable")?;
        let extents = crate::rendering::bevy_backend::fog_targets::FogExtents::new(
            full,
            motion.settings.downsample,
        )?;
        material.resize(&mut images, extents)?;
        let Projection::Custom(custom) = projection else {
            return Err("fog camera projection unavailable".into());
        };
        let projection = custom
            .get::<super::camera_projection::GameProjection>()
            .ok_or("fog camera projection type unavailable")?;
        let position = Vec3::from_array(super::view_to_game(transform.translation));
        let previous = motion
            .previous_position
            .replace(position)
            .ok_or("fog camera history awaits its second observation")?;
        let settings = fog.update(
            &scene.area_environments,
            scene.environment_blend_duration,
            position.to_array(),
            delta,
            projection.far,
        )?;
        let matrix = projection.rows();
        let clip_rows = crate::tome::render_query_base_rows(
            matrix,
            super::game_pose_from_view_transform(transform),
        );
        let area_view = super::fog_volumes::AreaFogView {
            eye: position.to_array(),
            viewport: full.to_array(),
            clip_rows,
            // Renderer+3a6c is the horizontal FOV; 374e40 reconstructs it
            // from the vertical FOV and viewport aspect before publication.
            fov_radians: 2. * (1. / matrix[0][0]).atan(),
            lod_scale: view_settings
                .0
                .as_ref()
                .map_err(Clone::clone)?
                .model_fade_lod_scale,
        };
        let canvas = motion.canvas.update(canvas_camera_view(
            transform.rotation,
            matrix,
            position - previous - wind.camera_velocity() * delta,
            settings.inverse_frequency,
            settings.focus_distance,
        ));
        let light_canvas = motion.light_canvas.update(canvas_camera_view(
            transform.rotation,
            matrix,
            position - previous - settings.light_canvas.wind * delta,
            settings.light_canvas.inverse_frequency,
            settings.light_canvas.focus_distance,
        ));
        motion.revision = motion
            .revision
            .checked_add(1)
            .ok_or("fog frame revision overflow")?;
        let (curve,curve_weight)=fog.lookup_frame()?;
        let frame = FogFrame {
            shadow: fog.shadow_frame(&settings.shadow)?,
            areas: fog.volume_frames(
                &area_view,
                object_sources
                    .queue
                    .shadow_ordinary_query
                    .as_ref()
                    .and_then(|output| output.state.as_ref()),
            )?,
            area_states: Some(fog.area_states),
            revision: motion.revision,
            camera,
            parameters: FogParameters::new(settings.fog, settings.effects),
            canvas: FogCanvas::from(&canvas),
            light_canvas: Some(FogCanvas::from(&light_canvas)),
            eye: position,
            sky_masking_factor: fog.masking_factor()?,
            curve,
            curve_weight,
            // Scripted/external environment-curve controllers are a separate
            // producer; ordinary camera-area tracks do not synthesize overlays.
            curve_overrides:None,
            noise: fog.image(&settings.noise_map)?,
            settings: motion.settings,
            rays: settings.rays,
            sun_sprite: if settings.sun_sprite.parameters.enabled {
                Some(crate::rendering::bevy_backend::fog::SunSpriteFrame {
                    parameters: settings.sun_sprite.parameters,
                    image: fog.image(&settings.sun_sprite.image)?,
                    state: fog.sun_program,
                })
            } else {
                None
            },
        };
        let view_z = crate::tome::render_query_base_rows(
            Mat4::IDENTITY.to_cols_array_2d(),
            super::game_pose_from_view_transform(transform),
        )[2];
        let objects = super::fog_objects::prepare(
            &frame,
            &scene,
            &object_sources,
            &probe_materials,
            &decal_materials,
            view_z,
        )?;
        material.publish_objects(objects, &mut object_sources.buffers)?;
        Ok(frame)
    })();
    match result {
        Ok(frame) => {
            if let Some(bindings) = material.publish_bindings() {
                for (_, value) in probe_materials.iter_mut() {
                    let value = &mut value.extension;
                    value.fog_parameters = bindings.parameters.clone();
                    value.fog_objects = bindings.objects.clone();
                    value.fog_world = bindings.world.clone();
                    value.fog_canvas = bindings.canvas.clone();
                    value.fog_curve = bindings.curve.clone();
                    value.fog_depth = bindings.opaque_depth.clone();
                }
                for (_, value) in decal_materials.iter_mut() {
                    let value = &mut value.extension;
                    value.fog_parameters = bindings.parameters.clone();
                    value.fog_objects = bindings.objects.clone();
                    value.fog_world = bindings.world.clone();
                    value.fog_canvas = bindings.canvas.clone();
                    value.fog_curve = bindings.curve.clone();
                    value.fog_depth = bindings.opaque_depth.clone();
                }
            }
            output.0 = Some(frame);
            *last_error = None;
        }
        Err(error) => {
            if last_error.as_ref() != Some(&error) {
                warn!("Fog frame pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}
