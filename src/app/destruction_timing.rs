//! Opt-in scoped receipts for real Breakable/explosion work, not frame history.
use crate::gameplay::{component_state::entity::ComponentHandle, entity_lifetime::EntityId};

#[derive(Clone, Copy, Debug)]
pub(in crate::app) enum Owner {
    Entity(Option<EntityId>),
    Component { handle: ComponentHandle, generation: u32 },
}
pub(in crate::app) struct Span {
    phase: &'static str,
    owner: Owner,
    start: Option<(std::time::Instant, Option<u64>)>,
    counts: [u64; 4],
}
pub(in crate::app) fn measure(phase: &'static str, owner: Owner) -> Span {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let enabled = *ENABLED.get_or_init(|| std::env::var("DH2_DEBUG_DESTRUCTION_TIMING").as_deref() == Ok("1"));
    Span { phase, owner, start: enabled.then(|| (std::time::Instant::now(), crate::loading::thread_cycles())), counts: [0; 4] }
}
impl Span {
    pub(in crate::app) fn count(&mut self, slot: usize, value: usize) {
        if self.start.is_some() { self.counts[slot] = self.counts[slot].saturating_add(value as u64); }
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        let Some((start, cycles)) = self.start else { return; };
        let cycles = cycles.zip(crate::loading::thread_cycles()).and_then(|(start, end)| end.checked_sub(start));
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        // Context and counts come from the actual request/loop. A failed timer
        // stays None, never a fabricated zero CPU-work observation.
        eprintln!("DESTRUCTION_PHASE phase={} owner={:?} wall_ms={elapsed:.3} thread_cycles={cycles:?} counts={:?}", self.phase, self.owner, self.counts);
    }
}
