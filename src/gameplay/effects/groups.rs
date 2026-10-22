//! Parent group sampling (217940) and environment chain update (21eec0).
use super::{
    environment::EnvironmentBlendEffect, parameters::sample_range, pool::EffectPoolLinks,
    timing::EffectTimingRanges,
};

/// Group ranges are sampled in declaration order even if no child is admitted.
/// 222f80 then adds the parent's start age to the delay offset alone.
pub fn sample_offsets(ranges: &EffectTimingRanges, start: f32, seed: &mut u32) -> [f32; 4] {
    let mut result = [
        sample_range(ranges.delay, seed),
        sample_range(ranges.fade_in, seed),
        sample_range(ranges.duration, seed),
        sample_range(ranges.fade_out, seed),
    ];
    result[0] = super::add(result[0], start);
    result
}

/// Visit the pool chain in its retained order. Retirement precedes returning a
/// slot to the free list. An effect that finishes during this tick stays linked
/// until the following tick, and still counts as one processed effect.
pub fn update_environment_chain(
    pool: &mut EffectPoolLinks,
    head: &mut u16,
    effects: &mut [Option<EnvironmentBlendEffect>],
    now: f32,
    view_mask: u8,
    seed: &mut u32,
    mut retire: impl FnMut(u16, &EnvironmentBlendEffect) -> Result<(), String>,
) -> Result<u32, String> {
    let mut current = *head;
    let mut previous: Option<u16> = None;
    let mut updated = 0;
    let mut visited = 0;
    while current != u16::MAX {
        visited += 1;
        if visited > pool.next.len() {
            return Err("FX environment chain contains a cycle".into());
        }
        let effect = effects
            .get_mut(usize::from(current))
            .and_then(Option::as_mut)
            .ok_or_else(|| format!("FX environment slot {current} is not initialized"))?;
        let next = *pool
            .next
            .get(usize::from(current))
            .ok_or_else(|| format!("FX environment link {current} exceeds pool capacity"))?;
        if effect.active {
            effect.update(now, view_mask, seed)?;
            updated += 1;
            previous = Some(current);
        } else {
            retire(current, effect)?;
            if let Some(previous) = previous {
                pool.release(current)?;
                pool.next[usize::from(previous)] = next;
            } else {
                pool.release_head(head)?;
            }
        }
        current = next;
    }
    Ok(updated)
}
