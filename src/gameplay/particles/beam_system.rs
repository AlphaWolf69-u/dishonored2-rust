//! Particle beam emitters (arkBeam, system list +0x398, 0x1e8 bytes).
//! Constructor 1401d7be0, start 1401d9460 (delay)/1401d9410 (clock), frame
//! update 1401fde70 (1401d97b0 live beams, 1401d9180 rate, 1401d8b20
//! burst, 1401d9630 spawn), strip 1401d8d50.
use super::{
    beam::{BeamFrame, BeamRecord, BeamTarget},
    beam_noise::SimplexTable,
    spawn::SpawnVelocity,
    timing::CycleClock,
    trail::{TrailBudget, TrailVertex, EMPTY_BOUNDS},
};
use crate::{
    assets::fx::{
        particle_beam::ParticleBeamAsset,
        particle_declaration::{ParticleDeclaration, ParticleWind},
    },
    float::{add, divide, multiply, subtract},
    gameplay::{effects::timing::sample_range, reference_frame::ReferenceFrame},
    random::{lcg_step, lcg_unit},
};

const EPSILON: f32 = f32::from_bits(0x3400_0000);

pub struct BeamEmitter {
    /// +0x10 and +0x14.
    seed: u32,
    cycle_seed: u32,
    /// +0x18: time since the last rate spawn, FLT_MAX until the first.
    since_spawn: f32,
    clock: CycleClock,
    /// +0x30: remaining start delay.
    delay: f32,
    records: Vec<Option<BeamRecord>>,
    live: Vec<bool>,
    /// +0x68, initialized descending so the first spawn uses slot 0.
    free: Vec<usize>,
    /// +0x78: 1401d99f0's wind force for this frame.
    wind: [f32; 3],
    /// +0x84: 1401d9970's sampled velocity inheritance.
    inherited: SpawnVelocity,
    frame: BeamFrame,
    /// +0x1a8: min(1402134d0(capacity, 1401d0ee0), r_prt_beamrenderer_budget
    /// x 4096); the cvar defaults to 1.
    budget: u32,
}

impl BeamEmitter {
    /// 1401d7be0 draws two seeds from the system seed; the system start
    /// (1401fd7b0) then samples the delay (1401d9460) and resets the clock
    /// (1401d9410) from the cycle seed.
    pub fn new(asset: &ParticleBeamAsset, system_seed: &mut u32, system: ReferenceFrame) -> Result<Self, String> {
        lcg_step(system_seed);
        let mut cycle_seed = lcg_step(system_seed);
        let capacity = asset.timings.capacity()?.min(64) as usize;
        let delay = sample_range(asset.timings.emitter.delay, &mut cycle_seed);
        let clock = CycleClock::new(asset.timings.emitter.duration, &mut cycle_seed);
        let cycle_seed = lcg_step(&mut cycle_seed);
        let tessellation = &asset.tessellation;
        let per = if tessellation.length_scaled { tessellation.maximum_segments } else { tessellation.segments as i32 };
        Ok(Self {
            seed: cycle_seed,
            cycle_seed,
            since_spawn: f32::MAX,
            clock,
            delay,
            records: (0..capacity).map(|_| None).collect(),
            live: vec![false; capacity],
            free: (0..capacity).rev().collect(),
            wind: [0.; 3],
            inherited: SpawnVelocity::default(),
            frame: BeamFrame::new(asset, system, &BeamTarget::default()),
            budget: TrailBudget::vertices(per, capacity).min(4096),
        })
    }

    /// 1401d9c20 with the restart reseed of its callers.
    fn advance_clock(&mut self, asset: &ParticleBeamAsset, delta: f32) -> Result<(), String> {
        self.clock.advance(delta, asset.timings.emitter.cycles, asset.timings.emitter.duration, &mut self.cycle_seed)?;
        if self.clock.restarted {
            self.cycle_seed = lcg_step(&mut self.cycle_seed);
            self.seed = self.cycle_seed;
        }
        Ok(())
    }

    /// 1401d9630: spawn into the last free slot and give it its first step.
    fn spawn(&mut self, asset: &ParticleBeamAsset, wind: &ParticleWind, position: [f32; 3], delta: f32) -> Result<(), String> {
        let Some(&slot) = self.free.last() else { return Ok(()) };
        let mut record = BeamRecord::spawn(asset, self.clock.fraction, position, &self.frame, &self.inherited, &mut self.cycle_seed)?;
        let alive = record.advance(asset, wind, self.wind, position, &self.frame, &mut self.cycle_seed, self.clock.fraction, delta)?;
        self.records[slot] = Some(record);
        if alive {
            self.free.pop();
            self.live[slot] = true;
        }
        Ok(())
    }

    /// 1401d97b0.
    fn update_records(&mut self, asset: &ParticleBeamAsset, wind: &ParticleWind, position: [f32; 3], delta: f32) -> Result<(), String> {
        for slot in 0..self.records.len() {
            if !self.live[slot] {
                continue;
            }
            let record = self.records[slot].as_mut().ok_or("live beam record is absent")?;
            if !record.advance(asset, wind, self.wind, position, &self.frame, &mut self.cycle_seed, self.clock.fraction, delta)? {
                self.free.push(slot);
                self.live[slot] = false;
            }
        }
        Ok(())
    }

    /// 1401d9180: rate emission in 1/60 s steps, at most one spawn per step.
    fn emit(&mut self, asset: &ParticleBeamAsset, wind: &ParticleWind, position: [f32; 3], mut delta: f32, scale: f32) -> Result<(), String> {
        let step_limit = f32::from_bits(0x3c88_8889);
        while 0. < delta {
            if self.free.is_empty() || !self.clock.can_emit(asset.timings.emitter.cycles) {
                break;
            }
            let rate = &asset.timings.emitter.rate;
            let range_mix = lcg_unit(&mut self.seed);
            let curve_mix = lcg_unit(&mut self.seed);
            let [max, min] = rate.range.range;
            let rate = multiply(
                multiply(rate.curve.sample(self.clock.fraction, curve_mix)?, add(multiply(range_mix, subtract(max, min)), min)),
                scale,
            );
            if f32::from_bits(0x3dcc_cccd) <= rate && 0. <= subtract(self.since_spawn, divide(1., rate)) {
                self.since_spawn = 0.;
                self.spawn(asset, wind, position, delta)?;
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

    /// 1401d8b20: a cycle restart spawns round(rate x scale) beams.
    fn burst(&mut self, asset: &ParticleBeamAsset, wind: &ParticleWind, position: [f32; 3], delta: f32, scale: f32) -> Result<(), String> {
        if !self.clock.can_emit(asset.timings.emitter.cycles) {
            return Ok(());
        }
        if self.clock.restarted {
            let mix = lcg_unit(&mut self.cycle_seed);
            let [max, min] = asset.timings.emitter.rate.range.range;
            let mut count = add(multiply(add(multiply(mix, subtract(max, min)), min), scale), 0.5).floor() as i32;
            while 0 < count && !self.free.is_empty() {
                self.spawn(asset, wind, position, delta)?;
                count -= 1;
            }
        }
        self.advance_clock(asset, delta)
    }

    /// 1401fde70 for one frame. `delta` already includes the first frame's
    /// warmup; `rate` is the system's emission multiplier (+0x1d90).
    #[allow(clippy::too_many_arguments)]
    pub fn advance(
        &mut self,
        asset: &ParticleBeamAsset,
        declaration: &ParticleDeclaration,
        delta: f32,
        system: ReferenceFrame,
        target: &BeamTarget,
        world_wind: [f32; 3],
        source: Option<&SpawnVelocity>,
        rate: f32,
        simulating: bool,
    ) -> Result<(), String> {
        self.frame = BeamFrame::new(asset, system, target);
        // 1401d8c70
        let consumed = if delta <= self.delay { delta } else { self.delay };
        self.delay = subtract(self.delay, consumed);
        let mut remaining = subtract(delta, consumed);
        if !(EPSILON < remaining) {
            return Ok(());
        }
        if !simulating {
            // 1401d9900
            return self.advance_clock(asset, remaining);
        }
        if !(self.delay < EPSILON) {
            return Ok(());
        }
        // 1401d99f0
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
        // 1401d9970
        if declaration.velocity_inheritance.enabled && asset.enable_velocity_inheritance {
            let source = source.ok_or("beam velocity inheritance awaits the owning entity's published motion")?;
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
            self.update_records(asset, wind, system.position, step)?;
            if asset.timings.burst {
                self.burst(asset, wind, system.position, step, rate)?;
            } else {
                self.emit(asset, wind, system.position, step, rate)?;
            }
            remaining = subtract(remaining, step);
        }
        Ok(())
    }

    /// 1401d8d50: the live beams' strips within the renderer budget, joined
    /// into one strip, and their merged bounds.
    pub fn build(
        &self,
        asset: &ParticleBeamAsset,
        eye: [f32; 3],
        tint: [f32; 4],
        owner: ReferenceFrame,
        noise: Option<&SimplexTable>,
        vertices: &mut Vec<TrailVertex>,
    ) -> Result<[[f32; 3]; 2], String> {
        let mut bounds = EMPTY_BOUNDS;
        let live: Vec<&BeamRecord> = (0..self.records.len())
            .filter(|&slot| self.live[slot])
            .filter_map(|slot| self.records[slot].as_ref())
            .collect();
        let budget = TrailBudget::new(live.iter().map(|record| record.segments), self.budget);
        let mut joined = false;
        for (index, record) in live.iter().enumerate() {
            let segments = budget.segments(record.segments);
            if segments != 0 {
                let more = index + 1 < live.len();
                let strip = record.build(asset, segments, eye, tint, owner, noise, joined, more, vertices)?;
                joined = true;
                crate::rendering::particles::bounds::include(&mut bounds, strip);
            }
        }
        Ok(bounds)
    }

    pub fn capacity(&self) -> usize {
        self.records.len()
    }

    /// 1401d9070.
    pub fn live_count(&self) -> usize {
        self.live.iter().filter(|&&live| live).count()
    }

    /// 1401d8c60.
    pub fn can_emit(&self, asset: &ParticleBeamAsset) -> bool {
        self.clock.can_emit(asset.timings.emitter.cycles)
    }
}
