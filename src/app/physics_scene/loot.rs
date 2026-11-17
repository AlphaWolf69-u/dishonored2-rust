//!623a70 ->743c30 runtime recovered-projectile Loot membership.
use super::*;
use crate::gameplay::{
    component_state::entity_lookup::ComponentLookupState,
    entity_events::EntityEvents,
    entity_lifetime::{EntityId, LiveEntities},
    loot_component::LootMember,
    name_pool::NameId,
    pickable_component::CheckedComponent,
};
impl PhysicsScene {
    pub(super) fn loot_component_lookup(
        &self,
        index: usize,
        factory: usize,
        masks: [u32; 2],
    ) -> Result<Option<ComponentHandle>, String> {
        let range = self
            .types
            .classes()
            .hierarchy()
            .type_range(self.types.factory(self.factories[factory])?.class)?;
        self.entities
            .get(index)
            .ok_or("Loot lookup entity absent")?
            .components
            .find_component(range, masks, |handle| {
                let owner = self.owner(handle)?;
                Ok(ComponentLookupState {
                    flags: owner.flags,
                    start_phases: self
                        .declarations
                        .get(&handle.packed())
                        .map(|decl| decl.scheduling.start_phases),
                })
            })
    }
    ///623a70 looks up target Loot in policy2; absence is a normal outcome.
    ///743c30 requires the actual projectile Pickable in policy0 and appends
    ///one record with atomic empty tag and sourceflagtrue. No inventory copy.
    pub(in crate::app) fn register_stuck_projectile(
        &mut self,
        live: &mut LiveEntities,
        _events: &mut EntityEvents,
        projectile: EntityId,
        target: EntityId,
    ) -> Result<(), String> {
        if live.get(projectile).is_none() || live.get(target).is_none() {
            return Err("stuck Loot managed entity stale".into());
        }
        let target_index = *self
            .shared_indices
            .get(&target)
            .ok_or("stuck Loot target header absent")?;
        let Some(loot) = self.loot_component_lookup(target_index, 7, [0x28, 0])? else {
            return Ok(());
        };
        let projectile_index = *self
            .shared_indices
            .get(&projectile)
            .ok_or("stuck Loot projectile header absent")?;
        let pickable = self
            .loot_component_lookup(projectile_index, 6, [0, 0])?
            .ok_or("Trying to add a Loot item that has no Pickable component")?;
        if loot.class_id != self.classes[7] || pickable.class_id != self.classes[6] {
            return Err("Loot handoff has an unadmitted derived payload".into());
        }
        let component = self
            .loots
            .get_mut(loot.slot)
            .ok_or("stuck Loot payload absent")?;
        if component.retiring.is_some() {
            return Err("cannot append to retiring Loot".into());
        }
        component
            .members
            .try_reserve(1)
            .map_err(|error| format!("Loot member list: {error}"))?;
        let reference = CheckedComponent {
            handle: loot,
            generation: component.owner.generation,
        };
        //76ee70 checked-pointer publication; its e8 virtual is76f020 for
        //Pickable_Simple. A nonnull Loot always disables the interaction.
        let pickup = self
            .pickables
            .get_mut(pickable.slot)
            .ok_or("Loot Pickable payload absent")?;
        pickup.loot_owner = Some(reference);
        if pickup.owner.flags & 0xc >= 8 {
            pickup.interaction_enabled = false;
        }
        component.members.push(LootMember {
            entity: projectile,
            tag: NameId::empty(),
            requires_incapacitated_owner: true,
            collected: false,
        });
        Ok(())
    }
    ///744b30 for the admitted authored-empty declarations. The existing
    ///resource reader rejects child itemDefs before admission; dynamically
    ///registered items stay in the same member list across Stop/Start.
    pub(super) fn initialize_loot(&mut self, handle: ComponentHandle) -> Result<(), String> {
        let component = self
            .loots
            .get_mut(handle.slot)
            .ok_or("Loot Initialize payload absent")?;
        component.owner.mark_initialized();
        component.retiring = None;
        component.starting = None;
        Ok(())
    }
    ///744f90 ->875e80/752900. Runtime recovered projectile tags are empty;
    ///the source's null offset/orientation keeps their current world pose.
    pub(super) fn start_loot(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        registry: &mut ComponentRegistry,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let component = self
            .loots
            .get(handle.slot)
            .ok_or("Loot Start payload absent")?;
        let owner_index = component.owner.entity.ok_or("Loot Start entity absent")?;
        let parent = self.entities[owner_index].shared;
        loop {
            let component = self
                .loots
                .get_mut(handle.slot)
                .ok_or("Loot Start payload absent")?;
            let cursor = *component.starting.get_or_insert(0);
            let Some(member) = component.members.get(cursor).copied() else {
                component.starting = None;
                return Ok(());
            };
            if live.get(member.entity).is_some() {
                let index = *self
                    .shared_indices
                    .get(&member.entity)
                    .ok_or("Loot member header absent")?;
                if self.owner(handle)?.flags & (1 << 30) == 0
                    || self.entity_flags_d4[index] & 4 == 0
                {
                    //No current entity producer admits source87caa0's disabled
                    //D1bit0 state. Do not silently acknowledge its missing wake.
                    if self.entities[index].entity_flags & 0x100 != 0 {
                        return Err("Loot disabled member requires87caa0 activation owner".into());
                    }
                    if member.tag != NameId::empty() {
                        return Err(
                            "Loot authored tag requires admitted JointFrames binding".into()
                        );
                    }
                    let (offset, orientation) = crate::gameplay::reference_frame::relative_binding(
                        self.entities[owner_index].frame.pose,
                        self.entities[index].frame.pose,
                    );
                    self.bind_entity_in_registry(
                        live,
                        member.entity,
                        parent,
                        super::throwable::BindingPose {
                            joint: -1,
                            offset,
                            orientation,
                            parent_placement: None,
                        },
                        events,
                        registry,
                    )?;
                }
            }
            self.loots.get_mut(handle.slot).unwrap().starting = Some(cursor + 1);
        }
    }
    ///744ef0 removes live uncollected members in ascending list order before
    ///the base component Deinitialize. Pool destruction then drops this Vec;
    ///member IDs retain neither decoded assets nor live entity resources.
    pub(super) fn deinitialize_loot(
        &mut self,
        live: &mut LiveEntities,
        _events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        if handle.class_id != self.classes[7] {
            return Err("Loot Deinitialize owner differs".into());
        }
        loop {
            let component = self
                .loots
                .get_mut(handle.slot)
                .ok_or("Loot retiring payload absent")?;
            let cursor = *component.retiring.get_or_insert(0);
            let Some(member) = component.members.get(cursor).copied() else {
                return Ok(());
            };
            if !member.collected && live.get(member.entity).is_some() {
                live.request_removal(member.entity)?;
                self.dispatch_remove_notice(live, member.entity)?;
            }
            self.loots.get_mut(handle.slot).unwrap().retiring = Some(cursor + 1);
        }
    }
}
