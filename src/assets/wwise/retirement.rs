//! Host cache storage retires with the audio session, not just its entries.
use std::{collections::HashMap, sync::Mutex};

/// Called only after the session closes producers. Decoder leases own their
/// payloads independently (141122400 / 1410f1910).
pub(crate) fn retire_cache<K, V>(cache: &Mutex<HashMap<K, V>>, name: &str) -> usize {
    let retain_capacity = std::env::var("DH2_DEBUG_RETAIN_SOUND_CACHE_CAPACITY").as_deref() == Ok("1");
    let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
    let count = cache.len();
    let before = cache.capacity();
    let retired = if retain_capacity {
        // Same-binary control for the former clear-only retirement.
        cache.clear();
        None
    } else {
        Some(std::mem::take(&mut *cache))
    };
    let after = cache.capacity();
    drop(cache);
    // Payload destructors must run outside the process-wide cache lock.
    drop(retired);
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("MEMORY sound cache retirement: name={name} entries={count} capacity={before}->{after} logical_slot_bytes={}",
            std::mem::size_of::<(K, V)>());
    }
    count
}
