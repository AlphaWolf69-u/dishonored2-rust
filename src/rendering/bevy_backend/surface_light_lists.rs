//! Per-tile light lists for opaque surface lighting, culled before the main
//! pass. lighting.renderprog walks each light type's tile list
//! (CreateLightContext/NextLightData, at most maxLights entries); the cull
//! runs before the original's main pass. The shared TLF owner in fog_scene
//! runs after alpha depth for fog and particles, too late for opaque shading,
//! so this owner culls the same packets against the prepass's opaque depth
//! (as both depth inputs). Approximation: the original also bins alpha depth,
//! so its lists can additionally hold lights reaching only alpha-object
//! depth bins in a tile; every light reaching an opaque pixel's bins is kept.
use super::{
    area_light_scene::AreaLightScene, fog_light_scattering::LocalFogFrame,
    fog::FogSceneFrame, shadow_pass::LocalShadowSceneFrame,
    tiled_lighting_gpu::TlfPipelines, tiled_lighting_pass::{TiledLightView, TiledLightingPass},
    water_scene::WaterSceneFrame,
};
use bevy::{
    core_pipeline::{core_3d::main_opaque_pass_3d, Core3d, Core3dSystems},
    prelude::*,
    render::{
        camera::ExtractedCamera,
        render_resource::{TextureAspect, TextureView, TextureViewDescriptor},
        renderer::{RenderContext, RenderQueue},
        sync_world::MainEntity,
        view::{ExtractedView, ViewDepthTexture},
        RenderApp,
    },
};

/// lighting.renderprog.decl:81-85: maxLights per type at HIGH_QUALITY, the
/// forward light quality the rewrite selects (global.decl).
const MAX_LIGHTS: u32 = 32;

#[derive(Resource, Default)]
struct SurfaceLightLists {
    pass: Option<TiledLightingPass>,
    depth: Option<(bevy::render::render_resource::TextureId, TextureView)>,
}

pub(super) fn register(app: &mut App) {
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.init_resource::<SurfaceLightLists>().add_systems(
            Core3d,
            cull_surface_lights
                .after(Core3dSystems::Prepass)
                .in_set(Core3dSystems::MainPass)
                .before(main_opaque_pass_3d),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn cull_surface_lights(
    frame: Res<FogSceneFrame>,
    views: Query<
        (&MainEntity, &ViewDepthTexture, &ExtractedView, &ExtractedCamera),
        With<super::fog::FogCamera>,
    >,
    (water_frame, programs, local_shadows): (
        Option<Res<WaterSceneFrame>>,
        Option<Res<TlfPipelines>>,
        Option<Res<LocalShadowSceneFrame>>,
    ),
    local_fog: Res<LocalFogFrame>,
    area_lights: Res<AreaLightScene>,
    queue: Res<RenderQueue>,
    mut owner: ResMut<SurfaceLightLists>,
    mut bindings: ResMut<bevy::pbr::AreaLightBindings>,
    mut context: RenderContext,
    mut last_error: Local<Option<String>>,
) {
    let result = (|| -> Result<(), String> {
        let frame = frame.0.as_ref().ok_or("surface light lists await a scene frame")?;
        let (main, depth, view, camera) = views.single().map_err(|error| error.to_string())?;
        if main.id() != frame.camera || local_fog.camera != Some(frame.camera) {
            return Err("surface light lists belong to another camera".into());
        }
        let full = camera.physical_viewport_size.ok_or("surface light viewport unavailable")?;
        let water = water_frame
            .as_ref()
            .and_then(|frame| frame.0.as_ref())
            .filter(|water| water.camera == frame.camera)
            .ok_or("surface light view matrices unavailable")?;
        let programs = programs.as_ref().ok_or("surface light culling programs unavailable")?;
        let local = local_shadows.as_ref().and_then(|frame| frame.0.as_ref());
        let shadows = if local_fog.lights.iter().any(|light| light[24] != u32::MAX) {
            let local = local.ok_or("surface light shadow records unavailable")?;
            if local_fog.shadow_revision != Some(local.revision) {
                return Err("surface light shadow indices belong to another frame".into());
            }
            local.receiver.as_ref().map_err(Clone::clone)?.shadows.as_slice()
        } else {
            &[]
        };
        if !area_lights.matches(&local_fog) {
            return Err("surface area lights await this frame's preparation".into());
        }
        let device = context.render_device().clone();
        let grid = crate::tiled_lighting::TileGrid::new(full.x, full.y, frame.settings.tiled_light_dimension)?;
        let pass = owner
            .pass
            .get_or_insert_with(|| TiledLightingPass::new(&device, programs, grid));
        pass.resize(&device, programs, grid);
        pass.area_pool = area_lights.pool.as_ref().map(|pool| pool.view.clone());
        let source = &depth.texture;
        let id = source.id();
        if owner.depth.as_ref().is_none_or(|(old, _)| *old != id) {
            owner.depth = Some((
                id,
                source.create_view(&TextureViewDescriptor {
                    aspect: TextureAspect::DepthOnly,
                    ..default()
                }),
            ));
        }
        let SurfaceLightLists { pass, depth: opaque } = &mut *owner;
        let (pass, opaque) = (pass.as_mut().unwrap(), &opaque.as_ref().unwrap().1);
        let matrices = water.matrices.current;
        pass.prepare(
            &device,
            &queue,
            programs,
            TiledLightView {
                uv_to_ray: Vec4::new(
                    0.,
                    0.,
                    1. / view.clip_from_view.x_axis.x,
                    1. / view.clip_from_view.y_axis.y,
                ),
                view: matrices.view.map(Vec4::from_array),
                projection: matrices.projection.map(Vec4::from_array),
                ..default()
            },
            opaque,
            opaque,
            area_lights.packets.as_slice(),
            shadows,
        )?;
        pass.record(context.command_encoder(), programs)?;
        bindings.publish_tiles(
            &queue,
            pass.lights.clone(),
            pass.normal_heads.clone(),
            pass.buffers.pool.clone(),
            [grid.dimensions[0], grid.lighting_shift, MAX_LIGHTS, 1],
        );
        Ok(())
    })();
    match result {
        Ok(()) => *last_error = None,
        Err(error) => {
            // Without this frame's lists opaque surfaces receive no area light.
            bindings.disable_tiles(&queue);
            if last_error.as_ref() != Some(&error) {
                warn!("Surface light lists pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}
