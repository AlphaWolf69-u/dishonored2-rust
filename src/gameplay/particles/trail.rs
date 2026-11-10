//! Per-particle trails (arkParticleTrailRenderer records, 0x168 bytes, one
//! per particle slot). Start 1402117d0 -> 14020dd80, advance 140211bc0 ->
//! 14020dea0 -> 14020cfc0. The trail follows its particle: only live
//! particles' trails are built (1401f3060) and a respawn restarts the record.
//! Basis arguments are the stage owner's row-major 3x3 (stage +0x1c).
use super::pose::EmitterPose;
use crate::{
    assets::fx::particle_trail::ParticleTrailAsset,
    float::{add, divide, multiply, subtract},
    gameplay::reference_frame::basis_quaternion,
};

/// 14020ba40's trail-life motion sample: acceleration (stage space unless
/// world), gravity, velocity scale and linear acceleration, all at the
/// particle's age. Indices 8..10 are 14020cfc0's zero vector.
#[derive(Clone, Copy)]
pub struct TrailMotion {
    acceleration: [f32; 3],
    gravity: f32,
    velocity_scale: [f32; 3],
    linear: f32,
}

pub struct TrailRecord {
    pub(crate) life: f32,
    pub(crate) seed: u32,
    location: [f32; 3],
    pub(crate) colour: [f32; 4],
    pub(crate) width: f32,
    pub(crate) flips: [bool; 2],
    pose: EmitterPose,
    last_point_time: f32,
    emitting: bool,
    interpolation_distance: f32,
    interpolation_squared: f32,
    pub(crate) times: Vec<f32>,
    pub(crate) positions: Vec<[f32; 3]>,
    base_velocities: Vec<[f32; 3]>,
    velocities: Vec<[f32; 3]>,
    pub(crate) orientations: Vec<[f32; 3]>,
    pub(crate) time: f32,
    pub(crate) age: f32,
    pub(crate) frame: f32,
    point_seed: u32,
    pub(crate) length: f32,
    pub(crate) segments: i32,
    pub(crate) bounds: [[f32; 3]; 2],
}

const EMPTY_BOUNDS: [[f32; 3]; 2] = [[f32::from_bits(0x7149_f2ca); 3], [f32::from_bits(0xf149_f2ca); 3]];

/// The row-major basis product used by 14020d300, 140209070 and 14020ba40:
/// x' = y*m3 + x*m0 + z*m6, y' = x*m1 + y*m4 + z*m7, z' = x*m2 + y*m5 + z*m8.
fn rotate(v: [f32; 3], m: &[[f32; 3]; 3]) -> [f32; 3] {
    [
        add(add(multiply(v[1], m[1][0]), multiply(v[0], m[0][0])), multiply(v[2], m[2][0])),
        add(add(multiply(v[0], m[0][1]), multiply(v[1], m[1][1])), multiply(v[2], m[2][1])),
        add(add(multiply(v[0], m[0][2]), multiply(v[1], m[1][2])), multiply(v[2], m[2][2])),
    ]
}

fn squared(d: [f32; 3]) -> f32 {
    add(add(multiply(d[1], d[1]), multiply(d[0], d[0])), multiply(d[2], d[2]))
}

/// 140204d70's binary search over the point times.
fn search(times: &[f32], time: f32) -> usize {
    let (mut base, mut count, mut above) = (0usize, times.len(), 0usize);
    if count != 0 {
        loop {
            let half = count >> 1;
            let value = times[base + half];
            if time == value {
                return base + half;
            }
            if value < time {
                base += half;
            }
            above = usize::from(value < time);
            count -= half;
            if half == 0 {
                break;
            }
        }
    }
    above + base
}

/// Catmull-Rom control point `k` of the point positions, extrapolating the
/// first or last pair outside the list (14020dff0/14020fb80/1401d1500).
pub(crate) fn control_point(points: &[[f32; 3]], k: i64) -> [f32; 3] {
    let n = points.len() as i64;
    if k < 0 {
        let (a, b) = (points[0], points[1]);
        std::array::from_fn(|i| add(a[i], multiply(subtract(b[i], a[i]), k as f32)))
    } else if k >= n {
        let (a, b) = (points[n as usize - 1], points[n as usize - 2]);
        let steps = (k - (n - 1)) as f32;
        std::array::from_fn(|i| add(a[i], multiply(subtract(a[i], b[i]), steps)))
    } else {
        points[k as usize]
    }
}

impl TrailRecord {
    /// 14020dd80 (with 14020dd00, 14020dc10 and 14020bec0). `seed` is the
    /// particle's spawn seed chain; the record keeps a copy before drawing.
    /// 14020dd80 does not reset +0x13c, so the first point's offset is
    /// sampled at the slot's previous age (`previous_age`, zero when new).
    pub fn start(
        previous_age: f32,
        trail: &ParticleTrailAsset,
        fraction: f32,
        position: [f32; 3],
        basis: [[f32; 3]; 3],
        seed: &mut u32,
    ) -> Result<Self, String> {
        let copy = *seed;
        let life = trail.life.sample(fraction, seed)?;
        let flips = [
            trail.flipbook.flips[0].enabled((crate::random::lcg_step(seed) >> 19) & 1 != 0),
            trail.flipbook.flips[1].enabled((crate::random::lcg_step(seed) >> 18) & 1 != 0),
        ];
        let spawn = &trail.spawn;
        let location = spawn.location.value.sample(fraction, seed)?;
        let tint = spawn.colour.sample(fraction, seed)?;
        let alpha = spawn.alpha.sample(fraction, seed)?;
        let width = spawn.width.sample(fraction, seed)?;
        let base = spawn.base_colour;
        let colour = [
            multiply(tint[0], base[0]),
            multiply(tint[1], base[1]),
            multiply(tint[2], base[2]),
            multiply(alpha, base[3]),
        ];
        let distance = spawn.interpolation_distance;
        let (interpolation_distance, interpolation_squared) = if distance <= 0. {
            (f32::MAX, f32::MAX)
        } else {
            (distance, multiply(distance, distance))
        };
        let mut pose = EmitterPose::new(crate::gameplay::reference_frame::ReferenceFrame { position, basis });
        pose.set_position(position);
        pose.set_orientation(basis_quaternion(basis));
        pose.commit_orientation();
        let frame = if trail.flipbook.cycle_rate { trail.flipbook.initial.sample(seed) } else { 0. };
        let point_seed = crate::random::lcg_step(seed);
        let mut record = Self {
            life,
            seed: copy,
            location,
            colour,
            width,
            flips,
            pose,
            last_point_time: 0.,
            emitting: true,
            interpolation_distance,
            interpolation_squared,
            times: Vec::new(),
            positions: Vec::new(),
            base_velocities: Vec::new(),
            velocities: Vec::new(),
            orientations: Vec::new(),
            time: 0.,
            age: previous_age,
            frame,
            point_seed,
            length: 0.,
            segments: 0,
            bounds: EMPTY_BOUNDS,
        };
        let point = record.initial_point(trail, position, &basis)?;
        record.add_point(trail, fraction, 0., point, &basis)?;
        Ok(record)
    }

    /// 14020dea0 -> 14020cfc0.
    pub fn advance(
        &mut self,
        trail: &ParticleTrailAsset,
        age: f32,
        fraction: f32,
        delta: f32,
        position: [f32; 3],
        basis: [[f32; 3]; 3],
    ) -> Result<(), String> {
        self.age = age;
        self.time = add(delta, self.time);
        // 14020eb00
        if !trail.flipbook.cycle_rate {
            self.frame = self.age;
        } else {
            let mut seed = self.seed;
            self.frame = add(multiply(trail.flipbook.rate.sample(self.age, &mut seed)?, delta), self.frame);
        }
        self.prune();
        let motion = self.motion(trail, &basis)?;
        for point in 0..self.times.len() {
            self.move_point(trail, &motion, point, &basis, delta)?;
        }
        if !self.emitting {
            self.pose.set_position(position);
            self.pose.commit_orientation();
            self.last_point_time = self.time;
        } else {
            self.emit(trail, fraction, position, basis, &motion)?;
        }
        self.measure();
        // 14020d6f0
        if self.length < f32::from_bits(0x38d1_b717) {
            self.length = 0.;
            self.segments = 0;
        } else if trail.tessellation.length_scaled {
            let count = add(multiply(self.length, trail.tessellation.segments), 0.5).floor() as i32;
            self.segments = count.max(1).min(trail.tessellation.maximum_segments);
        } else {
            self.segments = add(trail.tessellation.segments, 0.5).floor() as i32;
        }
        Ok(())
    }

    /// 14020d300: the spawn location (stage-rotated unless world space),
    /// the emitter position and m_offsetOverLife at the particle's age.
    fn initial_point(&mut self, trail: &ParticleTrailAsset, position: [f32; 3], basis: &[[f32; 3]; 3]) -> Result<[f32; 3], String> {
        let mut point = self.location;
        if !trail.spawn.location.worldspace {
            point = rotate(point, basis);
        }
        point = std::array::from_fn(|i| add(point[i], position[i]));
        let offset = trail.offset_over_life.value.sample(self.age, &mut self.point_seed)?;
        let offset = if trail.offset_over_life.worldspace { offset } else { rotate(offset, basis) };
        Ok(std::array::from_fn(|i| add(point[i], offset[i])))
    }

    /// 140209070.
    fn add_point(&mut self, trail: &ParticleTrailAsset, fraction: f32, time: f32, point: [f32; 3], basis: &[[f32; 3]; 3]) -> Result<(), String> {
        self.times.push(time);
        self.positions.push(point);
        let velocity = trail.spawn.velocity.value.sample(fraction, &mut self.point_seed)?;
        self.base_velocities.push(if trail.spawn.velocity.worldspace { velocity } else { rotate(velocity, basis) });
        self.velocities.push([0.; 3]);
        if trail.vector_oriented {
            let orientation = trail.orientation_vector.sample(self.age, &mut self.point_seed)?;
            self.orientations.push(rotate(orientation, basis));
        }
        Ok(())
    }

    /// 14020d7c0: once the oldest point is 1.5 lives old, drop the points
    /// older than one life, keeping the last one before it.
    fn prune(&mut self) {
        let count = self.times.len();
        if count <= 3 || !(f32::from_bits(0x3fc0_0000) < divide(subtract(self.time, self.times[0]), self.life)) {
            return;
        }
        let first = search(&self.times, subtract(self.time, self.life)) as i64 - 1;
        if first > 0 {
            let first = first as usize;
            self.times.drain(..first);
            self.positions.drain(..first);
            self.base_velocities.drain(..first);
            self.velocities.drain(..first);
            if !self.orientations.is_empty() {
                self.orientations.drain(..first);
            }
        }
    }

    /// 14020ba40.
    fn motion(&self, trail: &ParticleTrailAsset, basis: &[[f32; 3]; 3]) -> Result<TrailMotion, String> {
        let curves = &trail.trail_life;
        let mut seed = self.seed;
        let mut acceleration = curves.acceleration.value.sample(self.age, &mut seed)?;
        if !curves.acceleration.worldspace {
            acceleration = rotate(acceleration, basis);
        }
        let gravity = curves.acceleration.gravity.sample(self.age, &mut seed)?;
        let velocity_scale = curves.velocity_over_life.sample(self.age, &mut seed)?;
        let linear = curves.acceleration.linear.sample(self.age, &mut seed)?;
        Ok(TrailMotion { acceleration, gravity, velocity_scale, linear })
    }

    /// 14020c620. The gravity term samples the trail-life gravity at the
    /// segment age; the segment-life gravity is never read (original).
    fn move_point(&mut self, trail: &ParticleTrailAsset, motion: &TrailMotion, point: usize, basis: &[[f32; 3]; 3], delta: f32) -> Result<(), String> {
        let mut seed = self.seed;
        let age = divide(subtract(self.time, self.times[point]), self.life);
        let segment = &trail.segment_life;
        let mut a = segment.acceleration.value.sample(age, &mut seed)?;
        if !segment.acceleration.worldspace {
            let m = basis;
            let z = add(add(multiply(a[0], m[0][2]), multiply(a[1], m[1][2])), multiply(a[2], m[2][2]));
            let x = add(add(multiply(a[1], m[1][0]), multiply(a[0], m[0][0])), multiply(a[2], m[2][0]));
            let y = add(add(multiply(a[0], m[0][1]), multiply(a[1], m[1][1])), multiply(a[2], m[2][1]));
            a = [x, y, z];
        }
        let gravity = multiply(trail.trail_life.acceleration.gravity.sample(age, &mut seed)?, motion.gravity);
        let (down, flat) = (multiply(gravity, f32::from_bits(0xc11c_cccd)), multiply(gravity, 0.));
        let v = &mut self.velocities[point];
        v[0] = add(multiply(add(multiply(a[0], motion.acceleration[0]), flat), delta), v[0]);
        v[1] = add(multiply(add(multiply(a[1], motion.acceleration[1]), flat), delta), v[1]);
        v[2] = add(multiply(add(multiply(a[2], motion.acceleration[2]), down), delta), v[2]);
        let base = self.base_velocities[point];
        let scale = segment.velocity_over_life.sample(age, &mut seed)?;
        let velocity: [f32; 3] = std::array::from_fn(|i| {
            add(add(multiply(multiply(base[i], motion.velocity_scale[i]), scale[i]), self.velocities[point][i]), 0.)
        });
        let linear = segment.acceleration.linear.sample(age, &mut seed)?;
        let inverse = divide(1., squared(velocity).max(f32::MIN_POSITIVE).sqrt());
        for i in 0..3 {
            self.velocities[point][i] = add(
                multiply(multiply(multiply(multiply(linear, velocity[i]), inverse), motion.linear), delta),
                self.velocities[point][i],
            );
            self.positions[point][i] = add(multiply(velocity[i], delta), self.positions[point][i]);
        }
        Ok(())
    }

    /// 140209de0: add a point once the emitter moved m_minimalEmitterMove
    /// and the new point is m_minimalParticleDistance from the last one,
    /// first inserting interpolated points every m_interpolationDistance.
    fn emit(&mut self, trail: &ParticleTrailAsset, fraction: f32, position: [f32; 3], basis: [[f32; 3]; 3], motion: &TrailMotion) -> Result<(), String> {
        let start = self.pose.start();
        let moved = squared(std::array::from_fn(|i| subtract(position[i], start[i])));
        let spawn = &trail.spawn;
        if !(multiply(spawn.minimal_emitter_move, spawn.minimal_emitter_move) <= moved) {
            return Ok(());
        }
        let point = self.initial_point(trail, position, &basis)?;
        let minimum = multiply(spawn.minimal_particle_distance, spawn.minimal_particle_distance);
        let last = *self.positions.last().ok_or("trail record has no points")?;
        if !(minimum <= squared(std::array::from_fn(|i| subtract(point[i], last[i])))) {
            return Ok(());
        }
        self.pose.set_orientation(basis_quaternion(basis));
        if self.interpolation_squared <= moved && moved != self.interpolation_squared {
            let mut steps = divide(moved.sqrt(), self.interpolation_distance).floor() as i32;
            let elapsed = subtract(self.time, self.last_point_time);
            let step = divide(elapsed, (steps + 1) as f32);
            self.pose.target_position(position);
            self.pose.set_duration(elapsed);
            self.pose.reset_fraction();
            let mut time = self.last_point_time;
            loop {
                time = add(time, step);
                self.pose.advance(step);
                let sampled = self.pose.sample();
                let between = self.initial_point(trail, sampled.position, &sampled.basis)?;
                let last = *self.positions.last().ok_or("trail record has no points")?;
                let from_last = squared(std::array::from_fn(|i| subtract(between[i], last[i])));
                let to_point = squared(std::array::from_fn(|i| subtract(between[i], point[i])));
                let nearest = if to_point <= from_last { to_point } else { from_last };
                if minimum < nearest {
                    self.add_point(trail, fraction, time, between, &sampled.basis)?;
                    let index = self.times.len() - 1;
                    self.move_point(trail, motion, index, &sampled.basis, subtract(self.time, time))?;
                }
                steps -= 1;
                if steps == 0 {
                    break;
                }
            }
        }
        let time = self.time;
        self.add_point(trail, fraction, time, point, &basis)?;
        self.pose.set_position(position);
        self.pose.commit_orientation();
        self.last_point_time = self.time;
        Ok(())
    }

    /// 14020dff0: trail length and bounds over 2n-3 Catmull-Rom samples.
    fn measure(&mut self) {
        self.length = 0.;
        self.bounds = EMPTY_BOUNDS;
        let n = self.times.len();
        if n == 0 {
            return;
        }
        let include = |bounds: &mut [[f32; 3]; 2], p: [f32; 3]| {
            for i in 0..3 {
                if p[i] <= bounds[0][i] { bounds[0][i] = p[i]; }
                if bounds[1][i] <= p[i] { bounds[1][i] = p[i]; }
            }
        };
        let mut previous = self.positions[0];
        include(&mut self.bounds, previous);
        let distance = |a: [f32; 3], b: [f32; 3]| {
            let d = squared(std::array::from_fn(|i| subtract(b[i], a[i])));
            multiply(divide(1., d.max(f32::MIN_POSITIVE).sqrt()), d)
        };
        let samples = 2 * n as i32 - 3;
        if samples > 0 {
            let mut time = subtract(self.time, self.life);
            if time <= self.times[0] {
                time = self.times[0];
            }
            let span = subtract(self.times[n - 1], time);
            for _ in 0..samples {
                let p = self.centre(time);
                time = add(time, divide(span, (2 * n as i32 - 1) as f32));
                self.length = add(distance(previous, p), self.length);
                include(&mut self.bounds, p);
                previous = p;
            }
        }
        let last = self.positions[n - 1];
        self.length = add(distance(previous, last), self.length);
        include(&mut self.bounds, last);
    }

    /// 14020dff0's inline centre-line sample at `time`.
    fn centre(&self, time: f32) -> [f32; 3] {
        let n = self.times.len();
        if n == 1 || time <= self.times[0] {
            return self.positions[0];
        }
        if self.times[n - 1] <= time {
            return self.positions[n - 1];
        }
        let mut index = 0;
        while index + 1 < n && !(time <= self.times[index + 1]) {
            index += 1;
        }
        let [start, end] = crate::gameplay::effects::curve::curve_key_times(&self.times, index as i32);
        let mut t = subtract(time, start);
        if f32::from_bits(0x3400_0000) < subtract(end, start) {
            t = divide(t, subtract(end, start));
        }
        let half = multiply(t, 0.5);
        let w3 = multiply(multiply(subtract(t, 1.), t), half);
        let w0 = multiply(subtract(multiply(subtract(2., t), t), 1.), half);
        let w1 = add(multiply(multiply(subtract(multiply(t, 3.), 5.), t), half), 1.);
        let w2 = multiply(add(multiply(subtract(4., multiply(t, 3.)), t), 1.), half);
        let i = index as i64;
        let [p0, p1, p2, p3] = [i - 1, i, i + 1, i + 2].map(|k| control_point(&self.positions, k));
        std::array::from_fn(|c| {
            add(add(add(multiply(p1[c], w1), multiply(p0[c], w0)), multiply(p2[c], w2)), multiply(p3[c], w3))
        })
    }
}
