//! Cell centers and exact motion assignment from the space splitter.
//! Rebalancing and solver pair scheduling are separate producers.
pub enum SpaceCells {
    Single,
    Dynamic(Vec<[f32; 4]>),
}
impl SpaceCells {
    /// cbaa30's initial centers. The query processes four cells per group;
    /// require fully initialized groups rather than reading allocator padding.
    pub fn dynamic(count: usize) -> Result<Self, String> {
        if count == 0 || count > 32 || count % 4 != 0 {
            return Err(
                "dynamic motion cells require 4..32 initialized centers in groups of four".into(),
            );
        }
        Ok(Self::Dynamic(
            (0..count)
                .map(|i| {
                    [
                        (i % 4) as f32 * 10.,
                        (i / 4) as f32 * 10.,
                        (i / 4) as f32 * 5.,
                        0.,
                    ]
                })
                .collect(),
        ))
    }
    /// cbaaf0's four-lane reduction. Equal/unordered compares choose the later
    /// candidate; replacing this with a scalar minimum changes ties.
    pub fn select(&self, point: [f32; 4]) -> Result<u8, String> {
        let Self::Dynamic(centers) = self else {
            return Ok(0);
        };
        if centers.is_empty() || centers.len() > 32 || centers.len() % 4 != 0 {
            return Err("motion cell center array is not fully initialized".into());
        }
        let distance = |center: [f32; 4]| {
            let d: [f32; 3] = std::array::from_fn(|i| center[i] - point[i]);
            (d[2] * d[2] + d[1] * d[1]) + d[0] * d[0]
        };
        let mut distances: [f32; 4] = std::array::from_fn(|i| distance(centers[i]));
        let mut ids = [0u8, 1, 2, 3];
        for (group, values) in centers[4..].chunks_exact(4).enumerate() {
            for lane in 0..4 {
                let next = distance(values[lane]);
                if !(distances[lane] < next) {
                    distances[lane] = next;
                    ids[lane] = ((group + 1) * 4 + lane) as u8;
                }
            }
        }
        let left = if distances[0] < distances[2] { 0 } else { 2 };
        let right = if distances[1] < distances[3] { 1 } else { 3 };
        Ok(ids[if distances[left] < distances[right] {
            left
        } else {
            right
        }])
    }
}
/// cba6c0's lookup: absent cells fold to the other cell's diagonal.
pub fn cell_pair_index(a: Option<u8>, b: Option<u8>) -> Result<u16, String> {
    if a.is_some_and(|v| v >= 32) || b.is_some_and(|v| v >= 32) {
        return Err("motion cell exceeds pair table".into());
    }
    let (a, b) = match (a, b) {
        (None, None) => return Ok(0xff),
        (Some(a), None) | (None, Some(a)) => (a, a),
        (Some(a), Some(b)) => (a, b),
    };
    let (low, high) = (u16::from(a.min(b)), u16::from(a.max(b)));
    Ok((high + 1) * high / 2 + low)
}
#[cfg(test)]
#[path = "space_cells/tests.rs"]
mod tests;
