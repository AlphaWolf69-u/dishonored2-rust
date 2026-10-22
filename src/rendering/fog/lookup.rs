//! Fog-curve history selection from 3110e0. The owner supplies the previous
//! raw blend weight and retains two 4096x1 R16F history images plus an override
//! image; this module selects their roles without allocating or rendering.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogLookupPlan {
    pub source: usize,
    pub output: usize,
    pub final_image: usize,
    pub weight: f32,
    pub apply_overrides: bool,
}

pub struct FogLookupHistory {
    source: usize,
    output: usize,
    previous_weight: f32,
}

impl FogLookupHistory {
    /// Initial image roles are the owner's slots 248 and 250. Their contents
    /// and the previous raw weight belong to that owner's initialization.
    pub fn new(previous_weight: f32) -> Self {
        Self {
            source: 0,
            output: 1,
            previous_weight,
        }
    }

    pub fn prepare(&mut self, weight: f32, overrides: [f32; 4]) -> FogLookupPlan {
        // Ordered strict comparison: equal weights and unordered values do
        // not swap. Store the RAW weight before clamping the shader input.
        if weight < self.previous_weight {
            std::mem::swap(&mut self.source, &mut self.output);
        }
        self.previous_weight = weight;
        // COMISS/JBE followed by COMISS/JA preserves unordered weights and
        // negative zero. f32::min/max or clamp would change those semantics.
        let clamped = if weight < 0. {
            0.
        } else if weight > 1. {
            1.
        } else {
            weight
        };
        let sum = ((overrides[0] + 0.) + overrides[1]) + overrides[2] + overrides[3];
        let apply_overrides = sum > f32::from_bits(0x3586_37bd);
        FogLookupPlan {
            source: self.source,
            output: self.output,
            final_image: if apply_overrides { 2 } else { self.output },
            weight: clamped,
            apply_overrides,
        }
    }
}

#[cfg(test)]
#[path = "lookup/tests.rs"]
mod tests;
