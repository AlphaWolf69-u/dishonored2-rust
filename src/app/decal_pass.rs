//! Host submission for ordinary decals: preserve their blending and batching,
//! but draw them in the game's opaque interval, before atmosphere composition.
use super::DecalMaterial;
use bevy::{
    camera::{MainPassResolutionOverride, Viewport},
    core_pipeline::{
        core_3d::{main_opaque_pass_3d, main_transparent_pass_3d, Transparent3d},
        Core3d, Core3dSystems,
    },
    prelude::*,
    render::{
        camera::ExtractedCamera,
        render_phase::{SortedRenderPhase, ViewSortedRenderPhases},
        render_resource::{RenderPassDescriptor, StoreOp},
        renderer::{RenderContext, ViewQuery},
        sync_world::MainEntity,
        view::{ExtractedView, RetainedViewEntity, ViewDepthTexture, ViewTarget},
        Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
    },
};
use std::collections::{HashMap, HashSet};

pub(super) struct DecalPassPlugin;
impl Plugin for DecalPassPlugin {
    fn build(&self, app: &mut App) {
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render
            .init_resource::<DecalEntities>()
            .init_resource::<DecalPhases>()
            .add_systems(ExtractSchedule, extract)
            .add_systems(
                Render,
                partition.in_set(RenderSystems::PrepareBindGroups).before(
                    crate::rendering::bevy_backend::alpha_depth_scene::AlphaDepthPreparation,
                ),
            )
            .add_systems(Render, restore.in_set(RenderSystems::Cleanup))
            .add_systems(
                Core3d,
                draw.in_set(Core3dSystems::MainPass)
                    .after(main_opaque_pass_3d)
                    .before(main_transparent_pass_3d)
                    .before(crate::rendering::bevy_backend::fog_scene::AtmosphereDepthPass),
            );
    }
}

#[derive(Resource, Default)]
struct DecalEntities(HashSet<MainEntity>);
#[derive(Resource, Default)]
struct DecalPhases(HashMap<RetainedViewEntity, SortedRenderPhase<Transparent3d>>);

fn extract(
    mut decals: ResMut<DecalEntities>,
    entities: Extract<Query<(Entity, &MeshMaterial3d<DecalMaterial>)>>,
    materials: Extract<Res<Assets<DecalMaterial>>>,
) {
    decals.0.clear();
    decals
        .0
        .extend(entities.iter().filter_map(|(entity, handle)| {
            materials
                .get(&handle.0)
                .is_some_and(|m| m.extension.surface.misc.w & (1 << 9) != 0)
                .then_some(MainEntity::from(entity))
        }));
}

fn copy_item(item: &Transparent3d) -> Transparent3d {
    Transparent3d {
        sorting_info: item.sorting_info,
        distance: item.distance,
        pipeline: item.pipeline,
        entity: item.entity,
        draw_function: item.draw_function,
        batch_range: item.batch_range.clone(),
        extra_index: item.extra_index.clone(),
        indexed: item.indexed,
    }
}

fn partition(
    decals: Res<DecalEntities>,
    mut early: ResMut<DecalPhases>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
) {
    early.0.clear();
    for (view, phase) in phases.iter_mut() {
        let mut selected = SortedRenderPhase::default();
        let mut index = 0;
        while index < phase.items.len() {
            let item = &phase.items[index];
            let count = item.batch_range.len().max(1);
            if !item.batch_range.is_empty() && decals.0.contains(&item.entity.1) {
                // Keep every batch member so SortedRenderPhase's instance-count
                // skip remains valid. The pipeline key separates decal routes.
                for member in index..index + count {
                    let item = &mut phase.items[member];
                    selected.add_retained(copy_item(item));
                    item.batch_range = 0..0;
                }
            }
            index += count;
        }
        if !selected.items.is_empty() {
            early.0.insert(*view, selected);
        }
    }
}

fn restore(mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>, early: Res<DecalPhases>) {
    // The host phase retains items across frames. Restore only the per-frame
    // draw ranges after submission; visibility and material ownership stay there.
    for (view, selected) in &early.0 {
        if let Some(phase) = phases.get_mut(view) {
            for (key, item) in &selected.items {
                if let Some(retained) = phase.items.get_mut(key) {
                    retained.batch_range = item.batch_range.clone();
                }
            }
        }
    }
}

fn draw(
    world: &World,
    view: ViewQuery<(
        &ExtractedCamera,
        &ExtractedView,
        &ViewTarget,
        &ViewDepthTexture,
        Option<&MainPassResolutionOverride>,
    )>,
    decals: Res<DecalPhases>,
    mut context: RenderContext,
) {
    let entity = view.entity();
    let (camera, extracted, target, depth, resolution_override) = view.into_inner();
    let Some(phase) = decals.0.get(&extracted.retained_view_entity) else {
        return;
    };
    let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("opaque decals before atmosphere"),
        color_attachments: &[Some(target.get_color_attachment())],
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        ..Default::default()
    });
    if let Some(viewport) =
        Viewport::from_viewport_and_override(camera.viewport.as_ref(), resolution_override)
    {
        pass.set_camera_viewport(&viewport);
    }
    if let Err(error) = phase.render(&mut pass, world, entity) {
        error!("Decal submission failed: {error:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        core_pipeline::core_3d::TransparentSortingInfo3d,
        ecs::system::RunSystemOnce,
        render::{
            render_phase::{
                Draw, DrawError, DrawFunctions, PhaseItemExtraIndex, TrackedRenderPass,
            },
            render_resource::CachedRenderPipelineId,
        },
    };

    struct UnusedDraw;
    impl Draw<Transparent3d> for UnusedDraw {
        fn draw<'w>(
            &mut self,
            _: &'w World,
            _: &mut TrackedRenderPass<'w>,
            _: Entity,
            _: &Transparent3d,
        ) -> Result<(), DrawError> {
            unreachable!()
        }
    }

    #[test]
    fn decal_batches_render_once_and_retain_their_next_frame_ranges() {
        let mut world = World::new();
        let entities: Vec<_> = (0..4).map(|_| world.spawn_empty().id()).collect();
        let view = RetainedViewEntity::new(MainEntity::from(entities[3]), None, 0);
        let draw_functions = DrawFunctions::<Transparent3d>::default();
        let draw_function = draw_functions.write().add(UnusedDraw);
        let mut phase = SortedRenderPhase::default();
        for (index, &entity) in entities[..3].iter().enumerate() {
            phase.add_retained(Transparent3d {
                sorting_info: TransparentSortingInfo3d::AlwaysOnTop,
                distance: index as f32,
                pipeline: CachedRenderPipelineId::INVALID,
                entity: (entity, MainEntity::from(entity)),
                draw_function,
                batch_range: match index {
                    0 => 5..7,
                    1 => 0..0,
                    _ => 7..8,
                },
                extra_index: PhaseItemExtraIndex::None,
                indexed: true,
            });
        }
        let mut phases = ViewSortedRenderPhases::<Transparent3d>::default();
        phases.insert(view, phase);
        world.insert_resource(phases);
        world.insert_resource(DecalEntities(
            entities[..2]
                .iter()
                .copied()
                .map(MainEntity::from)
                .collect(),
        ));
        world.init_resource::<DecalPhases>();
        for _ in 0..3 {
            world.run_system_once(partition).unwrap();
            let early = &world.resource::<DecalPhases>().0[&view];
            assert_eq!(
                early.items.len(),
                2,
                "keep the instanced batch's skipped member"
            );
            assert_eq!(early.items[0].batch_range, 5..7);
            let late = &world.resource::<ViewSortedRenderPhases<Transparent3d>>()[&view];
            assert_eq!(late.items.len(), 3, "preserve retained host items");
            assert!(
                late.items[0].batch_range.is_empty(),
                "never draw the decal again after fog"
            );
            assert_eq!(
                late.items[2].batch_range,
                7..8,
                "leave genuine transparent draws in place"
            );
            world.run_system_once(restore).unwrap();
            assert_eq!(
                world.resource::<ViewSortedRenderPhases<Transparent3d>>()[&view].items[0]
                    .batch_range,
                5..7
            );
        }
        world.resource_mut::<DecalEntities>().0.clear();
        world.run_system_once(partition).unwrap();
        assert!(
            world.resource::<DecalPhases>().0.is_empty(),
            "no stale draws after membership changes"
        );
    }
}
