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
    pool.update_chain(head, |current| {
        let effect = effects
            .get_mut(usize::from(current))
            .and_then(Option::as_mut)
            .ok_or_else(|| format!("FX environment slot {current} is not initialized"))?;
        if effect.active {
            effect.update(now, view_mask, seed)?;
            Ok(true)
        } else {
            retire(current, effect)?;
            Ok(false)
        }
    })
}
