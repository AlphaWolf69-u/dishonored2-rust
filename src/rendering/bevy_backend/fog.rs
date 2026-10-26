//! World-fog GPU parameter transport. Producers retain ownership of their depth,
//! volume, sky/cloud and shadow resources; fog consumes those same-view inputs.
use crate::rendering::fog::world::{WorldFog, WorldFogEffects};
use bevy::{prelude::*, render::render_resource::ShaderType};

/// The gameplay camera that owns atmosphere, independently of shadow quality.
#[derive(Component, Clone, Copy, Default, bevy::render::extract_component::ExtractComponent)]
pub struct FogCamera;

/// Interior nullifier geometry participates in visibility and mesh streaming,
/// but has no forward colour material. Its only draw writes signed fog depth.
#[derive(Component, Clone, Copy)]
pub struct InteriorVolume;

#[derive(Component)]
pub struct InteriorInstance {
    pub mesh: bevy::asset::AssetId<Mesh>,
    pub world_from_local: Mat4,
}

impl bevy::render::sync_component::SyncComponent for InteriorVolume {
    type Target = InteriorInstance;
}
impl bevy::render::extract_component::ExtractComponent for InteriorVolume {
    type QueryData = (
        &'static Mesh3d,
        &'static GlobalTransform,
        &'static ViewVisibility,
    );
    type QueryFilter = With<InteriorVolume>;
    type Out = InteriorInstance;
    fn extract_component(
        (mesh, transform, visible): bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>,
    ) -> Option<Self::Out> {
        visible.get().then(|| InteriorInstance {
            mesh: mesh.0.id(),
            world_from_local: transform.to_matrix(),
        })
    }
}

/// A complete game-thread atmosphere publication. Matrices, exposure and depth
/// attachments are supplied by its matching render-world camera.
#[derive(Clone)]
pub struct SunSpriteFrame {
    pub parameters: crate::rendering::fog::sun_sprite::SunSpriteParameters,
    pub image: Handle<Image>,
    pub state: crate::render_state::RenderState,
}

#[derive(Clone)]
pub struct AreaFogFrame {
    pub meshes: Vec<Handle<Mesh>>,
    pub world_from_local: Mat4,
    pub origin: Vec3,
    /// Whole render-model bounds in game coordinates, used by 324be0's
    /// per-object alpha selection. These are independent of trigger hulls.
    pub bounds: [[f32; 3]; 2],
    pub parameters: crate::rendering::fog::alpha::VolumeFog,
}

#[derive(Clone, Copy)]
pub struct AreaFogStates {
    pub interval: crate::render_state::RenderState,
    pub radiance: crate::render_state::RenderState,
}

#[derive(Clone)]
pub struct FogLookupOverrideFrame {
    pub images: [Handle<Image>; 4],
    pub weights: Vec4,
}
impl FogLookupOverrideFrame {
    /// Publish 88e220's prepared image groups. The caller supplies the actual
    /// default image that 3110e0 resolves for an empty resource slot.
    pub fn from_environment(
        blend: crate::rendering::environment_images::ImageOverrideBlend<Handle<Image>>,
        default_image: &Handle<Image>,
    ) -> Option<Self> {
        blend.is_active().then(|| Self {
            images: blend
                .images
                .map(|image| image.unwrap_or_else(|| default_image.clone())),
            weights: Vec4::from_array(blend.weights),
        })
    }
}

#[derive(Clone)]
pub struct FogFrame {
    pub shadow: FogShadowFrame,
    pub revision: u64,
    pub camera: Entity,
    pub parameters: FogParameters,
    pub canvas: FogCanvas,
    pub light_canvas: Option<FogCanvas>,
    pub eye: Vec3,
    pub sky_masking_factor: f32,
    pub curve: Handle<Image>,
    pub curve_weight: f32,
    pub curve_overrides: Option<FogLookupOverrideFrame>,
    pub noise: Handle<Image>,
    pub settings: crate::platform::game_settings::FogRenderSettings,
    pub rays: crate::rendering::fog::rays::RaySettings,
    pub sun_sprite: Option<SunSpriteFrame>,
    pub areas: Vec<AreaFogFrame>,
    pub area_states: Option<AreaFogStates>,
}

#[derive(Clone)]
pub struct FogShadowFrame {
    pub image: Handle<Image>,
    pub extent: f32,
    pub projected: bool,
    pub temporal: bool,
}
#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
pub struct FogSceneFrame(pub Option<FogFrame>);

pub struct FogPlugin;
impl Plugin for FogPlugin {
    fn build(&self, app: &mut App) {
        super::fog_material::register_shaders(app);
        app.init_resource::<FogSceneFrame>();
        app.init_resource::<super::sun_rays::SunRaySources>();
        app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<super::sun_rays::SunRaySources>::default());
        app.init_resource::<super::fog_light_scattering::LocalFogFrame>();
        app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<super::fog_light_scattering::LocalFogFrame>::default());
        app.add_plugins(bevy::render::extract_component::ExtractComponentPlugin::<
            FogCamera,
        >::default());
        super::fog_scene::register(app);
        super::fog_capture::register(app);
        app.init_asset::<super::sky_mask::SkyMaterial>();
        app.add_plugins(bevy::render::render_asset::RenderAssetPlugin::<
            super::sky_mask::GpuSkyMaterial,
            bevy::render::texture::GpuImage,
        >::default());
        if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
            use bevy::render::GpuResourceAppExt;
            render.init_gpu_resource::<super::sky_mask::SkyProgram>();
        }
        app.add_plugins(bevy::render::extract_component::ExtractComponentPlugin::<
            super::sky_mask::SkyMesh,
        >::default());
        app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
            FogSceneFrame,
        >::default());
        app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
            super::fog_material::FogMaterialResources,
        >::default());
        app.add_plugins(bevy::render::extract_component::ExtractComponentPlugin::<
            InteriorVolume,
        >::default());
    }
}

/// Camera and resource dimensions consumed by the fullscreen fog passes.
/// Matrices use the renderer's X-right/Y-up basis; `eye` uses game Z-up.
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct FogView {
    pub clip_from_world: Mat4,
    pub world_from_view: Mat4,
    pub world_from_clip: Mat4,
    pub ray_scale: Vec4,
    pub unproject: Vec4,
    pub extent: Vec4,
    pub reduced_extent: Vec4,
    pub eye: Vec4,
    pub jitter: Vec4,
}

impl FogView {
    /// This backend's world basis is X-right/Y-up; atmosphere inputs are Z-up.
    pub fn clip_from_game_world(&self) -> Mat4 {
        self.clip_from_world * Mat4::from_cols(Vec4::X, -Vec4::Z, Vec4::Y, Vec4::W)
    }
    /// The installed depth shader uses logical inverse-projection rows 2/3.
    /// glam stores columns, so these three terms deliberately cross axes.
    pub fn depth_coefficients(inverse_projection: Mat4, exposure: f32) -> Vec4 {
        Vec4::new(
            -inverse_projection.w_axis.z,
            inverse_projection.z_axis.w,
            inverse_projection.w_axis.w,
            exposure,
        )
    }
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct FogCanvas {
    pub s: Vec4,
    pub t: Vec4,
    pub octaves: [Vec4; 10],
}

impl From<&crate::rendering::fog::canvas::CanvasParameters> for FogCanvas {
    fn from(parameters: &crate::rendering::fog::canvas::CanvasParameters) -> Self {
        Self {
            s: parameters.s,
            t: parameters.t,
            octaves: parameters.octaves.map(|octave| octave.extend(0.)),
        }
    }
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct FogParameters {
    pub distance: Vec4,
    pub distance_start: Vec4,
    pub distance_end: Vec4,
    pub vertical: Vec4,
    pub vertical_start: Vec4,
    pub vertical_end: Vec4,
    pub height: Vec4,
    pub scattering_direction_phase: Vec4,
    pub scattering_color_intensity: Vec4,
    pub effects: Vec4,
    pub turbulence: Vec4,
    pub turbulence_color: Vec4,
    pub flags: UVec4,
}

impl FogParameters {
    pub fn new(fog: WorldFog, effects: WorldFogEffects) -> Self {
        Self {
            distance: fog.distance,
            distance_start: fog.distance_start,
            distance_end: fog.distance_end,
            vertical: fog.vertical,
            vertical_start: fog.vertical_start,
            vertical_end: fog.vertical_end,
            height: Vec4::new(fog.height_start, fog.height_end, fog.density_falloff, 0.),
            scattering_direction_phase: fog.scattering_direction.extend(fog.scattering_phase),
            scattering_color_intensity: fog.scattering_color.extend(fog.scattering_intensity),
            effects: Vec4::new(
                effects.far_clip,
                effects.scattering_depth_start,
                effects.god_ray_shadow,
                0.,
            ),
            turbulence: Vec4::new(effects.turbulence_start, effects.turbulence_end, 0., 0.),
            turbulence_color: effects.turbulence_color.extend(0.),
            flags: UVec4::new(fog.flags, 0, 0, 0),
        }
    }
}
