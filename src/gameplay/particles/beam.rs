//! Particle beam records (0x198 bytes, emitter +0x38). Spawn 1401d5870
//! (1401d5180 colour, 1401d2a70 points, 1401d21b0 control points), update
//! 1401d5e00 (1401d47b0 reprojection, 1401d30f0 point acceleration,
//! 1401d52d0 -> 1401d6f70 length), strip 1401d32e0 (1401d5390 point colours
//! and widths, 1401d1390/1401d1c30 interpolation, 1401d4580 frame, 1401d5ce0
//! owner transform, 14010e2e0 noise).
//!
//! The point curve (+0) keys the start point, the control points and the end
//! point at k/(n+1). 1401d50c0 sizes its values like its keys, so the
//! curve's three-values-per-key layout the decompile also tests never
//! occurs for beams.
use super::{
    beam_noise::SimplexTable,
    space::ParticleSpace,
    spawn::SpawnVelocity,
    trail::{centre_and_tangent, rotate, unorm, vertex_pair, StripFrame, StripMapping, TrailVertex, EMPTY_BOUNDS},
};
use crate::{
    assets::fx::{
        particle_beam::{BeamPosition, ParticleBeamAsset},
        particle_declaration::ParticleWind,
        particle_stage::SpawnVector,
        particle_trail::TrailLifeCurves,
    },
    float::{add, divide, multiply, subtract},
    gameplay::reference_frame::{axis_angle_basis, basis_product, invert_basis, ReferenceFrame},
    random::{lcg_step, lcg_unit},
};

const EPSILON: f32 = f32::from_bits(0x3400_0000);
const IDENTITY: [[f32; 3]; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];

/// The FX target parameters (system +0x1d10, written by 140227b90 from the
/// FX owner's packet): a position and a basis. 1401fa780 initializes them
/// to the origin and identity. The packet's target point list (+0x70) is
/// empty in that state, so 1401ce790 builds no control-point anchors.
#[derive(Clone, Copy)]
pub struct BeamTarget {
    pub position: [f32; 3],
    pub basis: [[f32; 3]; 3],
}
impl Default for BeamTarget {
    fn default() -> Self {
        Self { position: [0.; 3], basis: IDENTITY }
    }
}

/// The beam emitter's frame (+0xb0), refreshed by 1401ceeb0 every frame.
pub struct BeamFrame {
    space: ParticleSpace,
    /// +0x48: the target's basis pair; local beams replace its local half
    /// with the target basis in stage space.
    target_space: ParticleSpace,
    /// +0x90 and +0x9c, in stage space for local beams.
    target_position: [f32; 3],
    target_basis: [[f32; 3]; 3],
}
impl BeamFrame {
    /// 1401ceeb0.
    pub fn new(asset: &ParticleBeamAsset, system: ReferenceFrame, target: &BeamTarget) -> Self {
        let worldspace = asset.worldspace;
        let space = ParticleSpace::new(system.basis, worldspace);
        let target_space = ParticleSpace::new(target.basis, worldspace);
        if worldspace {
            return Self { space, target_space, target_position: target.position, target_basis: target.basis };
        }
        let world = space.basis(true);
        let offset = std::array::from_fn(|i| subtract(target.position[i], system.position[i]));
        let target_basis = basis_product(target.basis, world);
        Self {
            target_space: ParticleSpace::from_bases(target_space.basis(true), target_basis),
            space,
            target_position: rotate(offset, &world),
            target_basis,
        }
    }
}

fn sample_vector(vector: &SpawnVector, space: &ParticleSpace, seed: &mut u32, time: f32) -> Result<[f32; 3], String> {
    Ok(space.transform(vector.value.sample(time, seed)?, vector.worldspace))
}

fn flush(value: f32) -> f32 {
    if value.abs() <= f32::from_bits(0x2193_92ef) { 0. } else { value }
}

fn signed_unit(seed: &mut u32) -> f32 {
    subtract(multiply(lcg_unit(seed), 2.), 1.)
}

fn inverse_length(squared: f32) -> f32 {
    divide(1., squared.max(f32::MIN_POSITIVE).sqrt())
}

/// x, y, z squared in the order the beam functions sum them (y, x, z).
fn squared_yxz(v: [f32; 3]) -> f32 {
    add(add(multiply(v[1], v[1]), multiply(v[0], v[0])), multiply(v[2], v[2]))
}

fn include(bounds: &mut [[f32; 3]; 2], p: [f32; 3]) {
    for i in 0..3 {
        if p[i] <= bounds[0][i] {
            bounds[0][i] = p[i];
        }
        if bounds[1][i] <= p[i] {
            bounds[1][i] = p[i];
        }
    }
}

pub struct BeamRecord {
    /// +0 point curve: key times and positions.
    times: Vec<f32>,
    positions: Vec<[f32; 3]>,
    /// +0x58 and +0x68.
    base_velocities: Vec<[f32; 3]>,
    velocities: Vec<[f32; 3]>,
    /// +0x78 and +0x88: each control point's time since its placement and
    /// its re-randomization period.
    timers: Vec<f32>,
    periods: Vec<f32>,
    /// +0x98 normalized age, +0x9c flipbook frame, +0xa0 time.
    age: f32,
    frame: f32,
    time: f32,
    /// +0xa4/+0xb0 and +0xd4/+0xe0: the system and target poses of the last
    /// update, their bases inverted.
    origin: [f32; 3],
    origin_inverse: [[f32; 3]; 3],
    target_origin: [f32; 3],
    target_inverse: [[f32; 3]; 3],
    /// +0x104/+0x110: the end points at the last reprojection.
    ends: [[f32; 3]; 2],
    /// +0x11c (1401d5180): colour, seed, 1/life, width, flips.
    colour: [f32; 4],
    seed: u32,
    inverse_life: f32,
    width: f32,
    flips: [bool; 2],
    /// +0x160 and +0x164.
    length: f32,
    pub segments: i32,
    /// +0x168: the measured points' bounds.
    length_bounds: [[f32; 3]; 2],
}

impl BeamRecord {
    /// 1401d5aa0.
    fn store_origins(&mut self, position: [f32; 3], basis: [[f32; 3]; 3], frame: &BeamFrame) {
        self.origin = position;
        self.origin_inverse = basis;
        invert_basis(&mut self.origin_inverse);
        self.target_origin = frame.target_position;
        self.target_inverse = frame.target_basis;
        invert_basis(&mut self.target_inverse);
    }

    /// 1401d0d60 without a spawn mesh (the loader refuses beam positions
    /// with one): the location and velocity samples.
    fn place_end(position: &BeamPosition, space: &ParticleSpace, seed: &mut u32, fraction: f32) -> Result<([f32; 3], [f32; 3]), String> {
        let location = sample_vector(&position.location, space, seed, fraction)?;
        let velocity = sample_vector(&position.velocity, space, seed, fraction)?;
        Ok((location, velocity))
    }

    /// 1401d21b0: (re)place control point `k` (1..=n). `place` draws a new
    /// position; otherwise only the period and base velocity are renewed.
    fn place_control(&mut self, asset: &ParticleBeamAsset, k: usize, fraction: f32, frame: &BeamFrame, seed: &mut u32, place: bool) -> Result<(), String> {
        let cp = &asset.control_points[k - 1];
        self.timers[k - 1] = 0.;
        let [maximum, minimum] = cp.randomize_period;
        self.periods[k - 1] = if 0. < minimum && 0. < maximum {
            // 140001050(min, max, f) = (1 - f) min + max f.
            crate::gameplay::effects::parameters::sample_range([minimum, maximum], seed)
        } else {
            0.
        };
        let last = self.positions.len() - 1;
        let t = self.times[k];
        // No target point list supplies anchors (see BeamTarget).
        let first = self.positions[0].map(flush);
        let end = self.positions[last].map(flush);
        let inverse_t = subtract(1., t);
        let anchor: [f32; 3] = std::array::from_fn(|i| add(multiply(first[i], inverse_t), multiply(end[i], t)));
        let direction = if place {
            let x = signed_unit(seed);
            let y = signed_unit(seed);
            let z = signed_unit(seed);
            [x, y, z]
        } else {
            std::array::from_fn(|i| subtract(self.positions[k][i], anchor[i]))
        };
        let inverse = inverse_length(squared_yxz(direction));
        if place {
            let offset = sample_vector(&cp.initial_position, &frame.space, seed, fraction)?;
            let radius = cp.radius.sample(fraction, seed)?;
            self.positions[k] = std::array::from_fn(|i| {
                add(add(anchor[i], offset[i]), multiply(multiply(direction[i], inverse), radius))
            });
        }
        if cp.radial_constrained {
            let e = [signed_unit(seed), signed_unit(seed), signed_unit(seed)];
            let s = add(add(multiply(e[0], e[0]), multiply(e[1], e[1])), multiply(e[2], e[2]));
            let unit = inverse_length(s);
            let radius = cp.radius.sample(fraction, seed)?;
            let p = self.positions[k];
            let mut base = [
                subtract(add(multiply(multiply(radius, e[0]), unit), anchor[0]), p[0]),
                subtract(add(multiply(multiply(e[1], unit), radius), anchor[1]), p[1]),
                subtract(add(multiply(multiply(e[2], unit), radius), anchor[2]), p[2]),
            ];
            let period = self.periods[k - 1];
            if period <= 0. {
                base = [0.; 3];
            } else {
                let scale = divide(1., period);
                base = base.map(|value| multiply(scale, value));
            }
            self.base_velocities[k] = base;
        } else {
            let speed = cp.radial_velocity.sample(fraction, seed)?;
            let mut base: [f32; 3] = std::array::from_fn(|i| multiply(multiply(direction[i], inverse), speed));
            let first = self.base_velocities[0].map(flush);
            let end = self.base_velocities[last].map(flush);
            let inherited = cp.inherited_velocity.sample(fraction, seed)?;
            for i in 0..3 {
                base[i] = add(multiply(add(multiply(first[i], inverse_t), multiply(end[i], t)), inherited), base[i]);
            }
            let initial = cp.initial_velocity.sample(fraction, seed)?;
            self.base_velocities[k] = std::array::from_fn(|i| add(base[i], initial[i]));
        }
        Ok(())
    }

    /// 1401d5870 (with 1401d5180 and 1401d2a70). `seed` is the emitter's
    /// cycle seed (+0x14); `position` the system position.
    pub fn spawn(
        asset: &ParticleBeamAsset,
        fraction: f32,
        position: [f32; 3],
        frame: &BeamFrame,
        inherited: &SpawnVelocity,
        seed: &mut u32,
    ) -> Result<Self, String> {
        // 1401d5180
        let record_seed = *seed;
        let life = asset.timings.life.sample(fraction, seed)?;
        let base = asset.base_colour;
        let tint = asset.colour.sample(fraction, seed)?;
        let alpha = asset.alpha.sample(fraction, seed)?;
        let colour = [multiply(tint[0], base[0]), multiply(tint[1], base[1]), multiply(tint[2], base[2]), multiply(alpha, base[3])];
        let width = asset.width.sample(fraction, seed)?;
        let flips = [
            asset.flipbook.flips[0].enabled((lcg_step(seed) >> 19) & 1 != 0),
            asset.flipbook.flips[1].enabled((lcg_step(seed) >> 18) & 1 != 0),
        ];
        // 1401d0fe0: frames advance at the flipbook rate only with CYCLE_RATE.
        let animation_frame = if asset.flipbook.cycle_rate { asset.flipbook.initial.sample(seed) } else { 0. };
        // 1401d2a70
        let n = asset.control_points.len();
        let count = n + 2;
        let step = divide(1., (n + 1) as f32);
        let mut times = Vec::with_capacity(count);
        let mut time = 0_f32;
        for _ in 0..count {
            times.push(time);
            time = add(time, step);
        }
        let mut record = Self {
            times,
            positions: vec![[0.; 3]; count],
            base_velocities: vec![[0.; 3]; count],
            velocities: vec![[0.; 3]; count],
            timers: vec![0.; n],
            periods: vec![0.; n],
            age: 0.,
            frame: animation_frame,
            time: 0.,
            origin: [0.; 3],
            origin_inverse: IDENTITY,
            target_origin: [0.; 3],
            target_inverse: IDENTITY,
            ends: [[0.; 3]; 2],
            colour,
            seed: record_seed,
            inverse_life: divide(1., life),
            width,
            flips,
            length: 0.,
            segments: 0,
            length_bounds: EMPTY_BOUNDS,
        };
        let (mut start, start_velocity) = Self::place_end(&asset.start, &frame.space, seed, fraction)?;
        if asset.worldspace {
            start = std::array::from_fn(|i| add(position[i], start[i]));
        }
        record.positions[0] = start;
        record.base_velocities[0] = start_velocity;
        let last = count - 1;
        let (mut end, end_velocity) = if asset.end.use_target_parameters {
            let space = if asset.end.use_target_axis { &frame.target_space } else { &frame.space };
            let (end, velocity) = Self::place_end(&asset.end, space, seed, fraction)?;
            (std::array::from_fn(|i| add(frame.target_position[i], end[i])), velocity)
        } else {
            Self::place_end(&asset.end, &frame.space, seed, fraction)?
        };
        if !asset.end.use_target_parameters && asset.worldspace {
            end = std::array::from_fn(|i| add(position[i], end[i]));
        }
        record.positions[last] = end;
        record.base_velocities[last] = end_velocity;
        for k in 1..=n {
            record.place_control(asset, k, fraction, frame, seed, true)?;
        }
        for k in 0..count {
            let velocity = inherited.at_point(record.positions[k]);
            record.base_velocities[k] = std::array::from_fn(|i| add(velocity[i], record.base_velocities[k][i]));
        }
        record.store_origins(position, frame.space.basis(asset.worldspace), frame);
        if asset.reproject_control_points {
            record.ends = [record.positions[0], record.positions[last]];
        }
        record.measure(asset);
        Ok(record)
    }

    /// 1401d47b0: carry the interior points along when the end points
    /// moved, by the rotation between the old and new chords plus the
    /// projection onto the new chord scaled by the length ratio.
    fn reproject(&mut self) {
        let count = self.positions.len();
        let last = count - 1;
        if 2 < count {
            let moved = |p: [f32; 3], q: [f32; 3]| squared_yxz(std::array::from_fn(|i| subtract(p[i], q[i])));
            let (first, end) = (self.positions[0], self.positions[last]);
            if !(moved(first, self.ends[0]) <= EPSILON && moved(end, self.ends[1]) <= EPSILON) {
                let chord: [f32; 3] = std::array::from_fn(|i| subtract(end[i], first[i]));
                let inverse = inverse_length(squared_yxz(chord));
                let unit = chord.map(|c| multiply(c, inverse));
                let squared = squared_yxz(unit);
                let inverse = inverse_length(squared);
                let new = unit.map(|c| multiply(c, inverse));
                if EPSILON < multiply(inverse, squared) {
                    let old: [f32; 3] = std::array::from_fn(|i| subtract(self.ends[1][i], self.ends[0][i]));
                    let old_squared = squared_yxz(old);
                    let old_inverse = inverse_length(old_squared);
                    let o = old.map(|c| multiply(c, old_inverse));
                    let old_length = multiply(old_inverse, old_squared);
                    if f32::from_bits(0x38d1_b717) < old_length {
                        let dot = add(add(multiply(o[1], new[1]), multiply(o[0], new[0])), multiply(o[2], new[2]));
                        let angle = if -1. < dot {
                            if dot < 1. { (dot as f64).acos() as f32 } else { 0. }
                        } else {
                            std::f32::consts::PI
                        };
                        let m = if f32::from_bits(0x3a83_126f) < angle.abs() {
                            let axis = [
                                subtract(multiply(o[1], new[2]), multiply(o[2], new[1])),
                                subtract(multiply(o[2], new[0]), multiply(o[0], new[2])),
                                subtract(multiply(o[0], new[1]), multiply(o[1], new[0])),
                            ];
                            let inverse = inverse_length(squared_yxz(axis));
                            axis_angle_basis(axis.map(|c| multiply(c, inverse)), angle)
                        } else {
                            IDENTITY
                        };
                        let turn = |b: [f32; 3]| {
                            [
                                add(add(multiply(b[1], m[1][0]), multiply(b[0], m[0][0])), multiply(b[2], m[2][0])),
                                add(add(multiply(b[0], m[0][1]), multiply(b[1], m[1][1])), multiply(b[2], m[2][1])),
                                add(add(multiply(b[0], m[0][2]), multiply(b[1], m[1][2])), multiply(b[2], m[2][2])),
                            ]
                        };
                        for k in 1..last {
                            let rel: [f32; 3] = std::array::from_fn(|i| subtract(self.positions[k][i], self.ends[0][i]));
                            let projection = add(add(multiply(rel[1], o[1]), multiply(rel[0], o[0])), multiply(rel[2], o[2]));
                            let perp: [f32; 3] = std::array::from_fn(|i| subtract(rel[i], multiply(o[i], projection)));
                            let ratio = divide(projection, old_length);
                            let p0 = self.positions[0];
                            self.positions[k] = [
                                add(add(add(add(multiply(chord[0], ratio), p0[0]), multiply(m[1][0], perp[1])), multiply(m[0][0], perp[0])), multiply(m[2][0], perp[2])),
                                add(add(add(add(multiply(chord[1], ratio), p0[1]), multiply(m[0][1], perp[0])), multiply(m[1][1], perp[1])), multiply(m[2][1], perp[2])),
                                add(add(add(add(multiply(chord[2], ratio), p0[2]), multiply(m[1][2], perp[1])), multiply(m[0][2], perp[0])), multiply(m[2][2], perp[2])),
                            ];
                            self.base_velocities[k] = turn(self.base_velocities[k]);
                            self.velocities[k] = turn(self.velocities[k]);
                        }
                    }
                }
            }
        }
        self.ends = [self.positions[0], self.positions[last]];
    }

    /// 1401d30f0: a point's acceleration from its life curves plus the
    /// beam-life terms.
    #[allow(clippy::too_many_arguments)]
    fn acceleration(
        &self,
        life: &TrailLifeCurves,
        velocity: [f32; 3],
        beam: [f32; 3],
        beam_gravity: f32,
        beam_linear: f32,
        frame: &BeamFrame,
        seed: &mut u32,
    ) -> Result<[f32; 3], String> {
        let value = life.acceleration.value.sample(self.age, seed)?;
        let a = frame.space.transform(value, life.acceleration.worldspace);
        let gravity = multiply(life.acceleration.gravity.sample(self.age, seed)?, beam_gravity);
        let flat = multiply(gravity, 0.);
        let mut a = [
            add(add(flat, a[0]), beam[0]),
            add(add(flat, a[1]), beam[1]),
            add(add(multiply(gravity, f32::from_bits(0xc11c_cccd)), a[2]), beam[2]),
        ];
        let s = add(add(multiply(velocity[0], velocity[0]), multiply(velocity[1], velocity[1])), multiply(velocity[2], velocity[2]));
        let inverse = inverse_length(s);
        let linear = add(life.acceleration.linear.sample(self.age, seed)?, beam_linear);
        for i in 0..3 {
            a[i] = add(multiply(multiply(linear, velocity[i]), inverse), a[i]);
        }
        Ok(a)
    }

    /// The tracked end point: relative to the stored pose, turned by the
    /// current basis and moved to the current position.
    fn track(point: [f32; 3], origin: [f32; 3], inverse: &[[f32; 3]; 3], basis: &[[f32; 3]; 3], position: [f32; 3]) -> [f32; 3] {
        let relative = std::array::from_fn(|i| subtract(point[i], origin[i]));
        let turned = rotate(rotate(relative, inverse), basis);
        std::array::from_fn(|i| add(turned[i], position[i]))
    }

    /// 1401d5e00. Returns false to retire the record. `wind` is the
    /// emitter's wind force (+0x78), `seed` its cycle seed (+0x14).
    #[allow(clippy::too_many_arguments)]
    pub fn advance(
        &mut self,
        asset: &ParticleBeamAsset,
        wind_curves: &ParticleWind,
        wind: [f32; 3],
        position: [f32; 3],
        frame: &BeamFrame,
        seed: &mut u32,
        fraction: f32,
        delta: f32,
    ) -> Result<bool, String> {
        let mut local = self.seed;
        let space_basis = frame.space.basis(asset.worldspace);
        let age = add(multiply(delta, self.inverse_life), self.age);
        self.time = add(delta, self.time);
        self.age = age;
        if 1. < age {
            if !asset.timings.infinite_life {
                return Ok(false);
            }
            self.age = subtract(age, age.floor());
        }
        if asset.flipbook.cycle_rate {
            // 1401cfde0
            self.frame = add(multiply(asset.flipbook.rate.sample(self.age, &mut local)?, delta), self.frame);
        } else {
            self.frame = self.age;
        }
        let last = self.positions.len() - 1;
        if asset.start.track_origin {
            self.positions[0] = Self::track(self.positions[0], self.origin, &self.origin_inverse, &space_basis, position);
        }
        if asset.end.use_target_parameters && asset.end.track_target_parameters {
            self.positions[last] = Self::track(
                self.positions[last], self.target_origin, &self.target_inverse, &frame.target_basis, frame.target_position,
            );
        } else if asset.end.track_origin {
            self.positions[last] = Self::track(self.positions[last], self.origin, &self.origin_inverse, &space_basis, position);
        }
        self.store_origins(position, space_basis, frame);
        if asset.reproject_control_points {
            self.reproject();
        }
        for k in 0..asset.control_points.len() {
            self.timers[k] = add(delta, self.timers[k]);
            let period = self.periods[k];
            if 0. < period && period < self.timers[k] {
                self.place_control(asset, k + 1, fraction, frame, seed, asset.control_points[k].randomize_position)?;
            }
        }
        let beam_life = &asset.beam_life;
        let beam_value = beam_life.acceleration.value.sample(self.age, &mut local)?;
        let beam = frame.space.transform(beam_value, beam_life.acceleration.worldspace);
        let beam_gravity = beam_life.acceleration.gravity.sample(self.age, &mut local)?;
        let beam_linear = beam_life.acceleration.linear.sample(self.age, &mut local)?;
        let accelerate = |record: &mut Self, k: usize, life: &TrailLifeCurves, local: &mut u32| -> Result<(), String> {
            let a = record.acceleration(life, record.velocities[k], beam, beam_gravity, beam_linear, frame, local)?;
            record.velocities[k] = std::array::from_fn(|i| add(multiply(delta, a[i]), record.velocities[k][i]));
            Ok(())
        };
        accelerate(self, 0, &asset.start_life, &mut local)?;
        for (k, cp) in asset.control_points.iter().enumerate() {
            accelerate(self, k + 1, &cp.life, &mut local)?;
        }
        accelerate(self, last, &asset.end_life, &mut local)?;
        let strength = wind_curves.over_life.sample(self.age, &mut local)?;
        let wind = wind.map(|value| multiply(strength, value));
        let beam_scale = beam_life.velocity_over_life.sample(self.age, &mut local)?;
        let move_point = |record: &mut Self, k: usize, scale: [f32; 3], wind: [f32; 3]| {
            let (v, b) = (record.velocities[k], record.base_velocities[k]);
            record.positions[k] = std::array::from_fn(|i| {
                add(multiply(add(add(v[i], multiply(multiply(scale[i], beam_scale[i]), b[i])), wind[i]), delta), record.positions[k][i])
            });
        };
        let scale = asset.start_life.velocity_over_life.sample(self.age, &mut local)?;
        let start = (self.velocities[0], self.base_velocities[0]);
        self.positions[0] = std::array::from_fn(|i| {
            add(multiply(add(start.0[i], multiply(multiply(scale[i], beam_scale[i]), start.1[i])), delta), self.positions[0][i])
        });
        for (k, cp) in asset.control_points.iter().enumerate() {
            let scale = cp.life.velocity_over_life.sample(self.age, &mut local)?;
            move_point(self, k + 1, scale, wind);
        }
        let scale = asset.end_life.velocity_over_life.sample(self.age, &mut local)?;
        let end = (self.velocities[last], self.base_velocities[last]);
        self.positions[last] = std::array::from_fn(|i| {
            add(multiply(add(end.0[i], multiply(multiply(scale[i], beam_scale[i]), end.1[i])), delta), self.positions[last][i])
        });
        self.measure(asset);
        Ok(true)
    }

    /// 1401d52d0 after 1401d6f70.
    fn measure(&mut self, asset: &ParticleBeamAsset) {
        let n = self.positions.len();
        self.length_bounds = EMPTY_BOUNDS;
        include(&mut self.length_bounds, self.positions[0]);
        include(&mut self.length_bounds, self.positions[n - 1]);
        self.length = if n < 3 {
            if n == 2 {
                squared_yxz(std::array::from_fn(|i| subtract(self.positions[0][i], self.positions[1][i]))).sqrt()
            } else {
                0.
            }
        } else {
            let step = divide(1., (2 * n - 1) as f32);
            let mut time = step;
            let mut previous = self.positions[0];
            let mut length = 0_f32;
            let distance = |a: [f32; 3], b: [f32; 3]| {
                let d = squared_yxz(std::array::from_fn(|i| subtract(b[i], a[i])));
                multiply(inverse_length(d), d)
            };
            for _ in 0..2 * n - 3 {
                let p = super::trail::curve_centre(&self.times, &self.positions, time);
                time = add(time, step);
                include(&mut self.length_bounds, p);
                length = add(length, distance(previous, p));
                previous = p;
            }
            add(length, distance(previous, self.positions[n - 1]))
        };
        let tessellation = &asset.tessellation;
        if self.length < f32::from_bits(0x38d1_b717) {
            self.length = 0.;
            self.segments = 0;
        } else if tessellation.length_scaled {
            let scaled = add(multiply(self.length, tessellation.segments), 0.5).floor() as i32;
            self.segments = scaled.max(1).min(tessellation.maximum_segments);
        } else {
            self.segments = add(tessellation.segments, 0.5).floor() as i32;
        }
    }

    /// 1401d5390: the start, control and end point colours and widths at
    /// the record's age.
    fn point_appearance(&self, asset: &ParticleBeamAsset, colour: [f32; 4], width: f32, seed: &mut u32) -> Result<(Vec<[f32; 4]>, Vec<f32>), String> {
        let mut colours = Vec::with_capacity(self.positions.len());
        let mut widths = Vec::with_capacity(self.positions.len());
        let lives = std::iter::once(&asset.start_life)
            .chain(asset.control_points.iter().map(|cp| &cp.life))
            .chain(std::iter::once(&asset.end_life));
        for life in lives {
            let alpha = life.alpha.sample(self.age, seed)?;
            let c = life.colour.sample(self.age, seed)?;
            colours.push([
                multiply(c[0], colour[0]),
                multiply(c[1], colour[1]),
                multiply(c[2], colour[2]),
                multiply(alpha, colour[3]),
            ]);
            widths.push(multiply(life.width.sample(self.age, seed)?, width));
        }
        Ok((colours, widths))
    }

    /// 1401d32e0: append `segments + 1` vertex pairs. `joined` reserves a
    /// vertex before the strip for the join with an earlier one (filled
    /// with the first vertex), `more` appends a copy of the last vertex for
    /// the next. The original copies only the position into these
    /// degenerate vertices; the rewrite copies the whole vertex. Returns
    /// the strip bounds: the measured bounds grown by the widest point
    /// width x (1 + noise scale).
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        &self,
        asset: &ParticleBeamAsset,
        segments: u32,
        eye: [f32; 3],
        tint: [f32; 4],
        owner: ReferenceFrame,
        noise: Option<&SimplexTable>,
        joined: bool,
        more: bool,
        vertices: &mut Vec<TrailVertex>,
    ) -> Result<[[f32; 3]; 2], String> {
        if segments == 0 {
            return Ok(EMPTY_BOUNDS);
        }
        let step = divide(1., segments as f32);
        let mut mapping = StripMapping::new(&asset.mapping, self.length, segments as f32, self.flips);
        let settings = &asset.noise;
        let noise_x = multiply(settings.frequency, self.time);
        let noise = if EPSILON < settings.scale {
            Some(noise.ok_or("beam noise table was not loaded")?)
        } else {
            None
        };
        let noise_step = if settings.by_length { multiply(step, self.length) } else { step };
        let mut seed = self.seed;
        let life = &asset.beam_life;
        let alpha = life.alpha.sample(self.age, &mut seed)?;
        let c = life.colour.sample(self.age, &mut seed)?;
        let colour = [
            multiply(multiply(c[0], self.colour[0]), tint[0]),
            multiply(multiply(c[1], self.colour[1]), tint[1]),
            multiply(multiply(c[2], self.colour[2]), tint[2]),
            multiply(multiply(alpha, self.colour[3]), tint[3]),
        ];
        let width = multiply(life.width.sample(self.age, &mut seed)?, self.width);
        let (colours, widths) = self.point_appearance(asset, colour, width, &mut seed)?;
        let mut widest = 0_f32;
        for &w in &widths {
            if widest <= w.abs() {
                widest = w.abs();
            }
        }
        let widest = add(widest, multiply(widest, settings.scale));
        let mut bounds = self.length_bounds;
        for i in 0..3 {
            bounds[0][i] = subtract(bounds[0][i], widest);
            bounds[1][i] = add(widest, bounds[1][i]);
        }
        let mut random = self.seed;
        mapping.extra = [super::trail::half(self.frame), super::trail::half(lcg_unit(&mut random))];
        let reserved = joined.then(|| {
            vertices.push(TrailVertex::default());
            vertices.len() - 1
        });
        let n = self.times.len();
        let mut index = 0usize;
        let mut time = 0_f32;
        let mut running = 0_f32;
        let b = owner.basis;
        for _ in 0..=segments {
            while index < n && !(time <= self.times[index]) {
                index += 1;
            }
            let interval = index as i64 - 1;
            let [start, end] = crate::gameplay::effects::curve::curve_key_times(&self.times, interval as i32);
            let mut t = subtract(time, start);
            if EPSILON < subtract(end, start) {
                t = divide(t, subtract(end, start));
            }
            let h = multiply(t, 0.5);
            let w3 = multiply(multiply(subtract(t, 1.), t), h);
            let w0 = multiply(subtract(multiply(subtract(2., t), t), 1.), h);
            let w1 = add(multiply(multiply(subtract(multiply(t, 3.), 5.), t), h), 1.);
            let w2 = multiply(add(multiply(subtract(4., multiply(t, 3.)), t), 1.), h);
            let (mut centre, mut tangent) = centre_and_tangent(&self.times, &self.positions, time, interval);
            if !asset.worldspace {
                // 1401d5ce0
                let p = owner.position;
                centre = std::array::from_fn(|r| {
                    add(add(add(multiply(centre[0], b[0][r]), multiply(centre[1], b[1][r])), multiply(centre[2], b[2][r])), p[r])
                });
                tangent = std::array::from_fn(|r| {
                    add(add(multiply(tangent[0], b[0][r]), multiply(tangent[1], b[1][r])), multiply(tangent[2], b[2][r]))
                });
            }
            let mut frame = StripFrame::beam(centre, tangent, eye);
            time = add(time, step);
            // 1401d1390 -> 1401d1e80
            let c = [interval - 1, interval, interval + 1, interval + 2].map(|k| control_point4(&colours, k));
            let packed = unorm(std::array::from_fn(|i| {
                add(add(add(multiply(c[3][i], w3), multiply(c[2][i], w2)), multiply(c[1][i], w1)), multiply(c[0][i], w0))
            }));
            // 1401d1c30
            let w = [interval - 1, interval, interval + 1, interval + 2].map(|k| control_scalar(&widths, k));
            let width = add(add(add(multiply(w[1], w1), multiply(w[0], w0)), multiply(w[2], w2)), multiply(w[3], w3));
            if let Some(table) = noise {
                let value = table.octaves(settings.octaves, settings.persistence, settings.perlin_frequency, noise_x, running)?;
                running = add(running, noise_step);
                let offset = multiply(settings.scale, value);
                for i in 0..3 {
                    frame.centre[i] = add(frame.centre[i], multiply(offset, multiply(width, frame.side[i])));
                }
            }
            vertices.extend(vertex_pair(&mapping, &frame, width, packed));
            mapping.advance();
        }
        if let Some(reserved) = reserved {
            vertices[reserved] = vertices[reserved + 1];
        }
        if more {
            let last = *vertices.last().ok_or("beam strip has no vertices")?;
            vertices.push(last);
        }
        Ok(bounds)
    }
}

/// 1401d1e80/1401d1c30's control fetch: the list value at `k`, its first
/// or last pair extrapolated outside the list; one value repeats.
pub(crate) fn control_point4(values: &[[f32; 4]], k: i64) -> [f32; 4] {
    match values.len() {
        0 => [0.; 4],
        1 => values[0],
        n => {
            let n = n as i64;
            if k < 0 {
                let (a, b) = (values[0], values[1]);
                std::array::from_fn(|i| add(multiply(subtract(b[i], a[i]), k as f32), a[i]))
            } else if k < n {
                values[k as usize]
            } else {
                let (a, b) = (values[n as usize - 1], values[n as usize - 2]);
                std::array::from_fn(|i| add(multiply(subtract(a[i], b[i]), (k - n + 1) as f32), a[i]))
            }
        }
    }
}

pub(crate) fn control_scalar(values: &[f32], k: i64) -> f32 {
    match values.len() {
        0 => 0.,
        1 => values[0],
        n => {
            let n = n as i64;
            if k < 0 {
                add(multiply(subtract(values[1], values[0]), k as f32), values[0])
            } else if k < n {
                values[k as usize]
            } else {
                let (a, b) = (values[n as usize - 1], values[n as usize - 2]);
                add(multiply(subtract(a, b), (k - n + 1) as f32), a)
            }
        }
    }
}

