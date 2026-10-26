//! Support-point construction for the penetration solver (c24510/c24590).
use super::vertices::support_pair;
use crate::gameplay::shape_transform::ShapeMatrix;

#[derive(Clone, Copy, Debug)]
pub struct ConvexSupportPoint {
    pub difference: [f32; 4],
    pub query: [f32; 4],
    /// Target XYZ in query coordinates, with the raw target vertex's W bits.
    pub target: [f32; 4],
}

pub struct PenetrationSupport<'a> {
    pub query: &'a [[f32; 4]],
    pub target: &'a [[f32; 4]],
    pub relative: ShapeMatrix,
}
impl PenetrationSupport<'_> {
    /// c24510 computes the difference before restoring the target's index W.
    pub fn point(&self, direction: [f32; 4]) -> Result<ConvexSupportPoint, String> {
        let pair = support_pair(self.query, self.target, direction, self.relative)?;
        let difference = std::array::from_fn(|i| pair.query[i] - pair.target_in_query[i]);
        let mut target = pair.target_in_query;
        target[3] = pair.target[3];
        Ok(ConvexSupportPoint {
            difference,
            query: pair.query,
            target,
        })
    }

    /// c24590 retains the negative direction on equality. Its UCOMISS/JNC
    /// selects the positive packet on unordered values, independently of MAXPS.
    pub fn extent(
        &self,
        direction: [f32; 4],
        origin: [f32; 4],
    ) -> Result<(f32, ConvexSupportPoint), String> {
        let negative = direction.map(|v| -v);
        let positive_point = self.point(direction)?;
        let negative_point = self.point(negative)?;
        let project = |point: ConvexSupportPoint, axis: [f32; 4]| {
            let d: [f32; 3] = std::array::from_fn(|i| (point.difference[i] - origin[i]) * axis[i]);
            (d[1] + d[0]) + d[2]
        };
        let a = project(positive_point, direction);
        let b = project(negative_point, negative);
        let extent = if a > b { a } else { b };
        let point = if !(b >= a) {
            positive_point
        } else {
            negative_point
        };
        Ok((extent, point))
    }
}
