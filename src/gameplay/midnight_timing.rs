//! Marker-based Midnight clip timing shared by world and player scenes.
use crate::entities::midnight::AnimKey;

#[derive(Clone, Copy, Debug)]
struct Point {
    kind: u8,
    animation_ms: i32,
    scene_ms: i32,
}

/// The original runtime builds these points once when a scene binds a clip.
#[derive(Clone, Debug)]
pub struct MarkerTiming {
    points: Vec<Point>,
    loops: i32,
}

impl MarkerTiming {
    /// 14018e5a0/14018e620: pair each `ae_marker` with the scene marker
    /// of the same name; missing matches are an invalid authored binding.
    pub(crate) fn new(key: &AnimKey, markers: &[crate::edge_animation::AnimMarker], frame_rate: u32, duration_ms: i32) -> Result<Self, String> {
        if frame_rate == 0 || key.duration_ms <= 0 {
            return Err("invalid marker clip timing".into());
        }
        let scene_markers = key.marker_values.as_ref().map_err(Clone::clone)?;
        if scene_markers.len() != key.markers { return Err("Midnight marker count differs".into()); }
        let mut points = vec![Point { kind: 0, animation_ms: key.start_offset, scene_ms: key.start_ms }];
        for event in markers {
            let name = match event.kind {
                0 => "EMAM_IN", 1 => "EMAM_APEX", 2 => "EMAM_ACCENT", 3 => "EMAM_LOOP",
                4 => "EMAM_LOOPEND", 5 => "EMAM_OUT", 6 => "EMAM_INVALID",
                _ => return Err("unknown animation marker kind".into()),
            };
            let kind = if event.kind == 4 { 3 } else { 2 };
            let marker = scene_markers.iter().find(|marker| marker.name == name)
                .ok_or_else(|| format!("scene clip lacks animation marker {name}"))?;
            if marker.time_ms < key.start_offset || marker.time_ms > key.duration_ms - key.end_offset { continue; }
            let point = Point {
                kind,
                animation_ms: i32::from(event.frame).wrapping_mul(1000) / frame_rate as i32,
                scene_ms: key.start_ms + marker.time_ms.min(key.duration_ms - 1),
            };
            // 14018f2a0 replaces a coincident previous point unless it is a loop end.
            if points.last().is_some_and(|previous| previous.scene_ms == point.scene_ms && previous.kind != 3) { points.pop(); }
            points.push(point);
        }
        let scene_duration = if !key.stretched && scene_markers.is_empty() { duration_ms } else { key.duration_ms };
        let end = Point { kind: 1, animation_ms: duration_ms - key.end_offset,
            scene_ms: key.start_ms + scene_duration - 1 };
        if points.last().is_some_and(|previous| previous.scene_ms == end.scene_ms && previous.kind != 3) { points.pop(); }
        points.push(end);
        if points.len() < 2 || points.windows(2).any(|pair| pair[0].scene_ms > pair[1].scene_ms) {
            return Err("animation markers are not in scene order".into());
        }
        // 14018fcf0: the final interval determines the number of cycles.
        let loops = if points.iter().any(|point| point.kind == 3) {
            let a = points[points.len() - 2];
            let b = points[points.len() - 1];
            let span = b.animation_ms - a.animation_ms;
            let ratio = (b.scene_ms - a.scene_ms) as f32 / span as f32;
            if ratio > 1.0 { (ratio + 0.5).floor() as i32 } else { 1 }
        } else { 0 };
        Ok(Self { points, loops })
    }

    /// 14018fdd0/1406ff870: select the marker interval and map scene time
    /// into animation frames, wrapping intervals that end at a loop marker.
    pub fn frame(&self, scene_ms: i32, frame_rate: u32) -> Option<f32> {
        let first = self.points.first()?;
        let last = self.points.last()?;
        if scene_ms < first.scene_ms || scene_ms > last.scene_ms { return None; }
        let mut index = 0;
        while index + 2 < self.points.len() {
            let next = self.points[index + 1];
            if scene_ms <= next.scene_ms && (scene_ms != next.scene_ms || last.scene_ms <= next.scene_ms) { break; }
            index += 1;
        }
        let (a, b) = (self.points[index], self.points[index + 1]);
        let span = b.scene_ms - a.scene_ms;
        let mut fraction = if span == 0 { 0.0 } else { (scene_ms - a.scene_ms) as f32 / span as f32 };
        if b.kind == 3 && self.loops > 0 {
            let part = 1.0 / self.loops as f32;
            fraction = (fraction % part) / part;
        }
        Some(frame_rate as f32 * (a.animation_ms as f32 + (b.animation_ms - a.animation_ms) as f32 * fraction) * 0.001)
    }
}
