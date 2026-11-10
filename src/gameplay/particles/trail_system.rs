//! Declaration-level trails (arkTrail, system list +0x39a): an emitter with
//! its own cycle clock that spawns trail records over time at the system's
//! interpolated pose. Constructor 140211de0, reset 140212a10/1402129c0,
//! frame update 140200bf0, record update 140212c70, rate emission
//! 140212760, burst 1402120f0, strip 140212410.
use super::{
    pose::EmitterPose,
    timing::CycleClock,
    trail::{TrailBudget, TrailRecord, TrailTransform, TrailVertex},
};
use crate::{
    assets::fx::{particle_declaration::ParticleWind, particle_trail::DeclarationTrailAsset},
    float::{add, divide, multiply, subtract},
    gameplay::{effects::timing::sample_range, reference_frame::{basis_quaternion, ReferenceFrame}},
    random::lcg_step,
};

const EPSILON: u32 = 0x3400_0000;

pub struct DeclarationTrail {
    /// +0x10 and +0x14.
    seed: u32,
    cycle_seed: u32,
    /// +0x88: 140212e90's wind for this frame.
    wind: [f32; 3],
    clock: CycleClock,
    /// +0xd4: time since the last rate spawn, FLT_MAX until the first.
    since_spawn: f32,
    /// +0xd8: remaining start delay.
    delay: f32,
    records: Vec<Option<TrailRecord>>,
    live: Vec<bool>,
    /// +0x110, initialized descending so the first spawn uses slot 0.
    free: Vec<usize>,
    /// +0xbc (140212aa0).
    emitting: bool,
    pose: EmitterPose,
}

impl DeclarationTrail {
    /// 140211de0 draws two seeds from the system seed; the system start
    /// (1401fd7b0) then samples the delay (140212a10) and resets the clock
    /// (1402129c0) from the cycle seed.
    pub fn new(asset: &DeclarationTrailAsset, system_seed: &mut u32, pose: ReferenceFrame) -> Result<Self, String> {
        // +0x10 is overwritten by 1402129c0 below.
        lcg_step(system_seed);
        let mut cycle_seed = lcg_step(system_seed);
        let capacity = asset.capacity()?;
        let delay = sample_range(asset.emitter.delay, &mut cycle_seed);
        let clock = CycleClock::new(asset.emitter.duration, &mut cycle_seed);
        let cycle_seed = lcg_step(&mut cycle_seed);
        Ok(Self {
            seed: cycle_seed,
            cycle_seed,
            wind: [0.; 3],
            clock,
            since_spawn: f32::MAX,
            delay,
            records: (0..capacity).map(|_| None).collect(),
            live: vec![false; capacity],
            free: (0..capacity).rev().collect(),
            emitting: false,
            pose: EmitterPose::new(pose),
        })
    }

    /// 140200bf0's first-frame path: the pose starts at the system.
    pub fn reset_pose(&mut self, system: ReferenceFrame) {
        self.pose.set_position(system.position);
        self.pose.set_orientation(basis_quaternion(system.basis));
        self.pose.commit_orientation();
    }

    /// 140212de0: advance the clock only, reseeding at a cycle restart.
    fn advance_clock(&mut self, asset: &DeclarationTrailAsset, delta: f32) -> Result<(), String> {
        self.clock.advance(delta, asset.emitter.cycles, asset.emitter.duration, &mut self.cycle_seed)?;
        if self.clock.restarted {
            self.cycle_seed = lcg_step(&mut self.cycle_seed);
            self.seed = self.cycle_seed;
        }
        Ok(())
    }

    /// 1402121f0: the record's wind force from the declaration's
    /// wind-over-life at the record's age, on a copy of its seed.
    fn record_wind(&self, wind: &ParticleWind, record: &TrailRecord) -> Result<[f32; 3], String> {
        let mut seed = record.seed;
        let strength = wind.over_life.sample(record.age, &mut seed)?;
        Ok(self.wind.map(|value| multiply(strength, value)))
    }

    /// 140212ae0.
    fn spawn(&mut self, asset: &DeclarationTrailAsset, wind: &ParticleWind, position: [f32; 3], basis: [[f32; 3]; 3], delta: f32) -> Result<(), String> {
        let Some(&slot) = self.free.last() else { return Ok(()) };
        let previous_age = self.records[slot].as_ref().map_or(0., |record| record.age);
        let mut record = TrailRecord::start(previous_age, &asset.trail, Some(&asset.life), self.emitting,
            self.clock.fraction, position, basis, &mut self.cycle_seed)?;
        let force = self.record_wind(wind, &record)?;
        let alive = record.advance_declaration(&asset.trail, asset.infinite_life, self.clock.fraction, delta, position, basis, force)?;
        self.records[slot] = Some(record);
        if alive {
            self.free.pop();
            self.live[slot] = true;
        }
        Ok(())
    }

    /// 140212c70.
    fn update_records(&mut self, asset: &DeclarationTrailAsset, wind: &ParticleWind, position: [f32; 3], basis: [[f32; 3]; 3], delta: f32) -> Result<(), String> {
        for slot in 0..self.records.len() {
            if !self.live[slot] {
                continue;
            }
            let mut record = self.records[slot].take().ok_or("live trail record is absent")?;
            record.set_emitting(self.emitting);
            let force = self.record_wind(wind, &record)?;
            let alive = record.advance_declaration(&asset.trail, asset.infinite_life, self.clock.fraction, delta, position, basis, force)?;
            self.records[slot] = Some(record);
            if !alive {
                self.free.push(slot);
                self.live[slot] = false;
            }
        }
        Ok(())
    }

    /// 140212760: rate emission in 1/60 s steps, at most one spawn per step.
    fn emit(&mut self, asset: &DeclarationTrailAsset, wind: &ParticleWind, position: [f32; 3], basis: [[f32; 3]; 3], mut delta: f32, scale: f32) -> Result<(), String> {
        let step_limit = f32::from_bits(0x3c88_8889);
        while 0. < delta {
            if self.free.is_empty() || !self.clock.can_emit(asset.emitter.cycles) {
                break;
            }
            let rate = &asset.emitter.rate;
            let range_mix = crate::random::lcg_unit(&mut self.seed);
            let curve_mix = crate::random::lcg_unit(&mut self.seed);
            let [max, min] = rate.range.range;
            let rate = multiply(
                multiply(rate.curve.sample(self.clock.fraction, curve_mix)?, add(multiply(range_mix, subtract(max, min)), min)),
                scale,
            );
            if f32::from_bits(0x3dcc_cccd) <= rate && 0. <= subtract(self.since_spawn, divide(1., rate)) {
                self.since_spawn = 0.;
                self.spawn(asset, wind, position, basis, delta)?;
            }
            let step = if step_limit <= delta { step_limit } else { delta };
            self.since_spawn = add(step, self.since_spawn);
            self.advance_clock(asset, step)?;
            delta = subtract(delta, step);
        }
        if 0. < delta {
            self.advance_clock(asset, delta)?;
        }
        Ok(())
    }

    /// 1402120f0: a cycle restart spawns round(rate x scale) records.
    fn burst(&mut self, asset: &DeclarationTrailAsset, wind: &ParticleWind, position: [f32; 3], basis: [[f32; 3]; 3], delta: f32, scale: f32) -> Result<(), String> {
        if !self.clock.can_emit(asset.emitter.cycles) {
            return Ok(());
        }
        if self.clock.restarted {
            let mix = crate::random::lcg_unit(&mut self.cycle_seed);
            let [max, min] = asset.emitter.rate.range.range;
            let mut count = add(multiply(add(multiply(mix, subtract(max, min)), min), scale), 0.5).floor() as i32;
            while 0 < count && !self.free.is_empty() {
                self.spawn(asset, wind, position, basis, delta)?;
                count -= 1;
            }
        }
        self.advance_clock(asset, delta)
    }

    /// 140200bf0's trail part for one frame. `rate` is the system's
    /// emission multiplier (+0x1d90); `simulating` is its flag bit 0.
    #[allow(clippy::too_many_arguments)]
    pub fn advance(
        &mut self,
        asset: &DeclarationTrailAsset,
        wind: &ParticleWind,
        delta: f32,
        system: ReferenceFrame,
        world_wind: [f32; 3],
        rate: f32,
        simulating: bool,
    ) -> Result<(), String> {
        self.pose.set_orientation(basis_quaternion(system.basis));
        // 140212310
        let consumed = if delta <= self.delay { delta } else { self.delay };
        self.delay = subtract(self.delay, consumed);
        let mut remaining = subtract(delta, consumed);
        let epsilon = f32::from_bits(EPSILON);
        if epsilon < remaining {
            if !simulating {
                self.advance_clock(asset, remaining)?;
            } else if self.delay < epsilon {
                // 140212e90
                self.wind = if asset.enable_wind {
                    let mut seed = self.seed;
                    let strength = wind.over_cycle.sample(self.clock.fraction, &mut seed)?;
                    world_wind.map(|value| multiply(strength, value))
                } else {
                    [0.; 3]
                };
                self.emitting = epsilon < rate;
                self.pose.reset_fraction();
                self.pose.target_position(system.position);
                self.pose.set_duration(remaining);
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
                    self.pose.advance(step);
                    let sampled = self.pose.sample();
                    self.update_records(asset, wind, sampled.position, sampled.basis, step)?;
                    if asset.burst {
                        self.burst(asset, wind, sampled.position, sampled.basis, step, rate)?;
                    } else {
                        self.emit(asset, wind, sampled.position, sampled.basis, step, rate)?;
                    }
                    remaining = subtract(remaining, step);
                }
            }
        }
        self.pose.set_position(system.position);
        self.pose.commit_orientation();
        Ok(())
    }

    /// 140212410: the live records' strips within the renderer budget
    /// (min(1402134d0, r_prt_trailrenderer_budget x 4096), cvar default 1).
    pub fn build(
        &self,
        asset: &DeclarationTrailAsset,
        eye: [f32; 3],
        tint: [f32; 4],
        transform: Option<&TrailTransform>,
        vertices: &mut Vec<TrailVertex>,
    ) -> Result<[[f32; 3]; 2], String> {
        let mut bounds = [[f32::from_bits(0x7149_f2ca); 3], [f32::from_bits(0xf149_f2ca); 3]];
        let live: Vec<&TrailRecord> = (0..self.records.len())
            .filter(|&slot| self.live[slot])
            .filter_map(|slot| self.records[slot].as_ref())
            .collect();
        let limit = TrailBudget::limit(&asset.trail, self.records.len()).min(4096);
        let budget = TrailBudget::new(live.iter().map(|record| record.segments), limit);
        for record in live {
            let segments = budget.segments(record.segments);
            if segments != 0 {
                let strip = record.build(&asset.trail, segments, eye, tint, transform, vertices)?;
                crate::rendering::particles::bounds::include(&mut bounds, strip);
            }
        }
        Ok(bounds)
    }

    pub fn capacity(&self) -> usize {
        self.records.len()
    }

    /// 140212720.
    pub fn live_count(&self) -> usize {
        self.live.iter().filter(|&&live| live).count()
    }

    /// 1402122f0.
    pub fn can_emit(&self, asset: &DeclarationTrailAsset) -> bool {
        self.clock.can_emit(asset.emitter.cycles)
    }
}
