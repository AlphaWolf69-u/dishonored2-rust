//! 361d60's renderer-owned counters, before ocean readback and world draws.

/// Initial values come from the actual renderer owner. Ocean resources must
/// share this clock rather than incrementing a private counter on every update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderFrameIndices {
    /// Renderer+0xc780: two-buffer index.
    pub double_buffer: u32,
    /// Renderer+0xc784: four-buffer readback index.
    pub readback_buffer: u32,
}

impl RenderFrameIndices {
    /// Target flags are the source view record's two pointers at+0x20/+0x28.
    /// View active/presentation flags do not affect this admission predicate.
    /// Call once before selected-slot readback, not once per individual view.
    pub fn advance_world_views(&mut self, skip_world_views: bool, targets: &[[bool; 2]]) -> bool {
        if skip_world_views || !targets.iter().any(|target| target[0] || target[1]) {
            return false;
        }
        // Source signed remainder lowering preserves negative/overflow cases,
        // although valid renderer indices remain in their two/four-slot ranges.
        self.double_buffer = (self.double_buffer.wrapping_add(1) as i32 % 2) as u32;
        self.readback_buffer = (self.readback_buffer.wrapping_add(1) as i32 % 4) as u32;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn render_indices_match_executed_world_view_admission() {
        let mut cases = 0;
        for line in include_str!("../../tests/fixtures/render_frame_indices.txt").lines() {
            let row = line
                .split_whitespace()
                .map(|v| u64::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(row.len(), 11);
            let mut indices = RenderFrameIndices {
                double_buffer: row[0] as u32,
                readback_buffer: row[1] as u32,
            };
            let initial = indices;
            let targets = vec![[row[3] & 1 != 0, row[3] & 2 != 0]; row[2] as usize];
            let advanced = indices.advance_world_views(row[4] != 0, &targets);
            assert_eq!(
                indices,
                RenderFrameIndices {
                    double_buffer: row[7] as u32,
                    readback_buffer: row[8] as u32
                },
                "{line}"
            );
            assert_eq!(advanced, indices != initial);
            cases += 1;
        }
        assert_eq!(cases, 672);
    }
}
