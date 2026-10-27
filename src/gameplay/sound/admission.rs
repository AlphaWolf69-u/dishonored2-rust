//! Direct listener admission in 1400b7b30. Cross-zone portal traversal remains
//! an explicit query supplied by the owner of the acoustic graph.
use bevy_math::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Admission { SourceOutside = 0, ListenerOutside = 1, Admitted = 2, Inaudible = 3 }

pub struct Query {
    pub source: Vec3,
    pub listener: Vec3,
    pub range: f32,
    pub vertical_scale: f32,
    pub propagation: bool,
    /// None means the position was resolved outside every acoustic zone.
    /// An unavailable zone lookup must be reported by the owner as an error.
    pub source_zone: Option<usize>,
    pub listener_zone: Option<usize>,
}

/// Motion published by an active listener. Inactive listener slots are None;
/// unavailable motion must be resolved by the owner rather than set to zero.
#[derive(Clone, Copy)]
pub struct ListenerMotion {
    pub position: Vec3,
    pub velocity: Vec3,
}

/// 140108ab0's squared point-to-segment distance. The degeneracy threshold at
/// 14228a764 is FLT_MIN; ordered comparisons retain the source's NaN branches.
fn segment_distance_squared(point: Vec3, start: Vec3, end: Vec3) -> f32 {
    let segment = end - start;
    let length = segment.x * segment.x + segment.y * segment.y + segment.z * segment.z;
    let relative = point - start;
    if length < f32::MIN_POSITIVE {
        return relative.y * relative.y + relative.x * relative.x + relative.z * relative.z;
    }
    let fraction = (relative.x * segment.x + relative.y * segment.y + relative.z * segment.z) / length;
    let relative = if 0.0 <= fraction {
        if 1.0 < fraction {
            let relative = point - end;
            return relative.y * relative.y + relative.x * relative.x + relative.z * relative.z;
        }
        relative - segment * fraction
    } else { relative };
    relative.y * relative.y + relative.x * relative.x + relative.z * relative.z
}

/// 1400c1670/1400fd730: the initial short-event admission test uses a source
/// motion segment and expands its range by each listener's published speed.
/// This is distinct from c1240's portal query used for virtualized replay.
pub fn motion_inaudible(source: Vec3, displacement: Vec3, range: f32,
    listeners: &[Option<ListenerMotion>; 8]) -> bool
{
    let end = source + displacement;
    for listener in listeners.iter().flatten() {
        let velocity = listener.velocity;
        let squared = velocity.x * velocity.x + velocity.y * velocity.y + velocity.z * velocity.z;
        let denominator = if squared <= f32::MIN_POSITIVE { f32::MIN_POSITIVE } else { squared };
        let speed = (1.0 / denominator.sqrt()) * squared;
        let radius = range + speed;
        if segment_distance_squared(listener.position, source, end) <= radius * radius { return false; }
    }
    true
}

pub fn admission(query: &Query, routes: impl FnOnce() -> Result<bool, String>) -> Result<Admission, String> {
    let relative = query.source - query.listener;
    let distance_squared = relative.y * relative.y + relative.x * relative.x + relative.z * relative.z;
    // Ordered comparisons preserve the executable's NaN behavior.
    if !(distance_squared < query.range * query.range) { return Ok(Admission::Inaudible); }
    if !query.propagation { return Ok(Admission::Admitted); }
    if query.source_zone == query.listener_zone {
        let relative = query.listener - query.source;
        let vertical = relative.z * query.vertical_scale;
        let square = relative.y * relative.y + relative.x * relative.x + vertical * vertical;
        let lower_bound = f32::from_bits(0x00800000);
        let denominator = if square <= lower_bound { lower_bound } else { square };
        let distance = (1.0 / denominator.sqrt()) * square;
        return Ok(if query.range <= distance && query.range != distance { Admission::Inaudible } else { Admission::Admitted });
    }
    if query.source_zone.is_none() { return Ok(Admission::SourceOutside); }
    if query.listener_zone.is_none() { return Ok(Admission::ListenerOutside); }
    Ok(if routes()? { Admission::Admitted } else { Admission::Inaudible })
}
