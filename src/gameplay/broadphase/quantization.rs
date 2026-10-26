//! World-bound quantization setup, 11ff9f0 and 11fdfd0.
use super::super::point_query::{PointQuantization, QuantizedBounds};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackedBodyBounds {
    pub words: [u32; 4],
}

pub struct BoundsPackingLayout {
    pub pack_shuffle: [u8; 16],
    pub min_shuffle: [u8; 16],
    pub max_shuffle: [u8; 16],
    pub component_mask: [u32; 4],
    pub scale_units: [f32; 4],
    pub max_quantized: [u32; 4],
}

const Y_SHORT: BoundsPackingLayout = BoundsPackingLayout {
    pack_shuffle: [8, 9, 10, 4, 5, 0, 1, 2, 8, 9, 10, 4, 5, 0, 1, 2],
    min_shuffle: [5, 6, 7, 0, 3, 4, 0, 0, 0, 1, 2, 0, 0, 0, 0, 0],
    max_shuffle: [13, 14, 15, 0, 11, 12, 0, 0, 8, 9, 10, 15, 0, 0, 0, 0],
    component_mask: [0xff_ffff, 0xffff, 0xff_ffff, 0],
    scale_units: [8388608., 32768., 8388608., 1.],
    max_quantized: [0x7fff00, 0x7fff, 0x7fff00, 0],
};
const Z_SHORT: BoundsPackingLayout = BoundsPackingLayout {
    pack_shuffle: [4, 5, 6, 8, 9, 0, 1, 2, 4, 5, 6, 8, 9, 0, 1, 2],
    min_shuffle: [5, 6, 7, 0, 0, 1, 2, 0, 3, 4, 0, 0, 0, 0, 0, 0],
    max_shuffle: [13, 14, 15, 0, 8, 9, 10, 15, 11, 12, 0, 0, 0, 0, 0, 0],
    component_mask: [0xff_ffff, 0xff_ffff, 0xffff, 0],
    scale_units: [8388608., 8388608., 32768., 1.],
    max_quantized: [0x7fff00, 0x7fff00, 0x7fff, 0],
};

pub struct PackedQuantization {
    pub offset: [f32; 4],
    pub scale: [f32; 4],
    pub inverse_scale: [f32; 4],
    /// The packed format uses a 16-bit Y field normally, or Z when swapped.
    pub swap_yz: bool,
    pub layout: &'static BoundsPackingLayout,
    pub boundary_step: u32,
    pub nonzero_extent_mask: u32,
}

impl PackedQuantization {
    /// Static-body portion of c8e930: half-tolerance padding, unsigned clamp,
    /// truncation and the packed 24/16/24-bit coordinate layout.
    pub fn pack_bounds(&self, bounds: [[f32; 4]; 2], tolerance: [f32; 4]) -> PackedBodyBounds {
        let maximum = self.maximum();
        let mut quantized = [[0u32; 4]; 2];
        for side in 0..2 {
            for i in 0..4 {
                let padding = tolerance[i] * 0.5;
                let expanded = if side == 0 {
                    bounds[side][i] - padding
                } else {
                    bounds[side][i] + padding
                };
                let value = (self.offset[i] + expanded) * self.scale[i];
                let value = if 0. > value { 0. } else { value };
                let value = min(maximum[i], value);
                // Clamp maxima are below INT_MAX; NaNs retain CVTT's indefinite
                // word. Rust's saturating float-to-unsigned cast differs here.
                quantized[side][i] = crate::float::truncate_i32(value) as u32;
            }
        }
        let axis = usize::from(self.swap_yz);
        let low = quantized[0];
        let high = quantized[1];
        PackedBodyBounds {
            words: [
                (low[1 + axis] << 24) | low[2 - axis],
                (low[1 + axis] >> 8) | (low[0] << 8),
                (high[1 + axis] << 24) | high[2 - axis],
                ((high[1 + axis] as i32 >> 8) as u32) | (high[0] << 8),
            ],
        }
    }

    /// 128b200's extraction of the coarse bounds consumed by the tree.
    pub fn query_bounds(&self, packed: PackedBodyBounds) -> QuantizedBounds {
        let bytes: [u8; 16] = std::array::from_fn(|i| packed.words[i / 4].to_le_bytes()[i % 4]);
        let word = |at| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let axis = usize::from(self.swap_yz);
        let mut min = [0; 3];
        let mut max = [0; 3];
        min[0] = word(6);
        min[1 + axis] = word(3);
        min[2 - axis] = word(1);
        max[0] = word(14);
        max[1 + axis] = word(11);
        max[2 - axis] = word(9);
        QuantizedBounds { min, max }
    }

    /// c85490 decodes the full packed precision into world-space bounds. The
    /// contact owner uses these bounds for volume size and water surface height;
    /// they differ from both the tree's coarse bounds and an authored shape box.
    pub fn unpack_bounds(&self, packed: PackedBodyBounds) -> [[f32; 4]; 2] {
        let bytes: [u8; 16] = std::array::from_fn(|i| packed.words[i / 4].to_le_bytes()[i % 4]);
        let decode = |shuffle: [u8; 16], bias: u32| {
            std::array::from_fn(|lane| {
                let value = u32::from_le_bytes(std::array::from_fn(|byte| {
                    bytes[usize::from(shuffle[lane * 4 + byte])]
                })) & self.layout.component_mask[lane];
                let integer = value.wrapping_add(bias) as i32;
                crate::float::subtract(
                    crate::float::multiply(self.inverse_scale[lane], integer as f32),
                    self.offset[lane],
                )
            })
        };
        [
            decode(self.layout.min_shuffle, self.nonzero_extent_mask),
            decode(self.layout.max_shuffle, self.boundary_step),
        ]
    }

    pub fn maximum(&self) -> [f32; 4] {
        let mut values = self.layout.max_quantized.map(|v| v as f32);
        values[3] = 1.;
        values
    }
}

pub struct BroadphaseQuantization {
    pub bounds: [[f32; 4]; 2],
    pub offset: [f32; 4],
    pub scale: [f32; 4],
    pub inverse_scale: [f32; 4],
    pub packed: PackedQuantization,
}

// MINPS chooses its second operand for unordered or equal values.
fn min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

impl BroadphaseQuantization {
    pub fn new(bounds: [[f32; 4]; 2]) -> Self {
        let span: [f32; 4] = std::array::from_fn(|i| bounds[1][i] - bounds[0][i]);
        let offset = std::array::from_fn(|i| if i == 3 { 0. } else { -bounds[0][i] });
        // The generic point-query grid has 32768 cells along each world axis.
        // Unlike the packed setup, its zero extent produces infinity here.
        let scale = std::array::from_fn(|i| if i == 3 { 0. } else { (1. / span[i]) * 32768. });
        let inverse_scale = std::array::from_fn(|i| if i == 3 { 0. } else { 1. / scale[i] });
        let minimum = min(span[2], min(span[1], span[0]));
        let bits = (0..3).fold(0usize, |bits, i| {
            bits | (usize::from(span[i] <= minimum) << i)
        });
        let swap_yz = [0, 0, 1, 0, 2, 0, 1, 0][bits] == 2;
        let layout = if swap_yz { &Z_SHORT } else { &Y_SHORT };
        let packed_scale = std::array::from_fn(|i| {
            if i == 3 {
                0.
            } else {
                let reciprocal = if span[i] == 0. { 0. } else { 1. / span[i] };
                reciprocal * layout.scale_units[i]
            }
        });
        let packed_inverse = std::array::from_fn(|i| {
            if packed_scale[i] == 0. {
                0.
            } else {
                1. / packed_scale[i]
            }
        });
        let nonzero = span.iter().any(|v| v.to_bits() & 0x7fff_ffff != 0);
        Self {
            bounds,
            offset,
            scale,
            inverse_scale,
            packed: PackedQuantization {
                offset,
                scale: packed_scale,
                inverse_scale: packed_inverse,
                swap_yz,
                layout,
                boundary_step: if nonzero { 2 } else { 1 },
                nonzero_extent_mask: if nonzero { u32::MAX } else { 0 },
            },
        }
    }

    pub fn point_quantization(&self) -> PointQuantization {
        PointQuantization {
            offset: std::array::from_fn(|i| self.offset[i]),
            scale: std::array::from_fn(|i| self.inverse_scale[i]),
        }
    }
}

#[cfg(test)]
#[path = "quantization_tests.rs"]
mod tests;
