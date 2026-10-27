//! Process-local identity allocation shared by every transient sound producer.
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
use crate::gameplay::sound::Emitter;

/// Clones cross the main/render boundary without creating another ID namespace.
#[derive(Clone, Default)]
#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
pub struct EffectObjects(Arc<AtomicU64>);

impl EffectObjects {
    pub fn allocate(&self) -> Result<Emitter, String> {
        self.0.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map(|id| Emitter::Effect(id + 1))
            .map_err(|_| "transient sound object identities exhausted".into())
    }
}
