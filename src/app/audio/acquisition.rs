//! Host sound preparation, isolated from graphics/archive workers.
//! 1400bc8f0 / 1410bf740 configure Wwise's dedicated streaming device.
//! Encoded reads keep their existing owner in wwise::stream_io. Preparation
//! uses the same task-pool implementation as level sound acquisition, with
//! its own session worker and cancellable task leases. Create the worker
//! during session setup, so an audio callback never has to create a thread.
use bevy::tasks::{Task, TaskPool, TaskPoolBuilder};
use std::{sync::Arc, time::Instant};

pub(super) struct PreparationQueue {
    worker: Option<TaskPool>,
    trace: bool,
}
impl Default for PreparationQueue {
    fn default() -> Self {
        let debug_shared = std::env::var("DH2_DEBUG_SHARED_SOUND_JOBS").as_deref() == Ok("1");
        Self {
            worker: (!debug_shared).then(|| {
                TaskPoolBuilder::new()
                    // Keep the former preparation concurrency: one slow
                    // metadata read must not serialize all selected sources.
                    .num_threads(bevy::tasks::IoTaskPool::get().thread_num().max(1))
                    .thread_name("sound acquisition".to_owned())
                    .build()
            }),
            trace: std::env::var("DH2_DEBUG_SOUND_ACQUISITION").as_deref() == Ok("1"),
        }
    }
}

/// Dropping the task cancels queued work or releases a running job's result.
/// Bank/source snapshots retain only a Weak back-link to PreparationQueue.
pub(super) struct Job<T: Send + 'static>(Task<Prepared<T>>);

struct Prepared<T> {
    generation: u64,
    result: Option<T>,
}

impl<T> Drop for Prepared<T> {
    fn drop(&mut self) {
        let Some(result) = self.result.take() else { return };
        // An abandoned task can finish rebuilding decoder headers after the
        // map's usual sweep. Release its decoder leases before sweeping again.
        drop(result);
        if self.generation != super::graph::media_generation() {
            crate::assets::wwise::release_unused_headers();
        }
    }
}

impl<T: Send + 'static> Job<T> {
    pub(super) fn take(&mut self) -> Option<T> {
        let mut prepared = bevy::tasks::block_on(bevy::tasks::futures_lite::future::poll_once(&mut self.0))?;
        prepared.result.take()
    }
}
impl PreparationQueue {
    pub(super) fn submit<T: Send + 'static>(
        &self,
        label: &'static str,
        id: u32,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Job<T> {
        let trace = self.trace;
        let queued = trace.then(Instant::now);
        let debug_shared = self.worker.is_none();
        let generation = super::graph::media_generation();
        let execute = async move {
            let started = trace.then(Instant::now);
            let cycles = if trace {
                crate::loading::thread_cycles()
            } else {
                None
            };
            let result = work();
            if let (Some(queued), Some(started)) = (queued, started) {
                let cycles = cycles
                    .zip(crate::loading::thread_cycles())
                    .map_or(0, |(a, b)| b.saturating_sub(a));
                eprintln!(
                    "SOUND_ACQUISITION kind={label} id={id:#010x} shared={debug_shared} queue_us={} work_us={} work_cycles={cycles}",
                    started.duration_since(queued).as_micros(),
                    started.elapsed().as_micros()
                );
            }
            Prepared { generation, result: Some(result) }
        };
        let task = if let Some(worker) = &self.worker {
            worker.spawn(execute)
        } else {
            bevy::tasks::IoTaskPool::get().spawn(execute)
        };
        // TaskPool owns cancellation/shutdown and joins its worker on drop.
        // Inputs never own PreparationQueue, so queued tasks cannot keep it alive.
        Job(task)
    }
}

#[derive(bevy::prelude::Resource, Default)]
pub(super) struct SourceJobs(pub Arc<PreparationQueue>);
impl Drop for SourceJobs {
    fn drop(&mut self) {
        // Retire the session before joining its queue: an in-flight decode
        // may finish its read, but must not restore cache ownership afterward.
        super::graph::retain_media(&Default::default());
    }
}
