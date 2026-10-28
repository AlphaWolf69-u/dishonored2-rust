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

static LIST: std::sync::OnceLock<Vec<crate::assets::map_resources::Resource>> = std::sync::OnceLock::new();

/// Keep the current map's load lists for the coverage report.
pub(super) fn keep_load_list(catalog: &crate::assets::archive::Catalog) {
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_none_or(|v| v != "1") {
        return;
    }
    let Some(map) = crate::gameplay::map_info::current_map() else { return };
    let map = map.trim_start_matches("maps/").trim_end_matches(".map").to_owned();
    let language = crate::assets::lang::read_text_language(catalog.base()).unwrap_or("english");
    match crate::assets::map_resources::level_load_list(catalog.base(), catalog, &map, language) {
        Ok(list) => drop(LIST.set(list)),
        Err(e) => eprintln!("LOAD list {map}: {e}"),
    }
}

/// When every loading gate is complete: play starts, and the coverage of
/// the original's load lists is reported.
pub(super) fn level_ready(mut done: bevy::prelude::Local<bool>) {
    if *done || !crate::loading::ready() {
        return;
    }
    *done = true;
    crate::loading::play_started();
    mark("level ready");
    if let Some(list) = LIST.get() {
        let report = crate::loading::report(list);
        let (listed, loaded) = report.iter().fold((0, 0), |a, c| (a.0 + c.listed, a.1 + c.loaded));
        eprintln!("LOAD coverage: {loaded} of {listed} listed resources loaded");
        for c in report.iter().filter(|c| c.listed > 0 || c.unlisted > 0) {
            eprintln!("LOAD coverage {}: {} of {} listed, {} unlisted", c.kind, c.loaded, c.listed, c.unlisted);
        }
    }
}

/// Diagnostic: a resource loaded during play rather than at level load.
pub(super) fn late(kind: &str, what: &str) {
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_none_or(|v| v != "1") {
        return;
    }
    let at = MARKS.lock().unwrap().map_or(0.0, |(start, _)| start.elapsed().as_secs_f64());
    eprintln!("LOAD late {kind}: {what} (at {at:.3}s)");
}
