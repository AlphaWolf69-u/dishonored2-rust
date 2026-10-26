//! Allocation and routing of the three tone bands (14116f6b0/1411697b0).
use super::{FilterBand, Parameters};
use crate::gameplay::sound::filters::Biquad;

enum Path {
    Input(Biquad),
    Late(Biquad),
    Early([Biquad; 2]),
}
pub(super) struct ToneControls {
    enabled: bool,
    paths: Vec<(usize, Path)>,
}
impl ToneControls {
    pub fn new(parameters: &Parameters, early: bool, rate: u32) -> Result<Self, String> {
        let mut paths = Vec::new();
        if parameters.tone_controls {
            for (index, band) in parameters.filters.iter().enumerate() {
                let filter = || Biquad::new(band.curve, rate, band.frequency, band.gain_db, band.q);
                let path = match band.insertion {
                    0 => continue,
                    1 if !early => continue,
                    1 => Path::Early([filter()?, filter()?]),
                    2 => Path::Late(filter()?),
                    3 => Path::Input(filter()?),
                    value => return Err(format!("unsupported room tone insertion {value}")),
                };
                paths.push((index, path));
            }
        }
        Ok(Self { enabled: parameters.tone_controls, paths })
    }

    pub fn configure(&mut self, previous: &Parameters, current: &Parameters, early: bool, rate: u32) -> Result<(), String> {
        // 141169ee0 rebuilds only when the NEW configuration enables a band.
        // Turning every insertion off can retain the previous allocated paths.
        let changed = previous.early_reflections != current.early_reflections
            || previous.tone_controls != current.tone_controls
            || previous.filters.iter().zip(&current.filters).any(|(a, b)| a.insertion != b.insertion);
        if changed && current.tone_controls && current.filters.iter().any(|band| band.insertion != 0) {
            *self = Self::new(current, early, rate)?;
        }
        self.enabled = current.tone_controls;
        if !self.enabled { return Ok(()); }
        for (index, path) in &mut self.paths {
            let band = current.filters[*index];
            let old = previous.filters[*index];
            if band.insertion == 0 || (band.insertion == 1 && !early) { continue; }
            if band.curve == old.curve { continue; }
            configure(path, band, rate)?;
        }
        Ok(())
    }

    pub fn configure_rtpc(&mut self, previous: &Parameters, current: &Parameters, early: bool, rate: u32) -> Result<(), String> {
        if !current.tone_controls { return Ok(()); }
        for (index, path) in &mut self.paths {
            let band = current.filters[*index];
            let old = previous.filters[*index];
            if band.insertion == 0 || (band.insertion == 1 && !early) { continue; }
            if band.frequency == old.frequency && band.gain_db == old.gain_db && band.q == old.q { continue; }
            configure(path, band, rate)?;
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        for (_, path) in &mut self.paths {
            match path {
                Path::Input(filter) | Path::Late(filter) => filter.reset(),
                Path::Early(filters) => for filter in filters { filter.reset(); },
            }
        }
    }

    pub fn input(&mut self, samples: &mut [f32]) {
        if !self.enabled { return; }
        for (_, path) in &mut self.paths {
            if let Path::Input(filter) = path { filter.process_in_place(samples); }
        }
    }
    pub fn late(&mut self, samples: &mut [f32]) {
        if !self.enabled { return; }
        for (_, path) in &mut self.paths {
            if let Path::Late(filter) = path { filter.process_in_place(samples); }
        }
    }
    pub fn early(&mut self, left: &mut [f32], right: &mut [f32]) {
        if !self.enabled { return; }
        for (_, path) in &mut self.paths {
            if let Path::Early(filters) = path { filters[0].process_in_place(left); filters[1].process_in_place(right); }
        }
    }
}

fn configure(path: &mut Path, band: FilterBand, rate: u32) -> Result<(), String> {
    let configure = |filter: &mut Biquad| filter.configure(band.curve, rate, band.frequency, band.gain_db, band.q);
    match path {
        Path::Input(filter) | Path::Late(filter) => configure(filter)?,
        Path::Early(filters) => for filter in filters { configure(filter)?; },
    }
    Ok(())
}
