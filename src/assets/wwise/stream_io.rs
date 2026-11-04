//! Encoded file-source IO residency. 1400bc8f0 / 1410bf740 configure a
//! 2 MiB cached blocking device with 16 KiB granularity. Host: a dedicated
//! reader submits no filesystem work on the audio callback. Cache entries
//! share package ranges; active leases count against the same byte budget.
use std::{collections::{HashMap, HashSet, VecDeque}, fs::File, io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf}, sync::{Arc, Condvar, Mutex, OnceLock},
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering}, thread};

pub(crate) const BLOCK_BYTES: usize = 0x4000;
const DEVICE_BYTES: usize = 0x200000;

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("DH2_DEBUG_BOUNDED_SOUND_STREAMS").as_deref() != Ok("0"))
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key { path: Arc<PathBuf>, offset: u64, length: usize }

struct Budget {
    used: Mutex<usize>,
    released: Condvar,
    peak: AtomicUsize,
    wakes: AtomicU64,
}

impl Budget {
    fn release(&self, length: usize) {
        *self.used.lock().unwrap_or_else(|e| e.into_inner()) -= length;
        self.released.notify_one();
    }
}

pub(crate) struct Block { bytes: Vec<u8>, budget: Arc<Budget> }
impl std::ops::Deref for Block {
    type Target = [u8];
    fn deref(&self) -> &[u8] { &self.bytes }
}
impl Drop for Block { fn drop(&mut self) { self.budget.release(self.bytes.len()); } }

struct Request {
    result: Mutex<Option<Result<Arc<Block>, Arc<io::Error>>>>,
}
pub(crate) struct ReadLease { request: Option<Arc<Request>>, budget: Arc<Budget> }
impl ReadLease {
    /// None means IO pending, never EOF. The consumer retains this request
    /// until ready or cancelled; ready block leases retain their own storage.
    pub(crate) fn ready(&self) -> io::Result<Option<Arc<Block>>> {
        match self.request.as_ref().unwrap().result.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            None => Ok(None),
            Some(Ok(block)) => Ok(Some(block.clone())),
            Some(Err(error)) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }
}
impl Drop for ReadLease {
    fn drop(&mut self) {
        // Retiring a request can make its cached block evictable even when
        // the cache still owns those bytes. Wake the device to rescan LRU.
        drop(self.request.take());
        let _used = self.budget.used.lock().unwrap_or_else(|e| e.into_inner());
        self.budget.wakes.fetch_add(1, Ordering::Relaxed);
        self.budget.released.notify_one();
    }
}

struct Entry { request: Arc<Request>, age: u64 }
#[derive(Default)]
struct State {
    entries: HashMap<Key, Entry>,
    pending: VecDeque<(Key, Arc<Request>)>,
    age: u64,
    packages: Option<HashSet<PathBuf>>,
    files_changed: bool,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    budget: Arc<Budget>,
    stopped: AtomicBool,
    reads: AtomicU64,
    bytes: AtomicU64,
    read_ns: AtomicU64,
    cancellations: AtomicU64,
}

pub(crate) struct ReadPool {
    shared: Arc<Shared>,
    worker: OnceLock<io::Result<thread::JoinHandle<()>>>,
}

impl Default for ReadPool {
    fn default() -> Self {
        Self { shared: Arc::new(Shared {
            state: Mutex::default(), changed: Condvar::new(),
            budget: Arc::new(Budget { used: Mutex::new(0), released: Condvar::new(), peak: AtomicUsize::new(0), wakes: AtomicU64::new(0) }),
            stopped: AtomicBool::new(false), reads: AtomicU64::new(0), bytes: AtomicU64::new(0),
            read_ns: AtomicU64::new(0), cancellations: AtomicU64::new(0),
        }), worker: OnceLock::new() }
    }
}

impl ReadPool {
    pub(crate) fn allow_package(&self, path: &Path) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(packages) = &mut state.packages { packages.insert(path.to_owned()); }
    }

    pub(crate) fn retain_packages(&self, packages: &HashSet<PathBuf>) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.packages = Some(packages.clone());
        state.files_changed = true;
        let queued = state.pending.len();
        state.pending.retain(|(key, _)| packages.contains(key.path.as_ref()));
        self.shared.cancellations.fetch_add((queued - state.pending.len()) as u64, Ordering::Relaxed);
        state.entries.retain(|key, entry| {
            if packages.contains(key.path.as_ref()) { return true; }
            let mut result = entry.request.result.lock().unwrap_or_else(|e| e.into_inner());
            if result.is_none() { *result = Some(Err(Arc::new(io::Error::new(io::ErrorKind::Interrupted, "sound package unloaded")))); }
            false
        });
        self.shared.changed.notify_one();
        drop(state);
        // A retired entry may make a budget-blocked read cancellable.
        let _used = self.shared.budget.used.lock().unwrap_or_else(|e| e.into_inner());
        self.shared.budget.wakes.fetch_add(1, Ordering::Relaxed);
        self.shared.budget.released.notify_one();
    }

    pub(crate) fn report(&self) {
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() != Ok("1") { return; }
        let state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let used = self.shared.budget.used.lock().unwrap_or_else(|e| e.into_inner());
        eprintln!("MEMORY sound IO device: reads={} bytes={} resident_bytes={} peak_bytes={} cached={} pending={} cancellations={}",
            self.shared.reads.load(Ordering::Relaxed), self.shared.bytes.load(Ordering::Relaxed), *used,
            self.shared.budget.peak.load(Ordering::Relaxed), state.entries.len(), state.pending.len(),
            self.shared.cancellations.load(Ordering::Relaxed));
    }
    pub(crate) fn start(&self) -> io::Result<()> {
        match self.worker.get_or_init(|| {
            let shared = self.shared.clone();
            thread::Builder::new().name("sound file reader".into()).spawn(move || run(shared))
        }) {
            Ok(_) => Ok(()),
            Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }
    /// A package-bounded block. The last block is short; adjacent package
    /// members are never accidentally acquired as part of source read-ahead.
    pub(crate) fn request(&self, path: Arc<PathBuf>, offset: u64, length: usize) -> io::Result<ReadLease> {
        if length == 0 || length > BLOCK_BYTES { return Err(io::Error::other("invalid sound IO block size")); }
        // Initial source preparation starts this on its IO task before
        // admission; this is only a cached readiness check on later requests.
        self.start()?;
        let key = Key { path, offset, length };
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.packages.as_ref().is_some_and(|packages| !packages.contains(key.path.as_ref())) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "sound package is no longer loaded"));
        }
        state.age = state.age.wrapping_add(1);
        let age = state.age;
        if let Some(entry) = state.entries.get_mut(&key) {
            entry.age = age;
            return Ok(ReadLease { request: Some(entry.request.clone()), budget: self.shared.budget.clone() });
        }
        let request = Arc::new(Request { result: Mutex::new(None) });
        state.entries.insert(key.clone(), Entry { request: request.clone(), age });
        state.pending.push_back((key, request.clone()));
        self.shared.changed.notify_one();
        Ok(ReadLease { request: Some(request), budget: self.shared.budget.clone() })
    }
}

/// One decoder's two recent blocks and up to two read-ahead request leases.
/// All completed/requested buffers share the device's encoded-byte budget.
pub(crate) struct Window {
    pool: Arc<ReadPool>,
    path: Arc<PathBuf>,
    offset: u64,
    size: usize,
    prefix: Option<super::MediaBytes>,
    blocks: Vec<(usize, Arc<Block>)>,
    pending: Vec<(usize, ReadLease)>,
}

impl Window {
    pub(crate) fn new(pool: Arc<ReadPool>, path: Arc<PathBuf>, offset: u64, size: usize, prefix: Option<super::MediaBytes>) -> Self {
        Self { pool, path, offset, size, prefix, blocks: Vec::with_capacity(2), pending: Vec::with_capacity(2) }
    }

    fn prefix_bytes(&self) -> usize { self.prefix.as_ref().map_or(0, |prefix| prefix.len()) }

    pub(crate) fn contiguous(&self, offset: usize, length: usize) -> bool {
        let prefix = self.prefix_bytes();
        if offset < prefix { length <= prefix - offset }
        else { length <= BLOCK_BYTES - (offset - prefix) % BLOCK_BYTES }
    }

    fn request(&mut self, block: usize) -> io::Result<()> {
        let start = block.checked_mul(BLOCK_BYTES).and_then(|start| start.checked_add(self.prefix_bytes()))
            .ok_or_else(|| io::Error::other("sound block offset overflow"))?;
        if start >= self.size || self.blocks.iter().any(|(index, _)| *index == block)
            || self.pending.iter().any(|(index, _)| *index == block) { return Ok(()); }
        if self.pending.len() == 2 { self.pending.remove(0); }
        let offset = self.offset.checked_add(start as u64).ok_or_else(|| io::Error::other("sound package offset overflow"))?;
        let read = self.pool.request(self.path.clone(), offset, BLOCK_BYTES.min(self.size - start))?;
        self.pending.push((block, read));
        Ok(())
    }

    fn resolve(&mut self, offset: usize) -> io::Result<Option<usize>> {
        if offset >= self.size { return Err(io::Error::from(io::ErrorKind::UnexpectedEof)); }
        let block = (offset - self.prefix_bytes()) / BLOCK_BYTES;
        let mut index = self.blocks.iter().position(|(index, _)| *index == block);
        if index.is_none() {
            self.request(block)?;
            let pending = self.pending.iter().position(|(index, _)| *index == block).unwrap();
            let Some(bytes) = self.pending[pending].1.ready()? else {
                // 141161240 releases consumed input before waiting for more.
                // Historical/read-ahead leases must not exhaust the shared
                // device and prevent this cursor's required read progressing.
                self.blocks.clear();
                self.pending.retain(|(index, _)| *index == block);
                return Ok(None);
            };
            self.pending.remove(pending);
            if self.blocks.len() == 2 { self.blocks.remove(0); }
            self.blocks.push((block, bytes));
            index = Some(self.blocks.len() - 1);
        }
        if index.unwrap() + 1 != self.blocks.len() { self.blocks.swap(0, 1); }
        self.request(block + 1)?;
        Ok(Some(self.blocks.len() - 1))
    }

    /// Zero-copy packet view when the packet fits in one IO block. The
    /// decoder borrows the existing lease; no per-packet Arc operation.
    pub(crate) fn slice(&mut self, offset: usize, length: usize) -> io::Result<Option<&[u8]>> {
        let prefix = self.prefix_bytes();
        if offset < prefix {
            self.request(0)?; // Native prefetch starts continuation alongside prefix playback.
            return self.prefix.as_ref().unwrap().get(offset..offset + length).map(Some)
                .ok_or_else(|| io::Error::other("sound packet crosses prefetch boundary"));
        }
        let within = (offset - prefix) % BLOCK_BYTES;
        if length > BLOCK_BYTES - within { return Err(io::Error::other("sound range crosses an IO block")); }
        let Some(index) = self.resolve(offset)? else { return Ok(None) };
        self.blocks[index].1.get(within..within + length).map(Some)
            .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))
    }

    /// Zero means IO pending, never EOF. Only cross-block packet fragments
    /// and small packet headers need copies; no file access or waiting here.
    pub(crate) fn copy(&mut self, offset: usize, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() { return Ok(0); }
        let prefix = self.prefix_bytes();
        if offset < prefix {
            self.request(0)?;
            let count = output.len().min(prefix - offset);
            output[..count].copy_from_slice(&self.prefix.as_ref().unwrap()[offset..offset + count]);
            return Ok(count);
        }
        let Some(index) = self.resolve(offset)? else { return Ok(0) };
        let bytes = &self.blocks[index].1;
        let within = (offset - prefix) % BLOCK_BYTES;
        let count = output.len().min(bytes.len() - within);
        output[..count].copy_from_slice(&bytes[within..within + count]);
        Ok(count)
    }

}

fn run(shared: Arc<Shared>) {
    // Package handles are owned by this reader, never by the audio callback.
    // Map-bank retirement also closes outgoing handles on this thread.
    let mut files = HashMap::<Arc<PathBuf>, File>::new();
    'jobs: loop {
        let (job, packages) = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            while state.pending.is_empty() && !state.files_changed && !shared.stopped.load(Ordering::Acquire) {
                state = shared.changed.wait(state).unwrap_or_else(|e| e.into_inner());
            }
            if shared.stopped.load(Ordering::Acquire) { return; }
            let packages = std::mem::take(&mut state.files_changed).then(|| state.packages.clone()).flatten();
            (state.pending.pop_front(), packages)
        };
        if let Some(packages) = packages {
            files.retain(|path, _| packages.contains(path.as_ref()));
            if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                eprintln!("MEMORY sound IO package retirement: open_handles={} allowed_packages={}", files.len(), packages.len());
            }
        }
        let Some((key, request)) = job else { continue };
        loop {
            // Check under the registry lock: a new lease cannot race with
            // cancellation, nor can an old job erase a replacement entry.
            let cancelled = {
                let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                let current = state.entries.get(&key).is_some_and(|entry| Arc::ptr_eq(&entry.request, &request));
                if !current || Arc::strong_count(&request) == 2 {
                    if current { state.entries.remove(&key); }
                    let mut result = request.result.lock().unwrap_or_else(|e| e.into_inner());
                    if result.is_none() { *result = Some(Err(Arc::new(io::Error::new(io::ErrorKind::Interrupted, "sound read cancelled")))); }
                    true
                } else { false }
            };
            if cancelled { shared.cancellations.fetch_add(1, Ordering::Relaxed); continue 'jobs; }
            let mut used = shared.budget.used.lock().unwrap_or_else(|e| e.into_inner());
            if shared.stopped.load(Ordering::Acquire) { return; }
            if *used + key.length <= DEVICE_BYTES {
                *used += key.length;
                shared.budget.peak.fetch_max(*used, Ordering::Relaxed);
                break;
            }
            let wake = shared.budget.wakes.load(Ordering::Relaxed);
            drop(used);
            // State -> request -> budget is the lock order. Never hold the
            // budget lock while retiring a cached block that releases bytes.
            let evicted = {
                let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                let oldest = state.entries.iter().filter(|(_, entry)|
                    Arc::strong_count(&entry.request) == 1
                    && entry.request.result.lock().unwrap_or_else(|e| e.into_inner()).is_some())
                    .min_by_key(|(_, entry)| entry.age).map(|(key, _)| key.clone());
                oldest.and_then(|key| state.entries.remove(&key))
            };
            if evicted.is_some() { drop(evicted); continue; }
            let used = shared.budget.used.lock().unwrap_or_else(|e| e.into_inner());
            // A lease release can make an entry evictable without changing
            // used bytes. Do not lose its wake between the scan and wait.
            if *used + key.length > DEVICE_BYTES && !shared.stopped.load(Ordering::Acquire)
                && shared.budget.wakes.load(Ordering::Relaxed) == wake {
                drop(shared.budget.released.wait(used).unwrap_or_else(|e| e.into_inner()));
            }
        }
        let started = std::time::Instant::now();
        let result = (|| {
            let file = match files.entry(key.path.clone()) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(File::open(key.path.as_ref())?),
            };
            file.seek(SeekFrom::Start(key.offset))?;
            let mut bytes = vec![0; key.length];
            file.read_exact(&mut bytes)?;
            Ok::<_, io::Error>(Arc::new(Block { bytes, budget: shared.budget.clone() }))
        })();
        shared.read_ns.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        shared.reads.fetch_add(1, Ordering::Relaxed);
        if result.is_ok() { shared.bytes.fetch_add(key.length as u64, Ordering::Relaxed); }
        else { shared.budget.release(key.length); }
        let failed = result.is_err();
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let current = state.entries.get(&key).is_some_and(|entry| Arc::ptr_eq(&entry.request, &request));
        let mut slot = request.result.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(if current { result.map_err(Arc::new) }
                else { Err(Arc::new(io::Error::new(io::ErrorKind::Interrupted, "sound package unloaded during read"))) });
        }
        drop(slot);
        if failed && current { state.entries.remove(&key); }
    }
}

impl Drop for ReadPool {
    fn drop(&mut self) {
        {
            let _state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            self.shared.stopped.store(true, Ordering::Release);
            self.shared.changed.notify_all();
        }
        // Serialize the stop notification with the worker's budget wait.
        let used = self.shared.budget.used.lock().unwrap_or_else(|e| e.into_inner());
        self.shared.budget.released.notify_all();
        drop(used);
        if let Some(Ok(worker)) = self.worker.take() { let _ = worker.join(); }
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner()).entries.clear();
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY sound IO device release: reads={} bytes={} peak_bytes={} cancellations={} read_ms={:.3}",
                self.shared.reads.load(Ordering::Relaxed), self.shared.bytes.load(Ordering::Relaxed),
                self.shared.budget.peak.load(Ordering::Relaxed), self.shared.cancellations.load(Ordering::Relaxed),
                self.shared.read_ns.load(Ordering::Relaxed) as f64 / 1e6);
        }
    }
}
