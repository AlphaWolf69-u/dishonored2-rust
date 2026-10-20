//! Optional wall-time boundaries for CPU frame phases; never used for game rules.
use bevy::{
    app::SubApp,
    prelude::*,
    render::{pipelined_rendering::RenderExtractApp, RenderApp, RenderSystems},
};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};

pub const LABELS: [&str; 50] = [
    "main",
    "extract",
    "sync_extract_wait",
    "assets",
    "meshes_views",
    "queue_specialize",
    "sort",
    "prepare",
    "backend",
    "cleanup",
    "render_total",
    "prepare_resources",
    "batch_phases",
    "write_phase_buffers",
    "collect_phase_buffers",
    "flush_staging",
    "bind_groups",
    "specialize",
    "prepare_views",
    "queue_meshes",
    "acquire_window",
    "parallel_cpu",
    "world_draws",
    "model_draws",
    "local_cpu",
    "local_draws",
    "model_collect",
    "parallel_render",
    "local_render",
    "shadow_prepare_records",
    "shadow_encode_draws",
    "host_cluster_stage_span",
    "host_opaque_draws",
    "host_transparent_draws",
    "host_camera_driver",
    "host_core3d_schedule",
    "host_postprocess",
    "host_queue_submit",
    "shadow_cascade_fit",
    "shadow_receiver",
    "shadow_tome_gather",
    "shadow_world_query",
    "screen_shadow_render",
    "coarse_depth_render",
    "visibility_publish",
    "visibility_mark_instances",
    "visibility_spatial_instances",
    "visibility_texture_demands",
    "visibility_light_outputs",
    "visibility_placed_models",
];
pub const COUNT: usize = LABELS.len();

struct Counters {
    epoch: Instant,
    start: [AtomicU64; COUNT],
    elapsed: [AtomicU64; COUNT],
}

#[derive(Resource, Clone)]
pub struct CpuFrameProfile(Arc<Counters>);

impl Default for CpuFrameProfile {
    fn default() -> Self {
        Self(Arc::new(Counters {
            epoch: Instant::now(),
            start: std::array::from_fn(|_| AtomicU64::new(0)),
            elapsed: std::array::from_fn(|_| AtomicU64::new(0)),
        }))
    }
}

impl CpuFrameProfile {
    pub fn measure(&self, index: usize) -> CpuSystemSpan<'_> {
        self.start(index);
        CpuSystemSpan {
            profile: self,
            index,
        }
    }
    fn start(&self, index: usize) {
        self.0.start[index].store(self.0.epoch.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }
    fn end(&self, index: usize) {
        let start = self.0.start[index].load(Ordering::Relaxed);
        self.0.elapsed[index].store(
            (self.0.epoch.elapsed().as_nanos() as u64).saturating_sub(start),
            Ordering::Release,
        );
    }
    pub fn latest_ns(&self) -> [u64; COUNT] {
        std::array::from_fn(|i| self.0.elapsed[i].load(Ordering::Acquire))
    }
}

pub struct CpuSystemSpan<'a> {
    profile: &'a CpuFrameProfile,
    index: usize,
}
impl Drop for CpuSystemSpan<'_> {
    fn drop(&mut self) {
        self.profile.end(self.index);
    }
}

fn start<const I: usize>(profile: Res<CpuFrameProfile>) {
    profile.start(I);
}
fn end<const I: usize>(profile: Res<CpuFrameProfile>) {
    profile.end(I);
}

fn wrap_extract(app: &mut SubApp, profile: CpuFrameProfile, index: usize) {
    if let Some(mut reference) = app.take_extract() {
        app.set_extract(move |main, sub| {
            profile.start(index);
            reference(main, sub);
            profile.end(index);
        });
    }
}

/// Preserve the existing extractors exactly. Sync timing includes the wait for
/// the prior render frame; it must not be interpreted as active CPU time.
pub fn install(app: &mut App) {
    let profile = CpuFrameProfile::default();
    app.insert_resource(profile.clone());
    app.add_systems(First, start::<0>);
    app.add_systems(Last, end::<0>);
    app.add_systems(
        PostUpdate,
        (
            start::<31>
                .after(bevy::transform::TransformSystems::Propagate)
                .after(bevy::camera::visibility::VisibilitySystems::CheckVisibility)
                .after(bevy::camera::CameraUpdateSystems)
                .before(bevy::light::SimulationLightSystems::AssignLightsToClusters),
            end::<31>.after(bevy::light::SimulationLightSystems::AssignLightsToClusters),
        ),
    );
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        wrap_extract(render, profile.clone(), 1);
        render.insert_resource(profile.clone());
        use bevy::core_pipeline::core_3d::{main_opaque_pass_3d, main_transparent_pass_3d};
        use bevy::core_pipeline::{Core3d, Core3dSystems};
        render.add_systems(
            Core3d,
            (
                start::<35>.before(Core3dSystems::Prepass),
                end::<35>.after(Core3dSystems::PostProcess),
                start::<36>
                    .after(Core3dSystems::MainPass)
                    .before(Core3dSystems::EarlyPostProcess),
                end::<36>.after(Core3dSystems::PostProcess),
                start::<32>
                    .in_set(Core3dSystems::MainPass)
                    .before(main_opaque_pass_3d),
                end::<32>
                    .in_set(Core3dSystems::MainPass)
                    .after(main_opaque_pass_3d)
                    .before(main_transparent_pass_3d),
                start::<33>
                    .in_set(Core3dSystems::MainPass)
                    .after(main_opaque_pass_3d)
                    .before(main_transparent_pass_3d),
                end::<33>
                    .in_set(Core3dSystems::MainPass)
                    .after(main_transparent_pass_3d),
            ),
        );
        render.add_systems(
            bevy::render::renderer::RenderGraph,
            (
                start::<34>
                    .in_set(bevy::render::renderer::RenderGraphSystems::Render)
                    .before(bevy::core_pipeline::schedule::camera_driver),
                end::<34>
                    .in_set(bevy::render::renderer::RenderGraphSystems::Render)
                    .after(bevy::core_pipeline::schedule::camera_driver),
                start::<37>
                    .after(bevy::render::renderer::RenderGraphSystems::Render)
                    .before(bevy::render::renderer::RenderGraphSystems::Submit),
                end::<37>
                    .after(bevy::render::renderer::RenderGraphSystems::Submit)
                    .before(bevy::render::renderer::RenderGraphSystems::Finish),
            ),
        );
        render.add_systems(First, start::<10>);
        render.add_systems(
            bevy::render::Render,
            end::<10>.after(RenderSystems::PostCleanup),
        );
        use RenderSystems::*;
        render.add_systems(
            bevy::render::Render,
            (
                start::<3>.after(ExtractCommands).before(PrepareAssets),
                end::<3>.after(PrepareAssets).before(PrepareMeshes),
                start::<4>.after(PrepareAssets).before(PrepareMeshes),
                end::<4>.after(CreateViews).before(Specialize),
                start::<5>.after(CreateViews).before(Specialize),
                end::<5>.after(Queue).before(PhaseSort),
                start::<6>.after(Queue).before(PhaseSort),
                end::<6>.after(PhaseSort).before(Prepare),
                start::<7>.after(PhaseSort).before(Prepare),
                end::<7>.after(Prepare).before(RenderSystems::Render),
                start::<8>.after(Prepare).before(RenderSystems::Render),
                end::<8>.after(RenderSystems::Render).before(Cleanup),
                start::<9>.after(RenderSystems::Render).before(Cleanup),
                end::<9>.after(PostCleanup),
            ),
        );
        render.add_systems(
            bevy::render::Render,
            (
                start::<11>.after(PhaseSort).before(PrepareResources),
                end::<11>
                    .after(PrepareResources)
                    .before(PrepareResourcesBatchPhases),
                start::<12>
                    .after(PrepareResources)
                    .before(PrepareResourcesBatchPhases),
                end::<12>
                    .after(PrepareResourcesBatchPhases)
                    .before(PrepareResourcesWritePhaseBuffers),
                start::<13>
                    .after(PrepareResourcesBatchPhases)
                    .before(PrepareResourcesWritePhaseBuffers),
                end::<13>
                    .after(PrepareResourcesWritePhaseBuffers)
                    .before(PrepareResourcesCollectPhaseBuffers),
                start::<14>
                    .after(PrepareResourcesWritePhaseBuffers)
                    .before(PrepareResourcesCollectPhaseBuffers),
                end::<14>
                    .after(PrepareResourcesCollectPhaseBuffers)
                    .before(PrepareResourcesFlush),
                start::<15>
                    .after(PrepareResourcesCollectPhaseBuffers)
                    .before(PrepareResourcesFlush),
                end::<15>
                    .after(PrepareResourcesFlush)
                    .before(PrepareBindGroups),
                start::<16>
                    .after(PrepareResourcesFlush)
                    .before(PrepareBindGroups),
                end::<16>
                    .after(PrepareBindGroups)
                    .before(RenderSystems::Render),
                start::<17>.after(CreateViews).before(Specialize),
                end::<17>.after(Specialize).before(PrepareViews),
                start::<18>.after(Specialize).before(PrepareViews),
                end::<18>.after(PrepareViews).before(Queue),
                start::<19>.after(PrepareViews).before(Queue),
                end::<19>.after(Queue).before(PhaseSort),
            ),
        );
        render.add_systems(
            bevy::render::Render,
            (
                start::<20>
                    .in_set(PrepareViews)
                    .before(bevy::render::view::window::prepare_windows),
                end::<20>
                    .in_set(PrepareViews)
                    .after(bevy::render::view::window::prepare_windows),
            ),
        );
    }
    if let Some(extract) = app.get_sub_app_mut(RenderExtractApp) {
        wrap_extract(extract, profile, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn instrumentation_preserves_extract_and_shared_counters() {
        let profile = CpuFrameProfile::default();
        let mut sub = SubApp::new();
        sub.set_extract(|_, world| {
            world.insert_resource(ClearColor(Color::BLACK));
        });
        wrap_extract(&mut sub, profile.clone(), 1);
        sub.extract(&mut World::new());
        assert!(sub.world().contains_resource::<ClearColor>());
        assert!(profile.latest_ns()[1] > 0);
        assert_eq!(profile.latest_ns(), profile.clone().latest_ns());
    }
}
