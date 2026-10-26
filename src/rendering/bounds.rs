//! Oriented render-volume bounds, with inclusive contact and no SAT padding.

#[derive(Clone, Copy, Debug)]
pub struct OrientedBox {
    pub centre: [f32; 3],
    pub half_extent: [f32; 3],
    /// Three world-space basis vectors, in the object's authored axis order.
    pub axes: [[f32; 3]; 3],
}

impl OrientedBox {
    /// 28c2b0 constructs this axis-aligned box for each WORLD record.
    pub fn from_aabb(bounds: [[f32; 3]; 2]) -> Self {
        let centre = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5);
        Self {
            centre,
            half_extent: std::array::from_fn(|i| bounds[1][i] - centre[i]),
            axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        }
    }

    /// 0fc490: the six face axes followed by nine edge cross-products.
    /// Keep the executable's reduction order and ordered comparisons: a NaN
    /// rejects overlap; adding a parallel-axis epsilon changes decal targets.
    pub fn overlaps(&self, other: &Self) -> bool {
        let dot = |a: [f32; 3], b: [f32; 3]| (a[0] * b[0] + a[1] * b[1]) + a[2] * b[2];
        let delta = std::array::from_fn(|i| other.centre[i] - self.centre[i]);
        let r: [[f32; 3]; 3] =
            std::array::from_fn(|i| std::array::from_fn(|j| dot(self.axes[i], other.axes[j])));
        let c = r.map(|row| row.map(f32::abs));
        let t = self.axes.map(|axis| dot(axis, delta));
        let a = self.half_extent;
        let b = other.half_extent;
        for i in 0..3 {
            if !(t[i].abs() <= ((b[0] * c[i][0] + b[1] * c[i][1]) + b[2] * c[i][2]) + a[i]) {
                return false;
            }
        }
        for j in 0..3 {
            if !(dot(other.axes[j], delta).abs()
                <= ((a[1] * c[1][j] + a[0] * c[0][j]) + a[2] * c[2][j]) + b[j])
            {
                return false;
            }
        }
        let cross_axes = [
            (
                (t[2] * r[1][0] - r[2][0] * t[1]).abs(),
                ((b[2] * c[0][1] + b[1] * c[0][2]) + a[2] * c[1][0]) + a[1] * c[2][0],
            ),
            (
                (t[2] * r[1][1] - r[2][1] * t[1]).abs(),
                ((b[2] * c[0][0] + b[0] * c[0][2]) + a[2] * c[1][1]) + a[1] * c[2][1],
            ),
            (
                (t[2] * r[1][2] - r[2][2] * t[1]).abs(),
                ((b[0] * c[0][1] + b[1] * c[0][0]) + a[2] * c[1][2]) + a[1] * c[2][2],
            ),
            (
                (r[2][0] * t[0] - t[2] * r[0][0]).abs(),
                ((a[2] * c[0][0] + a[0] * c[2][0]) + b[2] * c[1][1]) + b[1] * c[1][2],
            ),
            (
                (r[2][1] * t[0] - t[2] * r[0][1]).abs(),
                ((a[2] * c[0][1] + a[0] * c[2][1]) + b[2] * c[1][0]) + b[0] * c[1][2],
            ),
            (
                (r[2][2] * t[0] - t[2] * r[0][2]).abs(),
                ((a[2] * c[0][2] + a[0] * c[2][2]) + b[0] * c[1][1]) + b[1] * c[1][0],
            ),
            (
                (t[1] * r[0][0] - r[1][0] * t[0]).abs(),
                ((a[1] * c[0][0] + a[0] * c[1][0]) + b[2] * c[2][1]) + b[1] * c[2][2],
            ),
            (
                (t[1] * r[0][1] - r[1][1] * t[0]).abs(),
                ((a[1] * c[0][1] + a[0] * c[1][1]) + b[2] * c[2][0]) + b[0] * c[2][2],
            ),
            (
                (t[1] * r[0][2] - r[1][2] * t[0]).abs(),
                ((a[1] * c[0][2] + a[0] * c[1][2]) + b[0] * c[2][1]) + b[1] * c[2][0],
            ),
        ];
        cross_axes
            .into_iter()
            .all(|(distance, radius)| distance <= radius)
    }
}
