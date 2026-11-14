//! Typed arkCpntFaction component state (1405bd9a0 / 1405bda20 / 1405bdad0).
use std::sync::Arc;
use crate::gameplay::{component_state::{ComponentInstance, ComponentOwner}, entity_lifetime::EntityId,
    faction_manager::{FactionManager, Definition}, faction_registry::SessionFactionRegistry,
    entity_events::{EntityEvents, Event, Payload}};

#[derive(Default)]
pub(super) struct FactionComponent {
    pub owner: ComponentOwner,
    declaration: Option<Arc<Definition>>,
    index: Option<u8>,
    registered: Option<EntityId>,
    active_name: Option<Arc<str>>,
}
impl ComponentInstance for FactionComponent {
    fn owner(&self) -> &ComponentOwner { &self.owner }
    fn owner_mut(&mut self) -> &mut ComponentOwner { &mut self.owner }
}
impl FactionComponent {
    /// The caller resolves the authored declaration through the same session
    /// class registry before the generic component Initialize completes.
    pub(super) fn set_definition(&mut self, declaration: Arc<Definition>) -> Result<(), String> {
        if self.declaration.is_some() { return Err("faction component initialized twice".into()); }
        self.declaration = Some(declaration);
        Ok(())
    }
    /// 5bd9a0 resolves the declaration's actual class slot before Start.
    pub(super) fn initialize(&mut self, registry: &SessionFactionRegistry) -> Result<(), String> {
        if self.index.is_some() { return Err("faction component already initialized".into()); }
        let declaration = self.declaration.as_ref().ok_or("faction declaration is absent")?;
        self.index = Some(registry.index_of(&declaration.name)?);
        self.owner.mark_initialized();
        Ok(())
    }
    /// 5bda20 inserts only for an actual component on a spawned entity.
    /// Base Start and its roster mutation remain in the component host.
    pub(super) fn preflight_start(&self, entity: EntityId, manager: &FactionManager) -> Result<(), String> {
        if self.registered.is_some() || self.active_name.is_some() {
            return Err("faction component started twice".into());
        }
        let declaration = self.declaration.as_ref().ok_or("faction declaration not initialized")?;
        let index = self.index.ok_or("faction component is not initialized")?;
        manager.preflight_admit(entity, &declaration.name, index)
    }
    pub(super) fn start(&mut self, entity: EntityId, manager: &mut FactionManager, events: &mut EntityEvents) -> Result<(), String> {
        self.preflight_start(entity, manager)?;
        let declaration = self.declaration.as_ref().ok_or("faction declaration not initialized")?;
        // 5bda20 copies m_factionName before adding the member.
        self.active_name = Some(declaration.faction_name.clone());
        if let Err(error) = manager.admit(entity, &declaration.name) {
            self.active_name = None;
            return Err(error);
        }
        self.registered = Some(entity);
        if self.owner.flags & (1 << 30) == 0 {
            events.post(Event { id: 0x5e, target: Some(entity), payload: Payload::None });
            events.post(Event { id: 0x23, target: Some(entity), payload: Payload::Bytes(0x1000u32.to_le_bytes().to_vec()) });
        }
        Ok(())
    }
    /// 5bdad0 removes the actual member before base Stop. Retrying after the
    /// member is gone does not remove a different entity occupying the slot.
    pub(super) fn stop(&mut self, manager: &mut FactionManager) {
        if let Some(entity) = self.registered.take() { manager.retire(entity); }
        self.active_name = None;
    }
    pub(super) fn deinitialize(&mut self) -> Result<(), String> {
        if self.registered.is_some() || self.active_name.is_some() { return Err("faction component still registered or started".into()); }
        self.index = None;
        self.declaration = None;
        Ok(())
    }
}
