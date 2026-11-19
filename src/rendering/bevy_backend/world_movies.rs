//!arkInGameIggy world meshes belong to sortPostTonemap (model bucket41,
//!1403386a0), not the HDR alpha/OIT lists. Reuse their actual batched mesh
//!commands and material bindings; neither the movie nor its target is copied.
use bevy::{
    core_pipeline::{core_3d::Transparent3d, tonemapping::tonemapping, Core3d, Core3dSystems},
    prelude::*,
    render::{
        camera::ExtractedCamera,
        render_phase::{SortedRenderPhase, ViewSortedRenderPhases},
        render_resource::{CachedRenderPipelineId, PipelineCache, RenderPassDescriptor, StoreOp},
        renderer::{RenderContext, ViewQuery},
        view::{ExtractedView, RetainedViewEntity, ViewTarget, ViewDepthTexture},
        Render, RenderApp, RenderSystems,
    },
    shader::ShaderDefVal,
};
use hashbrown::HashMap;

#[derive(Resource, Default)]
struct Movies {
    phases: HashMap<RetainedViewEntity, SortedRenderPhase<Transparent3d>>,
    //Classification is immutable for an admitted pipeline, not repeated for
    //every material/draw each frame. The pipeline cache owns its lifetime.
    pipelines: HashMap<CachedRenderPipelineId, bool>,
}

pub(super) fn register(app: &mut App) {
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.init_resource::<Movies>()
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareBindGroups)
                .after(super::alpha_depth_scene::AlphaDepthPreparation))
            .add_systems(Core3d, draw.in_set(Core3dSystems::PostProcess)
                .after(super::display_scene::DisplayPass).before(tonemapping));
    }
}

pub(super) fn release_level(world: &mut World) {
    //No retained draw/material/view references cross the map lifetime. Global
    //pipeline classification stays with the application's pipeline cache.
    world.resource_mut::<Movies>().phases.clear();
}

fn prepare(mut movies: ResMut<Movies>, cache: Res<PipelineCache>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    views: Query<&ExtractedView>) {
    let Movies { phases: output, pipelines } = &mut *movies;
    output.retain(|view, _| views.iter().any(|current| current.retained_view_entity == *view));
    for phase in output.values_mut() { phase.clear(); }
    for view in &views {
        let Some(source) = phases.get_mut(&view.retained_view_entity) else { continue };
        let mut index = 0;
        while index < source.items.len() {
            let item = &source.items[index];
            let count = item.batch_range.len().max(1);
            let is_movie = if let Some(value) = pipelines.get(&item.pipeline) { *value }
                else if cache.get_render_pipeline(item.pipeline).is_some() {
                    let value = cache.get_render_pipeline_descriptor(item.pipeline).fragment.as_ref().is_some_and(|fragment|
                        fragment.shader_defs.iter().any(|value|
                            matches!(value, ShaderDefVal::Bool(name, true) if name == "DH2_IGGY3D")));
                    pipelines.insert(item.pipeline, value);
                    value
                } else { false };
            if is_movie && !item.batch_range.is_empty() {
                let selected = output.entry(view.retained_view_entity).or_default();
                for member in index..(index + count).min(source.items.len()) {
                    let item = &mut source.items[member];
                    selected.add_retained(Transparent3d {
                        sorting_info: item.sorting_info, distance: item.distance,
                        pipeline: item.pipeline, entity: item.entity,
                        draw_function: item.draw_function, batch_range: item.batch_range.clone(),
                        extra_index: item.extra_index.clone(), indexed: item.indexed,
                    });
                    //The retained source queue stays intact for the next
                    //frame; this batch draws only in the post-tone list.
                    item.batch_range.end = item.batch_range.start;
                }
            }
            index += count;
        }
    }
}

fn draw(world: &World, movies: Res<Movies>, mut context: RenderContext,
    view: ViewQuery<(&ExtractedView, &ViewTarget, &ViewDepthTexture, &ExtractedCamera)>) {
    let entity = view.entity();
    let (view, target, depth, camera) = view.into_inner();
    let Some(phase) = movies.phases.get(&view.retained_view_entity)
        .filter(|phase| !phase.items.is_empty()) else { return };
    let colours = [Some(target.get_color_attachment())];
    let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("post-tone world movies"), color_attachments: &colours,
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        ..Default::default()
    });
    if let Some(viewport) = camera.viewport.as_ref() { pass.set_camera_viewport(viewport); }
    if let Err(error) = phase.render(&mut pass, world, entity) {
        error!("World movie submission failed: {error:?}");
    }
}
