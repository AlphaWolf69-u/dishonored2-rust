//! Hull input-domain mapping (140b9c1e0), integer point preparation
//! (140baba50) and normalized export (140b925d0 with export mode 1).
use crate::gameplay::constraint_solver::reciprocal;

#[derive(Clone, Copy, Debug)]
pub(super) struct Domain {
    centre: [f32; 3],
    to_integer: [f32; 3],
    from_integer: [f32; 3],
}

impl Domain {
    pub(super) fn from_points(points: &[[f32; 4]]) -> Result<Self, String> {
        let first = points.first().ok_or("convex hull has no input points")?;
        let mut minimum = [first[0], first[1], first[2]];
        let mut maximum = minimum;
        for point in points {
            for axis in 0..3 {
                // Host admission rejects non-finite data before the original
                // integer conversion can produce its indefinite sentinel.
                if !point[axis].is_finite() {
                    return Err("convex hull input contains a non-finite coordinate".into());
                }
                // MINPS/MAXPS select the second operand on equality; preserve
                // signed-zero ordering rather than using Rust's min/max.
                minimum[axis] = if minimum[axis] < point[axis] {
                    minimum[axis]
                } else {
                    point[axis]
                };
                maximum[axis] = if maximum[axis] > point[axis] {
                    maximum[axis]
                } else {
                    point[axis]
                };
            }
        }
        let limit = f32::from_bits(0x7f7f_ffee); // 1423cce10
        let minimum_span = f32::from_bits(0x37a7_c5ac); // 141ea1020
        let integer_extent = f32::from_bits(0x45ff_f800); // 141ea1008: 8191
        let mut centre = [0.; 3];
        let mut to_integer = [0.; 3];
        let mut from_integer = [0.; 3];
        for axis in 0..3 {
            if minimum[axis] < -limit || limit < maximum[axis] {
                return Err("convex hull input domain is out of range".into());
            }
            let span = maximum[axis] - minimum[axis];
            let half = if span > minimum_span {
                span
            } else {
                minimum_span
            } * 0.5;
            centre[axis] = half + minimum[axis];
            to_integer[axis] = integer_extent * reciprocal(half);
            from_integer[axis] = reciprocal(to_integer[axis]);
        }
        Ok(Self {
            centre,
            to_integer,
            from_integer,
        })
    }

    /// The source clamps to [-8192,8191], rounds away from zero at half
    /// integers, then truncates with CVTTPS2DQ. This is not nearest-even.
    pub(super) fn quantize(&self, point: [f32; 4]) -> [i32; 3] {
        std::array::from_fn(|axis| {
            let value = (point[axis] - self.centre[axis]) * self.to_integer[axis];
            let upper = if value < 8191. { value } else { 8191. };
            let clamped = if -8192. > upper { -8192. } else { upper };
            let rounded = if clamped < 0. {
                clamped - 0.5
            } else {
                clamped + 0.5
            };
            rounded as i32
        })
    }

    /// 140b9bc60's integer containment path rejects out-of-domain points
    /// before testing the hull; quantize itself intentionally clamps them.
    pub(super) fn quantize_inside(&self, point: [f32; 4]) -> Option<[i32; 3]> {
        for axis in 0..3 {
            let value = (point[axis] - self.centre[axis]) * self.to_integer[axis];
            if value < -8192.0 || 8191.0 < value { return None; }
        }
        Some(self.quantize(point))
    }

    /// Export mode 1 uses quantized hull coordinates, not the retained source
    /// positions. W is zero here; support feature IDs belong to query packing.
    pub(super) fn export(&self, point: [i32; 3]) -> [f32; 4] {
        [
            point[0] as f32 * self.from_integer[0] + self.centre[0],
            point[1] as f32 * self.from_integer[1] + self.centre[1],
            point[2] as f32 * self.from_integer[2] + self.centre[2],
            0.,
        ]
    }
}
