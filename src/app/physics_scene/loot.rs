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
    pub(super) fn create_placed_loot(
        &mut self,
        index: usize,
        definition: &Arc<crate::gameplay::loot_component::LootDefinition>,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let declaration = self.types.declaration(
            self.factories[7], Arc::as_ptr(definition) as usize, definition.scheduling.clone(),
        )?;
        let handle = ComponentFactory {
            declaration: &declaration, pool: &mut self.loots, registry, grow_one_at_a_time: false,
        }.create(
            &mut self.entities[index].components,
            declaration.scheduling.starts_on_spawn(),
            |_, _, _| Ok(crate::gameplay::loot_component::LootComponent::new(definition.clone())),
            |_, _, _| Ok(()),
        )?.ok_or("Loot component creation disabled")?;
        self.declarations.insert(handle.packed(), declaration);
        Ok(())
    }

    /// Release the recoverable projectiles carried by a collected Pickable.
    /// The normal unbind restores the current body's mode and queues 0x125;
    /// the Throwable callback clears the same checked Loot membership.
    pub(super) fn release_collected_loot(
        &mut self, live: &mut LiveEntities, owner: EntityId, events: &mut EntityEvents,
    ) -> Result<(), String> {
        let index = *self.shared_indices.get(&owner).ok_or("collected Loot owner absent")?;
        let Some(handle) = self.loot_component_lookup(index, 7, [0x28, 0])? else { return Ok(()); };
        loop {
            let loot = self.loots.get(handle.slot).ok_or("collected Loot component absent")?;
            let child = live.children(owner)?.iter().rev().copied().find(|&child| is_recoverable(loot, child));
            let Some(child) = child else { return Ok(()); };
            self.unbind_entity(live, child, events)?;
            if live.parent(child)? == Some(owner) { return Err("collected Loot child is still bound after release".into()); }
        }
    }

    pub(super) fn loot_contains_only_recoverable_attachments(&self, index: usize, attached: &[EntityId]) -> Result<bool, String> {
        let Some(handle) = self.loot_component_lookup(index, 7, [0x28, 0])? else { return Ok(false) };
        let loot = self.loots.get(handle.slot).ok_or("portable Loot component absent")?;
        Ok(attached.iter().all(|&id| is_recoverable(loot, id)))
    }

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
        if loot.class_id != self.classes[7] || !self.is_pickable_class(pickable.class_id) {
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
        let pool = if pickable.class_id == self.classes[61] { &mut self.charm_pickables } else { &mut self.pickables };
        let pickup = pool.get_mut(pickable.slot)
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
    ///4c7570 ->7443f0: detach from the actual checked Loot owner. The
    ///ordinary item path uses76ee70(NULL)/76f020 to restore interaction.
    pub(super) fn detach_stuck_loot_member(&mut self,index:usize)->Result<(),String>{
        let Some(pickable)=self.loot_component_lookup(index,6,[0x28,0])? else{return Ok(());};
        let pickup=self.pickable(pickable).ok_or("detached Loot Pickable absent")?;
        let Some(reference)=pickup.loot_owner else{return Ok(());};
        if reference.handle.class_id!=self.classes[7]{return Err("detached checked Loot class differs".into());}
        let Some(loot)=self.loots.get(reference.handle.slot)
            .filter(|loot|loot.owner.generation==reference.generation)else{return Ok(());};
        let id=self.entities[index].shared;
        let Some(member)=loot.members.iter().position(|member|member.entity==id)else{
            //7443f0 diagnoses missing membership;166830 returns in this build.
            eprintln!("Failed to detach loot item {:?}",id);return Ok(());
        };
        //76f020's item26 branch requires the actual bound power predicate
        //4c1f20; it is outside currently admitted projectile pickup families.
        //Never silently turn that special item into an ordinary enabled item.
        if pickup.owner.flags&0xc>=8
            &&pickup.item.as_ref().map(|item|crate::assets::inventory_item::item_type_number(item.item_type())).transpose()?==Some(26){
            return Err("Loot detachment for item26 requires actual4c1f20 bound power eligibility".into());
        }
        self.entity_flags_d4[index]|=4;
        let pickup=self.pickable_mut(pickable).unwrap();
        pickup.loot_owner=None;
        if pickup.owner.flags&0xc>=8{pickup.interaction_enabled=true;}
        self.loots.get_mut(reference.handle.slot).unwrap().members.remove(member);
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
                            initial_pose: None, //Source743c30 preserves the member's actual world frame.
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

fn is_recoverable(loot: &crate::gameplay::loot_component::LootComponent, id: EntityId) -> bool {
    loot.members.iter().any(|member| member.entity == id && member.requires_incapacitated_owner && !member.collected)
}
