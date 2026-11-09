//! Diagnostic (`debug_allocations` feature): live heap bytes per tag.
//!
//! The global allocator prefixes each block with the tag active on the
//! allocating thread ([`tag`]), so a block freed elsewhere is still charged
//! to its owner. Untagged allocations go to tag 0, "untagged". Without the
//! feature, [`tag`] is free and [`report`] is empty.
use std::sync::atomic::{AtomicIsize, Ordering};

const TAGS: usize = u8::MAX as usize + 1;
static LIVE: [AtomicIsize; TAGS] = [const { AtomicIsize::new(0) }; TAGS];
#[cfg(feature = "debug_allocations")]
static RESERVED: [AtomicIsize; TAGS] = [const { AtomicIsize::new(0) }; TAGS];
static NAMES: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());

thread_local! {
    static CURRENT: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// Restores the previous tag when dropped.
pub struct Guard(u8);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = CURRENT.try_with(|c| c.set(self.0));
    }
}

/// Charge this thread's allocations to `name` until the guard drops.
pub fn tag(name: &'static str) -> Guard {
    if !cfg!(feature = "debug_allocations") {
        return Guard(0);
    }
    let index = {
        let mut names = NAMES.lock().unwrap();
        if names.is_empty() {
            // Process diagnostic metadata must not grow under a resource tag.
            // The registry is bounded by TAGS; allocate once before first use.
            names.reserve_exact(TAGS);
            names.push("untagged");
        }
        match names.iter().position(|n| *n == name) {
            Some(i) => i,
            None if names.len() < TAGS => {
                names.push(name);
                names.len() - 1
            }
            None => 0,
        }
    } as u8;
    Guard(CURRENT.try_with(|c| c.replace(index)).unwrap_or(0))
}

/// Permanently select a stable loading-phase context on this thread. Labels
/// are process diagnostic metadata: intern each category once, rather than
/// leaking a new string on every marker or embedding varying counts/timings.
pub fn enter_phase(phase: &'static str) {
    if !cfg!(feature = "debug_allocations") { return; }
    let untagged = tag("untagged");
    let index = {
        let mut names = NAMES.lock().unwrap();
        match names.iter().position(|name| name.strip_prefix("after ") == Some(phase)) {
            Some(index) => index,
            None if names.len() < TAGS => {
                names.push(Box::leak(format!("after {phase}").into_boxed_str()));
                names.len() - 1
            }
            None => 0,
        }
    } as u8;
    drop(untagged);
    let _ = CURRENT.try_with(|current| current.set(index));
}

/// Interned heap-label payload only; static owner labels and the registry
/// vector are separate metadata. Used by real repeated-load observations.
pub fn phase_label_storage() -> Option<(usize, usize)> {
    if !cfg!(feature = "debug_allocations") { return None; }
    let names = NAMES.lock().unwrap_or_else(|error| error.into_inner());
    Some(names.iter().filter(|name| name.starts_with("after "))
        .fold((0, 0), |(count, bytes), name| (count + 1, bytes + name.len())))
}

/// Live bytes per tag, largest first.
pub fn report() -> Vec<(&'static str, isize)> {
    let names = NAMES.lock().unwrap();
    let mut out: Vec<_> = names.iter().enumerate().map(|(i, n)| (*n, LIVE[i].load(Ordering::Relaxed))).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

/// Read one diagnostic allocation tag without allocating a report snapshot.
/// None means tracking is disabled or this name has not been registered;
/// an unobserved/overflowed context must not look like verified zero bytes.
pub fn live_bytes(name: &str) -> Option<isize> {
    if !cfg!(feature = "debug_allocations") { return None; }
    let names = NAMES.lock().unwrap_or_else(|error| error.into_inner());
    names.iter().position(|candidate| *candidate == name)
        .map(|index| LIVE[index].load(Ordering::Relaxed))
}

/// Bytes requested from the diagnostic allocator's System backend, including
/// its actual per-allocation tag prefix; excludes System's internal overhead.
pub fn live_reserved_bytes(name: &str) -> Option<isize> {
    #[cfg(feature = "debug_allocations")]
    {
        let names = NAMES.lock().unwrap_or_else(|error| error.into_inner());
        names.iter().position(|candidate| *candidate == name)
            .map(|index| RESERVED[index].load(Ordering::Relaxed))
    }
    #[cfg(not(feature = "debug_allocations"))]
    { let _ = name; None }
}

/// Live blocks of at least `BIG_BYTES`, with the stack that allocated them.
#[cfg(feature = "debug_allocations")]
static BIG: std::sync::Mutex<Option<std::collections::HashMap<usize, (usize, std::backtrace::Backtrace)>>> =
    std::sync::Mutex::new(None);
#[cfg(feature = "debug_allocations")]
const BIG_BYTES: usize = 1 << 20;
#[cfg(feature = "debug_allocations")]
thread_local! {
    /// Set while this thread records a big block, so the bookkeeping's own
    /// allocations are not recorded.
    static RECORDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(feature = "debug_allocations")]
fn record_big(ptr: usize, size: usize, live: bool) {
    if RECORDING.try_with(|r| r.replace(true)).unwrap_or(true) {
        return;
    }
    {
        let mut big = BIG.lock().unwrap_or_else(|e| e.into_inner());
        let big = big.get_or_insert_with(Default::default);
        if live {
            big.insert(ptr, (size, std::backtrace::Backtrace::force_capture()));
        } else {
            big.remove(&ptr);
        }
    }
    let _ = RECORDING.try_with(|r| r.set(false));
}

/// Live big blocks grouped by the first frames of this crate that allocated
/// them, largest first: (frames, bytes, blocks).
pub fn big_blocks() -> Vec<(String, usize, usize)> {
    #[cfg(feature = "debug_allocations")]
    {
        let _ = RECORDING.try_with(|r| r.set(true));
        let mut groups = std::collections::HashMap::<String, (usize, usize)>::new();
        if let Some(big) = BIG.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            for (size, trace) in big.values() {
                let text = trace.to_string();
                let frames: Vec<&str> = text.lines().map(str::trim)
                    .filter(|line| line.contains(": ") && !line.starts_with("at "))
                    .map(|line| line.split_once(": ").map_or(line, |(_, name)| name))
                    .filter(|name| (name.contains("dishonored2_rust") || name.contains("bevy") || name.contains("wgpu"))
                        && !name.contains("allocation_tags") && !name.contains("alloc::"))
                    .take(4)
                    .collect();
                let entry = groups.entry(frames.join(" <- ")).or_default();
                entry.0 += size;
                entry.1 += 1;
            }
        }
        let _ = RECORDING.try_with(|r| r.set(false));
        let mut out: Vec<_> = groups.into_iter().map(|(k, (b, n))| (k, b, n)).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1));
        return out;
    }
    #[allow(unreachable_code)]
    Vec::new()
}

#[cfg(feature = "debug_allocations")]
pub struct TaggedAllocator;

#[cfg(feature = "debug_allocations")]
unsafe impl std::alloc::GlobalAlloc for TaggedAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let prefix = layout.align().max(16);
        let Ok(full) = std::alloc::Layout::from_size_align(layout.size() + prefix, layout.align().max(16)) else {
            return std::ptr::null_mut();
        };
        let base = unsafe { std::alloc::System.alloc(full) };
        if base.is_null() {
            return base;
        }
        let tag = CURRENT.try_with(|c| c.get()).unwrap_or(0);
        unsafe { base.write(tag) };
        LIVE[tag as usize].fetch_add(layout.size() as isize, Ordering::Relaxed);
        RESERVED[tag as usize].fetch_add(full.size() as isize, Ordering::Relaxed);
        let ptr = unsafe { base.add(prefix) };
        if layout.size() >= BIG_BYTES {
            record_big(ptr as usize, layout.size(), true);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        if layout.size() >= BIG_BYTES {
            record_big(ptr as usize, layout.size(), false);
        }
        let prefix = layout.align().max(16);
        let base = unsafe { ptr.sub(prefix) };
        let tag = unsafe { base.read() };
        LIVE[tag as usize].fetch_sub(layout.size() as isize, Ordering::Relaxed);
        let full = unsafe { std::alloc::Layout::from_size_align_unchecked(layout.size() + prefix, layout.align().max(16)) };
        RESERVED[tag as usize].fetch_sub(full.size() as isize, Ordering::Relaxed);
        unsafe { std::alloc::System.dealloc(base, full) };
    }
}
