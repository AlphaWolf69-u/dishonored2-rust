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

pub const LABELS: [&str; 21] = [
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

fn start<const I: usize>(profile: Res<CpuFrameProfile>) {
    profile.start(I);
}
fn end<const I: usize>(profile: Res<CpuFrameProfile>) {
    profile.end(I);
}

fn wrap_extract(app: &mut SubApp, profile: CpuFrameProfile, index: usize) {
    if let Some(mut original) = app.take_extract() {
        app.set_extract(move |main, sub| {
            profile.start(index);
            original(main, sub);
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
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        wrap_extract(render, profile.clone(), 1);
        render.insert_resource(profile.clone());
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
    fn instrumentation_preserves_original_extract_and_shared_counters() {
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
