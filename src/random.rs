//! Counter-seeded multiply-with-carry generator shared by scripts and rendering.
//! Evidence: 7bebd0/7bf440/8bcce0 and the canvas constructor 2ff2f0.
pub struct MwcRandom {
    low: u32,
    high: u32,
}
impl MwcRandom {
    pub fn new(counter: u64) -> Self {
        let mut r = Self {
            low: 0x1f123bb5,
            high: 0x159a55e5,
        };
        if (counter >> 16) as u32 != 0 {
            r.low = (counter >> 16) as u32;
        }
        if counter as u32 != 0 {
            r.high = counter as u32;
        }
        r
    }
    pub fn between(&mut self, a: f32, b: f32) -> f32 {
        self.high = (self.high & 65535)
            .wrapping_mul(36969)
            .wrapping_add(self.high >> 16);
        self.low = (self.low & 65535)
            .wrapping_mul(18000)
            .wrapping_add(self.low >> 16);
        let v = self.low.wrapping_add(self.high.wrapping_shl(16));
        let fraction = (v as f64 * f64::from_bits(0x3df0000000100000)) as f32;
        let lo = a.min(b);
        let hi = a.max(b);
        fraction * (hi - lo) + lo
    }
}
