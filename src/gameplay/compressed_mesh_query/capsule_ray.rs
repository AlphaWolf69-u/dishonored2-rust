//! Packed capsule branch of14126c750, chunk751 lines29960..30747.
//! This retains its closest-lines cylinder/cap selection and inside-exit ray
//! reversal, rather than substituting a generic convex or quadratic cast.
use crate::gameplay::{
    constraint_solver::{inverse_length, reciprocal},
    convex_support::ray::ConvexRayHit,
};

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn point(origin: [f32; 3], direction: [f32; 3], fraction: f32) -> [f32; 3] {
    std::array::from_fn(|i| direction[i] * fraction + origin[i])
}
fn length(squared: f32) -> f32 {
    inverse_length(squared) * squared
}
fn segment_distance_squared(a: [f32; 3], b: [f32; 3], origin: [f32; 3], squared: f32) -> f32 {
    let reverse = sub(a, b);
    let delta = sub(a, origin);
    let fraction = reciprocal(squared) * dot(delta, reverse);
    // MINPS/MAXPS's second-operand selection, including unordered input.
    let fraction = if fraction < 1. { fraction } else { 1. };
    let fraction = if 0. < fraction { fraction } else { 0. };
    let distance = std::array::from_fn(|i| delta[i] - fraction * reverse[i]);
    dot(distance, distance)
}

pub fn capsule(
    vertices: &[[f32; 4]],
    mut origin: [f32; 3],
    mut direction: [f32; 3],
    radius: f32,
    maximum_fraction: f32,
    query_flags: u32,
) -> Result<Option<ConvexRayHit>, String> {
    let a: [f32; 3] = vertices
        .first()
        .ok_or("compressed capsule first endpoint absent")?[..3]
        .try_into()
        .unwrap();
    let b: [f32; 3] = vertices
        .get(1)
        .ok_or("compressed capsule second endpoint absent")?[..3]
        .try_into()
        .unwrap();
    if direction == [0.; 3] {
        return Ok(None);
    }
    let axis = sub(b, a);
    let axis_squared = dot(axis, axis);
    // The source's zero-axis reciprocal produces an unordered closest-lines
    // distance, so the cylinder/cap admission fails; no sphere is fabricated.
    if axis_squared == 0. {
        return Ok(None);
    }
    let radius_squared = radius * radius;
    let inside = segment_distance_squared(a, b, origin, axis_squared) < radius_squared;
    let mut limit = maximum_fraction;
    let mut reversal = 1.;
    if inside {
        // 141e846a0's queryflag4 mask is all ones only for exit queries.
        if query_flags & 4 == 0 {
            return Ok(None);
        }
        let end = point(origin, direction, maximum_fraction);
        if !(radius_squared <= segment_distance_squared(a, b, end, axis_squared)) {
            return Ok(None);
        }
        let direction_length = length(dot(direction, direction));
        // Installed1423ccc60 is4.0: move outside by at most axis length+4r.
        let extension = length(axis_squared) + radius * 4.;
        let requested = maximum_fraction * direction_length;
        let extension = if extension < requested {
            extension
        } else {
            requested
        };
        reversal = if direction_length == 0. {
            0.
        } else {
            reciprocal(direction_length) * extension
        };
        origin = point(origin, direction, reversal);
        direction = std::array::from_fn(|i| -(direction[i] * reversal));
        limit = 1.;
    }

    let delta = sub(a, origin);
    let direction_squared = dot(direction, direction);
    let axis_direction = dot(direction, axis);
    let delta_axis = dot(delta, axis);
    let delta_direction = dot(delta, direction);
    let product = axis_squared * direction_squared;
    let denominator = product - axis_direction * axis_direction;
    //1423ccca0=8;1423cce30=EPSILON, read from the executable constant table.
    let parallel_threshold = (axis_direction * axis_direction + product) * 8. * f32::EPSILON;
    let closest_ray = if parallel_threshold < denominator {
        reciprocal(denominator) * (delta_direction * axis_squared - delta_axis * axis_direction)
    } else {
        1.
    };
    let closest_axis = reciprocal(axis_squared) * (closest_ray * axis_direction - delta_axis);
    let separation = sub(
        point(a, axis, closest_axis),
        point(origin, direction, closest_ray),
    );
    let distance_squared = dot(separation, separation);
    if !(distance_squared <= radius_squared) {
        return Ok(None);
    }

    let inverse = if f32::EPSILON < axis_squared {
        inverse_length(axis_squared)
    } else {
        0.
    };
    let unit_axis: [f32; 3] = std::array::from_fn(|i| axis[i] * inverse);
    let axis_length = inverse * axis_squared;
    let direction_projection = dot(unit_axis, direction);
    let perpendicular =
        std::array::from_fn(|i| (0. - direction_projection) * unit_axis[i] + direction[i]);
    let perpendicular_squared = dot(perpendicular, perpendicular);
    let offset = length(reciprocal(perpendicular_squared) * (radius_squared - distance_squared));
    //1423ccc10=-1.0, the source's parallel-cylinder sentinel.
    let fraction = if perpendicular_squared == 0. {
        -1.
    } else {
        closest_ray - offset
    };
    if !(fraction < limit) {
        return Ok(None);
    }
    let hit_position = point(origin, direction, fraction);
    let projection = dot(hit_position, unit_axis) - dot(a, unit_axis);
    let (fraction, normal) = if fraction < 0. || projection <= 0. || axis_length <= projection {
        let axis_origin = (dot(origin, unit_axis) - dot(a, unit_axis)) * reciprocal(axis_length);
        let distance = sub(origin, point(a, axis, axis_origin));
        if fraction < 0. && !(dot(distance, distance) <= radius_squared) {
            return Ok(None);
        }
        let center = if projection <= 0. { a } else { b };
        let Some(hit) = super::convex_ray::sphere(
            [center[0], center[1], center[2], 0.],
            origin,
            direction,
            radius,
            limit,
            0,
        ) else {
            return Ok(None);
        };
        (hit.fraction, hit.normal)
    } else {
        let along_axis = reciprocal(axis_length) * projection;
        let normal = sub(hit_position, point(a, axis, along_axis));
        let inverse = inverse_length(dot(normal, normal));
        (
            fraction,
            [
                normal[0] * inverse,
                normal[1] * inverse,
                normal[2] * inverse,
                0.,
            ],
        )
    };
    let fraction = if inside {
        (1. - fraction) * reversal
    } else {
        fraction
    };
    if !(fraction <= maximum_fraction) {
        return Ok(None);
    }
    Ok(Some(ConvexRayHit {
        fraction,
        normal,
        flags: if inside { 5 } else { 2 },
    }))
}
