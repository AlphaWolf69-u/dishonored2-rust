//! Host cache storage retires with the audio session, not just its entries.
use std::sync::Mutex;

/// Called only after the session closes producers. Decoder leases own their
/// payloads independently (141122400 / 1410f1910).
pub(crate) fn retire_cache<T: Default>(
    cache: &Mutex<T>, name: &str,
    usage: impl Fn(&T) -> (usize, usize), clear: impl FnOnce(&mut T),
) -> usize {
    let retain_capacity = std::env::var("DH2_DEBUG_RETAIN_SOUND_CACHE_CAPACITY").as_deref() == Ok("1");
    let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
    let (count, before) = usage(&cache);
    let retired = if retain_capacity {
        // Same-binary control for the former clear-only retirement.
        clear(&mut cache);
        None
    } else {
        Some(std::mem::take(&mut *cache))
    };
    let (_, after) = usage(&cache);
    drop(cache);
    // Payload destructors must run outside the process-wide cache lock.
    drop(retired);
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("MEMORY sound cache retirement: name={name} entries={count} capacity={before}->{after}");
    }
    count
}
