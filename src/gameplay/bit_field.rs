//! Word-backed physics bitfield growth and tail fill (099fc0).
#[derive(Clone, Debug, Default)]
pub struct BitField {
    words: Vec<u32>,
    bits: usize,
}
impl BitField {
    pub fn zeroed(bits: usize) -> Self {
        Self {
            words: vec![0; bits.div_ceil(32)],
            bits,
        }
    }
    pub fn len(&self) -> usize {
        self.bits
    }
    pub fn is_empty(&self) -> bool {
        self.bits == 0
    }
    pub fn words(&self) -> &[u32] {
        &self.words
    }
    pub fn set(&mut self, bit: usize) -> Result<(), String> {
        if bit >= self.bits {
            return Err("bit index exceeds bitfield length".into());
        }
        self.words[bit >> 5] |= 1 << (bit & 31);
        Ok(())
    }
    /// Preserve existing used bits, fill added bits with wrapping -fill and
    /// normalize unused tail bits according to fill==0, as the source does.
    pub fn resize(&mut self, bits: usize, fill: i32) {
        let old = self.bits;
        self.words.resize(bits.div_ceil(32), 0);
        self.bits = bits;
        if bits > old {
            let first = old >> 5;
            let end = bits >> 5;
            let low_mask = u32::MAX << (old & 31);
            let high_mask = !(u32::MAX << (bits & 31));
            let value = 0u32.wrapping_sub(fill as u32);
            if first == end {
                let mask = low_mask & high_mask;
                if mask != 0 {
                    self.words[first] = (self.words[first] & !mask) | (value & mask);
                }
            } else {
                self.words[first] = (self.words[first] & !low_mask) | (value & low_mask);
                if high_mask != 0 {
                    self.words[end] = (self.words[end] & !high_mask) | (value & high_mask);
                }
                self.words[first + 1..end].fill(value);
            }
        }
        if bits != 0 && bits & 31 != 0 {
            let mask = u32::MAX << (bits & 31);
            let last = self.words.last_mut().unwrap();
            if fill == 0 {
                *last &= !mask;
            } else {
                *last |= mask;
            }
        }
    }
}

#[cfg(test)]
#[path = "bit_field/tests.rs"]
mod tests;
