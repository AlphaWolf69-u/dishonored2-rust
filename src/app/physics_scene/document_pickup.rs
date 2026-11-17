//! 844bf0 / 76e2b0 / 76ec20: admitted document pickup, body and render ownership.
use super::*;
use crate::gameplay::{entity_lifetime::{EntityId, LiveEntities}, pickable_component::CheckedComponent};

impl PhysicsScene {
    pub(super) fn document_pickable(&self, live: &LiveEntities, reference: CheckedComponent) -> Result<(usize, EntityId), String> {
        if reference.handle.class_id != self.classes[6] { return Err("pickup is not PickableSimple".into()); }
        let owner = self.owner(reference.handle)?;
        if owner.generation != reference.generation || owner.flags & 0xf != 0xa {
            return Err("document Pickable is stale or stopped".into());
        }
        let index = owner.entity.ok_or("document pickup is unlinked")?;
        let id = self.entities[index].shared;
        if live.stage(id) != Some(crate::gameplay::entity_lifetime::Stage::Spawned) {
            return Err("document pickup identity is stale".into());
        }
        Ok((index,id))
    }

    pub(super) fn document_rigid(&self, index: usize) -> Result<Option<ComponentHandle>, String> {
        self.entities[index].components.find_component(self.rigid_class_range()?, [0x20,0], |handle| {
            Ok(crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                flags: self.owner(handle)?.flags,
                start_phases: self.declarations.get(&handle.packed()).map(|declaration| declaration.scheduling.start_phases),
            })
        })
    }

    pub(in crate::app) fn preflight_document_pickup(&self, live: &LiveEntities, reference: CheckedComponent,
        sources: &super::super::shadow_updates::ModelShadowSources,
        placements: &super::super::RenderSourcePlacements) -> Result<(),String> {
        let (index,id) = self.document_pickable(live,reference)?;
        let item = self.pickables.get(reference.handle.slot).and_then(|component| component.item.as_ref())
            .ok_or("document item declaration absent")?;
        if !matches!(item.item_type(), "TYPE_NOTE_DOCUMENT" | "12" | "TYPE_BOOK" | "25" | "TYPE_INTERACTIVE_MAP" | "24") {
            return Err("styled portable item requires actual item-specific targeting callbacks".into());
        }
        self.validate_document_hierarchy(live, index)?;
        if !live.children(id)?.is_empty() { return Err("portable document descendants require their actual hide virtuals".into()); }
        let entity = &self.entities[index];
        let metadata = self.types.classes().metadata(entity.class).ok_or("document entity class absent")?;
        if !matches!(self.types.classes().name_text(metadata.name), Some(b"idEntity" | b"arkPickup")) {
            return Err("document hide requires actual derived entity virtual".into());
        }
        if !sources.0.iter().any(|source| placements.0.get(source.placement).is_some_and(|model| model.id == entity.name)) {
            return Err("document render source is not ready".into());
        }
        if let Some(handle) = self.document_rigid(index)? {
            let component = self.rigid_component(handle)?;
            if component.freeze_state != 0 { return Err("frozen document requires actual thaw callback".into()); }
            let body = component.body().ok_or("document body absent")?;
            let world = self.world.read().map_err(|_| "level physics query owner is poisoned")?;
            world.storage.preflight_keyframe_single_body(body)?;
            if !world.storage.publication_idle() { return Err("document body publication is pending".into()); }
            let movement = component.movement_body()?;
            let actor = self.actors.bodies.get(movement).and_then(|body| body.actor).ok_or("document actor absent")?;
            self.actors.actors.get_at(actor).ok_or("document actor is stale")?;
        }
        Ok(())
    }

    pub(in crate::app) fn begin_document_pickup(&mut self, live: &mut LiveEntities, events: &mut crate::gameplay::entity_events::EntityEvents, reference: CheckedComponent, step: &mut u8)
        -> Result<Option<Arc<crate::assets::wwise::SoundEventDeclaration>>,String> {
        let (index,_) = self.document_pickable(live,reference)?;
        if *step == 0 {
            self.begin_projectile_pickup(live, events, self.entities[index].shared)?;
            self.begin_grenade_pickup(live,self.entities[index].shared)?;
        }
        let mut registry = self.registry.take().ok_or("document registry absent")?;
        let result = (|| {
            // 844bf0 calls the real 8762d0 attachment owner before changing
            // the body type/filter. Repeated pending callbacks see null parent.
            if *step == 0 {
                if live.parent(self.entities[index].shared)?.is_some() {
                    self.pickables.get_mut(reference.handle.slot).ok_or("bound Pickable absent")?.pickup_parent=live.parent(self.entities[index].shared)?;
                    // 8762d0 clears d0 bit7, then727450(null)/727880 restores
                    // the live movement interface to type2 before parent links
                    // disappear. The same mode cursor resumes a failed change.
                    self.entities[index].entity_flags &= !0x80;
                    if let Some(handle) = self.document_rigid(index)? {
                        if self.owner(handle)?.flags & 3 > 1 {
                            let body = self.rigid_component(handle)?.body().ok_or("bound document body absent")?;
                            let shared = self.world.clone();
                            let mut guard = shared.write().map_err(|_| "level physics query owner is poisoned")?;
                            let world = &mut *guard;
                            scene_publication_events!(self, &mut registry, events);
                            world.storage.dynamic_single_body(&mut world.geometry, &mut events, body)?;
                        }
                    }
                }
                self.detach_document(live, index)?;
            }
            if let Some(handle) = self.document_rigid(index)? {
                let body = self.rigid_component(handle)?.body().ok_or("document body retired")?;
                let shared = self.world.clone();
                let mut guard = shared.write().map_err(|_| "level physics query owner is poisoned")?;
                let world = &mut *guard;
                if *step == 0 {
                    { scene_publication_events!(self, &mut registry, events);
                      world.storage.keyframe_single_body(&mut world.geometry,&mut events,body)?; }
                    *step = 1;
                }
                let filter = world.storage.bodies.get(body).ok_or("document body retired")?.filter_info & 0x0820_001f;
                if *step == 1 {
                    { scene_publication_events!(self, &mut registry, events);
                      world.storage.set_body_filter(&mut world.geometry,&mut events,body,filter)?; }
                    *step = 2;
                }
                self.entity_flags_d4[index] |= 4;
                if *step == 2 {
                    { scene_publication_events!(self, &mut registry, events);
                      world.storage.set_body_velocity(&mut world.geometry,&mut events,body,[0.;4],[0.;4],0)?; }
                    *step = 3;
                }
            } else if *step == 0 { *step = 3;
            }
            if *step >= 5 { return Ok(None); }
            if *step == 3 {
                let component = self.pickables.get_mut(reference.handle.slot).ok_or("document Pickable retired")?;
                component.interaction_flags = (component.interaction_flags & !4) | 2;
                registry.resume_update_flags(reference.handle,&mut component.owner.flags)?;
                *step = 4;
            }
            registry.drain_update_changes(&mut super::interaction_components::Startup{world:None,scene:self,live,events})?;
            let sound = self.pickables.get(reference.handle.slot).ok_or("document Pickable retired")?.definition.begin_pickup_sound.clone();
            *step = 5;
            Ok(sound)
        })();
        self.registry = Some(registry);
        result
    }

    pub(in crate::app) fn finish_document_pickup(&mut self, live: &LiveEntities, reference: CheckedComponent,
        sources: &mut super::super::shadow_updates::ModelShadowSources,
        placements: &super::super::RenderSourcePlacements,
        hidden: &mut super::super::interaction_focus::RetiredPlacements) -> Result<(),String> {
        let (index,_) = self.document_pickable(live,reference)?;
        // Resources, canonical body and geometry remain until real Deinitialize.
        self.entities[index].entity_flags |= 1;
        let name = &self.entities[index].name;
        for source in &mut sources.0 {
            if placements.0.get(source.placement).is_some_and(|placement| placement.id == *name) {
                source.flags_1c30 |= 1;
                hidden.0.insert(source.placement);
            }
        }
        self.entities[index].entity_flags |= 2;
        let component = self.pickables.get_mut(reference.handle.slot).ok_or("document Pickable retired")?;
        component.interaction_flags = (component.interaction_flags & !2) | 8;
        self.entity_flags_d4[index] |= 4;
        Ok(())
    }
}
