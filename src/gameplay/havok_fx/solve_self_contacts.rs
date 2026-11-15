//! hkfxEffects::SolveSelfContacts (constructor 140fd2620, compute
//! 140fd33d0): sphere-sphere contact constraints between the bodies of one
//! stage, created from the SpatialPartition's new overlapping pairs and
//! solved with RigidConstraintSolver<0> (140fd5d00, linear) or <1>
//! (140fd5320, with AngularMotion). The game runs the hkfx CPU backend's
//! SingleThreadedParallel (one thread; hkfxParallelPhysics' constructor
//! has no caller), so constraints are solved in creation order as one batch.
//!
//! In the shipped game the effect never receives a pair: the stage's
//! SpatialPartition keeps computeNewPairs (+0x2a) off (140fd6a00; the
//! simulator 1401dfc20 never sets it, and the config copy 140fee960 is only
//! reached from the uncalled 140fecbc0). The solvers are ported and checked
//! against the installed functions for completeness; the stage simulation
//! does not instantiate the effect, since without constraints it changes no
//! body (prepare clears bit 2, so the sleep test (state & 6) == 6 of the
//! end pass 140fd30e0 and ProcessDeactivation 140fd67b0 never apply).
use crate::float::{add, divide, multiply, subtract};

const HUGE: f32 = f32::from_bits(0x5f7f_fff0);
/// 141ec4f44: the tangential velocity share removed per contact (2/7).
const TANGENT: f32 = f32::from_bits(0x3e92_4925);

fn maxps(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

fn minps(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

fn rsqrt_estimate(x: f32) -> f32 {
    super::physics_collide::rsqrt_estimate(x)
}

/// ((3 - e x e) e) 0.5, masked to 0 at or below zero.
fn inverse_length(x: f32) -> f32 {
    if x <= 0. {
        return 0.;
    }
    let e = rsqrt_estimate(x);
    multiply(multiply(subtract(3., multiply(multiply(e, x), e)), e), 0.5)
}

/// RCPPS with one Newton step, masked to 0 at zero.
fn reciprocal(x: f32) -> f32 {
    if x == 0. {
        return 0.;
    }
    let r = super::physics_collide::rcp_estimate(x);
    multiply(subtract(2., multiply(r, x)), r)
}

fn clamp_material(x: f32) -> f32 {
    maxps(0., if x < HUGE { x } else { HUGE })
}

/// One constraint (+0xa0, 16 bytes): the two bodies, the batch key, the
/// steps it has existed and the top half of its last contact distance².
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Constraint {
    pub a: i32,
    pub b: i32,
    pub key: u32,
    pub age: u16,
    pub distance: u16,
}

/// The solver's material (+0x2c friction, +0x30 restitution, +0x34
/// stickiness, +0x38 restitution threshold) and deactivation delay (+0x4c).
#[derive(Clone, Copy, Debug)]
pub struct SelfMaterial {
    pub friction: f32,
    pub restitution: f32,
    pub stickiness: f32,
    pub restitution_threshold: f32,
    pub deactivation_delay: f32,
}

/// 140fd5d00: one batch of constraints for linear-only bodies. `states` is
/// +0xe0 (bit 0 asleep, bits 1-2 the wake count, bits 3+ the constraint
/// count), `linear` LinearMotion's channels, `radius` the pipeline's +0x34.
pub fn solve_linear(
    dt: f32,
    radius: f32,
    material: &SelfMaterial,
    constraints: &mut [Constraint],
    order: &[i32],
    states: &mut [[u16; 2]],
    linear: &mut [[[f32; 4]; 2]],
) {
    let inverse_dt = reciprocal(dt);
    let friction = clamp_material(material.friction);
    let stickiness = clamp_material(material.stickiness);
    let restitution = clamp_material(material.restitution);
    let threshold = clamp_material(material.restitution_threshold);
    let delay = (add(multiply(inverse_dt, material.deactivation_delay), 0.5) as i32).min(0x7ffe);
    for &index in order {
        let c = &mut constraints[index as usize];
        let (ia, ib) = (c.a as usize, c.b as usize);
        let (state_a, state_b) = (states[ia][0], states[ib][0]);
        let (asleep_a, asleep_b) = (state_a & 1 != 0, state_b & 1 != 0);
        if asleep_a && asleep_b {
            c.age = 0;
            c.distance = 0;
            continue;
        }
        let [pa, va] = linear[ia];
        let [pb, vb] = linear[ib];
        let previous = f32::from_bits(((c.distance as i16 as i32) << 16) as u32);
        let d: [f32; 3] = std::array::from_fn(|k| subtract(pb[k], pa[k]));
        let reach = add(multiply(vb[3], radius), multiply(va[3], radius));
        let reach2 = multiply(reach, reach);
        let squared = add(add(multiply(d[1], d[1]), multiply(d[0], d[0])), multiply(d[2], d[2]));
        let m = minps(maxps(minps(squared, reach2), previous), reach2);
        c.distance = (m.to_bits() >> 16) as u16;
        let half = {
            if m <= 0. {
                0.
            } else {
                let e = rsqrt_estimate(m);
                multiply(multiply(subtract(3., multiply(multiply(e, m), e)), multiply(e, 0.5)), m)
            }
        };
        let half = multiply(half, 0.5);
        let inverse = inverse_length(squared);
        let n: [f32; 3] = std::array::from_fn(|k| multiply(d[k], inverse));
        let w = subtract(
            subtract(0., add(add(multiply(pa[1], n[1]), multiply(pa[0], n[0])), multiply(pa[2], n[2]))),
            half,
        );
        let share_b = divide(1., f32::from(state_b >> 3));
        let share_a = divide(1., f32::from(state_a >> 3));
        let kb = if asleep_b { 0. } else { share_b };
        let ka = if asleep_a { 0. } else { share_a };
        let s = reciprocal(add(kb, ka));
        let pn: [f32; 3] = std::array::from_fn(|k| multiply(pb[k], n[k]));
        let dv: [f32; 3] = std::array::from_fn(|k| subtract(vb[k], va[k]));
        let vn = add(add(multiply(dv[1], n[1]), multiply(dv[0], n[0])), multiply(dv[2], n[2]));
        let separation = subtract(add(add(w, pn[1]), add(pn[2], pn[0])), half);
        let stick = maxps(0., if vn < stickiness { vn } else { stickiness });
        let ahead = add(multiply(vn, dt), separation);
        let push = minps(0., multiply(ahead, 0.5));
        let bounce = maxps(0., subtract(multiply(subtract(0., vn), restitution), multiply(threshold, dt)));
        let raw = add(
            subtract(multiply(subtract(0., push), inverse_dt), if ahead < 0. { 0. } else { stick }),
            if separation < 0. { 0. } else { bounce },
        );
        let impulse = maxps(0., raw);
        let mut tangent: [f32; 3] =
            std::array::from_fn(|k| multiply(subtract(dv[k], multiply(vn, n[k])), subtract(0., TANGENT)));
        let cone = multiply(impulse, friction);
        let t2 = add(add(multiply(tangent[1], tangent[1]), multiply(tangent[0], tangent[0])), multiply(tangent[2], tangent[2]));
        if multiply(cone, cone) < t2 {
            let scale = inverse_length(t2);
            tangent = tangent.map(|t| multiply(multiply(t, scale), cone));
        }
        let awake = i32::from(c.age) > delay
            && add(add(multiply(tangent[1], tangent[1]), multiply(tangent[0], tangent[0])), multiply(tangent[2], tangent[2])) > 0.;
        // The unclamped impulse along the normal (140fd63b5).
        let j: [f32; 3] = std::array::from_fn(|k| add(multiply(raw, n[k]), tangent[k]));
        for k in 0..3 {
            linear[ib][1][k] = add(multiply(multiply(s, kb), j[k]), vb[k]);
        }
        let wake = |state: u16| {
            let count = (((state >> 1) & 3) + 1).min(3);
            ((count * 2) ^ state) & 6 ^ state
        };
        if awake {
            states[ib][0] = wake(state_b);
        }
        for k in 0..3 {
            linear[ia][1][k] = add(va[k], multiply(subtract(0., multiply(s, ka)), j[k]));
        }
        if awake {
            states[ia][0] = wake(state_a);
        }
    }
}

/// Runs tests/fixtures/self_contact_cases.txt (analysis/probe_self_contacts.py)
/// for the linear solver; returns (cases, mismatches).
#[doc(hidden)]
pub fn compare_fixture(text: &str, angular: bool) -> (usize, Vec<String>) {
    let mut failures = Vec::new();
    let mut cases = 0;
    for line in text.lines().filter(|line| !line.starts_with('#') && !line.is_empty()) {
        let (input, output) = line.split_once(" ; ").unwrap();
        let w: Vec<u32> = input.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
        let o: Vec<u32> = output.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
        if (w[0] != 0) != angular {
            continue;
        }
        let f = |x: u32| f32::from_bits(x);
        let (bodies, pairs) = (w[1] as usize, w[2] as usize);
        let radius = f(w[3]);
        let material = SelfMaterial {
            friction: f(w[4]),
            restitution: f(w[5]),
            stickiness: f(w[6]),
            restitution_threshold: f(w[7]),
            deactivation_delay: f(w[8]),
        };
        let dt = f(w[9]);
        let start = w[10] as usize;
        let mut at = 11;
        let read_bodies = |at: &mut usize, w: &[u32]| -> Vec<[[f32; 4]; 2]> {
            let v = (0..bodies)
                .map(|b| [std::array::from_fn(|k| f(w[*at + 8 * b + k])), std::array::from_fn(|k| f(w[*at + 8 * b + 4 + k]))])
                .collect();
            *at += 8 * bodies;
            v
        };
        let mut linear = read_bodies(&mut at, &w);
        let mut spin = read_bodies(&mut at, &w);
        let mut states: Vec<[u16; 2]> = (0..bodies).map(|b| [w[at + b] as u16, (w[at + b] >> 16) as u16]).collect();
        at += bodies;
        let mut constraints: Vec<Constraint> = (0..pairs)
            .map(|i| {
                let c = &w[at + 4 * i..at + 4 * i + 4];
                Constraint { a: c[0] as i32, b: c[1] as i32, key: c[2], age: c[3] as u16, distance: (c[3] >> 16) as u16 }
            })
            .collect();
        at += 4 * pairs;
        let order: Vec<i32> = w[at + start..at + pairs].iter().map(|&x| x as i32).collect();
        if angular {
            solve_angular(dt, radius, &material, &mut constraints, &order, &mut states, &mut linear, &mut spin);
        } else {
            solve_linear(dt, radius, &material, &mut constraints, &order, &mut states, &mut linear);
        }
        let mut got = Vec::new();
        for body in linear.iter().chain(&spin) {
            got.extend(body.iter().flatten().map(|x| x.to_bits()));
        }
        got.extend(states.iter().map(|s| u32::from(s[0]) | u32::from(s[1]) << 16));
        for c in &constraints {
            got.extend([c.a as u32, c.b as u32, c.key, u32::from(c.age) | u32::from(c.distance) << 16]);
        }
        cases += 1;
        if got != o {
            let first = got.iter().zip(&o).position(|(g, e)| g != e).unwrap_or(0);
            failures.push(format!("case {cases}: word {first} got {:08x} want {:08x}", got.get(first).unwrap_or(&0), o[first]));
        }
    }
    (cases, failures)
}

/// 140fd5320: one batch of constraints for bodies with AngularMotion
/// (`spin`: orientation, angular velocity). The contacts sit half the
/// overlap distance along the normal from each centre; the tangential
/// impulse also turns both bodies with an inertia of 10 / reach².
#[allow(clippy::too_many_arguments)]
pub fn solve_angular(
    dt: f32,
    radius: f32,
    material: &SelfMaterial,
    constraints: &mut [Constraint],
    order: &[i32],
    states: &mut [[u16; 2]],
    linear: &mut [[[f32; 4]; 2]],
    spin: &mut [[[f32; 4]; 2]],
) {
    let inverse_dt = reciprocal(dt);
    let friction = clamp_material(material.friction);
    let stickiness = clamp_material(material.stickiness);
    let restitution = clamp_material(material.restitution);
    let threshold = clamp_material(material.restitution_threshold);
    let delay = (add(multiply(inverse_dt, material.deactivation_delay), 0.5) as i32).min(0x7ffe);
    // (b x a) of a pair, as 140fd5320 shuffles it: [a.z b.y - b.z a.y, ...].
    let turn = |a: [f32; 4], b: [f32; 4]| -> [f32; 4] {
        [
            subtract(multiply(a[2], b[1]), multiply(b[2], a[1])),
            subtract(multiply(a[0], b[2]), multiply(b[0], a[2])),
            subtract(multiply(a[1], b[0]), multiply(b[1], a[0])),
            subtract(multiply(a[3], b[3]), multiply(b[3], a[3])),
        ]
    };
    for &index in order {
        let c = &mut constraints[index as usize];
        let (ia, ib) = (c.a as usize, c.b as usize);
        let (state_a, state_b) = (states[ia][0], states[ib][0]);
        let (asleep_a, asleep_b) = (state_a & 1 != 0, state_b & 1 != 0);
        if asleep_a && asleep_b {
            c.age = 0;
            c.distance = 0;
            continue;
        }
        let [pa, va] = linear[ia];
        let [pb, vb] = linear[ib];
        let wa = spin[ia][1];
        let wb = spin[ib][1];
        let previous = f32::from_bits(((c.distance as i16 as i32) << 16) as u32);
        let reach = add(multiply(vb[3], radius), multiply(va[3], radius));
        let reach2 = multiply(reach, reach);
        let inertia = multiply(reciprocal(reach2), 10.);
        let d: [f32; 3] = std::array::from_fn(|k| subtract(pb[k], pa[k]));
        let squared = add(add(multiply(d[1], d[1]), multiply(d[0], d[0])), multiply(d[2], d[2]));
        let m = minps(maxps(minps(squared, reach2), previous), reach2);
        c.distance = (m.to_bits() >> 16) as u16;
        let half = if m <= 0. {
            0.
        } else {
            let e = rsqrt_estimate(m);
            multiply(multiply(subtract(3., multiply(multiply(e, m), e)), multiply(e, 0.5)), m)
        };
        let half = multiply(half, 0.5);
        let inverse = inverse_length(squared);
        let n: [f32; 3] = std::array::from_fn(|k| multiply(d[k], inverse));
        let w = subtract(
            subtract(0., add(add(multiply(pa[1], n[1]), multiply(pa[0], n[0])), multiply(pa[2], n[2]))),
            half,
        );
        let plane = [n[0], n[1], n[2], w];
        let share_b = divide(1., f32::from(state_b >> 3));
        let share_a = divide(1., f32::from(state_a >> 3));
        let kb = if asleep_b { 0. } else { share_b };
        let ka = if asleep_a { 0. } else { share_a };
        let s = reciprocal(add(kb, ka));
        let (skb, ska) = (multiply(s, kb), multiply(s, ka));
        let r: [f32; 4] = plane.map(|x| multiply(half, x));
        let negative_r = [-r[0], -r[1], -r[2], r[3]];
        let at_b = turn(negative_r, wb);
        let at_a = turn(r, wa);
        let dv: [f32; 4] = std::array::from_fn(|k| subtract(add(at_b[k], vb[k]), add(at_a[k], va[k])));
        let vn = add(add(multiply(dv[1], n[1]), multiply(dv[0], n[0])), multiply(dv[2], n[2]));
        let pn: [f32; 3] = std::array::from_fn(|k| multiply(pb[k], n[k]));
        let separation = subtract(add(add(pn[1], w), add(pn[0], pn[2])), half);
        let stick = maxps(0., if vn < stickiness { vn } else { stickiness });
        let ahead = add(multiply(vn, dt), separation);
        let push = minps(0., multiply(ahead, 0.5));
        let bounce = maxps(0., subtract(multiply(subtract(0., vn), restitution), multiply(threshold, dt)));
        let raw = add(
            subtract(multiply(subtract(0., push), inverse_dt), if ahead < 0. { 0. } else { stick }),
            if separation < 0. { 0. } else { bounce },
        );
        let impulse = maxps(0., raw);
        let mut tangent: [f32; 4] =
            std::array::from_fn(|k| multiply(subtract(dv[k], multiply(vn, plane[k])), subtract(0., TANGENT)));
        let cone = multiply(impulse, friction);
        let t2 = add(add(multiply(tangent[1], tangent[1]), multiply(tangent[0], tangent[0])), multiply(tangent[2], tangent[2]));
        if multiply(cone, cone) < t2 {
            let scale = inverse_length(t2);
            tangent = tangent.map(|t| multiply(multiply(t, scale), cone));
        }
        let awake = i32::from(c.age) > delay
            && add(add(multiply(tangent[1], tangent[1]), multiply(tangent[0], tangent[0])), multiply(tangent[2], tangent[2])) > 0.;
        let j: [f32; 4] = std::array::from_fn(|k| add(multiply(raw, plane[k]), tangent[k]));
        let wake = |state: u16| {
            let count = (((state >> 1) & 3) + 1).min(3);
            ((count * 2) ^ state) & 6 ^ state
        };
        // B: velocity, state, then spin from the tangent at its contact.
        for k in 0..3 {
            linear[ib][1][k] = add(multiply(skb, j[k]), vb[k]);
        }
        if awake {
            states[ib][0] = wake(state_b);
        }
        let coefficient_b = multiply(multiply(skb, inertia), share_b);
        let torque_b = turn(tangent, negative_r);
        spin[ib][1] = std::array::from_fn(|k| add(wb[k], multiply(coefficient_b, torque_b[k])));
        for k in 0..3 {
            linear[ia][1][k] = add(va[k], multiply(subtract(0., ska), j[k]));
        }
        if awake {
            states[ia][0] = wake(state_a);
        }
        let coefficient_a = multiply(multiply(subtract(0., inertia), ska), share_a);
        let torque_a = turn(tangent, r);
        spin[ia][1] = std::array::from_fn(|k| add(wa[k], multiply(coefficient_a, torque_a[k])));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn solvers_match_executed_original() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/self_contact_cases.txt")).unwrap();
        for angular in [false, true] {
            let (cases, failures) = super::compare_fixture(&text, angular);
            assert!(cases > 0 && failures.is_empty(), "angular {angular}: {} of {cases}: {:?}", failures.len(), &failures[..failures.len().min(3)]);
        }
    }
}
