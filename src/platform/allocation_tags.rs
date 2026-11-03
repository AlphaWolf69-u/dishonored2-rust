//! Diagnostic (`debug_allocations` feature): live heap bytes per tag.
//!
//! The global allocator prefixes each block with the tag active on the
//! allocating thread ([`tag`]), so a block freed elsewhere is still charged
//! to its owner. Untagged allocations go to tag 0, "untagged". Without the
//! feature, [`tag`] is free and [`report`] is empty.
use std::sync::atomic::{AtomicIsize, Ordering};

const TAGS: usize = 64;
static LIVE: [AtomicIsize; TAGS] = [const { AtomicIsize::new(0) }; TAGS];
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

/// Live bytes per tag, largest first.
pub fn report() -> Vec<(&'static str, isize)> {
    let names = NAMES.lock().unwrap();
    let mut out: Vec<_> = names.iter().enumerate().map(|(i, n)| (*n, LIVE[i].load(Ordering::Relaxed))).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
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
        unsafe { std::alloc::System.dealloc(base, full) };
    }
}
