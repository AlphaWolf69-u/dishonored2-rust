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
