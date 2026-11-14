//! Convex branch of 140ce3480, used while preparing a scaled compound instance.
//! The caller supplies the actual child vertices and instance frame; it must
//! retain the child shape and its resource identity for subsequent queries.
use super::{scaling::adjust_scale, transformed_vertex_bounds};
use crate::gameplay::shape_transform::{rotate_vector, transform_point, ShapeMatrix};

pub(crate) fn bounds(
    shape_type: u8,
    vertices: &[[f32; 4]],
    mut radius: f32,
    frame: ShapeMatrix,
    mut scale: [f32; 4],
    keep_radius: bool,
) -> Result<[[f32; 4]; 2], String> {
    let offset = adjust_scale(shape_type, vertices, keep_radius, &mut scale, &mut radius)?;
    if shape_type == 2 || shape_type == 3 {
        let count = if shape_type == 2 { 1 } else { 2 };
        if vertices.len() < count {
            return Err("scaled sphere/capsule is missing its source vertices".into());
        }
        let point = |vertex: [f32; 4]| transform_point(frame, vertex.map(|v| v * scale[0]));
        let first = point(vertices[0]);
        if count == 1 {
            return Ok([
                first.map(|value| value - radius),
                first.map(|value| radius + value),
            ]);
        }
        let second = point(vertices[1]);
        let limit = f32::from_bits(0x7f7f_ffee);
        let lower = first.map(|value| if limit < value { limit } else { value });
        let upper = first.map(|value| if -limit > value { -limit } else { value });
        return Ok([
            std::array::from_fn(|i| (if lower[i] < second[i] { lower[i] } else { second[i] }) - radius),
            std::array::from_fn(|i| radius + if upper[i] > second[i] { upper[i] } else { second[i] }),
        ]);
    }
    let mut scaled = frame;
    for column in 0..3 {
        for lane in 0..4 { scaled[column][lane] *= scale[column]; }
    }
    let shift = rotate_vector(frame, offset);
    for lane in 0..4 { scaled[3][lane] += shift[lane]; }
    let [lower, upper] = transformed_vertex_bounds(vertices, scaled);
    Ok([
        std::array::from_fn(|i| lower[i] - radius),
        std::array::from_fn(|i| radius + upper[i]),
    ])
}
