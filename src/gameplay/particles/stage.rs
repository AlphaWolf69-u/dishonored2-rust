//! Stable particle slots and child-spawn inputs (1efcc0, 1f4e00, 1f5620).
//! Emitter timing, materials and the draw owner remain separate from storage.
use super::{contains, MotionSample, ParticleState, SpawnInput};

pub struct Particle {
    pub state: ParticleState,
    pub base_velocity: [f32; 3],
    pub accumulated_velocity: [f32; 3],
    /// The stored seed at stage+1e8. Motion evaluation restarts from this seed
    /// for each substep; it does not advance the emitter's mutable seed.
    pub seed: u32,
}

pub struct ParticleStage {
    slots: Vec<Option<Particle>>,
    free: Vec<usize>,
    pub inputs: Vec<SpawnInput>,
}

impl ParticleStage {
    /// 1efcc0 caps the descriptor's estimated capacity at 8192 and initializes
    /// the free list in descending order. First emission therefore uses slot0.
    pub fn new(requested_capacity: usize) -> Self {
        let capacity = requested_capacity.min(8192);
        Self {
            slots: (0..capacity).map(|_| None).collect(),
            free: (0..capacity).rev().collect(),
            inputs: Vec::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }
    pub fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn is_full(&self) -> bool {
        self.free.is_empty()
    }

    /// 1f3350 returns live slots in ascending order to the existing free list.
    /// Keep its history; rebuilding a descending list changes subsequent slots.
    pub fn flush(&mut self) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.take().is_some() {
                self.free.push(index);
            }
        }
    }

    /// Slot allocation happens only after the emitter has accepted its sampled
    /// lifetime, spawn geometry and clipping inputs (1f4e00).
    pub fn insert(&mut self, particle: Particle) -> Option<usize> {
        let index = self.free.pop()?;
        self.slots[index] = Some(particle);
        Some(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &Particle)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.as_ref().map(|p| (i, p)))
    }

    /// One 1f5620 integration substep, in ascending live-slot order. Source
    /// velocity is supplied by the stage's wind/source-velocity producer.
    /// Retired slots return to the end of the free list without compacting
    /// other particles or changing their per-sprite indices.
    pub fn advance(
        &mut self,
        delta: f32,
        maximum_step: f32,
        loop_life: bool,
        bounds: [[f32; 3]; 2],
        planes: &[[f32; 4]],
        mut source_velocity: impl FnMut(usize, &ParticleState, &mut u32) -> Result<[f32; 3], String>,
        mut motion: impl FnMut(usize, f32, &mut u32) -> Result<MotionSample, String>,
        mut after_advance: impl FnMut(usize, &ParticleState, &mut u32) -> Result<(), String>,
    ) -> Result<(), String> {
        if !maximum_step.is_finite() || maximum_step <= 0. {
            return Err("particle integration step must be positive and finite".into());
        }
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let Some(particle) = slot else { continue };
            let mut remaining = delta;
            loop {
                // 1f5620 restarts this copy at every substep, including the
                // source-velocity sampling which precedes motion sampling.
                let mut seed = particle.seed;
                let source_velocity = source_velocity(index, &particle.state, &mut seed)?;
                let step = remaining.min(maximum_step);
                let alive = particle.state.advance(
                    step,
                    loop_life,
                    particle.base_velocity,
                    source_velocity,
                    &mut particle.accumulated_velocity,
                    |age| motion(index, age, &mut seed),
                )?;
                remaining = crate::float::subtract(remaining, step);
                if !alive || !contains(particle.state.position, bounds, planes) {
                    *slot = None;
                    self.free.push(index);
                    break;
                }
                if remaining <= f32::from_bits(0x3400_0000) {
                    // Sprite updates consume the seed after the final motion
                    // substep, once for the outer delta, before the next slot.
                    after_advance(index, &particle.state, &mut seed)?;
                    break;
                }
            }
        }
        Ok(())
    }

    /// 1f3590 appends to the receiver's ordered list. Rust's Vec owns growth;
    /// coordinate conversion is the same for every child-stage emitter.
    pub fn enqueue(
        &mut self,
        input: SpawnInput,
        worldspace: bool,
        origin: [f32; 3],
        basis: [[f32; 3]; 3],
    ) {
        self.inputs
            .push(input.in_stage_space(worldspace, origin, basis));
    }
}
