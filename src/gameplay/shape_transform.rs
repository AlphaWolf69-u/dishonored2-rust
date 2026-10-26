//! Closest-point query coordinate transforms, preserving Havok's four lanes.
pub type ShapeMatrix = [[f32; 4]; 4];

/// ae2f90 compares XYZ of all columns, retaining MAXPS's second-operand rule
/// for equal/unordered values. Body-cache W lanes do not affect pose equality.
pub fn matches_transform(first: ShapeMatrix, second: ShapeMatrix, tolerance: f32) -> bool {
    fn max(first: f32, second: f32) -> f32 {
        if first > second { first } else { second }
    }
    (0..3).all(|lane| {
        let delta: [f32; 4] = std::array::from_fn(|column|
            crate::float::subtract(second[column][lane], first[column][lane]).abs());
        max(max(delta[0], delta[1]), max(delta[2], delta[3])) <= tolerance
    })
}

/// aed170, also used by the compound shape's calcAabb virtual c97210.
/// Empty bounds are copied before applying any transform, including W.
pub fn transform_bounds(matrix: ShapeMatrix, bounds: [[f32; 4]; 2]) -> [[f32; 4]; 2] {
    if (0..3).any(|i| bounds[1][i] < bounds[0][i]) {
        return bounds;
    }
    let half: [f32; 3] = std::array::from_fn(|i| (bounds[1][i] - bounds[0][i]) * 0.5);
    let center: [f32; 3] = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5);
    let mut result = [[0.; 4]; 2];
    for i in 0..4 {
        let extent = ((half[0] * matrix[0][i]).abs() + (half[1] * matrix[1][i]).abs())
            + (half[2] * matrix[2][i]).abs();
        let transformed = ((center[0] * matrix[0][i] + center[1] * matrix[1][i])
            + center[2] * matrix[2][i])
            + matrix[3][i];
        result[0][i] = -extent + transformed;
        result[1][i] = transformed + extent;
    }
    result
}
#[path = "shape_transform/rotation.rs"]
pub mod rotation;

/// 140aa0620: rotation/linear part only; vector W is not an input.
pub fn rotate_vector(columns: ShapeMatrix, vector: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|i| {
        (vector[1] * columns[1][i] + vector[0] * columns[0][i]) + vector[2] * columns[2][i]
    })
}

/// 140aa06c0: linear transform followed by translation, in source order.
pub fn transform_point(columns: ShapeMatrix, point: [f32; 4]) -> [f32; 4] {
    let rotated = rotate_vector(columns, point);
    std::array::from_fn(|i| rotated[i] + columns[3][i])
}

/// 140aa0660: inverse rotation with inverse_rigid's exact W shuffle.
pub fn inverse_rotate_vector(columns: ShapeMatrix, vector: [f32; 4]) -> [f32; 4] {
    rotate_vector(inverse_rigid(columns), vector)
}

/// cada50: query frame in target coordinates. Subtract translation before
/// rotating it; composing with an inverse changes the source's rounding.
pub fn relative_rigid(query: ShapeMatrix, target: ShapeMatrix) -> ShapeMatrix {
    let mut relative = query.map(|column| inverse_rotate_vector(target, column));
    relative[3] =
        inverse_rotate_vector(target, std::array::from_fn(|i| query[3][i] - target[3][i]));
    relative
}

/// 140ae3000: inverse rigid transform, including the source shuffle's W lanes.
/// This operation requires a rigid basis; it is not a general matrix inverse.
pub fn inverse_rigid(columns: ShapeMatrix) -> ShapeMatrix {
    let a = columns;
    let mut result = [
        [a[0][0], a[1][0], a[2][0], a[2][1]],
        [a[0][1], a[1][1], a[2][1], a[2][3]],
        [a[0][2], a[1][2], a[2][2], a[2][3]],
        [0.; 4],
    ];
    result[3] = rotate_vector(result, a[3].map(|v| -v));
    result
}

/// 140ae3080: affine composition; translation is added after the three products.
pub fn compose(left: ShapeMatrix, right: ShapeMatrix) -> ShapeMatrix {
    let mut result = right.map(|column| rotate_vector(left, column));
    for i in 0..4 {
        result[3][i] = left[3][i] + result[3][i];
    }
    result
}

#[derive(Clone, Copy, Debug)]
pub struct ScaledShapeTransform {
    pub world_from_local: ShapeMatrix,
    pub scale: [f32; 4],
    pub offset: [f32; 4],
}

/// 14123eda0: map query geometry into the target's scaled shape coordinates.
/// Returns the relative transform and the target reciprocal scale separately.
pub fn relative_scaled(
    query: ScaledShapeTransform,
    target: ScaledShapeTransform,
) -> (ShapeMatrix, [f32; 4]) {
    let mut query_matrix = query.world_from_local;
    let offset = rotate_vector(query_matrix, query.offset);
    for lane in 0..4 {
        query_matrix[3][lane] += offset[lane];
    }
    for column in 0..3 {
        for lane in 0..4 {
            query_matrix[column][lane] *= query.scale[column];
        }
    }
    let reciprocal = target.scale.map(super::constraint_solver::reciprocal);
    let mut inverse = inverse_rigid(target.world_from_local);
    for column in 0..3 {
        for lane in 0..4 {
            inverse[column][lane] *= reciprocal[lane];
        }
    }
    for lane in 0..4 {
        inverse[3][lane] = (inverse[3][lane] - target.offset[lane]) * reciprocal[lane];
    }
    (compose(inverse, query_matrix), reciprocal)
}

#[cfg(test)]
#[path = "shape_transform/tests.rs"]
mod tests;
