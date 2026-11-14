//! hkfxEffects::SolveConvexContacts (chunk solver 140fbb350, shape records
//! 140fbdea0, pipeline radii 140fbdc80): the velocity response of rotating
//! convex particles to WorldCollide's static and dynamic contacts. Each hull
//! vertex is a contact point with its own effective mass; a single centre
//! point stands in while the body is far from the plane. Friction acts at
//! the inscribed-sphere contact point, with rolling resistance from the
//! sphere inertia and spin friction about the normal. Transcribed lane by
//! lane from the decompilation; checked against the installed solver with
//! tests/fixtures/convex_contact_cases.txt.
use super::solve_contacts::BodyState;
use super::world_collide::{Contact, WorldCollide};
use crate::float::{add, divide, multiply, subtract};

type V = [f32; 4];

const BIG: f32 = f32::from_bits(0x5f7f_fff0);
const ZERO: V = [0.; 4];
/// 141f15768: the sleep timer's decay per step count.
const DECAY: f32 = f32::from_bits(0x3d7a_e148);

/// A hull record of the shape table (+0x80, 0x120 bytes): up to 16
/// vertices around the centroid, the inscribed (+0x100) and outer (+0x104)
/// radii and the vertex count (+0x110).
#[derive(Clone, Debug, PartialEq)]
pub struct ConvexShape {
    pub vertices: Vec<[f32; 4]>,
    pub inner: f32,
    pub outer: f32,
}

pub struct SolveConvexContacts {
    /// +0x28..+0x38, written each post-step by 1401e1d10.
    pub friction: f32,
    pub restitution: f32,
    pub stickiness: f32,
    pub restitution_threshold: f32,
    pub friction_threshold: f32,
    /// +0x3c steps to fall asleep, +0x40 sleeping enabled.
    pub sleep_steps: i32,
    pub sleep: bool,
    /// +0x68: impulse iterations while the centre touches the plane.
    pub iterations: i32,
    pub bodies: Vec<BodyState>,
    /// +0x80 and the per-body shape index +0x98 (empty: shape 0).
    pub shapes: Vec<ConvexShape>,
    pub shape_index: Vec<i32>,
}

fn lanes(f: impl FnMut(usize) -> f32) -> V {
    std::array::from_fn(f)
}

fn splat(x: f32) -> V {
    [x; 4]
}

/// MAXPS a, b: a when a > b, else b.
fn maxps(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// MINPS a, b: a when a < b, else b.
fn minps(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

#[cfg(target_arch = "x86_64")]
fn rcp(x: f32) -> f32 {
    use std::arch::x86_64::{_mm_cvtss_f32, _mm_rcp_ss, _mm_set_ss};
    unsafe { _mm_cvtss_f32(_mm_rcp_ss(_mm_set_ss(x))) }
}

#[cfg(target_arch = "x86_64")]
fn rsqrt(x: f32) -> f32 {
    use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
    unsafe { _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(x))) }
}

#[cfg(not(target_arch = "x86_64"))]
fn rcp(x: f32) -> f32 {
    1. / x
}

#[cfg(not(target_arch = "x86_64"))]
fn rsqrt(x: f32) -> f32 {
    1. / x.sqrt()
}

/// RCPPS with one Newton step, (2 - r x) r.
fn newton(x: f32) -> f32 {
    let r = rcp(x);
    multiply(subtract(2., multiply(r, x)), r)
}

/// The same, masked to 0 where x == 0.
fn reciprocal(x: f32) -> f32 {
    if x == 0. { 0. } else { newton(x) }
}

/// RSQRTPS with one Newton step, (3 - r x r)(r 0.5), masked to 0 where
/// x <= 0.
fn inverse_sqrt(x: f32) -> f32 {
    if x <= 0. {
        return 0.;
    }
    let r = rsqrt(x);
    multiply(subtract(3., multiply(multiply(r, x), r)), multiply(r, 0.5))
}

/// The square root through the same estimate: ((3 - r x r)(r 0.5)) x.
fn sqrt(x: f32) -> f32 {
    if x <= 0. {
        return 0.;
    }
    let r = rsqrt(x);
    multiply(multiply(subtract(3., multiply(multiply(r, x), r)), multiply(r, 0.5)), x)
}

/// MAXPS(0, MIN(x, big)): 140fbb350's parameter clamp.
fn clamp(x: f32) -> f32 {
    let x = if x < BIG { x } else { BIG };
    maxps(0., x)
}

fn is_zero(v: V) -> bool {
    v.iter().all(|&lane| lane == 0.)
}

fn is_zero3(v: V) -> bool {
    v[..3].iter().all(|&lane| lane == 0.)
}

/// The contact surface's velocity at `point`: angular x (point - centre
/// of mass) + linear, zero without a plane.
fn surface_velocity(contact: &Contact, point: V) -> V {
    if is_zero(contact.plane) {
        return ZERO;
    }
    let d: V = lanes(|i| subtract(point[i], contact.centre_of_mass[i]));
    let w = contact.angular_velocity;
    let l = contact.linear_velocity;
    [
        add(subtract(multiply(d[2], w[1]), multiply(w[2], d[1])), l[0]),
        add(subtract(multiply(d[0], w[2]), multiply(w[0], d[2])), l[1]),
        add(subtract(multiply(d[1], w[0]), multiply(w[1], d[0])), l[2]),
        add(subtract(multiply(d[3], w[3]), multiply(w[3], d[3])), l[3]),
    ]
}

/// r x n with 140fbb350's operand order.
fn lever(r: V, n: V) -> V {
    [
        subtract(multiply(r[1], n[2]), multiply(n[1], r[2])),
        subtract(multiply(r[2], n[0]), multiply(n[2], r[0])),
        subtract(multiply(r[0], n[1]), multiply(n[0], r[1])),
        subtract(multiply(r[3], n[3]), multiply(n[3], r[3])),
    ]
}

/// ((y + x) + z) of a lane product.
fn sum3(p: V) -> f32 {
    add(add(p[1], p[0]), p[2])
}

/// The plane distance summed as ((w + y) + (z + x)).
fn plane_distance(n: V, p: V) -> f32 {
    add(add(n[3], multiply(n[1], p[1])), add(multiply(n[2], p[2]), multiply(n[0], p[0])))
}

/// One contact plane with its clamped material.
#[derive(Clone, Copy)]
struct Plane {
    contact: Contact,
    friction: f32,
    restitution: f32,
}

/// The per-chunk constants of 140fbb350's prologue.
struct Step {
    /// Clamped dt, the raw dt (param_2) and 1/dt.
    dt: f32,
    raw_dt: f32,
    inverse_dt: f32,
    stickiness: f32,
    restitution_threshold: f32,
    /// The squared, clamped friction threshold.
    friction_threshold: f32,
    /// +0x34 of the pipeline: the outer particle radius.
    radius: f32,
    /// 5 / (2 r^2): the sphere inertia factor.
    inertia: f32,
    /// 2 pi / dt: the angular speed limit.
    spin_limit: f32,
    steps: f32,
}

/// The impulse iterations of 140fbb350's stack slot (+0x44): the
/// configured count at the start of the chunk, replaced by 1 whenever a
/// body solves one point or is clear of its planes, and kept otherwise,
/// so it carries over to the following bodies of the chunk.
struct Iterations(i32);

/// The friction pass at the inscribed-sphere contact point `r0`, shared by
/// both paths: tangential impulse limited by the accumulated normal
/// impulse, rolling resistance, and spin friction about the normal.
struct Friction {
    r0: V,
    surface: V,
    spin: f32,
    friction: f32,
}

impl SolveConvexContacts {
    /// 140fbb520 and slot 6: the constructor defaults (as the sphere
    /// solver's) and two impulse iterations.
    pub fn new(capacity: usize) -> Self {
        Self {
            friction: 0.,
            restitution: 0.,
            stickiness: 0.,
            restitution_threshold: 12.,
            friction_threshold: 12.,
            sleep_steps: 4,
            sleep: true,
            iterations: 2,
            bodies: vec![BodyState::default(); capacity],
            shapes: Vec::new(),
            shape_index: Vec::new(),
        }
    }

    pub fn allocate(&mut self, bodies: &[i32]) {
        for &body in bodies {
            self.bodies[body as usize] = BodyState::default();
        }
    }

    /// 140fbb350 over one chunk. `radius` is the pipeline's outer radius
    /// (+0x34), `linear`/`angular` the motion channels (position, velocity
    /// with the size scale in w; orientation, angular velocity).
    pub fn solve(
        &mut self,
        raw_dt: f32,
        radius: f32,
        bodies: &[i32],
        collide: &WorldCollide,
        linear: &mut [[V; 2]],
        angular: &mut [[V; 2]],
    ) {
        let dt = clamp(raw_dt);
        let inverse_dt = reciprocal(dt);
        let threshold = clamp(multiply(multiply(self.friction_threshold, dt), self.friction));
        let step = Step {
            dt,
            raw_dt,
            inverse_dt,
            stickiness: clamp(self.stickiness),
            restitution_threshold: clamp(multiply(self.restitution_threshold, dt)),
            friction_threshold: multiply(threshold, threshold),
            radius,
            inertia: multiply(newton(multiply(multiply(radius, radius), 2.)), 5.),
            spin_limit: multiply(multiply(std::f32::consts::PI, 2.), inverse_dt),
            steps: self.sleep_steps as f32,
        };
        let mut iterations = Iterations(self.iterations);
        for &body in bodies {
            let i = body as usize;
            if self.bodies[i].flags & 1 != 0 {
                continue;
            }
            let scale = linear[i][1][3];
            let squared = multiply(scale, scale);
            let inertia = if squared == 0. { 0. } else { multiply(step.inertia, newton(squared)) };
            let mut planes: Vec<Plane> = Vec::with_capacity(2);
            for (contact, material) in [
                (&collide.dynamic_contacts[i], collide.dynamic_materials[i]),
                (&collide.static_contacts[i], collide.static_materials[i]),
            ] {
                if 0. < material[0] || material[0] == 0. {
                    planes.push(Plane { contact: *contact, friction: clamp(material[0]), restitution: clamp(material[1]) });
                }
            }
            let shape = if self.shape_index.is_empty() { 0 } else { self.shape_index[i] as usize };
            match planes.len() {
                0 => {
                    let state = &mut self.bodies[i];
                    if 0x7e < state.state {
                        state.state = 2;
                        state.timer = 0.;
                    }
                }
                1 => self.one_contact(i, planes[0], shape, inertia, &step, &mut iterations, &mut linear[i], &mut angular[i]),
                _ => self.two_contacts(i, [planes[0], planes[1]], shape, inertia, &step, &mut iterations, &mut linear[i], &mut angular[i]),
            }
        }
    }

    /// The friction impulse of one iteration (140fbbfd0.. / 140fbd0b0..):
    /// returns the new tangential accumulator and whether it slid.
    #[allow(clippy::too_many_arguments)]
    fn friction(
        f: &Friction,
        n: V,
        normal_total: V,
        tangent_total: V,
        rolling: f32,
        inverse_rs: f32,
        inertia: f32,
        v: &mut V,
        w: &mut V,
    ) -> (V, bool) {
        let r0 = f.r0;
        let vt = [
            subtract(add(subtract(multiply(w[1], r0[2]), multiply(w[2], r0[1])), v[0]), f.surface[0]),
            subtract(add(subtract(multiply(w[2], r0[0]), multiply(w[0], r0[2])), v[1]), f.surface[1]),
            subtract(add(subtract(multiply(w[0], r0[1]), multiply(w[1], r0[0])), v[2]), f.surface[2]),
        ];
        let vn = add(add(multiply(n[1], vt[1]), multiply(n[0], vt[0])), multiply(n[2], vt[2]));
        let t = [
            subtract(vt[0], multiply(n[0], vn)),
            subtract(vt[1], multiply(n[1], vn)),
            subtract(vt[2], multiply(n[2], vn)),
            subtract(
                subtract(add(subtract(multiply(w[3], r0[3]), multiply(w[3], r0[3])), v[3]), f.surface[3]),
                multiply(n[3], vn),
            ),
        ];
        let wn = add(add(multiply(n[1], w[1]), multiply(n[0], w[0])), multiply(n[2], w[2]));
        let spin = subtract(wn, f.spin);
        let length = sqrt(add(add(multiply(t[1], t[1]), multiply(t[0], t[0])), multiply(t[2], t[2])));
        let limit = multiply(length, inverse_rs);
        let clamped = maxps(subtract(0., limit), if spin < limit { spin } else { limit });
        let mut next: V = lanes(|l| subtract(tangent_total[l], multiply(rolling, t[l])));
        let bound: V = lanes(|l| multiply(f.friction, normal_total[l]));
        let size = add(add(multiply(next[1], next[1]), multiply(next[0], next[0])), multiply(next[2], next[2]));
        let slides = multiply(bound[0], bound[0]) < size;
        if slides {
            let inverse = inverse_sqrt(size);
            next = lanes(|l| multiply(multiply(next[l], inverse), bound[l]));
        }
        let d: V = lanes(|l| subtract(next[l], tangent_total[l]));
        *v = lanes(|l| add(v[l], d[l]));
        let turn = subtract(clamped, spin);
        *w = [
            add(add(w[0], multiply(turn, n[0])), multiply(subtract(multiply(d[2], r0[1]), multiply(d[1], r0[2])), inertia)),
            add(add(w[1], multiply(turn, n[1])), multiply(subtract(multiply(d[0], r0[2]), multiply(d[2], r0[0])), inertia)),
            add(add(w[2], multiply(turn, n[2])), multiply(subtract(multiply(d[1], r0[0]), multiply(d[0], r0[1])), inertia)),
            add(add(w[3], multiply(turn, n[3])), multiply(subtract(multiply(d[3], r0[3]), multiply(d[3], r0[3])), inertia)),
        ];
        (next, slides)
    }

    /// The angular velocity clamp to 2 pi per step.
    fn clamp_spin(step: &Step, w: &mut V) {
        let squared = sum3(lanes(|l| multiply(w[l], w[l])));
        let inverse = inverse_sqrt(squared);
        if step.spin_limit < multiply(inverse, squared) {
            let factor = multiply(inverse, step.spin_limit);
            *w = lanes(|l| multiply(factor, w[l]));
        }
    }

    /// The hull vertices rotated by the orientation and scaled by `k`.
    fn rotated(shape: &ConvexShape, orientation: V, k: f32) -> Vec<V> {
        let m = crate::assets::physworld::quaternion_basis(orientation).map(|row| row.map(|x| multiply(x, k)));
        shape
            .vertices
            .iter()
            .map(|v| lanes(|l| add(add(multiply(m[1][l], v[1]), multiply(m[0][l], v[0])), multiply(m[2][l], v[2]))))
            .collect()
    }

    /// 140fbb7ca..140fbc599: one contact plane.
    #[allow(clippy::too_many_arguments)]
    fn one_contact(&mut self, i: usize, plane: Plane, shape: usize, inertia: f32, step: &Step, carried: &mut Iterations, body: &mut [V; 2], spin: &mut [V; 2]) {
        let n = plane.contact.plane;
        let av = plane.contact.angular_velocity;
        let old_state = self.bodies[i].state;
        let mut stay_awake = !self.sleep;
        let s = body[1][3];
        let shape = &self.shapes[shape];
        let rs = multiply(step.radius, s);
        let inverse_rs = reciprocal(rs);
        let o = shape.outer;
        let k = multiply(if o == 0. { 0. } else { multiply(newton(o), step.radius) }, s);
        let re = multiply(shape.inner, k);
        let ratio = if rs == 0. { 0. } else { multiply(newton(rs), re) };
        let rolling = multiply(newton(add(multiply(multiply(ratio, ratio), 5.), 2.)), 2.);
        Self::clamp_spin(step, &mut spin[1]);
        let p = body[0];
        let sv = surface_velocity(&plane.contact, p);
        let rel: V = lanes(|l| subtract(body[1][l], sv[l]));
        let centre = plane_distance(n, p);
        let gap = subtract(centre, re);
        let vn = add(add(multiply(n[1], rel[1]), multiply(n[0], rel[0])), multiply(n[2], rel[2]));
        if 0x7e < self.bodies[i].state {
            let t: V = lanes(|l| subtract(rel[l], multiply(n[l], vn)));
            let sliding = sum3(lanes(|l| multiply(t[l], t[l])));
            if 0. <= vn || step.friction_threshold < sliding {
                // 140ffaaf0 notifies the listener; the body wakes.
                stay_awake = true;
            }
            if !stay_awake {
                let still = is_zero3(sv) && is_zero3(av);
                body[1] = [sv[0], sv[1], sv[2], body[1][3]];
                spin[1] = av;
                let state = &mut self.bodies[i];
                state.timer = 0.;
                state.state = 0x7f + i32::from(still);
                return;
            }
        }
        let approach = maxps(0., multiply(subtract(0., vn), step.dt));
        let v = body[1];
        let predicted: V = lanes(|l| add(multiply(v[l], step.raw_dt), p[l]));
        let timer = self.bodies[i].timer;
        let far = subtract(plane_distance(n, predicted), rs) > 0. && (timer == 0. || timer.is_nan());
        // Contact points: (lever arm, r x n, effective mass, separation
        // allowance, surface velocity).
        let mut points: Vec<(V, V, f32, f32, V)> = Vec::new();
        let mut iterations = 1;
        if shape.vertices.is_empty() || far {
            let r: V = lanes(|l| multiply(subtract(0., re), n[l]));
            let cp: V = lanes(|l| add(p[l], r[l]));
            let separation = maxps(0., multiply(subtract(plane_distance(n, cp), approach), step.inverse_dt));
            let rxn = [
                subtract(multiply(n[2], r[1]), multiply(r[2], n[1])),
                subtract(multiply(n[0], r[2]), multiply(r[0], n[2])),
                subtract(multiply(n[1], r[0]), multiply(r[1], n[0])),
                subtract(multiply(n[3], r[3]), multiply(r[3], n[3])),
            ];
            points.push((r, rxn, 1., separation, surface_velocity(&plane.contact, cp)));
        } else {
            for rv in Self::rotated(shape, spin[0], k) {
                let cp = [add(rv[0], p[0]), add(rv[1], p[1]), add(rv[2], p[2]), add(rv[3], p[3])];
                let separation = maxps(0., multiply(step.inverse_dt, subtract(plane_distance(n, cp), approach)));
                let rxn = lever(rv, n);
                let mass = newton(add(
                    add(
                        add(multiply(multiply(inertia, rxn[1]), rxn[1]), multiply(multiply(inertia, rxn[0]), rxn[0])),
                        multiply(multiply(inertia, rxn[2]), rxn[2]),
                    ),
                    1.,
                ));
                points.push((rv, rxn, mass, separation, surface_velocity(&plane.contact, cp)));
            }
            if points.len() >= 2 && !(subtract(plane_distance(n, p), rs) > 0.) {
                iterations = carried.0;
            }
        }
        carried.0 = iterations;
        let approach = maxps(approach, re);
        let r0: V = lanes(|l| multiply(subtract(0., re), n[l]));
        let penetration = minps(0., subtract(add(gap, re), approach));
        let friction = Friction {
            r0,
            surface: surface_velocity(&plane.contact, lanes(|l| add(r0[l], p[l]))),
            spin: sum3(lanes(|l| multiply(av[l], n[l]))),
            friction: plane.friction,
        };
        self.bodies[i].state = 0;
        let mut v = body[1];
        let mut w = spin[1];
        let mut bounce = 0f32;
        let mut normal_total = ZERO;
        let mut tangent_total = ZERO;
        for _ in 0..iterations {
            for &(_, rxn, mass, separation, surface) in &points {
                let a = multiply(w[0], rxn[0]);
                let b = multiply(w[1], rxn[1]);
                let c = multiply(w[2], rxn[2]);
                let e = multiply(n[0], subtract(v[0], surface[0]));
                let f = multiply(n[1], subtract(v[1], surface[1]));
                let g = multiply(n[2], subtract(v[2], surface[2]));
                let relative = add(add(add(f, e), g), add(add(b, a), c));
                let u = add(separation, relative);
                if u < step.stickiness {
                    let kick = minps(0., add(multiply(plane.restitution, relative), step.restitution_threshold));
                    bounce = add(bounce, kick);
                    let impulse = multiply(subtract(0., add(kick, u)), mass);
                    normal_total = lanes(|l| subtract(normal_total[l], multiply(subtract(u, step.stickiness), mass)));
                    v = lanes(|l| add(v[l], multiply(n[l], impulse)));
                    w = lanes(|l| add(w[l], multiply(multiply(inertia, rxn[l]), impulse)));
                }
            }
            if normal_total[0] == 0. {
                break;
            }
            let (next, slides) = Self::friction(&friction, n, normal_total, tangent_total, rolling, inverse_rs, inertia, &mut v, &mut w);
            tangent_total = next;
            self.bodies[i].state = 3 - i32::from(slides);
        }
        body[1] = [v[0], v[1], v[2], body[1][3]];
        spin[1] = w;
        let state = &mut self.bodies[i];
        if !(bounce >= 0.) {
            state.state = 1;
        }
        let dv: V = lanes(|l| subtract(sv[l], body[1][l]));
        let mut asleep = false;
        if self.sleep {
            let pushed = multiply(multiply(penetration, step.inverse_dt), multiply(penetration, step.inverse_dt));
            if pushed <= sum3(lanes(|l| multiply(dv[l], dv[l]))) {
                if add(step.stickiness, step.friction_threshold) < sum3(lanes(|l| multiply(dv[l], n[l]))) {
                    let timer = subtract(state.timer, divide(DECAY, step.steps));
                    state.timer = maxps(0., timer);
                }
            } else {
                state.timer = add(divide(1., step.steps), state.timer);
            }
            // COMISS timer, 1 then JB: an unordered timer stays awake.
            asleep = 1. <= state.timer;
        }
        if !(penetration >= 0.) {
            if asleep {
                body[1] = [sv[0], sv[1], sv[2], body[1][3]];
                spin[1] = av;
                state.state = 0x7f;
            } else {
                let q = body[0];
                body[0] = [
                    subtract(q[0], multiply(n[0], penetration)),
                    subtract(q[1], multiply(n[1], penetration)),
                    subtract(q[2], multiply(n[2], penetration)),
                    q[3],
                ];
            }
        }
        Self::tunnel_clamp(n, step.raw_dt, sv, body);
        if !stay_awake && state.state == 3 && 2 < old_state {
            state.state = old_state + 1;
        }
    }

    /// A body above the plane moving into it and about to pass below it
    /// takes the surface velocity.
    fn tunnel_clamp(n: V, dt: f32, surface: V, body: &mut [V; 2]) {
        let p = body[0];
        let v = body[1];
        if 0. <= plane_distance(n, p) && !(sum3(lanes(|l| multiply(n[l], v[l]))) >= 0.) {
            let next: V = lanes(|l| add(multiply(dt, v[l]), p[l]));
            if !(plane_distance(n, next) >= 0.) {
                body[1] = [surface[0], surface[1], surface[2], v[3]];
            }
        }
    }

    /// 140fbc599..140fbd74b: a dynamic and a static plane.
    #[allow(clippy::too_many_arguments)]
    fn two_contacts(&mut self, i: usize, planes: [Plane; 2], shape: usize, inertia: f32, step: &Step, carried: &mut Iterations, body: &mut [V; 2], spin: &mut [V; 2]) {
        let old_state = self.bodies[i].state;
        let s = body[1][3];
        let shape = &self.shapes[shape];
        let rs = multiply(step.radius, s);
        let inverse_rs = reciprocal(rs);
        let o = shape.outer;
        let k = multiply(if o == 0. { 0. } else { multiply(newton(o), step.radius) }, s);
        let re = multiply(shape.inner, k);
        let ratio = if rs == 0. { 0. } else { multiply(newton(rs), re) };
        let rolling = multiply(newton(add(multiply(multiply(ratio, ratio), 5.), 2.)), 2.);
        Self::clamp_spin(step, &mut spin[1]);
        let p = body[0];
        let v = body[1];
        let mut sv = [ZERO; 2];
        let mut vn = [0f32; 2];
        let mut gap = [0f32; 2];
        let mut rel = [ZERO; 2];
        let mut closest = 0;
        let mut best = f32::from_bits(0x7f7f_ffee);
        let mut agree = true;
        for c in 0..2 {
            let n = planes[c].contact.plane;
            sv[c] = surface_velocity(&planes[c].contact, p);
            rel[c] = lanes(|l| subtract(v[l], sv[c][l]));
            gap[c] = subtract(plane_distance(n, p), re);
            if gap[c] < best {
                best = gap[c];
                closest = c;
            }
            vn[c] = sum3(lanes(|l| multiply(n[l], rel[c][l])));
            if c == 1 {
                let d: V = lanes(|l| subtract(sv[0][l], sv[1][l]));
                agree = agree && !(step.friction_threshold <= sum3(lanes(|l| multiply(d[l], d[l]))));
            }
        }
        let mut stay_awake = false;
        if 0x7e < self.bodies[i].state {
            let (mut still, mut still_wake, mut moving_wake) = (false, true, false);
            for c in 0..2 {
                let n = planes[c].contact.plane;
                let t: V = lanes(|l| subtract(rel[c][l], multiply(vn[c], n[l])));
                let is_still = is_zero3(sv[c]);
                still |= is_still;
                let wake = 0. <= vn[c] || step.friction_threshold < sum3(lanes(|l| multiply(t[l], t[l])));
                if is_still {
                    still_wake = still_wake && wake;
                } else {
                    moving_wake |= wake;
                }
            }
            stay_awake = (still && still_wake) || moving_wake;
        }
        if 0x7e < self.bodies[i].state && !stay_awake {
            let av = planes[closest].contact.angular_velocity;
            let still = is_zero3(sv[closest]) && is_zero3(av);
            body[1] = [sv[closest][0], sv[closest][1], sv[closest][2], body[1][3]];
            spin[1] = av;
            let state = &mut self.bodies[i];
            state.timer = 0.;
            state.state = 0x7f + i32::from(still);
            return;
        }
        let mut approach = [0f32; 2];
        let mut near = false;
        for c in 0..2 {
            let n = planes[c].contact.plane;
            approach[c] = maxps(0., multiply(subtract(0., vn[c]), step.dt));
            let predicted: V = lanes(|l| add(p[l], multiply(step.dt, v[l])));
            near = near || !(subtract(plane_distance(n, predicted), rs) >= 0.);
        }
        let timer = self.bodies[i].timer;
        let single = shape.vertices.is_empty() || !(near || !(timer == 0. || timer.is_nan()));
        // Per plane, its contact points (r x n, mass, separation, surface
        // velocity).
        let mut points: [Vec<(V, f32, f32, V)>; 2] = [Vec::new(), Vec::new()];
        let mut iterations = 1;
        if single {
            for c in 0..2 {
                let contact = &planes[c].contact;
                let n = contact.plane;
                let r: V = lanes(|l| multiply(subtract(0., re), n[l]));
                let cp: V = lanes(|l| add(p[l], r[l]));
                let rxn = [
                    subtract(multiply(n[2], r[1]), multiply(n[1], r[2])),
                    subtract(multiply(n[0], r[2]), multiply(n[2], r[0])),
                    subtract(multiply(n[1], r[0]), multiply(n[0], r[1])),
                    subtract(multiply(n[3], r[3]), multiply(n[3], r[3])),
                ];
                let distance = add(add(n[3], multiply(n[1], cp[1])), add(multiply(n[2], cp[2]), multiply(n[0], cp[0])));
                let separation = maxps(0., multiply(subtract(distance, approach[c]), step.inverse_dt));
                points[c].push((rxn, 1., separation, surface_velocity(contact, cp)));
            }
        } else {
            let rotated = Self::rotated(shape, spin[0], k);
            let mut touching = false;
            for c in 0..2 {
                let contact = &planes[c].contact;
                let n = contact.plane;
                touching = touching || !(subtract(plane_distance(n, p), rs) >= 0.);
                for &rv in &rotated {
                    let cp = [add(p[0], rv[0]), add(p[1], rv[1]), add(p[2], rv[2]), add(p[3], rv[3])];
                    let distance = add(add(n[3], multiply(n[1], cp[1])), add(multiply(n[2], cp[2]), multiply(n[0], cp[0])));
                    let separation = maxps(0., multiply(subtract(distance, approach[c]), step.inverse_dt));
                    let rxn = lever(rv, n);
                    let mass = newton(add(
                        add(
                            add(multiply(multiply(inertia, rxn[1]), rxn[1]), multiply(multiply(inertia, rxn[0]), rxn[0])),
                            multiply(multiply(inertia, rxn[2]), rxn[2]),
                        ),
                        1.,
                    ));
                    points[c].push((rxn, mass, separation, surface_velocity(contact, cp)));
                }
            }
            if rotated.len() > 1 && touching {
                iterations = carried.0;
            }
        }
        carried.0 = iterations;
        let mut penetration = [0f32; 2];
        let mut friction: [Option<Friction>; 2] = [None, None];
        for c in 0..2 {
            let contact = &planes[c].contact;
            let n = contact.plane;
            let a = maxps(approach[c], re);
            penetration[c] = minps(0., subtract(add(gap[c], re), a));
            let r0: V = lanes(|l| multiply(subtract(0., re), n[l]));
            friction[c] = Some(Friction {
                r0,
                surface: surface_velocity(contact, lanes(|l| add(p[l], r0[l]))),
                spin: sum3(lanes(|l| multiply(contact.angular_velocity[l], n[l]))),
                friction: planes[c].friction,
            });
        }
        self.bodies[i].state = 0;
        let mut v = body[1];
        let mut w = spin[1];
        let mut bounce = 0f32;
        let mut normal_total = [ZERO; 2];
        let mut tangent_total = [ZERO; 2];
        for _ in 0..iterations {
            let mut done = true;
            for c in 0..2 {
                let n = planes[c].contact.plane;
                for &(rxn, mass, separation, surface) in &points[c] {
                    let a = multiply(w[0], rxn[0]);
                    let b = multiply(w[1], rxn[1]);
                    let cc = multiply(w[2], rxn[2]);
                    let e = multiply(n[0], subtract(v[0], surface[0]));
                    let f = multiply(n[1], subtract(v[1], surface[1]));
                    let g = multiply(n[2], subtract(v[2], surface[2]));
                    let relative = add(add(add(f, e), g), add(add(b, a), cc));
                    let u = add(separation, relative);
                    if u < step.stickiness {
                        let kick = minps(0., add(multiply(planes[c].restitution, relative), step.restitution_threshold));
                        bounce = add(bounce, kick);
                        let impulse = multiply(subtract(0., add(u, kick)), mass);
                        v = lanes(|l| add(v[l], multiply(impulse, n[l])));
                        w = lanes(|l| add(w[l], multiply(multiply(inertia, rxn[l]), impulse)));
                        normal_total[c] = lanes(|l| subtract(normal_total[c][l], multiply(subtract(u, step.stickiness), mass)));
                    }
                }
                if normal_total[c][0] != 0. {
                    let f = friction[c].as_ref().expect("set above");
                    let (next, slides) = Self::friction(f, n, normal_total[c], tangent_total[c], rolling, inverse_rs, inertia, &mut v, &mut w);
                    tangent_total[c] = next;
                    done = false;
                    self.bodies[i].state = 3 - i32::from(slides);
                }
            }
            if done {
                break;
            }
        }
        body[1] = [v[0], v[1], v[2], body[1][3]];
        spin[1] = w;
        let state = &mut self.bodies[i];
        if !(bounce >= 0.) {
            state.state = 1;
        }
        let n = planes[closest].contact.plane;
        let surface = sv[closest];
        let pen = penetration[closest];
        let dv: V = lanes(|l| subtract(surface[l], body[1][l]));
        let mut asleep = false;
        if agree {
            if self.sleep {
                let pushed = multiply(multiply(pen, step.inverse_dt), multiply(pen, step.inverse_dt));
                if pushed <= sum3(lanes(|l| multiply(dv[l], dv[l]))) {
                    if add(step.stickiness, step.friction_threshold) < sum3(lanes(|l| multiply(dv[l], n[l]))) {
                        let timer = subtract(state.timer, divide(DECAY, step.steps));
                        state.timer = maxps(0., timer);
                    }
                } else {
                    state.timer = add(divide(1., step.steps), state.timer);
                }
                // COMISS 1, timer then JA: an unordered timer sleeps.
                asleep = !(1. > state.timer);
            }
        } else {
            state.timer = 0.;
        }
        let fixed = planes[1].contact.plane;
        if !(pen >= 0.) {
            if asleep {
                body[1] = [surface[0], surface[1], surface[2], body[1][3]];
                spin[1] = planes[closest].contact.angular_velocity;
                state.state = 0x7f;
            } else {
                let q = body[0];
                let q = [
                    subtract(q[0], multiply(pen, n[0])),
                    subtract(q[1], multiply(pen, n[1])),
                    subtract(q[2], multiply(pen, n[2])),
                    q[3],
                ];
                body[0] = q;
                if closest != 1 {
                    let below = subtract(plane_distance(fixed, q), re);
                    if !(below >= 0.) {
                        body[0] = [
                            subtract(q[0], multiply(fixed[0], below)),
                            subtract(q[1], multiply(fixed[1], below)),
                            subtract(q[2], multiply(fixed[2], below)),
                            q[3],
                        ];
                    }
                }
            }
        }
        Self::tunnel_clamp(fixed, step.dt, sv[1], body);
        if !stay_awake && state.state == 3 && 2 < old_state {
            state.state = old_state + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The installed 140fbb350 on synthetic chunks
    /// (analysis/probe_convex_contacts.py).
    #[test]
    fn matches_installed_solver() {
        let (exact, total, first) = compare_fixture();
        assert_eq!(exact, total, "first difference: {first}");
    }
}

/// Run tests/fixtures/convex_contact_cases.txt through the transcription;
/// returns (exact bodies, bodies, first difference).
#[doc(hidden)]
pub fn compare_fixture() -> (usize, usize, String) {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/convex_contact_cases.txt")).expect("fixture");
    let (mut exact, mut total, mut first) = (0, 0, String::new());
    for (line_number, line) in text.lines().filter(|l| !l.starts_with('#')).enumerate() {
        let w: Vec<u32> = line.split_whitespace().map(|h| u32::from_str_radix(h, 16).expect("hex")).collect();
        let f = |i: usize| f32::from_bits(w[i]);
        let mut solver = SolveConvexContacts::new(3);
        solver.friction = f(1);
        solver.restitution = f(2);
        solver.stickiness = f(3);
        solver.restitution_threshold = f(4);
        solver.friction_threshold = f(5);
        solver.sleep_steps = w[6] as i32;
        solver.sleep = w[7] != 0;
        solver.iterations = w[8] as i32;
        let radius = f(10);
        let count = w[13] as usize;
        solver.shapes = vec![ConvexShape {
            inner: f(11),
            outer: f(12),
            vertices: (0..count).map(|v| std::array::from_fn(|l| f(14 + 4 * v + l))).collect(),
        }];
        let mut at = 14 + 64;
        let mut linear = vec![[ZERO; 2]; 3];
        let mut angular = vec![[ZERO; 2]; 3];
        let mut collide = WorldCollide::new(3);
        let contact = |at: usize| Contact {
            plane: std::array::from_fn(|l| f(at + l)),
            linear_velocity: std::array::from_fn(|l| f(at + 4 + l)),
            angular_velocity: std::array::from_fn(|l| f(at + 8 + l)),
            centre_of_mass: std::array::from_fn(|l| f(at + 12 + l)),
        };
        for b in 0..3 {
            solver.bodies[b] = BodyState { state: w[at] as i32, timer: f(at + 1), flags: w[at + 2] as u8 };
            linear[b] = [std::array::from_fn(|l| f(at + 3 + l)), std::array::from_fn(|l| f(at + 7 + l))];
            angular[b] = [std::array::from_fn(|l| f(at + 11 + l)), std::array::from_fn(|l| f(at + 15 + l))];
            collide.static_contacts[b] = contact(at + 19);
            collide.static_materials[b] = [f(at + 35), f(at + 36)];
            collide.dynamic_contacts[b] = contact(at + 37);
            collide.dynamic_materials[b] = [f(at + 53), f(at + 54)];
            at += 55;
        }
        solver.solve(f(0), radius, &[0, 1, 2], &collide, &mut linear, &mut angular);
        for b in 0..3 {
            let expected = &w[at..at + 19];
            at += 19;
            let s = solver.bodies[b];
            let ours: Vec<u32> = [s.state as u32, s.timer.to_bits(), u32::from(s.flags)]
                .into_iter()
                .chain(linear[b].iter().chain(angular[b].iter()).flat_map(|v| v.iter().map(|x| x.to_bits())))
                .collect();
            total += 1;
            if ours == expected {
                exact += 1;
            } else if first.is_empty() {
                let lanes: Vec<usize> = (0..19).filter(|&j| ours[j] != expected[j]).collect();
                first = format!("case {line_number} body {b}: lanes {lanes:?}");
            }
        }
    }
    (exact, total, first)
}
