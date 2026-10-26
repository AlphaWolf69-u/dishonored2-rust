//! Portal path admission (1400b63b0/1400b5f70).
use super::admission::{self, Admission, Query};
use crate::assets::sound_propagation::{Portal, Propagation, Route};
use bevy_math::Vec3;

fn dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn distance(first: Vec3, second: Vec3, vertical_scale: f32) -> f32 {
    let relative = second - first;
    let vertical = relative.z * vertical_scale;
    let square = relative.y * relative.y + relative.x * relative.x + vertical * vertical;
    (1.0 / square.max(f32::from_bits(0x00800000)).sqrt()) * square
}

impl Portal {
    /// Plane intersection followed by clamping in the portal's authored
    /// rectangle axes. Parallel/out-of-segment intersections use the listener.
    pub fn path_point(&self, from: Vec3, listener: Vec3) -> Vec3 {
        let floats = self.geometry.map(f32::from_bits);
        let vector = |offset| Vec3::new(floats[offset], floats[offset + 1], floats[offset + 2]);
        let normal = vector(0);
        let fraction = (-floats[3] - dot(normal, from)) / dot(normal, listener - from);
        let point = if 0.0 <= fraction && fraction <= 1.0 {
            from + (listener - from) * fraction
        } else {
            listener
        };
        let center = vector(6);
        let first = vector(9);
        let second = vector(12);
        // These ordered comparisons also preserve the native NaN branch.
        let clamp = |value: f32, extent: f32| {
            if -extent <= value {
                if extent < value {
                    extent
                } else {
                    value
                }
            } else {
                -extent
            }
        };
        let u = clamp(dot(point - center, first), floats[4]);
        let v = clamp(dot(point - center, second), floats[5]);
        Vec3::new(
            u * first.x + center.x + v * second.x,
            u * first.y + center.y + v * second.y,
            u * first.z + center.z + v * second.z,
        )
    }
}

fn path_admitted(
    graph: &Propagation,
    query: &Query,
    route: &Route,
    reduction_scale: f32,
) -> Result<bool, String> {
    let mut zone = query.source_zone.ok_or("portal path has no source zone")?;
    let listener_zone = query
        .listener_zone
        .ok_or("portal path has no listener zone")?;
    let source = graph
        .zones
        .get(zone)
        .ok_or("missing source acoustic zone")?;
    let listener = graph
        .zones
        .get(listener_zone)
        .ok_or("missing listener acoustic zone")?;
    let reverse = listener.id < source.id;
    let mut remaining = query.range;
    let mut length = 0.0;
    let mut from = query.source;
    let mut reduction = f32::from_bits(source.properties[2]) * reduction_scale;
    for step in 0..route.portals.len() {
        remaining -= reduction * remaining;
        let index = if reverse {
            route.portals.len() - 1 - step
        } else {
            step
        };
        let portal = graph
            .portals
            .get(usize::from(route.portals[index]))
            .ok_or("missing acoustic path portal")?;
        let point = portal.path_point(from, query.listener);
        length += distance(from, point, query.vertical_scale);
        if remaining < length {
            return Ok(false);
        }
        // Runtime portal pointers are built from these array indices.
        zone = usize::from(if zone == usize::from(portal.zones[0]) {
            portal.zones[1]
        } else {
            portal.zones[0]
        });
        reduction = f32::from_bits(
            graph
                .zones
                .get(zone)
                .ok_or("missing acoustic path zone")?
                .properties[2],
        );
        from = point;
    }
    let reduction = f32::from_bits(listener.properties[2]);
    Ok(
        length + distance(from, query.listener, query.vertical_scale)
            <= remaining - reduction * remaining,
    )
}

pub fn admission(
    graph: &Propagation,
    query: &Query,
    reduction_scale: f32,
) -> Result<Admission, String> {
    admission::admission(query, || {
        let source = graph
            .zones
            .get(query.source_zone.ok_or("missing source zone")?)
            .ok_or("missing acoustic source zone")?;
        let listener = graph
            .zones
            .get(query.listener_zone.ok_or("missing listener zone")?)
            .ok_or("missing acoustic listener zone")?;
        let cost_limit = crate::float::truncate_i64(query.range * 1000.0) as u32;
        for route in graph
            .routes
            .between(source.id, listener.id)
            .map_err(|error| error.to_string())?
        {
            if route.cost_millimetres < cost_limit
                && path_admitted(graph, query, &route, reduction_scale)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    })
}
