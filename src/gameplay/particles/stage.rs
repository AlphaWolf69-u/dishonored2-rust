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

    /// Havok stages mirror their pipeline bodies into the slot of the same
    /// index, so their sprite and mesh states share the ordinary draw path.
    pub fn set(&mut self, slot: usize, particle: Option<Particle>) {
        let was = self.slots[slot].is_some();
        match (&particle, was) {
            (Some(_), false) => self.free.retain(|&index| index != slot),
            (None, true) => self.free.push(slot),
            _ => {}
        }
        self.slots[slot] = particle;
    }

    /// Slot allocation happens only after the emitter has accepted its sampled
    /// lifetime, spawn geometry and clipping inputs (1f4e00).
    pub fn insert(&mut self, particle: Particle) -> Option<usize> {
        let index = self.free.pop()?;
        self.slots[index] = Some(particle);
        Some(index)
    }

    /// Live slots in publication order. 1f2420 publishes in slot order
    /// unless the stage sorts (+0x1910 nonzero) at least two particles: then
    /// each particle gets a 16-bit depth key along the camera's first axis
    /// over the stage's particle extent (1401dd220 / 1401dcac0 /
    /// 1401dcbf0), and the stable two-pass radix sort 140aeebc0 orders the
    /// slots by key. BACK_TO_FRONT swaps the extent's near and far ends.
    /// The extent and positions stay in the stage's simulation space while
    /// the camera is the render view's world position and axis (1ff460
    /// copies them unchanged), as in the original.
    pub fn publication_order(&self, sorting: u8, camera: [f32; 3], forward: [f32; 3]) -> Option<Vec<usize>> {
        use crate::float::{add, divide, multiply, subtract};
        if sorting == 0 || self.len() < 2 {
            return None;
        }
        // 1f4e00 accumulates the live particle positions from +/-1e30.
        let mut bounds = [[f32::from_bits(0x7149_f2ca); 3], [f32::from_bits(0xf149_f2ca); 3]];
        for (_, particle) in self.iter() {
            for i in 0..3 {
                let value = particle.state.position[i];
                if value <= bounds[0][i] { bounds[0][i] = value; }
                if bounds[1][i] <= value { bounds[1][i] = value; }
            }
        }
        let centre: [f32; 3] = std::array::from_fn(|i| multiply(add(bounds[1][i], bounds[0][i]), 0.5));
        let centre_depth = add(add(multiply(forward[0], centre[0]), multiply(forward[1], centre[1])),
            multiply(forward[2], centre[2]));
        let extent = add(add(multiply(forward[0], subtract(bounds[1][0], centre[0])).abs(),
            multiply(forward[1], subtract(bounds[1][1], centre[1])).abs()),
            multiply(forward[2], subtract(bounds[1][2], centre[2])).abs());
        let (mut near, mut far) = (subtract(centre_depth, extent), add(extent, centre_depth));
        if sorting == 1 {
            std::mem::swap(&mut near, &mut far);
        }
        let scale = divide(32767., subtract(far, near));
        let offset = subtract(near, add(add(multiply(forward[1], camera[1]), multiply(forward[0], camera[0])),
            multiply(forward[2], camera[2])));
        let mut keyed: Vec<(u16, usize)> = self.iter().map(|(slot, particle)| {
            let p = particle.state.position;
            let depth = add(add(multiply(subtract(p[1], camera[1]), forward[1]),
                multiply(subtract(p[0], camera[0]), forward[0])),
                multiply(subtract(p[2], camera[2]), forward[2]));
            let value = multiply(subtract(depth, offset), scale);
            let mut key = 32767_f32;
            if value <= key { key = value; }
            if key <= 0. { key = 0.; }
            (key as i32 as u16, slot)
        }).collect();
        keyed.sort_by_key(|&(key, _)| key);
        Some(keyed.into_iter().map(|(_, slot)| slot).collect())
    }

    /// Live particles in `order` when the stage sorts, else in slot order.
    pub fn in_order<'a>(&'a self, order: Option<&'a [usize]>) -> impl Iterator<Item = (usize, &'a Particle)> + 'a {
        let sorted = order.into_iter().flatten()
            .filter_map(move |&slot| self.get(slot).map(|particle| (slot, particle)));
        let plain = self.slots.iter().enumerate().filter(move |_| order.is_none())
            .filter_map(|(slot, particle)| particle.as_ref().map(|particle| (slot, particle)));
        sorted.chain(plain)
    }

    /// 1f4930: up to four random slot probes, drawing from the parent's own
    /// stage seed, for a live particle whose normalized age lies within
    /// `[lower, upper]`. A stage without live particles draws nothing.
    pub fn pick_parent(&self, lower: f32, upper: f32, seed: &mut u32) -> Option<usize> {
        if self.len() == 0 {
            return None;
        }
        let count = self.capacity();
        for _ in 0..4 {
            let slot = if (count as i32) - 1 < 1 {
                0
            } else {
                ((crate::random::lcg_step(seed) >> 10) & 0x7fff) as usize % count
            };
            if let Some(particle) = self.get(slot) {
                let age = particle.state.age;
                if lower <= age && age <= upper {
                    return Some(slot);
                }
            }
        }
        None
    }

    /// The live particle in `slot`.
    pub fn get(&self, slot: usize) -> Option<&Particle> {
        self.slots.get(slot)?.as_ref()
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
        after_advance: impl FnMut(usize, &ParticleState, &mut u32) -> Result<(), String>,
    ) -> Result<(), String> {
        self.advance_with_forces(
            delta,
            maximum_step,
            loop_life,
            bounds,
            planes,
            |index, particle, seed| {
                source_velocity(index, particle, seed).map(|velocity| (velocity, None))
            },
            |index, age, seed, _| motion(index, age, seed),
            after_advance,
        )
    }

    /// The acceleration-field input is sampled before source-over-life and
    /// passed to1dbce0 without consuming the generator a second time.
    pub fn advance_with_forces(
        &mut self,
        delta: f32,
        maximum_step: f32,
        loop_life: bool,
        bounds: [[f32; 3]; 2],
        planes: &[[f32; 4]],
        source_velocity: impl FnMut(
            usize,
            &ParticleState,
            &mut u32,
        ) -> Result<([f32; 3], Option<[f32; 3]>), String>,
        mut motion: impl FnMut(usize, f32, &mut u32, Option<[f32; 3]>) -> Result<MotionSample, String>,
        after_advance: impl FnMut(usize, &ParticleState, &mut u32) -> Result<(), String>,
    ) -> Result<(), String> {
        self.advance_with_events(
            delta,
            maximum_step,
            loop_life,
            bounds,
            planes,
            source_velocity,
            |index, _, age, seed, force| motion(index, age, seed, force),
            after_advance,
            |_, _, _| Ok(()),
            |_, _, _, _| Ok(()),
        )
    }

    pub fn advance_with_events(
        &mut self,
        delta: f32,
        maximum_step: f32,
        loop_life: bool,
        bounds: [[f32; 3]; 2],
        planes: &[[f32; 4]],
        mut source_velocity: impl FnMut(
            usize,
            &ParticleState,
            &mut u32,
        ) -> Result<([f32; 3], Option<[f32; 3]>), String>,
        mut motion: impl FnMut(
            usize,
            [f32; 3],
            f32,
            &mut u32,
            Option<[f32; 3]>,
        ) -> Result<MotionSample, String>,
        mut after_advance: impl FnMut(usize, &ParticleState, &mut u32) -> Result<(), String>,
        mut particle_event: impl FnMut(&ParticleState, f32, bool) -> Result<(), String>,
        mut attract: impl FnMut(&ParticleState, &mut u32, f32, &mut [f32; 3]) -> Result<(), String>,
    ) -> Result<(), String> {
        if !maximum_step.is_finite() || maximum_step <= 0. {
            return Err("particle integration step must be positive and finite".into());
        }
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let Some(particle) = slot else { continue };
            let previous_age = particle.state.age;
            let mut remaining = delta;
            loop {
                // 1f5620 restarts this copy at every substep, including the
                // source-velocity sampling which precedes motion sampling.
                let mut seed = particle.seed;
                let (source_velocity, forces) = source_velocity(index, &particle.state, &mut seed)?;
                let step = remaining.min(maximum_step);
                attract(&particle.state, &mut seed, step, &mut particle.accumulated_velocity)?;
                let position = particle.state.position;
                let alive = particle.state.advance(
                    step,
                    loop_life,
                    particle.base_velocity,
                    source_velocity,
                    &mut particle.accumulated_velocity,
                    |age| motion(index, position, age, &mut seed, forces),
                )?;
                remaining = crate::float::subtract(remaining, step);
                if !alive || !contains(particle.state.position, bounds, planes) {
                    particle_event(&particle.state, previous_age, false)?;
                    *slot = None;
                    self.free.push(index);
                    break;
                }
                if remaining <= f32::from_bits(0x3400_0000) {
                    // Sprite updates consume the seed after the final motion
                    // substep, once for the outer delta, before the next slot.
                    particle_event(&particle.state, previous_age, true)?;
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
