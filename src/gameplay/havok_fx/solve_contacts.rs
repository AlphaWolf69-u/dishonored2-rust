//! hkfxEffects::SolveSphereContacts (constructor 140fb9ae0 over SolveContacts
//! 140fb9510; per-body slot 19 140fb9c20): the velocity response of sphere
//! bodies to WorldCollide's static and dynamic contacts, with resting,
//! friction and sleep. Transcribed from the disassembly.
use super::world_collide::{Contact, WorldCollide};
use crate::float::{add, divide, multiply, subtract};
use crate::gameplay::constraint_solver::{inverse_length, reciprocal};

const BIG: f32 = f32::from_bits(0x5f7f_fff0);
const ZERO: [f32; 4] = [0.; 4];

/// The 12-byte per-body record (+0x50): contact state, sleep timer, flags.
/// States: 0 free, 1 bounced, 2 sliding, 3+ resting (counting up), 0x7f
/// asleep on a moving surface, 0x80 asleep on a still one.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct BodyState {
    pub state: i32,
    pub timer: f32,
    pub flags: u8,
}

pub struct SolveContacts {
    /// +0x28..+0x38, written each post-step by 1401e1d10.
    pub friction: f32,
    pub restitution: f32,
    pub stickiness: f32,
    pub restitution_threshold: f32,
    pub friction_threshold: f32,
    /// +0x3c steps to fall asleep, +0x40 sleeping enabled.
    pub sleep_steps: i32,
    pub sleep: bool,
    pub bodies: Vec<BodyState>,
}

fn clamp(value: f32) -> f32 {
    // MAXPS(0, value < big ? value : big).
    let value = if value < BIG { value } else { BIG };
    if 0. > value { 0. } else { value }
}

fn dot3(a: [f32; 4], b: [f32; 4]) -> f32 {
    add(add(multiply(a[1], b[1]), multiply(a[0], b[0])), multiply(a[2], b[2]))
}

/// The plane's signed distance of `p`: n.xyz . p.xyz + n.w, summed with
/// the shuffles of 140fba03c.
fn plane_distance(n: [f32; 4], p: [f32; 4]) -> f32 {
    let t = [multiply(n[0], p[0]), multiply(n[1], p[1]), multiply(n[2], p[2]), n[3]];
    add(add(t[3], t[1]), add(t[2], t[0]))
}

impl SolveContacts {
    /// 140fb9510 and slot 6 (140fb9580).
    pub fn new(capacity: usize) -> Self {
        Self {
            friction: 0.,
            restitution: 0.,
            stickiness: 0.,
            restitution_threshold: 12.,
            friction_threshold: 12.,
            sleep_steps: 4,
            sleep: true,
            bodies: vec![BodyState::default(); capacity],
        }
    }

    /// Slot 8 (140fb96f0).
    pub fn allocate(&mut self, bodies: &[i32]) {
        for &body in bodies {
            self.bodies[body as usize] = BodyState::default();
        }
    }

    /// 140fb9c20 over one chunk. `linear` holds LinearMotion's channels and
    /// `radius` is the pipeline radius (+0x30).
    pub fn solve_spheres(
        &mut self,
        dt: f32,
        radius: f32,
        bodies: &[i32],
        collide: &WorldCollide,
        linear: &mut [[[f32; 4]; 2]],
    ) -> Result<(), String> {
        let dt = clamp(dt);
        let inverse_dt = if dt == 0. { 0. } else { reciprocal(dt) };
        let stickiness = clamp(self.stickiness);
        let restitution_threshold = clamp(multiply(self.restitution_threshold, dt));
        let friction_threshold = clamp(multiply(multiply(self.friction_threshold, dt), self.friction));
        let friction_threshold = multiply(friction_threshold, friction_threshold);
        let steps = self.sleep_steps as f32;
        for &body in bodies {
            let index = body as usize;
            if self.bodies[index].flags & 1 != 0 {
                continue;
            }
            let body_radius = multiply(linear[index][1][3], radius);
            // Dynamic first, then static (140fb9e77..140fb9fbb).
            let mut contacts: Vec<(Contact, f32, f32)> = Vec::with_capacity(2);
            for (contact, material) in [
                (&collide.dynamic_contacts[index], collide.dynamic_materials[index]),
                (&collide.static_contacts[index], collide.static_materials[index]),
            ] {
                if !(0. > material[0]) {
                    contacts.push((*contact, clamp(material[0]), clamp(material[1])));
                }
            }
            match contacts.len() {
                0 => {
                    let state = &mut self.bodies[index];
                    if 0x7f <= state.state {
                        state.state = 2;
                        state.timer = 0.;
                    }
                }
                1 => {
                    let (contact, friction, restitution) = contacts[0];
                    self.one_contact(
                        index,
                        contact,
                        friction,
                        restitution,
                        body_radius,
                        [dt, inverse_dt, stickiness, restitution_threshold, friction_threshold, steps],
                        &mut linear[index],
                    );
                }
                _ => return Err("Havok sphere contacts with both a static and a dynamic contact are not ported".into()),
            }
        }
        Ok(())
    }

    /// 140fb9fce..140fba56a.
    #[allow(clippy::too_many_arguments)]
    fn one_contact(
        &mut self,
        index: usize,
        contact: Contact,
        friction: f32,
        restitution: f32,
        radius: f32,
        [dt, inverse_dt, stickiness, restitution_threshold, friction_threshold, steps]: [f32; 6],
        body: &mut [[f32; 4]; 2],
    ) {
        let n = contact.plane;
        let old_state = self.bodies[index].state;
        let mut stay_awake = !self.sleep;
        // The surface velocity at the body: lv + w x (p - com).
        let surface = if n.iter().all(|&lane| lane == 0.) {
            ZERO
        } else {
            let d: [f32; 4] = std::array::from_fn(|i| subtract(body[0][i], contact.centre_of_mass[i]));
            let w = contact.angular_velocity;
            let a = [multiply(w[1], d[0]), multiply(w[2], d[1]), multiply(w[0], d[2]), multiply(w[3], d[3])];
            let b = [multiply(d[1], w[0]), multiply(d[2], w[1]), multiply(d[0], w[2]), multiply(d[3], w[3])];
            let c = [subtract(b[0], a[0]), subtract(b[1], a[1]), subtract(b[2], a[2]), subtract(b[3], a[3])];
            let cross = [c[1], c[2], c[0], c[3]];
            std::array::from_fn(|i| add(cross[i], contact.linear_velocity[i]))
        };
        let surface_still = surface[..3].iter().all(|&lane| lane == 0.);
        let relative: [f32; 4] = std::array::from_fn(|i| subtract(body[1][i], surface[i]));
        let gap = subtract(plane_distance(n, body[0]), radius);
        let normal_speed = dot3(n, relative);
        let state = &mut self.bodies[index];
        if 0x7f <= old_state {
            let tangent: [f32; 4] = std::array::from_fn(|i| subtract(relative[i], multiply(n[i], normal_speed)));
            let squares = tangent.map(|lane| multiply(lane, lane));
            let sliding = add(add(squares[1], squares[0]), squares[2]);
            if 0. <= normal_speed || friction_threshold < sliding {
                // 140ffaaf0 notifies the listener; the body wakes.
                stay_awake = true;
            }
            if 0x7f <= state.state && !stay_awake {
                body[1] = [surface[0], surface[1], surface[2], body[1][3]];
                state.timer = 0.;
                state.state = 0x7f + i32::from(surface_still);
                return;
            }
        }
        state.state = 0;
        let velocity = body[1];
        let relative: [f32; 4] = std::array::from_fn(|i| subtract(velocity[i], surface[i]));
        let normal_speed = dot3(n, relative);
        if !(normal_speed >= stickiness) && !(add(multiply(normal_speed, dt), gap) > multiply(stickiness, dt)) {
            let bounce = add(multiply(restitution, normal_speed), restitution_threshold);
            // MINPS(0, x) keeps x unless 0 < x.
            let bounce = if 0. < bounce { 0. } else { bounce };
            if !(bounce >= 0.) {
                let push = subtract(0., add(bounce, normal_speed));
                body[1] = [
                    add(multiply(push, n[0]), velocity[0]),
                    add(multiply(push, n[1]), velocity[1]),
                    add(multiply(push, n[2]), velocity[2]),
                    add(multiply(push, 0.), velocity[3]),
                ];
                state.state = 1;
            } else {
                let separation = multiply(gap, inverse_dt);
                let separation = if 0. > separation { 0. } else { separation };
                let budget = multiply(subtract(stickiness, normal_speed), friction);
                let correction = subtract(subtract(0., normal_speed), separation);
                let mut tangent: [f32; 4] = std::array::from_fn(|i| subtract(relative[i], multiply(n[i], normal_speed)));
                let squares = tangent.map(|lane| multiply(lane, lane));
                let sliding = add(add(squares[1], squares[0]), squares[2]);
                let slides = sliding > multiply(budget, budget);
                if slides {
                    let inverse = inverse_length(sliding);
                    tangent = tangent.map(|lane| multiply(multiply(lane, inverse), budget));
                }
                let delta = [
                    subtract(multiply(correction, n[0]), tangent[0]),
                    subtract(multiply(correction, n[1]), tangent[1]),
                    subtract(multiply(correction, n[2]), tangent[2]),
                    0.,
                ];
                body[1] = std::array::from_fn(|i| add(velocity[i], delta[i]));
                if delta[..3].iter().any(|&lane| lane != 0.) {
                    state.state = if slides { 2 } else { 3 };
                }
            }
        }
        // 140fba306: the sleep timer.
        let relative: [f32; 4] = std::array::from_fn(|i| subtract(body[1][i], surface[i]));
        let penetration = if 0. < gap { 0. } else { gap };
        let mut asleep = false;
        if self.sleep {
            let normal_speed = dot3(n, relative);
            let squares = relative.map(|lane| multiply(lane, lane));
            let speed = add(add(squares[1], squares[0]), squares[2]);
            let pushed = multiply(penetration, inverse_dt);
            if multiply(pushed, pushed) > speed {
                state.timer = add(divide(1., steps), state.timer);
            } else if normal_speed > add(friction_threshold, stickiness) {
                let timer = subtract(state.timer, divide(f32::from_bits(0x3d7a_e148), steps));
                state.timer = if 0. > timer { 0. } else { timer };
            }
            asleep = state.timer >= 1.;
        }
        if penetration < 0. {
            if asleep {
                body[1] = [surface[0], surface[1], surface[2], body[1][3]];
                state.state = 0x7f;
            } else {
                let depth = add(multiply(0., radius), penetration);
                let depth = if 0. < depth { 0. } else { depth };
                let p = body[0];
                body[0] = [
                    subtract(p[0], multiply(depth, n[0])),
                    subtract(p[1], multiply(depth, n[1])),
                    subtract(p[2], multiply(depth, n[2])),
                    p[3],
                ];
            }
        }
        // 140fba49d: a body above the plane, moving into it and about to
        // pass below it takes the surface velocity.
        if plane_distance(n, body[0]) >= 0. && dot3(body[1], n) < 0. {
            let next: [f32; 4] = std::array::from_fn(|i| add(multiply(body[1][i], dt), body[0][i]));
            if plane_distance(n, next) < 0. {
                body[1] = [surface[0], surface[1], surface[2], body[1][3]];
            }
        }
        if !stay_awake && state.state == 3 && 3 <= old_state {
            state.state = old_state + 1;
        }
    }
}
