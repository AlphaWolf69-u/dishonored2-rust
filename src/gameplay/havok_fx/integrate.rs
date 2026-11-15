//! hkfxEffects::Integrate (constructor 140ff89d0, vtable 141f1dfd8): the
//! last effect of the pipeline. Slot 19 (140ff8cd0) integrates 6-DoF bodies
//! when the pipeline has AngularMotion (140ff95a0), else 3-DoF (140ff9fd0).
use super::packed::{multiply_add, pack, unpack, with_pack_w};
use crate::float::{add, multiply, subtract};
use crate::gameplay::constraint_solver::{inverse_length, reciprocal};

const ZERO: [f32; 4] = [0.; 4];
const ONE: f32 = 1.;
/// 0x5f7ffff0, the clamp constructor value; any other clamp is applied.
pub const NO_CLAMP: f32 = f32::from_bits(0x5f7f_fff0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutOfDomain {
    Clip,
    Trash,
}

pub struct Integrate {
    /// +0x30.
    pub gravity: [f32; 4],
    /// +0x40..+0x4c.
    pub linear_damping: f32,
    pub linear_clamping: f32,
    pub angular_damping: f32,
    pub angular_clamping: f32,
    /// +0x50.
    pub max_age: f32,
    /// +0x54.
    pub rma_frequency: f32,
    /// +0x58.
    pub out_of_domain: OutOfDomain,
    /// +0x5c.
    pub second_order_accuracy: bool,
    /// +0x60: age gained per second.
    pub aging: f32,
    /// +0x78: per body, the packed running-average velocity (bytes 0..4)
    /// and the packed velocity of the previous step (bytes 4..8).
    history: Vec<[u8; 8]>,
}

impl Integrate {
    /// 140ff89d0 and its slot 6 (140ff8ad0) sizing the history.
    pub fn new(capacity: usize) -> Self {
        Self {
            gravity: ZERO,
            linear_damping: 0.,
            linear_clamping: NO_CLAMP,
            angular_damping: 0.,
            angular_clamping: NO_CLAMP,
            max_age: f32::from_bits(0x7f7f_ffee),
            rma_frequency: ONE,
            out_of_domain: OutOfDomain::Clip,
            second_order_accuracy: false,
            aging: ONE,
            history: vec![[0; 8]; capacity],
        }
    }

    /// Slot 8 (140ff8ca0): new bodies start with an empty history.
    pub fn allocate(&mut self, bodies: &[i32]) {
        for &body in bodies {
            self.history[body as usize] = [0; 8];
        }
    }

    /// The running-average velocity of `body`.
    pub fn average_velocity(&self, body: i32) -> [f32; 4] {
        let h = self.history[body as usize];
        unpack([h[0], h[1], h[2], h[3]])
    }

    /// 140ff9fd0 (3-DoF, `angular` None) or 140ff95a0 (6-DoF) over one
    /// chunk of `bodies`; returns the bodies to trash (140ff2550): older than
    /// the maximum age, or out of the domain with OOD_TRASH.
    pub fn integrate(
        &mut self,
        dt: f32,
        domain: [[f32; 4]; 2],
        plane: [f32; 4],
        bodies: &[i32],
        linear: &mut [[[f32; 4]; 2]],
        mut angular: Option<&mut [[[f32; 4]; 2]]>,
    ) -> Vec<i32> {
        let dt4 = [dt; 4];
        let damping = |value: f32| -> f32 {
            let reduced = subtract(ONE, value);
            let base = if reduced < ONE { reduced } else { ONE };
            // MAXPS(0, x) picks its second operand on NaN.
            let base = if 0. > base { 0. } else { base };
            base.powf(dt)
        };
        let linear_damping = damping(self.linear_damping);
        let angular_damping = damping(self.angular_damping);
        let half_dt_rotation = multiply(dt, 0.5);
        // RCPPS with one Newton step, masked to 0 for a zero step.
        let inverse_dt = if dt == 0. { 0. } else { reciprocal(dt) };
        let k = multiply(self.rma_frequency, inverse_dt);
        let k_minus_one = subtract(k, ONE);
        let inverse_k = multiply(reciprocal(k), ONE);
        let half = reciprocal(2.);
        let half_dt = multiply(half, dt);
        let three_halves_dt = multiply(half, multiply(dt, 3.));
        let clamp = self.linear_clamping != NO_CLAMP;
        // 140ff97c2: without an authored clamp a body turns at most pi per
        // step.
        let angular_clamp = if self.angular_clamping != NO_CLAMP {
            self.angular_clamping
        } else {
            multiply(inverse_dt, f32::from_bits(0x4049_0fdb))
        };
        let planar = plane.iter().any(|&lane| lane != 0.);
        let dot3 = |a: [f32; 4], b: [f32; 4]| add(add(multiply(a[1], b[1]), multiply(a[0], b[0])), multiply(a[2], b[2]));
        // Gravity x dt, projected onto the plane (w 0).
        let mut gravity_dt: [f32; 4] = std::array::from_fn(|i| multiply(self.gravity[i], dt));
        let along = dot3(gravity_dt, plane);
        gravity_dt = std::array::from_fn(|i| subtract(gravity_dt[i], multiply(along, plane[i])));
        gravity_dt[3] = 0.;
        let mut trash = Vec::new();
        for &body in bodies {
            let state = &mut linear[body as usize];
            let mut p = state[0];
            if p[3] > self.max_age {
                trash.push(body);
                continue;
            }
            let inside = (0..3).all(|i| p[i] <= domain[1][i] && domain[0][i] <= p[i]);
            let history = &mut self.history[body as usize];
            if !inside {
                match self.out_of_domain {
                    OutOfDomain::Clip => {
                        p[3] = add(multiply(self.aging, dt), p[3]);
                        state[0] = p;
                        state[1] = [0., 0., 0., state[1][3]];
                        *history = [0; 8];
                        if let Some(angular) = angular.as_deref_mut() {
                            angular[body as usize][1] = ZERO;
                        }
                    }
                    OutOfDomain::Trash => trash.push(body),
                }
                continue;
            }
            let mut v = state[1];
            if planar {
                let along = dot3(plane, v);
                let projected: [f32; 4] = std::array::from_fn(|i| subtract(v[i], multiply(along, plane[i])));
                v = [projected[0], projected[1], projected[2], v[3]];
                state[1] = v;
            }
            if let Some(angular) = angular.as_deref_mut() {
                let [mut q, mut w] = angular[body as usize];
                if planar {
                    // Only the turn about the plane normal remains.
                    let along = dot3(plane, w);
                    w = plane.map(|lane| multiply(along, lane));
                }
                // 140ff9980: q + (dt/2 w) q, renormalized.
                let h = [multiply(half_dt_rotation, w[0]), multiply(half_dt_rotation, w[1]), multiply(half_dt_rotation, w[2]), 0.];
                let cross = [
                    subtract(multiply(q[2], h[1]), multiply(h[2], q[1])),
                    subtract(multiply(q[0], h[2]), multiply(h[0], q[2])),
                    subtract(multiply(q[1], h[0]), multiply(h[1], q[0])),
                ];
                let product: [f32; 4] = [
                    add(add(cross[0], multiply(h[3], q[0])), multiply(q[3], h[0])),
                    add(add(cross[1], multiply(h[3], q[1])), multiply(q[3], h[1])),
                    add(add(cross[2], multiply(h[3], q[2])), multiply(q[3], h[2])),
                    subtract(multiply(q[3], h[3]), dot3(q, h)),
                ];
                let turned: [f32; 4] = std::array::from_fn(|i| add(product[i], q[i]));
                let squares = turned.map(|lane| multiply(lane, lane));
                let length_squared = add(add(squares[3], squares[1]), add(squares[2], squares[0]));
                let inverse = inverse_length(length_squared);
                q = turned.map(|lane| multiply(inverse, lane));
                w = w.map(|lane| multiply(angular_damping, lane));
                let length_squared = add(add(multiply(w[1], w[1]), multiply(w[0], w[0])), multiply(w[2], w[2]));
                let inverse = inverse_length(length_squared);
                if angular_clamp < multiply(inverse, length_squared) {
                    let factor = multiply(inverse, angular_clamp);
                    w = w.map(|lane| multiply(factor, lane));
                }
                angular[body as usize] = [q, w];
            }
            // The running average ((k - 1) a + v) / k.
            let average = unpack([history[0], history[1], history[2], history[3]]);
            let average: [f32; 4] = std::array::from_fn(|i| {
                multiply(add(multiply(average[i], k_minus_one), v[i]), inverse_k)
            });
            let packed = pack(with_pack_w(average));
            history[..4].copy_from_slice(&packed);
            let mut delta: [f32; 4] = std::array::from_fn(|i| multiply(v[i], dt4[i]));
            if self.second_order_accuracy && v[3] > 0. {
                let previous = unpack([history[4], history[5], history[6], history[7]]);
                let current: [f32; 4] = std::array::from_fn(|i| multiply(three_halves_dt, v[i]));
                let previous: [f32; 4] = std::array::from_fn(|i| multiply(previous[i], half_dt));
                if dot3(current, previous) > 0. {
                    delta = std::array::from_fn(|i| subtract(current[i], previous[i]));
                }
            }
            delta[3] = multiply(self.aging, dt);
            let packed = pack(with_pack_w(v));
            history[4..].copy_from_slice(&packed);
            p = std::array::from_fn(|i| add(delta[i], p[i]));
            let scale = v[3];
            v = multiply_add(v, [linear_damping; 4], gravity_dt);
            if clamp {
                let length_squared = add(add(multiply(v[1], v[1]), multiply(v[0], v[0])), multiply(v[2], v[2]));
                let inverse = inverse_length(length_squared);
                if self.linear_clamping < multiply(inverse, length_squared) {
                    let factor = multiply(self.linear_clamping, inverse);
                    v = v.map(|lane| multiply(factor, lane));
                }
            }
            if planar {
                // pshufd 0x4e/0xb1 horizontal sum of n.xyz x p.xyz and n.w.
                let terms = [multiply(plane[0], p[0]), multiply(plane[1], p[1]), multiply(plane[2], p[2]), plane[3]];
                let distance = add(add(terms[1], terms[3]), add(terms[0], terms[2]));
                p = [
                    subtract(p[0], multiply(distance, plane[0])),
                    subtract(p[1], multiply(distance, plane[1])),
                    subtract(p[2], multiply(distance, plane[2])),
                    p[3],
                ];
            }
            state[0] = p;
            state[1] = [v[0], v[1], v[2], scale];
        }
        trash
    }
}
