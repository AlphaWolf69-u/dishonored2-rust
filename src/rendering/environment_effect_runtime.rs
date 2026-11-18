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
use std::sync::{Arc, Mutex, MutexGuard};

pub const ENVIRONMENT_CAPACITY:usize = 64;

/// SAME native19a1d0 allocator for shared world, pause and focus records.
#[derive(Clone)]
pub struct SharedEnvironmentPool(Arc<Mutex<EffectPoolLinks>>);
impl SharedEnvironmentPool {
    fn new(pool:EffectPoolLinks)->Self { Self(Arc::new(Mutex::new(pool))) }
    fn lock(&self)->Result<MutexGuard<'_ ,EffectPoolLinks>,String>{self.0.lock().map_err(|_| "environment family pool is poisoned".to_owned())}
    pub fn is_same(&self,other:&Self)->bool{Arc::ptr_eq(&self.0,&other.0)}
    ///221b80: admission precedes21a240 producer parameter/RNG sampling.
    pub fn acquire_external(&self)->Result<Option<EnvironmentPoolLease>,String>{
        let mut head=u16::MAX;
        if !self.lock()?.acquire(&mut head)? {return Ok(None);}
        Ok(Some(EnvironmentPoolLease {pool:self.clone(),head}))
    }
}
/// A real standalone head in the same native64-slot family, not a permit count.
pub struct EnvironmentPoolLease {pool:SharedEnvironmentPool,head:u16}
impl EnvironmentPoolLease {
    pub fn index(&self)->u16{self.head}
    pub fn release(&mut self)->Result<(),String>{
        if self.head!=u16::MAX {self.pool.lock()?.release_head(&mut self.head)?;}
        Ok(())
    }
}
impl Drop for EnvironmentPoolLease {
    fn drop(&mut self){
        // RAII is the host owner for a native singleton chain. Canonical close
        // paths explicitly release while errors can still propagate.
        if let Err(error)=self.release(){ eprintln!("Environment family lease retirement failed: {error}"); }
    }
}


pub struct EnvironmentEffectRuntime<E, V> {
    pool: SharedEnvironmentPool,
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
            pool: SharedEnvironmentPool::new(pool),
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
        let mut pool = self.pool.lock()?;
        if !pool.acquire(head)? {
            return Ok(false);
        }
        let index = usize::from(*head);
        self.effects[index] = match EnvironmentBlendEffect::new(definition, offsets, view_mask, seed) {
            Ok(effect) => Some(effect),
            Err(error) => { pool.release_head(head)?; return Err(error); }
        };
        self.bindings[index] = Some(EnvironmentEffectBinding::new(environment));
        if let Err(error) = self.registry.register(*head) {
            // No registration was published. Preserve owned fields if release fails.
            pool.release_head(head)?;
            self.bindings[index] = None;
            self.effects[index] = None;
            return Err(error);
        }
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
        if *head == u16::MAX { return Ok(0); }
        let bindings = &mut self.bindings;
        let registry = &self.registry;
        let result = update_environment_chain(
            &mut *self.pool.lock()?,
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
                bindings[usize::from(id)] = None;
                Ok(())
            },
        );
        result
    }

    pub fn stop_chain(&mut self, mut head: u16, now: f32, looping_only: bool) -> Result<(), String> {
        if head == u16::MAX { return Ok(()); }
        let pool = self.pool.lock()?;
        let mut visited = 0;
        while head != u16::MAX {
            visited += 1;
            if visited > self.effects.len() { return Err("environment stop chain cycle".into()); }
            let effect = self.effects.get_mut(usize::from(head)).and_then(Option::as_mut)
                .ok_or("environment stop runtime absent")?;
            if !looping_only || effect.can_loop() { effect.stop(now); }
            head = pool.next[usize::from(head)];
        }
        Ok(())
    }
    pub fn clear(&mut self, head: &mut u16) -> Result<(), String> {
        if *head == u16::MAX { return Ok(()); }
        let mut pool = self.pool.lock()?;
        let mut visited = 0;
        while *head != u16::MAX {
            visited += 1;
            if visited > self.effects.len() { return Err("environment clear chain cycle".into()); }
            let index = usize::from(*head);
            // The same locked canonical allocator makes the later release
            // infallible after this index validation; no owner is lost on Err.
            pool.next.get(index).ok_or("environment clear pool index absent")?;
            let binding = self.bindings.get(index).and_then(Option::as_ref)
                .ok_or("environment clear binding absent")?;
            let registration = binding.registration().map(|(view, slot)| (view.clone(), slot));
            if !self.registry.remove(head, registration)? { return Err("environment clear registration absent".into()); }
            self.bindings[index] = None;
            self.effects[index] = None;
            pool.release_head(head)?;
        }
        Ok(())
    }
    pub fn drained(&self) -> bool { self.pool.0.lock().map(|pool| pool.allocated_count == 0).unwrap_or(false) }
    pub fn shared_pool(&self)->SharedEnvironmentPool{self.pool.clone()}

    ///21d720 append identity is shared with other actual environment producers.
    pub fn visit_orders(&self, mask:u8, mut visit:impl FnMut(u64))->Result<(),String>{
        self.registry.with_active(|ids| {
            for &id in ids {
                let effect=self.effects[usize::from(id)].as_ref().ok_or("registered environment runtime absent")?;
                if effect.view_mask & mask != 0 { visit(effect.registration_order); }
            }
            Ok(())
        })?
    }
    pub fn publish_order(&mut self,order:u64,view:V,mask:u8,slots:&mut EnvironmentOverrides<E>)->Result<(),String>{
        self.registry.with_active(|ids| {
            for &id in ids {
                let index=usize::from(id);
                let effect=self.effects[index].as_ref().ok_or("registered environment runtime absent")?;
                if effect.registration_order==order {
                    return self.bindings[index].as_mut().ok_or("registered environment binding absent")?
                        .publish(view,slots,effect.view_mask,mask,effect.blend);
                }
            }
            Err(format!("environment registration {order} is no longer live"))
        })?
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
