//! DSP delay storage and sample exchange (141170700/1411707d0/141170890).

pub struct Delay {
    samples: Vec<f32>,
    length: usize,
    cursor: usize,
}
impl Delay {
    /// Combine allocation/cursor initialization with the effect's initial
    /// reset. Storage rounds to four samples; wrap uses the authored length.
    pub fn new(length: u32) -> Result<Self, String> {
        let length = length.max(4);
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
