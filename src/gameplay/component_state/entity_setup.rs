//! 878530 instantiates the entity's resolved component declarations in order.
use super::{
    entity::{ComponentHandle, EntityComponents},
    ComponentOwner,
};

/// The level owns the live declaration array, typed factories and components.
/// Creation must use the real875d50 factory; reference lookup uses877720's policy.
pub trait EntityComponentSetup {
    fn declaration_count(&self, entity: &EntityComponents) -> Result<usize, String>;
    fn declaration_at(
        &self,
        entity: &EntityComponents,
        index: usize,
    ) -> Result<Option<usize>, String>;
    fn declaration_flags(&self, declaration: usize) -> Result<u8, String>;
    fn create_component(
        &mut self,
        entity: &mut EntityComponents,
        declaration: usize,
        immediate: bool,
    ) -> Result<Option<ComponentHandle>, String>;
    /// Entity virtualB0. Its dynamically reported prefix receives owner bit29.
    fn component_prefix_count(&mut self, entity: &mut EntityComponents) -> Result<i32, String>;
    fn component_owner_mut(
        &mut self,
        component: ComponentHandle,
    ) -> Result<&mut ComponentOwner, String>;
    fn find_reference_actor(
        &mut self,
        entity: &mut EntityComponents,
    ) -> Result<Option<ComponentHandle>, String>;
}

impl EntityComponents {
    pub fn setup_components(
        &mut self,
        world: &mut impl EntityComponentSetup,
    ) -> Result<(), String> {
        let count = world.declaration_count(self)?;
        if count > i32::MAX as usize {
            return Err("entity declaration count exceeds the signed source field".into());
        }
        // A small entity keeps the uncached lookup path; only counts >2 trigger
        // allocation of an absent class cache. Existing cache contents survive.
        if count > 2 {
            let classes = self.cached_classes.get_or_insert_with(Vec::new);
            if classes.capacity() < count {
                classes
                    .try_reserve_exact(count.saturating_sub(classes.len()))
                    .map_err(|e| format!("entity class cache allocation failed: {e}"))?;
            }
        }
        if self.entries.capacity() < count as u32
            && !self.entries.set_capacity(count as i32, true)?
        {
            return Err("entity component storage allocation failed".into());
        }
        for index in 0..count {
            // Reload the declaration identity after each factory callback, as
            // the source reloads the live array pointer while retaining count.
            if let Some(declaration) = world.declaration_at(self, index)? {
                let immediate =
                    world.declaration_flags(declaration)? & 4 != 0 && !self.start_deferred;
                if let Some(component) = world.create_component(self, declaration, immediate)? {
                    if (index as i32) < world.component_prefix_count(self)? {
                        world.component_owner_mut(component)?.flags |= 0x2000_0000;
                    }
                }
            }
        }
        self.reference_actor = world.find_reference_actor(self)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "entity_setup_tests.rs"]
mod tests;
