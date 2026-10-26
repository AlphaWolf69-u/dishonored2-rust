//! Entity component construction and immediate-start orchestration (875d50).
use super::{order::insertion_slot, pool::ComponentPool, ComponentInstance, ComponentRegistry};
use crate::storage::PointerList;

/// Class and slot replace a process pointer. Class metadata is immutable for
/// the component's lifetime; generation remains in its single owner record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComponentHandle {
    pub class_id: i32,
    pub slot: u16,
}
impl ComponentHandle {
    pub fn packed(self) -> u64 {
        (u64::from(self.class_id as u32) << 32) | u64::from(self.slot)
    }
    pub fn from_packed(value: u64) -> Result<Self, String> {
        if value & 0xffff0000 != 0 {
            return Err("component handle contains an invalid pool slot".into());
        }
        Ok(Self {
            class_id: (value >> 32) as i32,
            slot: value as u16,
        })
    }
}

/// These values come from the actual declaration/type metadata, not allocation
/// tags. In particular, factory allocation tag 0x41 is not a class ID.
pub struct ComponentDeclaration {
    pub identity: usize,
    pub class_id: i32,
    pub scheduling: super::declaration::DeclarationScheduling,
}
impl ComponentDeclaration {
    pub fn from_class(
        identity: usize,
        class: usize,
        hierarchy: &crate::gameplay::class_hierarchy::ClassHierarchy,
        scheduling: super::declaration::DeclarationScheduling,
    ) -> Result<Self, String> {
        Ok(Self {
            identity,
            class_id: hierarchy.type_id(class)?,
            scheduling,
        })
    }
}

/// The entity owner supplies its construction/lifecycle state. No entity
/// constructor defaults or component document order are assumed here.
pub struct EntityComponents {
    pub identity: usize,
    pub creation_disabled: bool,
    pub start_deferred: bool,
    pub phase: u32,
    pub entries: PointerList,
    pub cached_classes: Option<Vec<u16>>,
    pub reference_actor: Option<ComponentHandle>,
}

impl EntityComponents {
    /// Component-facing fields of 874c50. The full entity's other owners are
    /// constructed separately; this does not replace its full constructor.
    pub fn new(identity: usize) -> Self {
        Self {
            identity,
            creation_disabled: false,
            start_deferred: false,
            phase: 0,
            entries: PointerList::owned(),
            cached_classes: None,
            reference_actor: None,
        }
    }

    /// 878e80 removes the first matching component and the corresponding
    /// cached class, if present. Unequal cache lengths are preserved.
    pub fn remove(&mut self, component: ComponentHandle) -> Result<bool, String> {
        let Some(index) = self
            .entries
            .values()
            .iter()
            .position(|&v| v == component.packed())
        else {
            return Ok(false);
        };
        self.entries.remove_at(index as i32)?;
        if let Some(classes) = &mut self.cached_classes {
            if index < classes.len() {
                classes.remove(index);
            }
        }
        Ok(true)
    }

    /// 7e7530: unlink from the owning entity before clearing the component's
    /// attachment flags and owner references. Generation and phase survive.
    pub fn release<T: ComponentInstance>(
        &mut self,
        handle: ComponentHandle,
        value: &mut T,
    ) -> Result<(), String> {
        if value.owner().entity != Some(self.identity) {
            return Err("component release entity does not match its owner".into());
        }
        self.remove(handle)?;
        let owner = value.owner_mut();
        owner.flags &= !3;
        owner.declaration = None;
        owner.entity = None;
        Ok(())
    }
}

pub struct ComponentFactory<'a, T> {
    pub declaration: &'a ComponentDeclaration,
    pub pool: &'a mut ComponentPool<T>,
    pub registry: &'a mut ComponentRegistry,
    pub grow_one_at_a_time: bool,
}
impl<T: ComponentInstance> ComponentFactory<'_, T> {
    /// Complete875d50. Both type construction and its virtual38 handler are
    /// required. Cache insertion precedes construction; list insertion precedes
    /// owner linking; immediate startup observes the fully linked component.
    pub fn create(
        &mut self,
        entity: &mut EntityComponents,
        immediate: bool,
        construct: impl FnOnce(u16, &EntityComponents, &ComponentRegistry) -> Result<T, String>,
        mut on_start_queued: impl FnMut(
            &mut ComponentRegistry,
            &mut T,
            &mut EntityComponents,
        ) -> Result<(), String>,
    ) -> Result<Option<ComponentHandle>, String> {
        if entity.creation_disabled {
            return Ok(None);
        }
        let position = insertion_slot(
            &entity.entries,
            self.declaration.class_id,
            entity.cached_classes.as_mut(),
            |value| Ok(ComponentHandle::from_packed(value)?.class_id),
        )?;
        let slot = self.registry.allocate(self.pool, |slot, registry| {
            construct(slot, entity, registry)
        })?;
        let handle = ComponentHandle {
            class_id: self.declaration.class_id,
            slot,
        };
        let identity = usize::try_from(handle.packed())
            .map_err(|_| "component handle exceeds host pointer width")?;
        entity
            .entries
            .insert(position as u32, handle.packed(), self.grow_one_at_a_time)?;
        let value = self
            .pool
            .get_mut(slot)
            .ok_or("new entity component is missing")?;
        self.registry.register(
            value.owner_mut(),
            identity,
            self.declaration.identity,
            entity.identity,
            self.grow_one_at_a_time,
        )?;
        if immediate
            && !entity.start_deferred
            && (self.declaration.scheduling.start_phases >> (entity.phase & 31)) & 1 != 0
        {
            self.registry.request_component_start(
                value,
                identity,
                self.grow_one_at_a_time,
                |registry, value| on_start_queued(registry, value, entity),
            )?;
        }
        Ok(Some(handle))
    }
}

#[cfg(test)]
#[path = "entity_tests.rs"]
mod tests;
