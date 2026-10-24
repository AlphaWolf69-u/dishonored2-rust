//! hkpRagdollConstraintData atoms through the generic atom Jacobian builder
//! 140c45400: local transforms (2), setup stabilization (0x17), ragdoll
//! motors (0x13), angular friction (0x11), twist limit (0xf), cone and
//! planes limits (0x10) and the ball socket (5, 140c49b20).
use super::{
    angular::AngularLimitRow,
    ball_socket::{AnchorFrame, BallSocketBody, BallSocketBuild, BallSocketRow},
    schema_axis,
};

fn rsqrt_estimate(value: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        unsafe { _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(value))) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        1. / value.sqrt()
    }
}

fn rcp_estimate(value: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rcp_ss, _mm_set_ss};
        unsafe { _mm_cvtss_f32(_mm_rcp_ss(_mm_set_ss(value))) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        1. / value
    }
}

/// 140a9f2b0's packed arctangent in its instruction order:
/// (((s*v)*A)*B)*(C*D) + (offset + v).
fn packed_atan(ratio: f32) -> f32 {
    let c = f32::from_bits;
    let invert = 1. < ratio.abs();
    let r = rcp_estimate(ratio);
    let v = if invert { -((2. - r * ratio) * r) } else { ratio };
    let offset = if invert {
        f32::from_bits((ratio.to_bits() & 0x8000_0000) | 0x3fc9_0fdb)
    } else {
        0.
    };
    let s = v * v;
    let a = c(0x3b3b_d74a) * s + c(0xbbeb_7f9e);
    let b = (c(0x3fda_2291) + s) * s + c(0x400f_22d2);
    let cc = (c(0xbf80_9ff7) + s) * s + c(0x4071_2d99);
    let d = (c(0xc075_3eb3) + s) * s + c(0x40b0_19ca);
    (((s * v) * a) * b) * (cc * d) + (offset + v)
}

/// 140a9f380: DIVPS, the packed arctangent masked off for a zero operand,
/// plus the quadrant term (+-pi for x < 0, +-pi/2 for x = 0, sign of y).
pub fn packed_atan2(y: f32, x: f32) -> f32 {
    let sign = y.to_bits() & 0x8000_0000;
    let quadrant = if x == 0. {
        if y == 0. { 0. } else { f32::from_bits(0x3fc9_0fdb ^ sign) }
    } else {
        f32::from_bits((if x < 0. { std::f32::consts::PI.to_bits() } else { 0 }) ^ sign)
    };
    let atan = packed_atan(y / x);
    quadrant + if x == 0. || y == 0. { 0. } else { atan }
}

const EPSILON: f32 = f32::from_bits(0x3400_0000);
const HUGE: f32 = f32::from_bits(0x7f7f_ffee);

/// hkTransform: three rotation columns and the translation.
pub type Transform = [[f32; 4]; 4];

#[derive(Clone, Copy, Debug, Default)]
pub struct AngularLimit {
    pub enabled: bool,
    /// twistAxis / twistAxisInA.
    pub axis: u8,
    /// refAxis / refAxisInB.
    pub reference: u8,
    /// Cone atoms only: 0 cone, otherwise planes.
    pub mode: u8,
    /// Cone atoms only: runtime offset of the angle offset (0/1 unused).
    pub memory_offset: u16,
    pub minimum: f32,
    pub maximum: f32,
    pub tau: f32,
    pub damping: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct RagdollAtoms {
    pub transform_a: Transform,
    pub transform_b: Transform,
    pub stabilization: Option<[f32; 3]>,
    pub friction_axes: u8,
    pub twist: AngularLimit,
    pub cone: AngularLimit,
    pub planes: AngularLimit,
    pub limited: bool,
    pub breach_impulse: f32,
    pub inertia_stabilization: f32,
}

/// The constraint setup's per-body record (1412a43e0): +0x30 inverse inertia
/// and mass, +0x40 three principal lanes, +0x70 center, +0x80 world-to-mass.
#[derive(Clone, Copy, Debug)]
pub struct AtomBody {
    pub transform: Transform,
    pub inverse_inertia: [f32; 4],
    pub rows: [[f32; 4]; 3],
    pub center: [f32; 4],
    pub world_to_mass: [f32; 4],
}

/// Build info from 1412a4360: +8 solver info +0x50 (inverse substep),
/// +0xc +0x20, +0x20 +0x110, +0x24 and +0x54 +0x10 (damping), +0x50 +0x00
/// (tau).
#[derive(Clone, Copy, Debug)]
pub struct AtomStep {
    pub inverse_step: f32,
    pub offset_rate: f32,
    pub export_seconds: f32,
    pub damping: f32,
    pub tau: f32,
    pub limit_damping: f32,
}

#[derive(Clone, Copy, Debug)]
pub enum LimitRow {
    Skip,
    Limit(AngularLimitRow),
}

#[derive(Clone, Copy, Debug)]
pub struct RagdollRows {
    pub friction_axes: u8,
    pub twist: LimitRow,
    pub cone: LimitRow,
    pub planes: LimitRow,
    pub pivot: BallSocketRow,
}

/// Runtime layout after 140c45400: ragdoll motors 0x30, friction 0x10 per
/// axis, then 8 per limit and the ball socket.
pub fn runtime_limit_slots(friction_axes: u8) -> [usize; 3] {
    let start = 0x30 + friction_axes as usize * 0x10;
    [start, start + 8, start + 16]
}

fn frame(body: &Transform, local: &Transform) -> [[f32; 4]; 4] {
    let mut out = [[0.; 4]; 4];
    for (v, o) in local.iter().zip(&mut out) {
        *o = std::array::from_fn(|i| (v[1] * body[1][i] + v[0] * body[0][i]) + v[2] * body[2][i]);
    }
    for i in 0..4 {
        out[3][i] += body[3][i];
    }
    out
}

fn principal(body: &AtomBody, v: [f32; 4]) -> [f32; 4] {
    let m = body.rows;
    std::array::from_fn(|i| (v[1] * m[1][i] + v[0] * m[0][i]) + v[2] * m[2][i])
}

fn limit_row(
    axis: [f32; 4],
    angle: f32,
    limit: &AngularLimit,
    tau: f32,
    a: &AtomBody,
    b: &AtomBody,
    step: &AtomStep,
) -> AngularLimitRow {
    let ja = principal(a, axis);
    let jb = principal(b, axis.map(|v| -v));
    let lane = |i: usize| {
        ja[i] * ja[i] * a.inverse_inertia[i] + jb[i] * jb[i] * b.inverse_inertia[i]
    };
    let inverse = (lane(1) + lane(0)) + lane(2) + EPSILON;
    let r = rcp_estimate(inverse);
    let effective_mass = tau * ((2. - inverse * r) * r);
    let negative_step = -step.inverse_step;
    AngularLimitRow {
        axis_a: [schema_axis(ja[0], 13), ja[1], ja[2]],
        axis_b: [jb[0], jb[1], jb[2]],
        effective_mass,
        correction: angle * negative_step,
        lower: limit.minimum * negative_step * tau,
        upper: limit.maximum * negative_step * tau,
        tau,
        damping: step.limit_damping * limit.damping,
    }
}

/// 140c45400 for one ragdoll atom stack. `runtime` is the constraint's
/// runtime block (cone angle offsets live at their memory offsets and read
/// the previous export of their own row, word 1 of its slot).
pub fn build(
    atoms: &RagdollAtoms,
    a: &AtomBody,
    b: &AtomBody,
    step: &AtomStep,
    runtime: &mut [u8],
) -> RagdollRows {
    let fa = frame(&a.transform, &atoms.transform_a);
    let fb = frame(&b.transform, &atoms.transform_b);
    let read = |runtime: &[u8], at: usize| f32::from_le_bytes(runtime[at..at + 4].try_into().unwrap());
    // 0x17: levers in each motion's principal lanes and the angular cap.
    let (lever_a, lever_b, angular_cap, linear_cap) = match atoms.stabilization {
        Some([maximum_linear, maximum_angular, maximum_angle]) => {
            let lever = |body: &AtomBody, pivot: [f32; 4]| {
                let d: [f32; 3] = std::array::from_fn(|i| pivot[i] - body.center[i]);
                principal(body, [d[0], d[1], d[2], 0.])
            };
            let la = lever(a, fa[3]);
            let lb = lever(b, fb[3]);
            let largest = |i: [f32; 4]| i[2].max(i[1].max(i[0]));
            let inertia_a = largest(a.inverse_inertia);
            let inertia_b = largest(b.inverse_inertia);
            let inertia = inertia_a.max(inertia_b).max(EPSILON);
            let scaled = |l: [f32; 4], maximum: f32| {
                let s = (l[1] * l[1] + l[0] * l[0]) + l[2] * l[2];
                let r = rsqrt_estimate(s);
                let inverse = (3. - r * s * r) * (r * 0.5);
                (if s <= 0. { 0. } else { inverse }) * maximum
            };
            let lever_scale = scaled(la, inertia_a).max(scaled(lb, inertia_b)).max(EPSILON);
            let limit = maximum_angle * step.inverse_step;
            let refined = |v: f32| {
                let r = rcp_estimate(v);
                (2. - r * v) * r * limit
            };
            (
                la,
                lb,
                refined(inertia).min(maximum_angular),
                refined(lever_scale).min(maximum_linear),
            )
        }
        None => ([0.; 4], [0.; 4], HUGE, HUGE),
    };
    let twist = {
        let t = &atoms.twist;
        let (tw, rf) = (t.axis as usize, t.reference as usize);
        let sum: [f32; 3] = std::array::from_fn(|i| fa[tw][i] + fb[tw][i]);
        let s = (sum[1] * sum[1] + sum[0] * sum[0]) + sum[2] * sum[2];
        let r = rsqrt_estimate(s);
        let length = if s <= 0. { 0. } else { (3. - r * s * r) * r * 0.5 * s };
        let (axis, half) = if length <= 1e-16 {
            (fb[tw], 0.)
        } else {
            let rc = rcp_estimate(length);
            let inverse = (2. - rc * length) * rc;
            (
                [sum[0] * inverse, sum[1] * inverse, sum[2] * inverse, (fa[tw][3] + fb[tw][3]) * inverse],
                length * 0.5,
            )
        };
        let br = fb[rf];
        let p = [
            br[2] * axis[1] - axis[2] * br[1],
            br[0] * axis[2] - axis[0] * br[2],
            br[1] * axis[0] - axis[1] * br[0],
        ];
        let q = [
            p[1] * axis[2] - p[2] * axis[1],
            p[2] * axis[0] - p[0] * axis[2],
            p[0] * axis[1] - p[1] * axis[0],
        ];
        let ar = fa[rf];
        let y = (p[1] * ar[1] + p[0] * ar[0]) + p[2] * ar[2];
        let x = (q[1] * ar[1] + q[0] * ar[0]) + q[2] * ar[2];
        let angle = packed_atan2(y, x);
        let tau = step.tau * t.tau * half * half;
        LimitRow::Limit(limit_row(axis, angle, t, tau, a, b, step))
    };
    let [_, cone_slot, planes_slot] = runtime_limit_slots(atoms.friction_axes);
    let mut cone = |limit: &AngularLimit, slot: usize| {
        let (tw, rf) = (limit.axis as usize, limit.reference as usize);
        let (av, bv) = (fa[tw], fb[rf]);
        let c = [
            bv[2] * av[1] - av[2] * bv[1],
            bv[0] * av[2] - av[0] * bv[2],
            bv[1] * av[0] - av[1] * bv[0],
        ];
        let s = (c[1] * c[1] + c[0] * c[0]) + c[2] * c[2];
        if s < EPSILON {
            return LimitRow::Skip;
        }
        let r = rsqrt_estimate(s);
        let length = (3. - r * s * r) * r * 0.5 * s;
        let dot = (bv[1] * av[1] + bv[0] * av[0]) + bv[2] * av[2];
        let raw = packed_atan2(length, dot);
        let planes = limit.mode != 0;
        let mut angle = if planes { f32::from_bits(0x3fc9_0fdb) - raw } else { raw };
        let r = rsqrt_estimate(s);
        let sign = if planes { 1. } else { -1. };
        let unit = |v: f32| (3. - r * s * r) * (r * 0.5) * v * sign;
        let axis = [unit(c[0]), unit(c[1]), unit(c[2]), unit(av[3] * bv[3] - bv[3] * av[3])];
        let tau = step.tau * limit.tau;
        let memory = limit.memory_offset as usize;
        if memory > 1 {
            let stored = read(runtime, memory);
            let rate = step.offset_rate * 0.5;
            angle -= stored;
            let previous = -read(runtime, slot + 4);
            let mut relax = stored * f32::from_bits(0x3d4c_cccd);
            if relax <= rate {
                relax = rate;
            }
            let gap = previous - angle;
            let mut remaining = stored - relax;
            angle += relax;
            let threshold = (relax * 2. + rate) * 0.5;
            if threshold < -gap && previous != 0. {
                remaining = (stored - relax) - gap;
                angle += gap;
            }
            let remaining = if remaining <= f32::from_bits(0x3400_0000) {
                f32::from_bits(0x3400_0000)
            } else {
                remaining
            };
            runtime[memory..memory + 4].copy_from_slice(&remaining.to_le_bytes());
        }
        LimitRow::Limit(limit_row(axis, angle, limit, tau, a, b, step))
    };
    let cone_row = cone(&atoms.cone, cone_slot);
    let planes_row = cone(&atoms.planes, planes_slot);
    let body = |body: &AtomBody| BallSocketBody {
        frame: AnchorFrame {
            center: body.center,
            world_to_mass: body.world_to_mass,
        },
        inverse_mass: body.inverse_inertia,
    };
    let pivot = BallSocketRow::build(BallSocketBuild {
        a: body(a),
        b: body(b),
        lever_a: [lever_a[0], lever_a[1], lever_a[2]],
        lever_b: [lever_b[0], lever_b[1], lever_b[2]],
        inverse_step: step.inverse_step,
        tau: step.tau,
        damping: step.damping,
        mass_stabilization: atoms.inertia_stabilization,
        maximum_force: atoms.breach_impulse,
        maximum_impulse: if atoms.limited { linear_cap } else { HUGE },
        angular_impulse_limit: angular_cap,
    });
    RagdollRows {
        friction_axes: atoms.friction_axes,
        twist,
        cone: cone_row,
        planes: planes_row,
        pivot,
    }
}

impl RagdollRows {
    /// The Jacobian stream 140c45400 writes: skip headers for the disabled
    /// ragdoll motors and zero-torque friction, three limits, the pivot.
    pub fn schema_words(&self) -> Vec<u32> {
        let skip = |count: u32| [0x3400_0003, count, 0, 0];
        let mut words = Vec::new();
        words.extend(skip(6));
        words.extend(skip(self.friction_axes as u32 * 2));
        for limit in [self.twist, self.cone, self.planes] {
            match limit {
                LimitRow::Skip => words.extend(skip(1)),
                LimitRow::Limit(row) => {
                    let fields = [
                        row.axis_a[0], row.axis_a[1], row.axis_a[2], row.effective_mass,
                        row.axis_b[0], row.axis_b[1], row.axis_b[2], row.correction,
                        row.lower, row.upper, row.tau, row.damping,
                    ];
                    words.extend(fields.map(f32::to_bits));
                }
            }
        }
        words.extend(self.pivot.schema_fields().map(f32::to_bits));
        words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(hex: &str) -> Vec<u8> {
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()
    }

    pub(super) fn atoms_from_runtime(b: &[u8]) -> RagdollAtoms {
        let f = |at: usize| f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let transform = |at: usize| std::array::from_fn(|c| std::array::from_fn(|i| f(at + 16 * c + 4 * i)));
        let limit = |at: usize, cone: bool| AngularLimit {
            enabled: b[at + 2] != 0,
            axis: b[at + 3],
            reference: b[at + 4],
            mode: if cone { b[at + 5] } else { 0 },
            memory_offset: if cone { u16::from_le_bytes([b[at + 6], b[at + 7]]) } else { 0 },
            minimum: f(at + 8),
            maximum: f(at + 12),
            tau: f(at + 16),
            damping: f(at + 20),
        };
        RagdollAtoms {
            transform_a: transform(0x10),
            transform_b: transform(0x50),
            stabilization: (b[0x92] != 0).then(|| [f(0x94), f(0x98), f(0x9c)]),
            friction_axes: b[0x104],
            twist: limit(0x110, false),
            cone: limit(0x130, true),
            planes: limit(0x150, true),
            limited: b[0x175] != 0,
            breach_impulse: f(0x178),
            inertia_stabilization: f(0x17c),
        }
    }

    #[test]
    fn ragdoll_atoms_match_executable() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/ragdoll_atoms.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let fields: Vec<&str> = line.split(' ').collect();
            let atoms = atoms_from_runtime(&bytes(fields[0]));
            let info: Vec<f32> = fields[1..7]
                .iter()
                .map(|w| f32::from_bits(u32::from_str_radix(w, 16).unwrap()))
                .collect();
            let floats = |hex: &str| -> Vec<f32> {
                bytes(hex).chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()
            };
            let step = AtomStep {
                inverse_step: info[0],
                offset_rate: info[1],
                export_seconds: info[2],
                damping: info[3],
                tau: info[4],
                limit_damping: info[5],
            };
            let body = |transform: &str, motion: &str| {
                let t = floats(transform);
                let m = floats(motion);
                AtomBody {
                    transform: std::array::from_fn(|c| std::array::from_fn(|i| t[4 * c + i])),
                    inverse_inertia: std::array::from_fn(|i| m[i]),
                    rows: std::array::from_fn(|r| std::array::from_fn(|i| m[4 + 4 * r + i])),
                    center: std::array::from_fn(|i| m[16 + i]),
                    world_to_mass: std::array::from_fn(|i| m[20 + i]),
                }
            };
            let a = body(fields[7], fields[9]);
            let b = body(fields[8], fields[10]);
            let runtime_in = bytes(fields[11]);
            let rows_out = bytes(fields[12]);
            let runtime_out = bytes(fields[13]);
            let mut runtime = runtime_in.clone();
            runtime.resize(0x100, 0);
            let rows = build(&atoms, &a, &b, &step, &mut runtime);
            let expected: Vec<u32> = rows_out
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                .collect();
            let actual = rows.schema_words();
            let mismatches: Vec<String> = actual
                .iter()
                .enumerate()
                // The ball socket's first word keeps c49b20's earlier write.
                .filter(|&(i, word)| i != actual.len() - 32 && *word != expected[i])
                .map(|(i, word)| format!("{i}: {} vs {}", f32::from_bits(*word), f32::from_bits(expected[i])))
                .collect();
            assert!(mismatches.is_empty(), "case {count}: {mismatches:?}");
            assert_eq!(&runtime[..0x100], &runtime_out[..], "case {count} runtime");
            count += 1;
        }
        assert_eq!(count, 384);
    }
}
