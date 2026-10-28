//! Diagnostic: `DH2_DEBUG_LOAD_TIMING=1` logs the time of each load phase,
//! since the previous phase and since the first mark.
use std::sync::Mutex;
use std::time::Instant;

static MARKS: Mutex<Option<(Instant, Instant)>> = Mutex::new(None);

pub(super) fn mark(phase: &str) {
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_none_or(|v| v != "1") {
        return;
    }
    let now = Instant::now();
    let mut marks = MARKS.lock().unwrap();
    let (start, last) = marks.get_or_insert((now, now));
    eprintln!("LOAD {phase}: +{:.3}s (at {:.3}s)", (now - *last).as_secs_f64(), (now - *start).as_secs_f64());
    *last = now;
}

/// Marks the first frame, then "streaming settled" once no texture upload
/// is pending and the resident count has held for 60 frames.
pub(super) fn track_streaming(scene: bevy::prelude::Res<super::SceneRenderQueue>, mut state: bevy::prelude::Local<(bool, usize, u32, bool)>) {
    let (first, resident, stable, settled) = &mut *state;
    if !*first {
        *first = true;
        mark("first frame");
    }
    if *settled {
        return;
    }
    let now = scene.diffuse_image_handles.iter().filter(|h| h.is_some()).count();
    let pending = scene.diffuse_image_pending.iter().any(|p| p.is_some());
    if now == *resident && !pending {
        *stable += 1;
    } else {
        *stable = 0;
        *resident = now;
    }
    if *stable == 60 {
        *settled = true;
        mark(&format!("streaming settled ({now} of {} images resident)", scene.diffuse_image_handles.len()));
    }
}

/// Diagnostic (with `DH2_DEBUG_LOAD_TIMING=1`): the scene's images against
/// the original's load lists for the current map (`init`, `common`, map).
pub(super) fn compare_images(catalog: &crate::assets::archive::Catalog, scene: &[String]) {
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_none_or(|v| v != "1") {
        return;
    }
    let Some(map) = crate::gameplay::map_info::current_map() else { return };
    let map = map.trim_start_matches("maps/").trim_end_matches(".map").to_owned();
    let language = crate::assets::lang::read_text_language(catalog.base()).unwrap_or("english");
    let list = match crate::assets::map_resources::level_load_list(catalog.base(), catalog, &map, language) {
        Ok(list) => list,
        Err(e) => return eprintln!("LOAD list {map}: {e}"),
    };
    let listed: std::collections::HashSet<&str> = list.iter().filter(|r| r.kind == "image").map(|r| r.path.as_str()).collect();
    let outside: Vec<&String> = scene.iter().filter(|p| !listed.contains(p.as_str())).collect();
    eprintln!("LOAD list {map}: {} resources, {} images; scene images {}, not listed {}: {:?}", list.len(), listed.len(), scene.len(), outside.len(), outside.iter().take(12).collect::<Vec<_>>());
}
