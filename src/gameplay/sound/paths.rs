//! Active-emitter propagation results (1400b7830/1400b8e00/1400b5350).
use crate::assets::sound_propagation::{Propagation, Route};
use bevy_math::Vec3;

#[derive(Clone, Copy)]
pub struct Query {
    pub source: Vec3,
    pub listener: Vec3,
    pub source_zone: Option<usize>,
    pub listener_zone: Option<usize>,
    pub range: f32,
    pub propagation: bool,
    pub reflection_normal: Option<Vec3>,
    pub source_obstruction_scale: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ZoneSegment {
    pub zone: usize,
    pub distance: f32,
}

#[derive(Clone, Debug)]
pub struct Path {
    pub position: Vec3,
    pub distance: f32,
    /// Direct/inaudible constructors leave the native per-path score unset.
    pub score: Option<f32>,
    pub occlusion: f32,
    pub obstruction: f32,
    /// The source retains a four-entry ring, including its final insertion slot.
    pub zones: [Option<ZoneSegment>; 4],
    pub zone_count: usize,
    pub last_zone_slot: usize,
}
impl Default for Path {
    fn default() -> Self {
        // 1400b97e0 initializes distance to 32768, not zero.
        Self {
            position: Vec3::ZERO,
            distance: 32768.0,
            score: None,
            occlusion: 0.0,
            obstruction: 0.0,
            zones: [None; 4],
            zone_count: 0,
            last_zone_slot: 0,
        }
    }
}

#[derive(Debug)]
pub struct Results {
    pub paths: Vec<Path>,
    pub closest: Option<usize>,
    pub score: f32,
    /// 1400b9580 retains the cheapest route ending at each listener-side portal.
    pub portal_paths: Vec<PortalPath>,
}
#[derive(Clone, Copy, Debug)]
pub struct PortalPath {
    pub portal: u16,
    pub path: Option<usize>,
}
impl Default for Results {
    fn default() -> Self {
        Self {
            paths: Vec::new(),
            closest: None,
            score: f32::MAX,
            portal_paths: Vec::new(),
        }
    }
}

fn squared(vector: Vec3) -> f32 {
    vector.y * vector.y + vector.x * vector.x + vector.z * vector.z
}
fn inverse_length(vector: Vec3) -> f32 {
    1.0 / squared(vector).max(f32::from_bits(0x00800000)).sqrt()
}
fn length(vector: Vec3) -> f32 {
    inverse_length(vector) * squared(vector)
}
fn dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}
fn unit_clamp(value: f32) -> f32 {
    if 0.0 <= value {
        if 1.0 < value {
            1.0
        } else {
            value
        }
    } else {
        0.0
    }
}
fn reflect(position: Vec3, query: &Query) -> Vec3 {
    let Some(normal) = query.reflection_normal else {
        return position;
    };
    let projection = dot(position - query.listener, normal);
    if projection < 0.0 {
        position + normal * (projection * -2.0)
    } else {
        position
    }
}

/// Per-listener cache and mask transition in 1400c6520. `previous_range`
/// is the update caller's saved range, distinct from the current object range.
#[derive(Default)]
pub struct ListenerState {
    evaluated: Option<Query>,
    pub results: Results,
    pub occlusion: f32,
    pub obstruction: f32,
}
impl ListenerState {
    pub fn update(
        &mut self,
        graph: &Propagation,
        query: &Query,
        previous_range: f32,
        force: bool,
        listener_mask: u32,
        audible_mask: &mut u32,
    ) -> Result<(), String> {
        let changed = self.evaluated.is_none_or(|previous| {
            let movement = previous.listener - query.listener;
            previous.source != query.source
                || previous.range != previous_range
                || !(movement.x.abs() <= 0.01
                    && movement.y.abs() <= 0.01
                    && movement.z.abs() <= 0.01)
        });
        if force || changed {
            self.results = evaluate(graph, query)?;
            self.evaluated = Some(*query);
        }
        if let Some(index) = self.results.closest {
            let path = self
                .results
                .paths
                .get(index)
                .ok_or("closest propagation path missing")?;
            self.occlusion = path.occlusion;
            self.obstruction = path.obstruction;
            *audible_mask |= listener_mask;
        } else {
            self.occlusion = 1.0;
            self.obstruction = 1.0;
            *audible_mask &= !listener_mask;
        }
        Ok(())
    }
}
fn obstruction(graph: &Propagation, zone: Option<usize>) -> Result<f32, String> {
    zone.map(|index| {
        graph
            .zones
            .get(index)
            .map(|zone| f32::from_bits(zone.properties[1]))
            .ok_or_else(|| "missing obstruction zone".into())
    })
    .unwrap_or(Ok(0.0))
}

/// 1400b90c0: retain a far-away virtual source and an invalid closest index.
fn inaudible(query: &Query) -> Results {
    Results {
        paths: vec![Path {
            position: query.listener + Vec3::Z * (32768.0 * 2.0),
            ..Path::default()
        }],
        ..Results::default()
    }
}

/// 1400b8e00 (range gate here is strict; 1400c6760 has its own inclusive gate).
pub fn direct(graph: &Propagation, query: &Query) -> Result<Results, String> {
    let distance = length(query.source - query.listener);
    if query.range <= distance && distance != query.range {
        return Ok(inaudible(query));
    }
    let source_obstruction =
        obstruction(graph, query.source_zone)? * query.source_obstruction_scale;
    let obstruction = source_obstruction + obstruction(graph, query.listener_zone)?;
    // Unlike the portal projection's JA branch, the direct projection takes
    // its reflection branch on unordered input as well (1400b8e00).
    let position = if let Some(normal) = query.reflection_normal {
        let projection = dot(normal, query.source - query.listener);
        if 0.0 <= projection {
            query.source
        } else {
            query.source + normal * (projection * -2.0)
        }
    } else {
        query.source
    };
    let mut path = Path {
        position,
        distance,
        obstruction: unit_clamp(obstruction),
        ..Path::default()
    };
    if let Some(zone) = query.source_zone {
        path.zones[0] = Some(ZoneSegment { zone, distance });
        path.zone_count = 1;
    }
    Ok(Results {
        paths: vec![path],
        closest: Some(0),
        score: (obstruction * 100.0 + 10.0 * 0.0 + 1.0) * (distance / query.range),
        ..Results::default()
    })
}

/// 1400b8c90: multi-position emitters bypass the range/portal solver but
/// retain source/listener zone obstruction and the raw distance as score.
pub fn unpropagated(graph: &Propagation, query: &Query) -> Result<Results, String> {
    let distance = length(query.source - query.listener);
    let mut path = Path {
        position: query.source,
        distance,
        obstruction: unit_clamp(
            obstruction(graph, query.source_zone)? + obstruction(graph, query.listener_zone)?,
        ),
        ..Path::default()
    };
    if let Some(zone) = query.source_zone {
        path.zones[0] = Some(ZoneSegment { zone, distance });
        path.zone_count = 1;
    }
    Ok(Results {
        paths: vec![path],
        closest: Some(0),
        score: distance,
        ..Results::default()
    })
}

fn portal_path(
    graph: &Propagation,
    query: &Query,
    route: &Route,
) -> Result<Option<(Path, u16)>, String> {
    if !(route.cost_millimetres as f32 <= query.range * 1000.0) {
        return Ok(None);
    }
    let mut zone = query.source_zone.ok_or("portal path has no source zone")?;
    let listener_zone = query
        .listener_zone
        .ok_or("portal path has no listener zone")?;
    let reverse = graph
        .zones
        .get(listener_zone)
        .ok_or("missing listener zone")?
        .id
        < graph.zones.get(zone).ok_or("missing source zone")?.id;
    if route.portals.is_empty() {
        return Err("acoustic route has no portal".into());
    }
    let mut path = Path::default();
    let mut point = query.source;
    let mut direction = Vec3::ZERO;
    let mut distance = 0.0;
    let mut bends = 0.0;
    let mut obstruction_sum = obstruction(graph, Some(zone))? * query.source_obstruction_scale;
    let mut last_portal = 0;
    for step in 0..route.portals.len() {
        let index = if reverse {
            route.portals.len() - 1 - step
        } else {
            step
        };
        last_portal = route.portals[index];
        let portal = graph
            .portals
            .get(usize::from(last_portal))
            .ok_or("missing path portal")?;
        let next = portal.path_point(point, query.listener);
        let relative = next - point;
        let segment = length(relative);
        distance += segment;
        if query.range < distance {
            return Ok(None);
        }
        let next_direction = relative * inverse_length(relative);
        if step != 0 {
            bends -= (dot(next_direction, direction) - 1.0) * 0.5;
            obstruction_sum += obstruction(graph, Some(zone))?;
            path.last_zone_slot = (path.last_zone_slot + 1) & 3;
        }
        path.zones[path.last_zone_slot] = Some(ZoneSegment {
            zone,
            distance: segment,
        });
        zone = usize::from(if zone == usize::from(portal.zones[0]) {
            portal.zones[1]
        } else {
            portal.zones[0]
        });
        direction = next_direction;
        point = next;
    }
    let relative = query.listener - point;
    let final_distance = length(relative);
    let total = final_distance + distance;
    if !(total <= query.range) {
        return Ok(None);
    }
    obstruction_sum += obstruction(graph, Some(listener_zone))?;
    let final_direction = relative * inverse_length(relative);
    let inverse = inverse_length(direction);
    let turn = final_direction.y * direction.y * inverse
        + final_direction.x * direction.x * inverse
        + final_direction.z * direction.z * inverse;
    bends -= (turn - 1.0) * 0.5;
    path.score = Some((obstruction_sum * 100.0 + bends * 10.0 + 1.0) * (total / query.range));
    path.occlusion = unit_clamp(bends);
    path.obstruction = unit_clamp(obstruction_sum);
    path.distance = total;
    path.position = reflect(query.listener + final_direction * -total, query);
    path.last_zone_slot = (path.last_zone_slot + 1) & 3;
    path.zones[path.last_zone_slot] = Some(ZoneSegment {
        zone: listener_zone,
        distance: final_distance,
    });
    path.zone_count = (route.portals.len() + 1).min(4);
    Ok(Some((path, last_portal)))
}

/// 1400c6760 range admission followed by 1400b7830's path dispatch.
pub fn evaluate(graph: &Propagation, query: &Query) -> Result<Results, String> {
    // 1400c6760 sizes the map to the listener zone's portal list and zeroes
    // every entry, including index zero. Preserve these entries and their
    // order instead of replacing this source container with a deduplicated map.
    let mut portal_paths = Vec::new();
    if let Some(zone) = query.listener_zone {
        graph.zones.get(zone).ok_or("missing listener zone")?;
        let count = graph
            .portals
            .iter()
            .flat_map(|portal| portal.zones)
            .filter(|&endpoint| usize::from(endpoint) == zone)
            .count();
        portal_paths.resize(
            count,
            PortalPath {
                portal: 0,
                path: Some(0),
            },
        );
    }
    if query.range <= length(query.listener - query.source) {
        let mut result = inaudible(query);
        result.portal_paths = portal_paths;
        return Ok(result);
    }
    if !query.propagation || query.source_zone.is_none() || query.listener_zone.is_none() {
        let mut result = direct(graph, query)?;
        result.portal_paths = portal_paths;
        return Ok(result);
    }
    let source = graph
        .zones
        .get(query.source_zone.unwrap())
        .ok_or("missing source zone")?;
    let listener = graph
        .zones
        .get(query.listener_zone.unwrap())
        .ok_or("missing listener zone")?;
    let mut results = if query.source_zone == query.listener_zone {
        direct(graph, query)?
    } else {
        Results::default()
    };
    results.portal_paths = portal_paths;
    for route in graph
        .routes
        .between(source.id, listener.id)
        .map_err(|error| error.to_string())?
    {
        let Some((path, portal)) = portal_path(graph, query, &route)? else {
            continue;
        };
        let score = path.score.ok_or("portal path has no score")?;
        let index = results.paths.len();
        if score < results.score {
            results.closest = Some(index);
            results.score = score;
        }
        results.paths.push(path);
        let entry = match results
            .portal_paths
            .iter()
            .position(|entry| entry.portal == portal)
        {
            Some(index) => index,
            None => {
                results.portal_paths.push(PortalPath { portal, path: None });
                results.portal_paths.len() - 1
            }
        };
        let previous = results.portal_paths[entry].path;
        let previous_score = previous
            .map(|previous| {
                results
                    .paths
                    .get(previous)
                    .ok_or("portal map references missing path")?
                    .score
                    .ok_or("portal map references an unset direct-path score")
            })
            .transpose()?
            .unwrap_or(f32::MAX);
        if score < previous_score {
            results.portal_paths[entry].path = Some(index);
        }
    }
    if results.paths.is_empty() {
        let mut result = inaudible(query);
        result.portal_paths = results.portal_paths;
        return Ok(result);
    }
    Ok(results)
}
