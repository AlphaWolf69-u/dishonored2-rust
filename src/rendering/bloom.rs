//! 325d80's inclusive mip traversal and separable bloom draw schedule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomDraw {
    pub source_extent: [u32; 2],
    pub target_extent: [u32; 2],
    pub delta: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct BloomSchedule {
    pub source_mip: usize,
    pub draws: Vec<BloomDraw>,
}

impl BloomSchedule {
    /// `mips` describes the actual shared luminance image's floor-sized views.
    /// Disabled bloom publishes the intrinsic black image instead of any draws.
    pub fn new(
        mips: &[[u32; 2]],
        source_mip: u32,
        target_mip: i32,
        flags: u32,
        resolution_scale: [f32; 2],
    ) -> Result<Option<Self>, String> {
        if flags & 0x100001 != 0 {
            return Ok(None);
        }
        let base = *mips.first().ok_or("bloom needs a luminance image")?;
        if mips.iter().flatten().any(|&v| v == 0) {
            return Err("bloom mip dimensions must be positive".into());
        }
        let source_mip = (source_mip as usize).min(mips.len() - 1);
        let target_mip = (target_mip.max(0) as usize).min(source_mip);
        let mut previous = mips[source_mip];
        let aspect = base[0] as f32 / base[1] as f32;
        let mut draws = Vec::with_capacity(2 * (source_mip - target_mip + 1));
        for level in (target_mip..=source_mip).rev() {
            // Source views floor each mip; temporary targets repeatedly ceil
            // the base, level-1 times. Neither can substitute for the other.
            let mut next = base;
            for _ in 1..level {
                next = next.map(|v| v.div_ceil(2));
            }
            let horizontal = [next[0], previous[1]];
            draws.push(BloomDraw {
                source_extent: previous,
                target_extent: horizontal,
                delta: [(aspect / previous[0] as f32) * resolution_scale[0], 0.0],
            });
            draws.push(BloomDraw {
                source_extent: horizontal,
                target_extent: next,
                delta: [0.0, (1.0 / previous[1] as f32) * resolution_scale[1]],
            });
            previous = next;
        }
        Ok(Some(Self { source_mip, draws }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_schedule_matches_installed_machine_code() {
        let mut lines = include_str!("../../tests/fixtures/bloom_schedule.txt").lines();
        let mut cases = 0;
        let mut draws = 0;
        while let Some(line) = lines.next() {
            let case: Vec<_> = line.split_whitespace().collect();
            assert_eq!(case[0], "CASE");
            let number = |i: usize| case[i].parse::<u32>().unwrap();
            let base = [number(1), number(2)];
            let mips =
                crate::rendering::luminance::pyramid_extents(base.map(|v| v * 2), 1).unwrap();
            let scale = std::array::from_fn(|i| {
                let bytes = std::array::from_fn(|j| {
                    u8::from_str_radix(&case[6][i * 8 + j * 2..i * 8 + j * 2 + 2], 16).unwrap()
                });
                f32::from_le_bytes(bytes)
            });
            let result =
                BloomSchedule::new(&mips, number(3), case[4].parse().unwrap(), number(5), scale)
                    .unwrap();
            assert_eq!(result.is_some(), number(7) != 0, "{line}");
            let expected_count = number(8) as usize;
            if let Some(result) = result {
                assert_eq!(result.draws.len(), expected_count, "{line}");
                for draw in result.draws {
                    let raw: Vec<_> = lines.next().unwrap().split_whitespace().collect();
                    let n = |i: usize| raw[i].parse::<u32>().unwrap();
                    assert_eq!(draw.source_extent, [n(1), n(2)], "{line}");
                    assert_eq!(draw.target_extent, [n(3), n(4)], "{line}");
                    assert_eq!(
                        draw.delta.map(f32::to_bits),
                        std::array::from_fn(|i| u32::from_str_radix(raw[i + 5], 16).unwrap()),
                        "{line}"
                    );
                    draws += 1;
                }
            } else {
                assert_eq!(expected_count, 0);
            }
            cases += 1;
        }
        assert_eq!((cases, draws), (192, 308));
    }
}
