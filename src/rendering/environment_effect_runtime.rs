//! Connect the FX pool/controller lifetime to ordered per-view publication.
//! The parent FX scheduler owns activation, shared RNG and group head indices.
use super::{
    environment_effects::{EnvironmentEffectBinding, EnvironmentEffectRegistry},
    environment_overrides::EnvironmentOverrides,
};
use crate::gameplay::effects::{
    environment::{EnvironmentBlendEffect, EnvironmentEffectDefinition},
    groups::update_environment_chain,
    pool::EffectPoolLinks,
};
use std::sync::Arc;

pub struct EnvironmentEffectRuntime<E, V> {
    pool: EffectPoolLinks,
    effects: Vec<Option<EnvironmentBlendEffect>>,
    bindings: Vec<Option<EnvironmentEffectBinding<E, V>>>,
    registry: EnvironmentEffectRegistry<u16, V>,
}
impl<E, V> Default for EnvironmentEffectRuntime<E, V> {
    fn default() -> Self {
        Self::new(EffectPoolLinks::environment()).expect("fixed environment FX pool is valid")
    }
}
impl<E, V> EnvironmentEffectRuntime<E, V> {
    /// The pool owner supplies its initialized link layout and capacity. This
    /// constructor does not choose game limits or guess free-list ordering.
    pub fn new(pool: EffectPoolLinks) -> Result<Self, String> {
        if pool.allocated_count != 0 || pool.next.len() > usize::from(u16::MAX) {
            return Err("environment runtime requires an empty, addressable FX pool".into());
        }
        let count = pool.next.len();
        Ok(Self {
            pool,
            effects: (0..count).map(|_| None).collect(),
            bindings: (0..count).map(|_| None).collect(),
            registry: EnvironmentEffectRegistry::default(),
        })
    }

    /// The environment family of 222f80, after its enabled/resource checks and
    /// group offset sampling. Exhaustion leaves the shared RNG untouched.
    pub fn start(
        &mut self,
        head: &mut u16,
        environment: E,
        definition: impl Into<Arc<EnvironmentEffectDefinition>>,
        offsets: [f32; 4],
        view_mask: u8,
        seed: &mut u32,
    ) -> Result<bool, String> {
        if !self.pool.acquire(head)? {
            return Ok(false);
        }
        let index = usize::from(*head);
        self.effects[index] = Some(EnvironmentBlendEffect::new(
            definition, offsets, view_mask, seed,
        )?);
        self.bindings[index] = Some(EnvironmentEffectBinding::new(environment));
        self.registry.register(*head)?;
        Ok(true)
    }

    pub fn stop(&mut self, id: u16, now: f32) -> Result<(), String> {
        self.effects
            .get_mut(usize::from(id))
            .and_then(Option::as_mut)
            .ok_or_else(|| format!("environment effect {id} is not initialized"))?
            .stop(now);
        Ok(())
    }
}
impl<E: Clone, V: Clone + PartialEq> EnvironmentEffectRuntime<E, V> {
    pub fn update_group(
        &mut self,
        head: &mut u16,
        now: f32,
        view_mask: u8,
        seed: &mut u32,
    ) -> Result<u32, String> {
        let bindings = &self.bindings;
        let registry = &self.registry;
        update_environment_chain(
            &mut self.pool,
            head,
            &mut self.effects,
            now,
            view_mask,
            seed,
            |id, _| {
                let binding = bindings[usize::from(id)]
                    .as_ref()
                    .ok_or_else(|| format!("environment effect {id} has no binding"))?;
                let registration = binding
                    .registration()
                    .map(|(view, slot)| (view.clone(), slot));
                registry.remove(&id, registration)?;
                Ok(())
            },
        )
    }

    /// 21a360 visits registration order, independently of group update order.
    /// Newly inactive effects still publish until their next lifecycle update.
    pub fn publish(
        &mut self,
        view: V,
        view_mask: u8,
        slots: &mut EnvironmentOverrides<E>,
    ) -> Result<(), String> {
        self.registry.with_active(|ids| {
            for &id in ids {
                let index = usize::from(id);
                let effect = self.effects[index]
                    .as_ref()
                    .ok_or("registered environment effect is absent")?;
                self.bindings[index]
                    .as_mut()
                    .ok_or("registered environment binding is absent")?
                    .publish(
                        view.clone(),
                        slots,
                        effect.view_mask,
                        view_mask,
                        effect.blend,
                    )?;
            }
            Ok(())
        })?
    }

    pub fn flush_retirements(
        &self,
        live_views: &[V],
        retire: impl FnMut(&V, Option<usize>),
    ) -> Result<(), String> {
        self.registry.flush_retirements(live_views, retire)
    }
}

#[cfg(test)]
#[path = "environment_effect_runtime/tests.rs"]
mod tests;
