//!716560/7e72c0 at the shared world's actual joint-removal acknowledgement.
//!This borrowed host never drains other classes or creates another registry.
use super::*;
use crate::gameplay::{
    component_state::{ComponentInstance, lifecycle::{ComponentReleaseWorld, ComponentLifecycleWorld},
        updates::ComponentUpdateWorld, types::ComponentTypes},
    constraint::Component,
    entity_events::{EntityEvents, Listener},
    physics_world::{ConstraintChange, PublicationEvent},
    signal::Signal,
};

///Both edit paths belong to the SAME EntityEvents owner. During its pump the
///existing Commands object applies edits before the next queued event.
pub(super) trait ListenerEdits {
    fn unregister(&mut self, event: u16,
        target: Option<crate::gameplay::entity_lifetime::EntityId>, listener: Listener);
}
impl ListenerEdits for EntityEvents {
    fn unregister(&mut self, event: u16,
        target: Option<crate::gameplay::entity_lifetime::EntityId>, listener: Listener) {
        EntityEvents::unregister(self, event, target, listener);
    }
}
impl ListenerEdits for crate::gameplay::entity_events::Commands {
    fn unregister(&mut self, event: u16,
        target: Option<crate::gameplay::entity_lifetime::EntityId>, listener: Listener) {
        crate::gameplay::entity_events::Commands::unregister(self, event, target, listener);
    }
}
pub(super) struct RemovalHost<'a> {
    pub owner_id: usize,
    pub class: i32,
    pub factory: u32,
    pub types: &'a ComponentTypes,
    pub pool: &'a mut ComponentPool<Component>,
    pub entities: &'a mut Vec<PhysicsEntity>,
    pub events: &'a mut dyn ListenerEdits,
}
impl RemovalHost<'_> {
    fn component(&self, handle: ComponentHandle) -> Result<&Component, String> {
        if handle.class_id != self.class { return Err("constraint callback class differs".into()); }
        self.pool.get(handle.slot).ok_or_else(|| "constraint callback slot absent".into())
    }
    fn component_mut(&mut self, handle: ComponentHandle) -> Result<&mut Component, String> {
        if handle.class_id != self.class { return Err("constraint callback class differs".into()); }
        self.pool.get_mut(handle.slot).ok_or_else(|| "constraint callback slot absent".into())
    }
    /// Subscribe using the actual packed component handle and its generation.
    /// The typed Removed signal itself identifies this source callback.
    pub(super) fn removed(&mut self, signal: &mut Signal<usize, usize>,
        subscriber: usize, generation: usize, event: PublicationEvent<'_>,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let PublicationEvent::Constraint { slot, generation: joint_generation,
            change: ConstraintChange::Removed } = event else {
                return Err("constraint removal callback received another signal".into());
            };
        let handle = ComponentHandle::from_packed(subscriber as u64)?;
        let component = self.component(handle)?;
        if component.owner.generation as usize != generation {
            return Err("constraint removal component generation differs".into());
        }
        let joint = component.joint.or(component.retiring_joint)
            .ok_or("constraint removal receipt has no actual joint")?;
        if joint.slot() != slot || joint.generation() != joint_generation {
            return Err("constraint removal joint generation differs".into());
        }
        //The source invalidates +80 before calling7e72c0. Preserve the same
        //receipt until that fallible canonical release request acknowledges.
        let component = self.component_mut(handle)?;
        component.joint = None;
        component.retiring_joint = Some(joint);
        component.pending_enable = false;
        self.withdraw_entity_listeners(handle)?;
        registry.request_release(handle, self)?;
        if let Some(component) = self.pool.get_mut(handle.slot) {
            if component.owner.generation as usize == generation {
                component.retiring_joint = None;
            }
        }
        //Do not unsubscribe this in-flight signal before a failed request:
        //its real resumable traversal must retry this same component first.
        signal.unsubscribe(subscriber, generation);
        Ok(())
    }
    fn withdraw_entity_listeners(&mut self, handle: ComponentHandle) -> Result<(), String> {
        let component = self.component(handle)?;
        let index = component.owner.entity.ok_or("constraint listener owner absent")?;
        let id = self.entities.get(index).ok_or("constraint listener entity absent")?.shared;
        let generation = component.owner.generation;
        for event in [0xad, 0xac] {
            self.events.unregister(event, Some(id), Listener::Component {
                owner: self.owner_id, handle, generation, handler: event,
            });
        }
        Ok(())
    }
    fn unlink(&mut self, handle: ComponentHandle) -> Result<(), String> {
        let component = self.component(handle)?;
        if component.joint.is_some() {
            return Err("constraint unlink precedes actual joint removal".into());
        }
        let Some(index) = component.owner.entity else { return Ok(()); };
        let component = self.pool.get_mut(handle.slot).ok_or("constraint unlink slot absent")?;
        component.initialized = false;
        self.entities.get_mut(index).ok_or("constraint unlink entity absent")?
            .components.release(handle, component)
    }
}
impl ComponentLifecycleWorld for RemovalHost<'_> {
    fn priority(&self, handle: ComponentHandle) -> Result<i32, String> {
        self.component(handle)?;
        self.types.initialization_rank(self.factory)
    }
    fn owner(&self, handle: ComponentHandle) -> Result<&ComponentOwner, String> {
        Ok(&self.component(handle)?.owner)
    }
    fn owner_mut(&mut self, handle: ComponentHandle) -> Result<&mut ComponentOwner, String> {
        Ok(&mut self.component_mut(handle)?.owner)
    }
    fn initialize(&mut self, _: ComponentHandle, _: &mut ComponentRegistry) -> Result<(), String> {
        Err("constraint removal host cannot initialize components".into())
    }
    fn start(&mut self, _: ComponentHandle, _: &mut ComponentRegistry) -> Result<(), String> {
        Err("constraint removal host cannot start components".into())
    }
    fn stop_queued(&mut self, handle: ComponentHandle, _: &mut ComponentRegistry) -> Result<(), String> {
        self.component(handle)?;
        Ok(()) //source base virtual40; no body or joint mutation.
    }
    fn initialization_failed(&mut self, _: ComponentHandle, _: &mut ComponentRegistry) -> Result<(), String> {
        Err("constraint removal host cannot process failed initialization".into())
    }
}
impl ComponentUpdateWorld for RemovalHost<'_> {
    fn update_remove(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String> {
        registry.base_update_remove(handle, self)
    }
    fn update_add(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String> {
        registry.base_update_add(handle, self)
    }
    fn set_update_scheduled(&mut self, handle: ComponentHandle, scheduled: bool) -> Result<(), String> {
        self.component(handle)?;
        if scheduled { self.pool.schedule(handle.slot) } else { self.pool.unschedule(handle.slot) }
    }
}
impl ComponentReleaseWorld for RemovalHost<'_> {
    fn stop(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String> {
        self.component_mut(handle)?.started = false;
        registry.stop_update_ownership(handle, self)
    }
    fn deinitialize(&mut self, handle: ComponentHandle, _: &mut ComponentRegistry) -> Result<(), String> {
        self.unlink(handle)
    }
    fn shutdown(&mut self, handle: ComponentHandle, _: &mut ComponentRegistry) -> Result<(), String> {
        self.unlink(handle)
    }
    fn released(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String> {
        registry.ensure_no_update_references(handle)?;
        let component = self.component(handle)?;
        if component.owner.entity.is_some() || component.joint.is_some() {
            return Err("constraint pool release retains actual owner/joint".into());
        }
        self.pool.release(handle.slot)
    }
}
