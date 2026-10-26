//! DSP delay storage and sample exchange (141170700/1411707d0/141170890).

pub struct Delay {
    samples: Vec<f32>,
    length: usize,
    cursor: usize,
}

/// Diffusion all-pass with paired input/output history (14116ffc0/1411700b0).
pub struct AllPass {
    history: Vec<[f32; 2]>,
    cursor: usize,
    coefficient: f32,
}
impl AllPass {
    pub fn new(length: u32, coefficient: f32) -> Result<Self, String> {
        let length = length.max(1) as usize;
        let mut history = Vec::new();
        history.try_reserve_exact(length).map_err(|error| format!("audio diffuser allocation: {error}"))?;
        history.resize(length, [0.0; 2]);
        Ok(Self { history, cursor: 0, coefficient })
    }

    pub fn reset(&mut self) { self.history.fill([0.0; 2]); } // 141170080 preserves cursor.
    pub fn set_coefficient(&mut self, coefficient: f32) { self.coefficient = coefficient; }

    pub fn sample(&mut self, input: f32) -> f32 {
        let history = &mut self.history[self.cursor];
        let output = (input - history[1]) * self.coefficient + history[0];
        *history = [input, output];
        self.cursor += 1;
        if self.cursor == self.history.len() { self.cursor = 0; }
        output
    }

    pub fn process_in_place(&mut self, samples: &mut [f32]) {
        for sample in samples { *sample = self.sample(*sample); }
    }
}
impl Delay {
    pub(super) fn current(&self) -> f32 { self.samples[self.cursor] }
    pub(super) fn contiguous(&self) -> usize { self.length - self.cursor }

    /// Combine allocation/cursor initialization with the effect's initial
    /// reset. Storage rounds to four samples; wrap uses the authored length.
    pub fn new(length: u32) -> Result<Self, String> {
        Self::exact(length.max(4))
    }

    /// Matrix Reverb's rings allow single-frame delays (14117ed20).
    pub(super) fn exact(length: u32) -> Result<Self, String> {
        let length = length.max(1);
        let allocated = length.checked_add(3).ok_or("audio delay length overflow")? & !3;
        let mut samples = Vec::new();
        samples.try_reserve_exact(allocated as usize).map_err(|error| format!("audio delay allocation: {error}"))?;
        samples.resize(allocated as usize, 0.0);
        Ok(Self { samples, length: length as usize, cursor: 0 })
    }

    /// 1411707a0 clears history without rewinding the current cursor.
    pub fn reset(&mut self) { self.samples[..self.length].fill(0.0); }

    pub fn sample(&mut self, input: f32) -> f32 {
        let output = std::mem::replace(&mut self.samples[self.cursor], input);
        self.cursor += 1;
        if self.cursor == self.length { self.cursor = 0; }
        output
    }

    pub fn process_in_place(&mut self, samples: &mut [f32]) {
        for sample in samples { *sample = self.sample(*sample); }
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), String> {
        if input.len() != output.len() { return Err("audio delay input/output frame counts differ".into()); }
        for (&input, output) in input.iter().zip(output) { *output = self.sample(input); }
        Ok(())
    }
}
