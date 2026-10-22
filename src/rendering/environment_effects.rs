//! FX environment bindings and retirement dispatch (21a190 and 21a3c0).
//! The FX timing/curve producer supplies the sampled blend and visibility mask.
use super::environment_overrides::EnvironmentOverrides;
use std::sync::Mutex;

struct RegistryState<K, V> {
    active: Vec<K>,
    retirements: EnvironmentEffectRetirements<V>,
}

/// Ordered FX registrations (21d720/21a4f0). The same lock protects active
/// entries and their deferred view removals. Duplicate registrations survive.
pub struct EnvironmentEffectRegistry<K, V> {
    state: Mutex<RegistryState<K, V>>,
}
impl<K, V> Default for EnvironmentEffectRegistry<K, V> {
    fn default() -> Self {
        Self {
            state: Mutex::new(RegistryState {
                active: Vec::new(),
                retirements: EnvironmentEffectRetirements::default(),
            }),
        }
    }
}
impl<K: PartialEq, V> EnvironmentEffectRegistry<K, V> {
    pub fn with_active<R>(&self, visit: impl FnOnce(&[K]) -> R) -> Result<R, String> {
        let state = self.state.lock().map_err(|e| e.to_string())?;
        Ok(visit(&state.active))
    }

    pub fn register(&self, key: K) -> Result<(), String> {
        self.state
            .lock()
            .map_err(|e| e.to_string())?
            .active
            .push(key);
        Ok(())
    }

    /// Remove the first matching registration and preserve the other entries'
    /// order. A view removal is queued only after a successful active removal.
    pub fn remove(&self, key: &K, registration: Option<(V, usize)>) -> Result<bool, String> {
        let mut state = self.state.lock().map_err(|e| e.to_string())?;
        let Some(index) = state.active.iter().position(|entry| entry == key) else {
            return Ok(false);
        };
        state.active.remove(index);
        if let Some((view, slot)) = registration {
            state.retirements.push(view, Some(slot));
        }
        Ok(true)
    }
}
impl<K, V: PartialEq> EnvironmentEffectRegistry<K, V> {
    pub fn flush_retirements(
        &self,
        live_views: &[V],
        retire: impl FnMut(&V, Option<usize>),
    ) -> Result<(), String> {
        self.state
            .lock()
            .map_err(|e| e.to_string())?
            .retirements
            .flush(live_views, retire);
        Ok(())
    }
}

pub struct EnvironmentEffectBinding<E, V> {
    environment: E,
    owner: Option<V>,
    slot: Option<usize>,
}
impl<E, V> EnvironmentEffectBinding<E, V> {
    pub fn new(environment: E) -> Self {
        Self {
            environment,
            owner: None,
            slot: None,
        }
    }
    pub fn registration(&self) -> Option<(&V, usize)> {
        self.owner.as_ref().zip(self.slot)
    }
    pub fn into_registration(self) -> Option<(V, usize)> {
        self.owner.zip(self.slot)
    }
}
impl<E: Clone, V: Clone + PartialEq> EnvironmentEffectBinding<E, V> {
    /// Register lazily on the first eligible view, then update only that same
    /// view's stable slot. The active/lifetime flag does not gate this callback.
    pub fn publish(
        &mut self,
        view: V,
        slots: &mut EnvironmentOverrides<E>,
        effect_mask: u8,
        view_mask: u8,
        blend: f32,
    ) -> Result<(), String> {
        if effect_mask & view_mask == 0 {
            return Ok(());
        }
        if self.slot.is_none() {
            self.owner = Some(view.clone());
            self.slot = Some(
                slots
                    .register(Some(self.environment.clone()))
                    .ok_or_else(|| "all environment override slots are taken".to_owned())?,
            );
        }
        if self.owner.as_ref() == Some(&view) {
            slots.set_weights(self.slot, blend, 1.);
        }
        Ok(())
    }
}

pub struct EnvironmentEffectRetirements<V> {
    pending: Vec<(V, Option<usize>)>,
}
impl<V> Default for EnvironmentEffectRetirements<V> {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
        }
    }
}
impl<V> EnvironmentEffectRetirements<V> {
    pub fn push(&mut self, view: V, slot: Option<usize>) {
        self.pending.push((view, slot));
    }
}
impl<V: PartialEq> EnvironmentEffectRetirements<V> {
    /// Only callbacks belonging to a currently live view may be invoked. All
    /// entries are consumed, including stale views and duplicate registrations.
    pub fn flush(&mut self, live_views: &[V], mut retire: impl FnMut(&V, Option<usize>)) {
        for (view, slot) in self.pending.drain(..) {
            if live_views.contains(&view) {
                retire(&view, slot);
            }
        }
    }
}

#[cfg(test)]
#[path = "environment_effects/tests.rs"]
mod tests;
