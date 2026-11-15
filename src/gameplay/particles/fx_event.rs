//! arkFXEventInstance (1401da2c0, vtable 141b05de8): a particle system's
//! listener for one m_fxEvents entry. Stage events that name it are
//! recorded at most once per m_interval (handler 1401db0b0, interval clock
//! 1401daf50); after the system's update each record starts the entry's FX
//! (1401da820) at the event, oriented by the event frame (1401da330), an
//! offset along it and a random rotation.
use crate::{
    assets::fx::particle_declaration::FxEventAsset,
    float::{add, divide, multiply, subtract},
};

type Basis = [[f32; 3]; 3];

/// One recorded event: world position, velocity and normal.
pub type FxEventRecord = [[f32; 3]; 3];

pub struct FxEventInstance {
    /// Index into the declaration's m_fxEvents.
    pub index: usize,
    /// +0x10: time accumulated toward the next record.
    elapsed: f32,
    /// +0x18: records waiting for the next spawn pass.
    pub pending: Vec<FxEventRecord>,
}

/// An FX the system asks its owner to start: the effect, its origin and
/// axis (rows), and whether its record target is the system's target
/// (PARTICLESYSTEM_TARGET) rather than its origin.
pub struct FxEventStart {
    pub fx: String,
    pub position: [f32; 3],
    pub axis: Basis,
    pub target_is_system_target: bool,
}

const SMALLEST: f32 = f32::from_bits(0x0080_0000);
const EPSILON: f32 = f32::from_bits(0x3400_0000);

impl FxEventInstance {
    pub fn new(index: usize) -> Self {
        Self { index, elapsed: 0., pending: Vec::new() }
    }

    /// 1401daf50: the clock only runs while below the interval.
    pub fn advance(&mut self, asset: &FxEventAsset, delta: f32) {
        if self.elapsed < asset.interval {
            self.elapsed = add(self.elapsed, delta);
        }
    }

    /// 1401db0b0.
    pub fn record(&mut self, asset: &FxEventAsset, record: FxEventRecord) {
        if asset.interval <= self.elapsed {
            self.elapsed = subtract(self.elapsed, asset.interval);
            self.pending.push(record);
        }
    }

    /// 1401da820 over the pending records, then cleared. `view` is the
    /// system's view origin (+0x1df8), `seed` the system's spawn stream
    /// (+0x1e80).
    pub fn start(&mut self, asset: &FxEventAsset, view: [f32; 3], seed: &mut u32, output: &mut Vec<FxEventStart>) {
        let Some(fx) = asset.fx.as_ref() else {
            self.pending.clear();
            return;
        };
        for record in self.pending.drain(..) {
            if let Some((position, axis)) = start_pose(asset, &record, view, seed) {
                output.push(FxEventStart { fx: fx.clone(), position, axis, target_is_system_target: asset.target_is_system_target });
            }
        }
    }
}

/// 1401da820 for one record: the event frame (1401da330), its orthonormal
/// check (14000ef30, 1e-5), the origin offset along it and the random
/// rotation (three draws, 1401da100, m_rotationOrder) applied to it. None
/// when the frame is degenerate.
pub fn start_pose(asset: &FxEventAsset, record: &FxEventRecord, view: [f32; 3], seed: &mut u32) -> Option<([f32; 3], Basis)> {
    let frame = event_frame(asset, record, view, seed)?;
    if !orthonormal(&frame, f32::from_bits(0x3727_c5ac)) {
        return None;
    }
    let [p, ..] = *record;
    let o = asset.offset;
    let position = std::array::from_fn(|i| {
        add(add(add(multiply(o[0], frame[0][i]), multiply(o[1], frame[1][i])), multiply(o[2], frame[2][i])), p[i])
    });
    let unit = |seed: &mut u32| multiply(((crate::random::lcg_step(seed) >> 10) & 0x7fff) as f32, f32::from_bits(0x3800_0100));
    let mix = [unit(seed), unit(seed), unit(seed)];
    let angles = rotation(asset.rotation, mix).map(|degrees| multiply(f32::from_bits(0x3c8e_fa35), degrees));
    let order = [[0, 1, 2], [2, 0, 1], [1, 2, 0], [0, 2, 1], [1, 0, 2], [2, 1, 0]][asset.rotation_order.min(5)];
    let mut turn = axis_rotation(order[0], angles[order[0]]);
    turn = product(turn, axis_rotation(order[1], angles[order[1]]));
    turn = product(turn, axis_rotation(order[2], angles[order[2]]));
    Some((position, apply(turn, frame)))
}

/// 1400b8670: normalize in place, returning the original length.
fn normalize(v: &mut [f32; 3]) -> f32 {
    let squared = add(add(multiply(v[0], v[0]), multiply(v[1], v[1])), multiply(v[2], v[2]));
    let inverse = divide(1., if squared <= SMALLEST { SMALLEST } else { squared }.sqrt());
    *v = v.map(|lane| multiply(lane, inverse));
    multiply(inverse, squared)
}

/// 1401dae10: the cross product of the two rows other than `index`.
fn complement(m: &Basis, index: usize) -> [f32; 3] {
    let f = |r: usize, c: usize| m[r][c];
    match index {
        0 => [
            subtract(multiply(f(1, 1), f(2, 2)), multiply(f(1, 2), f(2, 1))),
            subtract(multiply(f(2, 0), f(1, 2)), multiply(f(1, 0), f(2, 2))),
            subtract(multiply(f(1, 0), f(2, 1)), multiply(f(2, 0), f(1, 1))),
        ],
        1 => [
            subtract(multiply(f(0, 2), f(2, 1)), multiply(f(0, 1), f(2, 2))),
            subtract(multiply(f(0, 0), f(2, 2)), multiply(f(2, 0), f(0, 2))),
            subtract(multiply(f(2, 0), f(0, 1)), multiply(f(0, 0), f(2, 1))),
        ],
        _ => [
            subtract(multiply(f(1, 2), f(0, 1)), multiply(f(1, 1), f(0, 2))),
            subtract(multiply(f(1, 0), f(0, 2)), multiply(f(0, 0), f(1, 2))),
            subtract(multiply(f(0, 0), f(1, 1)), multiply(f(1, 0), f(0, 1))),
        ],
    }
}

/// 1401da330: the event frame. VELOCITY_PROJECTED takes the normal as the
/// third row and the velocity projected into its plane as the first
/// (a random direction when the velocity is along the normal); CUSTOM maps
/// the velocity and the direction to `view` onto the chosen rows and
/// completes the third.
#[doc(hidden)]
pub fn event_frame(asset: &FxEventAsset, record: &FxEventRecord, view: [f32; 3], seed: &mut u32) -> Option<Basis> {
    let [p, v, n] = *record;
    let mut m: Basis = [[0.; 3]; 3];
    if !asset.custom_axis {
        let mut z = n;
        let length = normalize(&mut z);
        m[2] = z;
        if !(EPSILON <= length) {
            return None;
        }
        let mut y = [
            subtract(multiply(v[2], z[1]), multiply(v[1], z[2])),
            subtract(multiply(v[0], z[2]), multiply(v[2], z[0])),
            subtract(multiply(v[1], z[0]), multiply(v[0], z[1])),
        ];
        let length = normalize(&mut y);
        m[1] = y;
        if length < EPSILON {
            let random = |seed: &mut u32| {
                subtract(
                    multiply(multiply(((crate::random::lcg_step(seed) >> 10) & 0x7fff) as f32, f32::from_bits(0x3800_0100)), 2.),
                    1.,
                )
            };
            let r = [random(seed), random(seed), random(seed)];
            m[1] = [
                subtract(multiply(m[2][1], r[2]), multiply(m[2][2], r[1])),
                subtract(multiply(m[2][2], r[0]), multiply(m[2][0], r[2])),
                subtract(multiply(m[2][0], r[1]), multiply(m[2][1], r[0])),
            ];
            if normalize(&mut m[1]) < EPSILON {
                return None;
            }
        }
        m[0] = [
            subtract(multiply(m[2][2], m[1][1]), multiply(m[2][1], m[1][2])),
            subtract(multiply(m[2][0], m[1][2]), multiply(m[1][0], m[2][2])),
            subtract(multiply(m[1][0], m[2][1]), multiply(m[2][0], m[1][1])),
        ];
        return Some(m);
    }
    let mut d: [f32; 3] = std::array::from_fn(|i| subtract(view[i], p[i]));
    if !(EPSILON <= normalize(&mut d)) {
        return None;
    }
    if asset.reverse_view_axis {
        d = d.map(|lane| -lane);
    }
    let mut w = v;
    if !(EPSILON <= normalize(&mut w)) {
        return None;
    }
    if asset.reverse_velocity_axis {
        w = w.map(|lane| -lane);
    }
    let (velocity_row, view_row) = (asset.velocity_axis, asset.view_axis);
    let third = 3 - view_row - velocity_row;
    m[velocity_row] = w;
    m[view_row] = d;
    m[third] = complement(&m, third);
    if !(EPSILON <= normalize(&mut m[third])) {
        return None;
    }
    m[view_row] = complement(&m, view_row);
    if !(EPSILON <= normalize(&mut m[view_row])) {
        return None;
    }
    Some(m)
}

/// 14000ef30 with 14000f090: unit rows and columns within `tolerance` of
/// one and a determinant above 0.1.
fn orthonormal(m: &Basis, tolerance: f32) -> bool {
    let s = |r: usize, c: usize| multiply(m[r][c], m[r][c]);
    let rows = [
        add(add(s(0, 0), s(0, 1)), s(0, 2)),
        add(add(s(1, 1), s(1, 0)), s(1, 2)),
        add(add(s(2, 1), s(2, 0)), s(2, 2)),
    ];
    let columns = [
        add(add(s(1, 0), s(0, 0)), s(2, 0)),
        add(add(s(1, 1), s(0, 1)), s(2, 1)),
        add(add(s(1, 2), s(0, 2)), s(2, 2)),
    ];
    let low = subtract(1., tolerance);
    let high = add(tolerance, 1.);
    if !rows.iter().chain(&columns).all(|&v| low < v && v < high) {
        return false;
    }
    let f = |r: usize, c: usize| m[r][c];
    let determinant = add(
        add(
            multiply(subtract(multiply(f(1, 0), f(0, 2)), multiply(f(0, 0), f(1, 2))), f(2, 1)),
            multiply(subtract(multiply(f(0, 1), f(1, 2)), multiply(f(0, 2), f(1, 1))), f(2, 0)),
        ),
        multiply(subtract(multiply(f(0, 0), f(1, 1)), multiply(f(1, 0), f(0, 1))), f(2, 2)),
    );
    f32::from_bits(0x3dcc_cccd) < determinant
}

/// 1401da100: per channel (1 - r) minimum + r maximum, endpoints within
/// 1e-18 of zero taken as zero.
fn rotation([maximum, minimum]: [[f32; 3]; 2], r: [f32; 3]) -> [f32; 3] {
    let small = |x: f32| if x.abs() <= f32::from_bits(0x2193_92ef) { 0. } else { x };
    std::array::from_fn(|i| add(multiply(subtract(1., r[i]), small(minimum[i])), multiply(r[i], small(maximum[i]))))
}

/// 14010e6a0: the rotation about one axis (rows).
fn axis_rotation(axis: usize, angle: f32) -> Basis {
    let (s, c) = (crate::float::sinf(angle), crate::float::cosf(angle));
    match axis {
        0 => [[1., 0., 0.], [0., c, s], [0., -s, c]],
        1 => [[c, 0., -s], [0., 1., 0.], [s, 0., c]],
        _ => [[c, s, 0.], [-s, c, 0.], [0., 0., 1.]],
    }
}

/// 1400a55d0: a = a b with its sum order.
fn product(a: Basis, b: Basis) -> Basis {
    std::array::from_fn(|r| {
        let [x, y, z] = a[r];
        [
            add(add(multiply(y, b[1][0]), multiply(x, b[0][0])), multiply(z, b[2][0])),
            add(add(multiply(y, b[1][1]), multiply(x, b[0][1])), multiply(z, b[2][1])),
            add(add(multiply(x, b[0][2]), multiply(y, b[1][2])), multiply(z, b[2][2])),
        ]
    })
}

/// 14000dce0: a b into a new matrix with its sum order.
fn apply(a: Basis, b: Basis) -> Basis {
    std::array::from_fn(|r| {
        let [x, y, z] = a[r];
        [
            add(add(multiply(y, b[1][0]), multiply(b[0][0], x)), multiply(z, b[2][0])),
            add(add(multiply(y, b[1][1]), multiply(b[0][1], x)), multiply(z, b[2][1])),
            add(add(multiply(b[1][2], y), multiply(x, b[0][2])), multiply(b[2][2], z)),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The installed 1401da330 / 14000ef30 / 1401da820 rotation
    /// (analysis/probe_fx_event_poses.py).
    #[test]
    fn poses_match_installed_code() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fx_event_poses.txt")).unwrap();
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let w: Vec<u32> = line.split_whitespace().map(|h| u32::from_str_radix(h, 16).unwrap()).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let asset = FxEventAsset {
                event: "e".into(),
                enabled: true,
                interval: 0.,
                fx: Some("x".into()),
                custom_axis: w[0] == 1,
                velocity_axis: w[1] as usize,
                reverse_velocity_axis: w[2] != 0,
                view_axis: w[3] as usize,
                reverse_view_axis: w[4] != 0,
                rotation: [[f(5), f(6), f(7)], [f(8), f(9), f(10)]],
                rotation_order: w[11] as usize,
                offset: [f(12), f(13), f(14)],
                target_is_system_target: true,
            };
            let record = [[f(15), f(16), f(17)], [f(18), f(19), f(20)], [f(21), f(22), f(23)]];
            let view = [f(24), f(25), f(26)];
            let mut seed = w[27];
            let mut probe = seed;
            let frame = event_frame(&asset, &record, view, &mut probe);
            let pose = start_pose(&asset, &record, view, &mut seed);
            let mut ours = vec![u32::from(pose.is_some()), seed];
            if let (Some(frame), Some((_, axis))) = (frame, pose) {
                ours.extend(frame.iter().flatten().map(|x| x.to_bits()));
                ours.extend(axis.iter().flatten().map(|x| x.to_bits()));
            }
            assert_eq!(ours, w[28..], "{line}");
        }
    }
}
