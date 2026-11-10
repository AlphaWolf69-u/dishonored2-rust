//! Particle ribbons (arkRibbon, system list +0x39c, 0x170 bytes): one
//! chain of emitted points per emitter, drawn as one strip. Constructor
//! 1402042c0, start 1402075c0 (delay)/140207570 (clock), frame update
//! 1402007b0 (1402086f0 points, 140208270 point step, 140207d20 emission,
//! 140207f70 spawn), strip 140207230 (140204df0 curve, 1402054c0 length,
//! 140206ed0 segments, 140206420 vertices).
use super::{
    beam::{control_point4, control_scalar},
    beam_noise::SimplexTable,
    spawn::{spray, SpawnVelocity},
    timing::CycleClock,
    trail::{centre_and_tangent, half, rotate, search, unorm, vertex_pair, StripFrame, StripMapping, TrailVertex},
};
use crate::{
    assets::fx::{
        particle_declaration::{ParticleDeclaration, ParticleWind},
        particle_ribbon::ParticleRibbonAsset,
    },
    float::{add, divide, multiply, subtract},
    gameplay::reference_frame::ReferenceFrame,
    random::{lcg_step, lcg_unit},
};

const EPSILON: f32 = f32::from_bits(0x3400_0000);

fn inverse_length(squared: f32) -> f32 {
    divide(1., squared.max(f32::MIN_POSITIVE).sqrt())
}

fn squared_yxz(v: [f32; 3]) -> f32 {
    add(add(multiply(v[1], v[1]), multiply(v[0], v[0])), multiply(v[2], v[2]))
}

/// A ribbon point (0x34 bytes).
#[derive(Clone, Copy)]
struct RibbonPoint {
    position: [f32; 3],
    /// +0xc: normalized age.
    age: f32,
    /// +0x10: spawn velocity.
    base: [f32; 3],
    /// +0x1c: velocity accumulated from accelerations.
    accumulated: [f32; 3],
    /// +0x28: the last step's velocity.
    velocity: [f32; 3],
}

pub struct Ribbon {
    /// +0x20 and +0x24.
    seed: u32,
    cycle_seed: u32,
    /// +0x2c: 1402089e0's wind force for this frame.
    wind: [f32; 3],
    /// +0x38: 140208970's sampled velocity inheritance.
    inherited: SpawnVelocity,
    /// +0x60: the emission step limit.
    step_limit: f32,
    /// +0x64: 1 / the ribbon life sampled at construction.
    inverse_life: f32,
    /// +0x68: time since the last spawn, FLT_MAX until the first.
    since_spawn: f32,
    clock: CycleClock,
    /// +0x80: remaining start delay.
    delay: f32,
    /// +0x94 flipbook frame, +0x98 time, +0x9c flips.
    frame: f32,
    time: f32,
    flips: [bool; 2],
    points: Vec<RibbonPoint>,
}

impl Ribbon {
    /// 1402042c0, then the system start's 1402075c0 and 140207570.
    pub fn new(asset: &ParticleRibbonAsset, system_seed: &mut u32) -> Result<Self, String> {
        let seed = lcg_step(system_seed);
        let mut cycle_seed = lcg_step(system_seed);
        let flips = [
            asset.flipbook.flips[0].enabled((lcg_step(system_seed) >> 19) & 1 != 0),
            asset.flipbook.flips[1].enabled((lcg_step(system_seed) >> 18) & 1 != 0),
        ];
        // 1401f9380: frames advance at the flipbook rate only with CYCLE_RATE.
        let frame = if asset.flipbook.cycle_rate { asset.flipbook.initial.sample(system_seed) } else { 0. };
        let life = crate::gameplay::effects::parameters::sample_range(asset.timings.life, system_seed);
        // 1401d0f00: the largest rate curve key, at least 0.001.
        let rate = &asset.timings.rate;
        let mut peak = f32::from_bits(0x3a83_126f);
        for curve in [&rate.curve.minimum, &rate.curve.maximum] {
            let triples = !curve.times.is_empty() && curve.values.len() == curve.times.len() * 3;
            for key in 0..curve.times.len() {
                let value = curve.values[if triples { key * 3 + 1 } else { key }];
                if peak <= value {
                    peak = value;
                }
            }
        }
        let [first, second] = rate.range;
        let largest = if first <= second { second } else { first };
        let mut step_limit = f32::from_bits(0x3c88_8889);
        let limit = divide(1., multiply(multiply(peak, largest), 2.));
        if limit < step_limit {
            step_limit = limit;
        }
        let _ = seed;
        let delay = crate::gameplay::effects::timing::sample_range(asset.timings.delay, &mut cycle_seed);
        let clock = CycleClock::new(asset.timings.duration, &mut cycle_seed);
        let cycle_seed = lcg_step(&mut cycle_seed);
        Ok(Self {
            seed: cycle_seed,
            cycle_seed,
            wind: [0.; 3],
            inherited: SpawnVelocity::default(),
            step_limit,
            inverse_life: divide(1., life),
            since_spawn: f32::MAX,
            clock,
            delay,
            frame,
            time: 0.,
            flips,
            points: Vec::new(),
        })
    }

    /// 1401d9c20 with its callers' restart reseed.
    fn advance_clock(&mut self, asset: &ParticleRibbonAsset, delta: f32) -> Result<(), String> {
        self.clock.advance(delta, asset.timings.cycles, asset.timings.duration, &mut self.cycle_seed)?;
        if self.clock.restarted {
            self.cycle_seed = lcg_step(&mut self.cycle_seed);
            self.seed = self.cycle_seed;
        }
        Ok(())
    }

    /// 140208270: one point step. Every point draws from the same copy of
    /// the emitter seed.
    fn step(&self, asset: &ParticleRibbonAsset, wind: &ParticleWind, point: &mut RibbonPoint, basis: &[[f32; 3]; 3], delta: f32) -> Result<(), String> {
        let mut seed = self.seed;
        let age = add(multiply(delta, self.inverse_life), point.age);
        point.age = age;
        let v = point.velocity;
        let inverse = inverse_length(squared_yxz(v));
        let linear = asset.linear_acceleration.sample(age, &mut seed)?;
        for i in [2, 1, 0] {
            point.accumulated[i] = add(multiply(multiply(multiply(linear, v[i]), inverse), delta), point.accumulated[i]);
        }
        let mut a = asset.acceleration.value.sample(age, &mut seed)?;
        if !asset.acceleration.worldspace {
            a = rotate(a, basis);
        }
        let gravity = asset.acceleration.gravity.sample(age, &mut seed)?;
        let (down, flat) = (multiply(gravity, f32::from_bits(0xc11c_cccd)), multiply(gravity, 0.));
        point.accumulated[0] = add(multiply(add(a[0], flat), delta), point.accumulated[0]);
        point.accumulated[1] = add(multiply(add(a[1], flat), delta), point.accumulated[1]);
        point.accumulated[2] = add(multiply(add(a[2], down), delta), point.accumulated[2]);
        let scale = asset.velocity_over_life.sample(age, &mut seed)?;
        let strength = wind.over_life.sample(age, &mut seed)?;
        for i in 0..3 {
            let velocity = add(multiply(scale[i], point.base[i]), point.accumulated[i]);
            point.velocity[i] = add(multiply(strength, self.wind[i]), velocity);
            point.position[i] = add(multiply(point.velocity[i], delta), point.position[i]);
        }
        Ok(())
    }

    /// 1402086f0.
    fn update_points(&mut self, asset: &ParticleRibbonAsset, wind: &ParticleWind, basis: &[[f32; 3]; 3], delta: f32) -> Result<(), String> {
        self.time = add(delta, self.time);
        if asset.flipbook.cycle_rate {
            let mut seed = self.seed;
            self.frame = add(multiply(asset.flipbook.rate.sample(self.clock.fraction, &mut seed)?, delta), self.frame);
        } else {
            self.frame = self.clock.fraction;
        }
        let mut points = std::mem::take(&mut self.points);
        for point in &mut points {
            self.step(asset, wind, point, basis, delta)?;
        }
        // Points past their life are dropped, keeping the last one before
        // the first live point.
        let n = points.len();
        let dead = (0..n)
            .filter(|&i| 1. < points[i].age && (n == 1 || (i + 1 < n && 1. <= points[i + 1].age)))
            .count();
        points.drain(..dead);
        self.points = points;
        Ok(())
    }

    /// 140207f70.
    fn spawn(&mut self, asset: &ParticleRibbonAsset, wind: &ParticleWind, system: ReferenceFrame, delta: f32) -> Result<(), String> {
        let spawn = &asset.spawn;
        let fraction = self.clock.fraction;
        // 140204080 -> 1401f6cf0: local vectors turn by the system basis.
        let vector = |vector: &crate::assets::fx::particle_stage::SpawnVector, seed: &mut u32| -> Result<[f32; 3], String> {
            let value = vector.value.sample(fraction, seed)?;
            Ok(if vector.worldspace { value } else { rotate(value, &system.basis) })
        };
        let mut position = vector(&spawn.location, &mut self.cycle_seed)?;
        let velocity = vector(&spawn.velocity, &mut self.cycle_seed)?;
        let angle = multiply(spawn.spray_cone.sample(fraction, &mut self.cycle_seed)?, 0.5);
        let velocity = spray(velocity, angle, &mut self.cycle_seed);
        if asset.worldspace {
            let turned = rotate(position, &system.basis);
            position = std::array::from_fn(|i| add(turned[i], system.position[i]));
        }
        let inherited = self.inherited.at_point(position);
        let base = std::array::from_fn(|i| add(inherited[i], velocity[i]));
        let mut point = RibbonPoint { position, age: 0., base, accumulated: [0.; 3], velocity: base };
        self.step(asset, wind, &mut point, &system.basis, delta)?;
        self.points.push(point);
        Ok(())
    }

    /// 140207d20: emission in steps of at most +0x60.
    fn emit(&mut self, asset: &ParticleRibbonAsset, wind: &ParticleWind, system: ReferenceFrame, mut delta: f32, scale: f32) -> Result<(), String> {
        while 0. < delta {
            if !self.clock.can_emit(asset.timings.cycles) {
                break;
            }
            let mut seed = self.seed;
            let rate = multiply(asset.timings.rate.sample(self.clock.fraction, &mut seed)?, scale);
            if f32::from_bits(0x3c23_d70a) <= rate && 0. <= subtract(self.since_spawn, divide(1., rate)) {
                self.since_spawn = 0.;
                self.spawn(asset, wind, system, delta)?;
            }
            let step = if self.step_limit <= delta { self.step_limit } else { delta };
            self.since_spawn = add(step, self.since_spawn);
            self.advance_clock(asset, step)?;
            delta = subtract(delta, step);
        }
        if 0. < delta {
            self.advance_clock(asset, delta)?;
        }
        Ok(())
    }

    /// 1402007b0 for one frame. `delta` already includes the first frame's
    /// warmup; `rate` is the system's emission multiplier (+0x1d90).
    #[allow(clippy::too_many_arguments)]
    pub fn advance(
        &mut self,
        asset: &ParticleRibbonAsset,
        declaration: &ParticleDeclaration,
        delta: f32,
        system: ReferenceFrame,
        world_wind: [f32; 3],
        source: Option<&SpawnVelocity>,
        rate: f32,
        simulating: bool,
    ) -> Result<(), String> {
        // 1402063f0
        let consumed = if delta <= self.delay { delta } else { self.delay };
        self.delay = subtract(self.delay, consumed);
        let mut remaining = subtract(delta, consumed);
        if !(EPSILON < remaining) {
            return Ok(());
        }
        if !simulating {
            // 140208910
            return self.advance_clock(asset, remaining);
        }
        if !(self.delay < EPSILON) {
            return Ok(());
        }
        // 1402089e0
        self.wind = if asset.enable_wind {
            let mut seed = self.seed;
            let strength = declaration.wind.over_cycle.sample(self.clock.fraction, &mut seed)?;
            let w = world_wind.map(|value| multiply(strength, value));
            if asset.worldspace {
                w
            } else {
                let b = system.basis;
                std::array::from_fn(|i| add(add(multiply(w[1], b[i][1]), multiply(w[0], b[i][0])), multiply(w[2], b[i][2])))
            }
        } else {
            [0.; 3]
        };
        // 140208970
        if declaration.velocity_inheritance.enabled && asset.enable_velocity_inheritance {
            let source = source.ok_or("ribbon velocity inheritance awaits the owning entity's published motion")?;
            let mut seed = self.seed;
            self.inherited = SpawnVelocity::sample(source, &declaration.velocity_inheritance, self.clock.fraction, &mut seed)?;
            if !asset.worldspace {
                self.inherited.make_local(system);
            }
        }
        let wind = &declaration.wind;
        while 0. < remaining {
            let mut step = remaining.min(f32::from_bits(0x3c88_8889));
            let cycle = multiply(self.clock.duration, f32::from_bits(0x3d23_d70a));
            if cycle <= step {
                step = cycle;
            }
            let left = subtract(self.clock.duration, self.clock.elapsed);
            if f32::from_bits(0x3727_c5ac) < left && left <= step {
                step = left;
            }
            self.update_points(asset, wind, &system.basis, step)?;
            self.emit(asset, wind, system, step, rate)?;
            remaining = subtract(remaining, step);
        }
        Ok(())
    }

    /// 140207230: build the point curve and its strip. 140204950 grows the
    /// strip bounds on the stack only; the emitter bounds (+0x140) stay
    /// empty, so ribbons never contribute to the system bounds (original).
    pub fn build(
        &self,
        asset: &ParticleRibbonAsset,
        eye: [f32; 3],
        tint: [f32; 4],
        owner: ReferenceFrame,
        noise: Option<&SimplexTable>,
        vertices: &mut Vec<TrailVertex>,
    ) -> Result<(), String> {
        // 140204df0
        let fraction = self.clock.fraction;
        let mut seed = self.seed;
        let cycle = &asset.cycle_life;
        let width = cycle.width.sample(fraction, &mut seed)?;
        let base = asset.base_colour;
        let c = cycle.colour.sample(fraction, &mut seed)?;
        let rgb: [f32; 3] = std::array::from_fn(|i| multiply(multiply(base[i], tint[i]), c[i]));
        let alpha = multiply(multiply(base[3], tint[3]), cycle.alpha.sample(fraction, &mut seed)?);
        let mut times = Vec::with_capacity(self.points.len() + 1);
        let mut positions = Vec::with_capacity(self.points.len() + 1);
        let mut colours = Vec::with_capacity(self.points.len() + 1);
        let mut widths = Vec::with_capacity(self.points.len() + 1);
        let segment = &asset.segment_life;
        let mut key = |time: f32, position: [f32; 3]| -> Result<(), String> {
            times.push(time);
            positions.push(position);
            let mut s = seed;
            widths.push(multiply(segment.width.sample(time, &mut s)?, width));
            let c = segment.colour.sample(time, &mut s)?;
            let a = segment.alpha.sample(time, &mut s)?;
            colours.push([multiply(c[0], rgb[0]), multiply(c[1], rgb[1]), multiply(c[2], rgb[2]), multiply(a, alpha)]);
            Ok(())
        };
        if asset.section.start_from_origin {
            key(0., if asset.worldspace { owner.position } else { [0.; 3] })?;
        }
        for point in self.points.iter().rev() {
            key(point.age, point.position)?;
        }
        let n = times.len();
        if n == 0 {
            return Ok(());
        }
        let start = if times[0] <= asset.section.minimum_life { asset.section.minimum_life } else { times[0] };
        let end = if asset.section.maximum_life <= times[n - 1] { asset.section.maximum_life } else { times[n - 1] };
        let span = subtract(end, start);
        // 1402054c0
        let mut length = 0_f32;
        if 2 <= n {
            let step = divide(span, (2 * n - 1) as f32);
            let mut previous = super::trail::curve_centre(&times, &positions, start);
            let mut time = add(step, start);
            for _ in 0..2 * n - 3 {
                let p = super::trail::curve_centre(&times, &positions, time);
                time = add(time, step);
                let d = squared_yxz(std::array::from_fn(|i| subtract(p[i], previous[i])));
                length = add(multiply(inverse_length(d), d), length);
                previous = p;
            }
        }
        // 140206ed0
        let tessellation = &asset.tessellation;
        let segments = if length < f32::from_bits(0x38d1_b717) {
            length = 0.;
            0
        } else if tessellation.length_scaled {
            let scaled = add(multiply(length, tessellation.segments), 0.5).floor() as i32;
            scaled.max(1).min(tessellation.maximum_segments)
        } else {
            add(tessellation.segments, 0.5).floor() as i32
        };
        if segments <= 0 {
            return Ok(());
        }
        // 140206420
        let mut mapping = StripMapping::new(&asset.mapping, length, segments as f32, self.flips);
        let mut random = self.cycle_seed;
        mapping.extra = [half(self.frame), half(lcg_unit(&mut random))];
        let settings = &asset.noise;
        let noise = if EPSILON < settings.scale {
            Some(noise.ok_or("ribbon noise table was not loaded")?)
        } else {
            None
        };
        let noise_x = multiply(settings.frequency, self.time);
        let mut noise_step = divide(1., segments as f32);
        if settings.by_length {
            noise_step = multiply(noise_step, length);
        }
        let mut running = 0_f32;
        let step = divide(span, segments as f32);
        let mut time = start;
        let mut index = search(&times, time);
        let b = owner.basis;
        for _ in 0..=segments {
            while index < n && !(time <= times[index]) {
                index += 1;
            }
            let interval = index as i64 - 1;
            let [first, last] = crate::gameplay::effects::curve::curve_key_times(&times, interval as i32);
            let mut t = subtract(time, first);
            if EPSILON < subtract(last, first) {
                t = divide(t, subtract(last, first));
            }
            let h = multiply(t, 0.5);
            let w3 = multiply(multiply(subtract(t, 1.), t), h);
            let w0 = multiply(subtract(multiply(subtract(2., t), t), 1.), h);
            let w2 = multiply(add(multiply(subtract(4., multiply(t, 3.)), t), 1.), h);
            let w1 = add(multiply(multiply(subtract(multiply(t, 3.), 5.), t), h), 1.);
            // 1401d1390 -> 1401d1e80
            let c = [interval - 1, interval, interval + 1, interval + 2].map(|k| control_point4(&colours, k));
            let packed = unorm(std::array::from_fn(|i| {
                add(add(add(multiply(c[3][i], w3), multiply(c[2][i], w2)), multiply(c[1][i], w1)), multiply(c[0][i], w0))
            }));
            // 1401d1c30
            let w = [interval - 1, interval, interval + 1, interval + 2].map(|k| control_scalar(&widths, k));
            let width = add(add(add(multiply(w[1], w1), multiply(w[0], w0)), multiply(w[2], w2)), multiply(w[3], w3));
            // 140207780
            let (mut centre, mut tangent) = centre_and_tangent(&times, &positions, time, interval);
            time = add(time, step);
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
            let mut frame = StripFrame::ribbon(centre, tangent, eye);
            if let Some(table) = noise {
                let value = table.octaves(settings.octaves, settings.persistence, settings.perlin_frequency, noise_x, running)?;
                running = add(running, noise_step);
                for i in 0..3 {
                    frame.centre[i] = add(frame.centre[i], multiply(multiply(multiply(width, frame.side[i]), value), settings.scale));
                }
            }
            vertices.extend(vertex_pair(&mapping, &frame, width, packed));
            mapping.advance();
        }
        Ok(())
    }

    /// 1402073f0: the live point count.
    pub fn live_count(&self) -> usize {
        self.points.len()
    }

    /// The clock's emission test (1401d9b70).
    pub fn can_emit(&self, asset: &ParticleRibbonAsset) -> bool {
        self.clock.can_emit(asset.timings.cycles)
    }
}
