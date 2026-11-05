//! Host sound-source preparation, separate from graphics/archive workers.
//! 1400bc8f0 / 1410bf740 configure Wwise's dedicated streaming device.
//! Encoded block reads still belong to wwise::stream_io; this worker prepares
//! source/codec metadata without waiting behind texture queue drain jobs.
use std::{collections::VecDeque, io, sync::{Arc, Condvar, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering}}, thread::{self, JoinHandle}, time::Instant};

type Work = Box<dyn FnOnce() + Send>;
struct Shared {
    queue: Mutex<VecDeque<Work>>,
    changed: Condvar,
    stopped: AtomicBool,
}

pub(super) struct Pool {
    shared: Arc<Shared>,
    worker: OnceLock<io::Result<JoinHandle<()>>>,
    debug_shared: bool,
    trace: bool,
}
impl Default for Pool {
    fn default() -> Self {
        Self { shared: Arc::new(Shared { queue: Mutex::default(), changed: Condvar::new(),
            stopped: AtomicBool::new(false) }), worker: OnceLock::new(),
            debug_shared: std::env::var("DH2_DEBUG_SHARED_SOUND_JOBS").as_deref() == Ok("1"),
            trace: std::env::var("DH2_DEBUG_SOUND_ACQUISITION").as_deref() == Ok("1") }
    }
}

/// A lease on a pending result. Dropping the final caller skips queued work
/// or releases a running job's unclaimed result after its existing read ends.
pub(super) enum Job<T: Send + 'static> {
    Dedicated(Arc<Mutex<Option<T>>>),
    DebugShared(bevy::tasks::Task<T>),
}
impl<T: Send + 'static> Job<T> {
    pub(super) fn take(&mut self) -> Option<T> {
        match self {
            Self::Dedicated(result) => result.lock().unwrap_or_else(|error| error.into_inner()).take(),
            Self::DebugShared(task) => bevy::tasks::block_on(bevy::tasks::futures_lite::future::poll_once(task)),
        }
    }
}

impl Pool {
    pub(super) fn submit<T: Send + 'static>(&self, label: &'static str, id: u32,
        work: impl FnOnce() -> T + Send + 'static) -> io::Result<Job<T>> {
        let trace = self.trace;
        let queued = trace.then(Instant::now);
        let debug_shared = self.debug_shared;
        let execute = move || {
            let started = trace.then(Instant::now);
            let cycles = if trace { crate::loading::thread_cycles() } else { None };
            let result = work();
            if let (Some(queued), Some(started)) = (queued, started) {
                let cycles = cycles.zip(crate::loading::thread_cycles()).map_or(0, |(a, b)| b.saturating_sub(a));
                eprintln!("SOUND_ACQUISITION kind={label} id={id:#010x} shared={debug_shared} queue_us={} work_us={} work_cycles={cycles}",
                    started.duration_since(queued).as_micros(), started.elapsed().as_micros());
            }
            result
        };
        if debug_shared {
            return Ok(Job::DebugShared(bevy::tasks::IoTaskPool::get().spawn(async move { execute() })));
        }
        if let Err(error) = self.worker.get_or_init(|| {
            let shared = self.shared.clone();
            thread::Builder::new().name("sound acquisition".into()).spawn(move || run(shared))
        }) { return Err(io::Error::new(error.kind(), error.to_string())); }
        let result = Arc::new(Mutex::new(None));
        let output = result.clone();
        let mut queue = self.shared.queue.lock().unwrap_or_else(|error| error.into_inner());
        if self.shared.stopped.load(Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "sound preparation owner retired"));
        }
        queue.push_back(Box::new(move || {
            if Arc::strong_count(&output) == 1 { return; }
            let value = execute();
            *output.lock().unwrap_or_else(|error| error.into_inner()) = Some(value);
        }));
        self.shared.changed.notify_one();
        Ok(Job::Dedicated(result))
    }
}

fn run(shared: Arc<Shared>) {
    loop {
        let work = {
            let mut queue = shared.queue.lock().unwrap_or_else(|error| error.into_inner());
            while queue.is_empty() && !shared.stopped.load(Ordering::Acquire) {
                queue = shared.changed.wait(queue).unwrap_or_else(|error| error.into_inner());
            }
            if shared.stopped.load(Ordering::Acquire) { return; }
            queue.pop_front()
        };
        if let Some(work) = work { work(); }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        let queued = {
            let mut queue = self.shared.queue.lock().unwrap_or_else(|error| error.into_inner());
            let queued = std::mem::take(&mut *queue);
            self.shared.changed.notify_all();
            queued
        };
        // Dropping closures can release source/bank leases. Do it outside the
        // queue lock; submitted inputs must carry only a Weak<Pool> back-link.
        drop(queued);
        if let Some(Ok(worker)) = self.worker.take() {
            if worker.thread().id() != thread::current().id() { let _ = worker.join(); }
        }
    }
}

#[derive(bevy::prelude::Resource, Default)]
pub(super) struct SourceJobs(pub Arc<Pool>);
