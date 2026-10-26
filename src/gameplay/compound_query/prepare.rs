//! 1412329d0 query bounds and distance preparation before float-tree traversal.
use super::CompoundQueryBounds;
use crate::gameplay::{
    constraint_solver::reciprocal,
    convex_support::transformed_vertex_bounds,
    shape_transform::{relative_scaled, ScaledShapeTransform, ShapeMatrix},
};

pub trait CompoundQueryGeometry {
    /// The query shape's actual calcAabb virtual operation, including its radius.
    fn calculate_bounds(&mut self, matrix: ShapeMatrix) -> Result<[[f32; 4]; 2], String>;
    /// Shape flag bit0 admits direct vertex bounds for a scaled target only.
    fn convex_vertices(&self) -> Option<(&[[f32; 4]], f32)>;
}

#[derive(Clone, Copy, Debug)]
pub struct CompoundQueryTransform {
    pub shape: ScaledShapeTransform,
    pub scaled: bool,
}

pub struct PreparedCompoundQuery {
    pub bounds: CompoundQueryBounds,
    pub squared_distance: f32,
}

fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

pub fn prepare(
    geometry: &mut impl CompoundQueryGeometry,
    query: CompoundQueryTransform,
    target: CompoundQueryTransform,
    relative: ShapeMatrix,
    distance: f32,
) -> Result<PreparedCompoundQuery, String> {
    let mut inverse_scale = [1.; 4];
    let bounds = if query.scaled {
        let (matrix, scale) = relative_scaled(query.shape, target.shape);
        inverse_scale = scale;
        geometry.calculate_bounds(matrix)?
    } else if target.scaled {
        inverse_scale = target.shape.scale.map(reciprocal);
        let matrix =
            relative.map(|column| std::array::from_fn(|lane| column[lane] * inverse_scale[lane]));
        if let Some((vertices, radius)) = geometry.convex_vertices() {
            let mut bounds = transformed_vertex_bounds(vertices, matrix);
            let expansion = max(
                radius * inverse_scale[2],
                max(radius * inverse_scale[1], radius * inverse_scale[0]),
            );
            for lane in 0..4 {
                bounds[0][lane] -= expansion;
                bounds[1][lane] += expansion;
            }
            bounds
        } else {
            geometry.calculate_bounds(matrix)?
        }
    } else {
        geometry.calculate_bounds(relative)?
    };
    let mut radius = max(0., distance);
    if target.scaled {
        let scale = inverse_scale.map(|v| f32::from_bits(v.to_bits() & 0x7fff_ffff));
        radius *= max(scale[2], max(scale[1], scale[0]));
    }
    Ok(PreparedCompoundQuery {
        bounds: CompoundQueryBounds {
            center: std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5),
            half_extent: std::array::from_fn(|i| (bounds[1][i] - bounds[0][i]) * 0.5),
        },
        squared_distance: radius * radius,
    })
}
