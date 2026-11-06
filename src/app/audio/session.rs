//! Host lifetime boundary for the process-wide prepared-audio caches.
//! Wwise source release 141122400 / 1410f1910 and dedicated stream-device
//! shutdown 1400bc8f0 / 1410bf740 are represented by the existing host queues.
//!
//! A lease is acquired before submitting work, so shutdown can close
//! admission and wait for work that has not begun running yet. Prepared
//! stream sources retain a handle: their output-thread submission uses only
//! atomics during normal playback, without taking the session mutex.
use std::sync::{Arc, Condvar, LazyLock, Mutex, Weak, atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering}};

static CURRENT: LazyLock<Mutex<Weak<Inner>>> = LazyLock::new(Default::default);

struct Inner {
    closing: AtomicBool,
    active: AtomicUsize,
    acquired: AtomicU64,
    peak: AtomicUsize,
    acquire_nanos: AtomicU64,
    wait: Mutex<()>,
    idle: Condvar,
    trace: bool,
}

pub(in crate::app) struct Session(Arc<Inner>);
pub(super) struct Handle(Arc<Inner>);
pub(in crate::app) struct Work(Arc<Inner>);

impl Session {
    pub fn begin() -> Result<Self, &'static str> {
        let mut current = CURRENT.lock().unwrap_or_else(|error| error.into_inner());
        if current.upgrade().is_some() { return Err("audio session already exists"); }
        let inner = Arc::new(Inner { closing: AtomicBool::new(false), active: AtomicUsize::new(0),
            acquired: AtomicU64::new(0), peak: AtomicUsize::new(0), acquire_nanos: AtomicU64::new(0),
            wait: Mutex::new(()), idle: Condvar::new(),
            trace: std::env::var("DH2_DEBUG_AUDIO_RESOURCES").as_deref() == Ok("1") });
        *current = Arc::downgrade(&inner);
        Ok(Self(inner))
    }
}

/// Acquire during preparation and retain with a stream source. A closed
/// handle never starts work in a later session, even if a decoder outlives it.
pub(super) fn handle() -> Result<Handle, &'static str> {
    let inner = CURRENT.lock().unwrap_or_else(|error| error.into_inner()).upgrade()
        .ok_or("audio session is unavailable")?;
    if inner.closing.load(Ordering::SeqCst) { return Err("audio session is closing"); }
    Ok(Handle(inner))
}

impl Handle {
    pub fn work(&self) -> Result<Work, &'static str> {
        let started = self.0.trace.then(std::time::Instant::now);
        if self.0.closing.load(Ordering::SeqCst) { return Err("audio session is closing"); }
        let active = self.0.active.fetch_add(1, Ordering::SeqCst) + 1;
        // A closer may have passed its zero check before this increment. In
        // that case this second check cancels the work before it can run.
        if self.0.closing.load(Ordering::SeqCst) {
            release(&self.0);
            return Err("audio session is closing");
        }
        let work = Work(self.0.clone());
        if let Some(started) = started {
            self.0.acquired.fetch_add(1, Ordering::Relaxed);
            self.0.peak.fetch_max(active, Ordering::Relaxed);
            self.0.acquire_nanos.fetch_add(started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64, Ordering::Relaxed);
        }
        Ok(work)
    }
}

/// Called before a job is submitted, not from inside the job's future.
pub(in crate::app) fn work() -> Result<Work, &'static str> { handle()?.work() }

impl Work {
    pub fn is_closing(&self) -> bool { self.0.closing.load(Ordering::SeqCst) }
}

fn release(inner: &Inner) {
    if inner.active.fetch_sub(1, Ordering::SeqCst) == 1 && inner.closing.load(Ordering::SeqCst) {
        // Synchronize with the waiter's check so the wakeup cannot be lost.
        let _wait = inner.wait.lock().unwrap_or_else(|error| error.into_inner());
        inner.idle.notify_all();
    }
}

impl Drop for Work {
    fn drop(&mut self) { release(&self.0); }
}

impl Drop for Session {
    fn drop(&mut self) {
        let started = self.0.trace.then(std::time::Instant::now);
        self.0.closing.store(true, Ordering::SeqCst);
        let mut wait = self.0.wait.lock().unwrap_or_else(|error| error.into_inner());
        while self.0.active.load(Ordering::SeqCst) != 0 {
            wait = self.0.idle.wait(wait).unwrap_or_else(|error| error.into_inner());
        }
        let elapsed = started.map(|started| started.elapsed());
        drop(wait);
        // No admitted producer can republish into these process-wide maps.
        let (media, streams) = super::graph::retire_session_media();
        if let Some(elapsed) = elapsed {
            eprintln!("AUDIO_SESSION jobs={} peak={} handle_acquire_ms={:.3} shutdown_wait_ms={:.3} retired_media={media} retired_streams={streams}",
                self.0.acquired.load(Ordering::Relaxed), self.0.peak.load(Ordering::Relaxed),
                self.0.acquire_nanos.load(Ordering::Relaxed) as f64 / 1_000_000.0,
                elapsed.as_secs_f64() * 1000.0);
        }
        let mut current = CURRENT.lock().unwrap_or_else(|error| error.into_inner());
        if current.upgrade().is_some_and(|inner| Arc::ptr_eq(&inner, &self.0)) {
            *current = Weak::new();
        }
    }
}
