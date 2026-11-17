//! Game-local motor mixer7b5fa0/7b5ff0, normal11a/11b callbacks7b61f0/7b63f0.
//! FX229f10 supplies its four channels; Windows0e0e30 consumes the first two.
#[derive(Default)]
pub struct Mixer {
    enabled: bool,
    impulses: Vec<Impulse>,
}
struct Impulse { low: f32, high: f32, until: f64 }
impl Mixer {
    ///7b67a0 drops outstanding impulses when the active-device/game-state
    ///policy becomes false. They do not replay after keyboard/pause changes.
    pub fn enabled(&mut self, value: bool) {
        if self.enabled && !value { self.impulses.clear(); }
        self.enabled = value;
    }
    pub fn impulse(&mut self, time: f64, low: f32, high: f32, duration: f32, default_duration: f32) {
        if !self.enabled { return; }
        let duration = if duration == 0. { default_duration } else { duration };
        self.impulses.push(Impulse { low, high, until: time + f64::from(duration) });
    }
    pub fn clear(&mut self) { self.impulses.clear(); }
    ///7b5ff0 expires in reverse with swap removal and takes the per-channel
    ///maximum. Comparisons deliberately preserve native unordered behavior.
    pub fn output(&mut self, time: f64, mut values: [f32; 4]) -> [f32; 4] {
        if !self.enabled { return [0.; 4]; }
        for i in (0..self.impulses.len()).rev() {
            let impulse = &self.impulses[i];
            if impulse.until < time { self.impulses.swap_remove(i); }
            else {
                if values[0] <= impulse.low { values[0] = impulse.low; }
                if values[1] <= impulse.high { values[1] = impulse.high; }
            }
        }
        for value in &mut values {
            *value = if *value >= 0. { if *value > 1. { 1. } else { *value } } else { 0. };
            //DAT141c693ec=2^-16, native dead motor threshold.
            if value.abs() <= f32::from_bits(0x3780_0000) { *value = 0.; }
        }
        values
    }
}
