//! Point identity and retained solver-array correspondence, 1412abc30 /140d14bf0.
//! Geometry supplies its actual clipped candidate indices and face identifiers.
//! No world-position hash or new geometry/resource owner is introduced here.
use super::ContactCache;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FacePair {
    pub a: u32,
    pub b: u32,
    /// The source's compact face-index representation (cache+56).
    pub compact: bool,
}

impl FacePair {
    /// 140d14bf0: full face indices use the source's two integer multipliers.
    pub fn point_id(self, candidate: u8) -> u32 {
        let prefix = if self.compact {
            u32::from(((self.a as u16) << 8) | self.b as u16) << 16
        } else {
            self.a.wrapping_mul(0x4b) ^ self.b.wrapping_mul(0xd)
        };
        prefix ^ u32::from(candidate)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PointCorrespondence {
    pub faces: Option<FacePair>,
    pub candidates: [u8; 4],
    pub count: u8,
}

impl PointCorrespondence {
    /// Return the source's final point order, which also orders the manifold's
    /// world witnesses and the three aligned retained solver arrays.
    pub fn update(
        &mut self,
        faces: FacePair,
        selected: &[u8],
        projected_candidates: &[[f32; 2]],
        minimum_body_radius: f32,
        cache: &mut ContactCache,
    ) -> Result<[u8; 4], &'static str> {
        if selected.len() > 4
            || selected
                .iter()
                .any(|&i| i as usize >= projected_candidates.len())
        {
            return Err("contact selection exceeds source candidate storage");
        }
        let mut remaining = [0u8; 4];
        remaining[..selected.len()].copy_from_slice(selected);
        let mut remaining_count = selected.len();
        let old = *cache;
        cache.normal_impulses = [0.; 4];
        cache.position_error = [0.; 4];
        cache.distance_offset = [0.; 4];
        let mut output = [0u8; 4];
        let mut output_count = 0;
        if self.faces == Some(faces) {
            // DAT1423ccb90 is exactly this stable partition of the four slots.
            let mut order = [0usize; 4];
            let mut cursor = 0;
            for positive in [true, false] {
                for i in 0..self.count as usize {
                    if (0. < old.normal_impulses[i]) == positive {
                        order[cursor] = i;
                        cursor += 1;
                    }
                }
            }
            for &old_slot in &order[..cursor] {
                let candidate = self.candidates[old_slot];
                let exact = remaining[..remaining_count]
                    .iter()
                    .position(|&x| x == candidate);
                let matched = exact.or_else(|| {
                    let old_xy = *projected_candidates.get(candidate as usize)?;
                    // 1423ccd90 is the source's 1/15, not an admission epsilon.
                    let tolerance = minimum_body_radius * f32::from_bits(0x3d88_8889);
                    remaining[..remaining_count].iter().position(|&new| {
                        let xy = projected_candidates[new as usize];
                        let dy = old_xy[1] - xy[1];
                        let dx = old_xy[0] - xy[0];
                        dy * dy + dx * dx < tolerance * tolerance
                    })
                });
                let Some(slot) = matched else { continue };
                output[output_count] = remaining[slot];
                cache.normal_impulses[output_count] = old.normal_impulses[old_slot];
                cache.position_error[output_count] = old.position_error[old_slot];
                cache.distance_offset[output_count] = old.distance_offset[old_slot];
                output_count += 1;
                remaining_count -= 1;
                remaining[slot] = remaining[remaining_count];
                if exact.is_some() {
                    // Native exact correspondence also drops duplicate IDs,
                    // with swap-last removal preserving its later order.
                    let mut i = slot;
                    while i < remaining_count {
                        if remaining[i] == candidate {
                            remaining_count -= 1;
                            remaining[i] = remaining[remaining_count];
                        } else {
                            i += 1;
                        }
                    }
                }
            }
            output[output_count..output_count + remaining_count]
                .copy_from_slice(&remaining[..remaining_count]);
            output_count += remaining_count;
        } else {
            output[..selected.len()].copy_from_slice(selected);
            output_count = selected.len();
            // Changed reference faces retain only total normal force. Source
            // DAT1423ccd00 is [0,1,1/2,1/3,1/4] in four replicated lanes.
            let reciprocal_count = match output_count {
                1 => 1.,
                2 => 0.5,
                3 => f32::from_bits(0x3eaa_aaab),
                4 => 0.25,
                _ => 0.,
            };
            let mut sum = 0.;
            for value in &old.normal_impulses[..self.count as usize] {
                sum += *value;
            }
            cache.normal_impulses[..output_count].fill(sum * reciprocal_count);
        }
        self.faces = Some(faces);
        self.count = output_count as u8;
        if output_count != 0 {
            let first = output[0];
            output[output_count..].fill(first);
        }
        self.candidates = output;
        Ok(output)
    }

    pub fn point_ids(&self) -> [u32; 4] {
        self.candidates
            .map(|candidate| self.faces.map_or(0, |faces| faces.point_id(candidate)))
    }
}
