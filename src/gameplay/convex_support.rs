//! Convex support preparation, point distance and penetration geometry.
#[path = "convex_support/distance.rs"]
pub mod distance;
#[path = "convex_support/faces.rs"]
pub mod faces;
#[path = "convex_support/feature_query.rs"]
pub mod feature_query;
#[path = "convex_support/horizon.rs"]
pub mod horizon;
#[path = "convex_support/hull.rs"]
pub mod hull;
#[path = "convex_support/penetration.rs"]
pub mod penetration;
#[path = "convex_support/point_distance.rs"]
pub mod point_distance;
#[path = "convex_support/scaling.rs"]
pub mod scaling;
#[path = "convex_support/seed.rs"]
pub mod seed;
#[path = "convex_support/solver.rs"]
pub mod solver;
#[path = "convex_support/vertices.rs"]
pub mod vertices;

#[derive(Clone, Copy, Debug)]
pub struct ConvexSupportScale {
    pub scale: [f32; 3],
    pub offset: [f32; 3],
    pub radius: f32,
}

#[derive(Clone, Copy)]
pub struct ConvexSupportInput<'a> {
    pub shape_type: u8,
    pub vertices: &'a [[f32; 4]],
    pub radius: f32,
    pub scaled: Option<ConvexSupportScale>,
}

pub struct ConvexSupport<'a> {
    pub kind: u8,
    pub vertices: &'a [[f32; 4]],
    pub radius: f32,
}

/// 140ca1d90: transform convex vertices into an AABB. Translation is applied
/// after extrema, preserving the four-lane source arithmetic and empty bounds.
pub fn transformed_vertex_bounds(vertices: &[[f32; 4]], columns: [[f32; 4]; 4]) -> [[f32; 4]; 2] {
    let [lower, upper] = vertex_extrema(vertices, columns);
    [
        std::array::from_fn(|lane| columns[3][lane] + lower[lane]),
        std::array::from_fn(|lane| upper[lane] + columns[3][lane]),
    ]
}

/// ca1e40: convex calcAabb expands all four lanes by the shape radius.
/// Its minimum translation operands differ from ca1d90's vertex-only helper.
pub fn convex_bounds(vertices: &[[f32; 4]], columns: [[f32; 4]; 4], radius: f32) -> [[f32; 4]; 2] {
    let [lower, upper] = vertex_extrema(vertices, columns);
    [
        std::array::from_fn(|lane| {
            crate::float::subtract(crate::float::add(lower[lane], columns[3][lane]), radius)
        }),
        std::array::from_fn(|lane| {
            crate::float::add(crate::float::add(columns[3][lane], upper[lane]), radius)
        }),
    ]
}

fn vertex_extrema(vertices: &[[f32; 4]], columns: [[f32; 4]; 4]) -> [[f32; 4]; 2] {
    let limit = f32::from_bits(0x7f7f_ffee);
    let mut lower = [limit; 4];
    let mut upper = [-limit; 4];
    for vertex in vertices {
        for lane in 0..4 {
            let value = (vertex[0] * columns[0][lane] + vertex[1] * columns[1][lane])
                + vertex[2] * columns[2][lane];
            // SSE MINPS/MAXPS select the second operand when unordered.
            lower[lane] = if lower[lane] < value {
                lower[lane]
            } else {
                value
            };
            upper[lane] = if upper[lane] > value {
                upper[lane]
            } else {
                value
            };
        }
    }
    [lower, upper]
}

/// A query's two inputs require independent workspaces while both are borrowed.
/// Unscaled input borrows the actual shape data without copying it.
#[derive(Default)]
pub struct ConvexSupportWorkspace {
    scaled: Vec<[f32; 4]>,
}

impl ConvexSupportWorkspace {
    pub fn prepare<'a>(
        &'a mut self,
        input: ConvexSupportInput<'a>,
    ) -> Result<ConvexSupport<'a>, String> {
        // 1423eeda9[shape_type*2]. Other shape types route through their own
        // handlers; do not send their table value8 into an8x8 convex kernel.
        let kind = *[4, 5, 0, 2, 3, 5, 5]
            .get(usize::from(input.shape_type))
            .ok_or("non-convex shape routed to convex support preparation")?;
        if input.vertices.len() > usize::from(u16::MAX) {
            return Err("convex support vertex count exceeds source16-bit field".into());
        }
        let (vertices, radius) = if let Some(scale) = input.scaled {
            self.scaled.clear();
            self.scaled.extend(input.vertices.iter().map(|vertex| {
                let mut result = *vertex;
                for axis in 0..3 {
                    result[axis] = scale.scale[axis] * vertex[axis] + scale.offset[axis];
                }
                result
            }));
            (self.scaled.as_slice(), scale.radius)
        } else {
            (input.vertices, input.radius)
        };
        Ok(ConvexSupport {
            kind,
            vertices,
            radius,
        })
    }
}

#[cfg(test)]
#[path = "convex_support/faces_tests.rs"]
mod faces_tests;
#[cfg(test)]
#[path = "convex_support/feature_query_tests.rs"]
mod feature_query_tests;
#[cfg(test)]
#[path = "convex_support/horizon_tests.rs"]
mod horizon_tests;
#[cfg(test)]
#[path = "convex_support/hull_tests.rs"]
mod hull_tests;
#[cfg(test)]
#[path = "convex_support/penetration_tests.rs"]
mod penetration_tests;
#[cfg(test)]
#[path = "convex_support/seed_tests.rs"]
mod seed_tests;
#[cfg(test)]
#[path = "convex_support/solver_tests.rs"]
mod solver_tests;
#[cfg(test)]
#[path = "convex_support/tests.rs"]
mod tests;
