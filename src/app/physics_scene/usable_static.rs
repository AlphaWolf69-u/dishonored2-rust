//!4cd0f0/4cd330/4cd390/4cda40: static Interactable instance lifecycle.
//!Geometry uses the existing usable fake-box host adapter; no state-list or
//!animation is fabricated for a static handle. Authored96 drives script listeners.
use super::*;
use crate::gameplay::{entity_lifetime::{EntityId,LiveEntities},entity_events::{EntityEvents,Event,Listener,Payload},pickable_component::CheckedComponent,interaction_definition::InteractionDef};

pub(super) struct State {
    definition:Arc<crate::assets::entities::usable::DefinitionData>,
    pub enabled:bool,
    locked:bool,
    uses:i32,
    remaining:Option<f32>,
    last_instigator:Option<EntityId>,
}
impl State {
    pub fn new(definition:Arc<crate::assets::entities::usable::DefinitionData>,locked:bool)->Self{
        Self{definition,enabled:true,locked,uses:0,remaining:None,last_instigator:None}
    }
    fn config(&self)->&crate::assets::entities::usable::StaticDefinition{self.definition.static_definition.as_ref().expect("typed static declaration")}
}
pub(super) struct AcceptedUse {
    reference:CheckedComponent,
    instigator:Option<EntityId>,
    state_applied:bool,
}
impl PhysicsScene {
    pub(in crate::app) fn interaction_static(&self,live:&LiveEntities,id:EntityId)->Result<Option<(CheckedComponent,&InteractionDef)>,String>{
        if live.stage(id)!=Some(crate::gameplay::entity_lifetime::Stage::Spawned){return Ok(None);}
        let Some(&index)=self.shared_indices.get(&id) else{return Ok(None);};
        let entity=&self.entities[index];
        if entity.retired||entity.entity_flags&1!=0{return Ok(None);}
        let Some(handle)=entity.components.find_component([self.classes[58];2],[0x28,0],|handle|Ok(crate::gameplay::component_state::entity_lookup::ComponentLookupState{
            flags:self.owner(handle)?.flags,start_phases:self.declarations.get(&handle.packed()).map(|d|d.scheduling.start_phases)}))? else{return Ok(None);};
        let component=self.static_usables.get(handle.slot).ok_or("static Usable component absent")?;
        let state=component.static_state.as_ref().ok_or("static Usable state absent")?;
        if !state.enabled{return Ok(None);}
        Ok(Some((CheckedComponent{handle,generation:component.owner.generation},if state.locked{&state.config().locked_interaction}else{&state.config().interaction})))
    }
    pub(in crate::app) fn static_usable_labels(&self,handle:ComponentHandle)->Result<&[String;2],String>{
        let state=self.static_usables.get(handle.slot).and_then(|component|component.static_state.as_ref()).ok_or("static Usable labels absent")?;
        Ok(&state.config().labels[usize::from(state.locked)])
    }
    pub(super) fn start_static_usable(&mut self,live:&LiveEntities,events:&mut EntityEvents,handle:ComponentHandle)->Result<(),String>{
        let component=self.static_usables.get_mut(handle.slot).ok_or("static Usable Start absent")?;
        let index=component.owner.entity.ok_or("static Usable owner absent")?;
        let state=component.static_state.as_mut().ok_or("static Usable state absent")?;
        let id=self.entities[index].shared;
        live.get(id).ok_or("static Usable generation stale")?;
        if component.owner.flags&(1<<30)==0{state.enabled=state.config().usable_by_player;}
        self.entities[index].entity_flags_d7=(self.entities[index].entity_flags_d7&!1)|u8::from(state.enabled);
        events.register(0x96,Some(id),Listener::Component{owner:self.owner_id,handle,generation:component.owner.generation,handler:0x96});
        Ok(())
    }
    pub(super) fn static_usable_message(&mut self,live:&LiveEntities,registry:&mut ComponentRegistry,listener:Listener,event:&Event)->Result<(),String>{
        let Listener::Component{owner,handle,generation,handler}=listener else{return Err("static Usable listener differs".into());};
        let component=self.static_usables.get_mut(handle.slot).ok_or("static Usable callback absent")?;
        let index=component.owner.entity.ok_or("static Usable callback owner absent")?;
        let id=self.entities[index].shared;
        if owner!=self.owner_id||component.owner.generation!=generation||handler!=0x96||event.id!=handler||event.target!=Some(id)||live.get(id).is_none(){return Err("static Usable callback generation differs".into());}
        let Payload::Use{instigator}=&event.payload else{return Err("static Usable96 payload differs".into());};
        let state=component.static_state.as_mut().ok_or("static Usable state absent")?;
        state.last_instigator=*instigator;
        state.uses=state.uses.wrapping_add(1);
        self.entity_flags_d4[index]|=4;
        let maximum_uses=state.config().maximum_uses;
        let delay=state.config().deactivation_delay;
        if maximum_uses>=0&&maximum_uses<=state.uses{
            state.enabled=false;
            self.entities[index].entity_flags_d7&=!1;
        }else if delay>0.{
            state.remaining=Some(delay);state.enabled=false;
            self.entities[index].entity_flags_d7&=!1;
            self.static_usable_updates_pending=true;
            registry.base_update_add(handle,self)?;
        }
        self.usable_uses.push_back(AcceptedUse{reference:CheckedComponent{handle,generation},instigator:*instigator,state_applied:true});
        Ok(())
    }
    pub(super) fn stop_static_usable(&mut self,events:&mut EntityEvents,handle:ComponentHandle)->Result<(),String>{
        let component=self.static_usables.get_mut(handle.slot).ok_or("static Usable Stop absent")?;
        let index=component.owner.entity.ok_or("static Usable Stop owner absent")?;
        events.unregister(0x96,Some(self.entities[index].shared),Listener::Component{owner:self.owner_id,handle,generation:component.owner.generation,handler:0x96});
        let state=component.static_state.as_mut().ok_or("static Usable state absent")?;
        state.enabled=false;
        self.entities[index].entity_flags_d7&=!1;
        self.usable_uses.retain(|request|request.reference.handle!=handle||request.reference.generation!=component.owner.generation);
        Ok(())
    }
    pub(super) fn start_usable_events(&mut self,live:&LiveEntities,events:&mut EntityEvents,handle:ComponentHandle)->Result<(),String>{
        let component=self.usables.get(handle.slot).ok_or("Usable Start absent")?;
        let index=component.owner.entity.ok_or("Usable Start owner absent")?;
        let id=self.entities[index].shared;live.get(id).ok_or("Usable generation stale")?;
        events.register(0x96,Some(id),Listener::Component{owner:self.owner_id,handle,generation:component.owner.generation,handler:0x96});
        Ok(())
    }
    pub(super) fn usable_message(&mut self,live:&LiveEntities,listener:Listener,event:&Event)->Result<(),String>{
        let Listener::Component{owner,handle,generation,handler}=listener else{return Err("Usable listener differs".into());};
        let component=self.usables.get(handle.slot).ok_or("Usable callback absent")?;
        let index=component.owner.entity.ok_or("Usable callback owner absent")?;
        let id=self.entities[index].shared;
        if owner!=self.owner_id||component.owner.generation!=generation||handler!=0x96||event.id!=handler||event.target!=Some(id)||live.get(id).is_none(){return Err("Usable callback generation differs".into());}
        let Payload::Use{instigator}=&event.payload else{return Err("Usable96 payload differs".into());};
        self.usable_uses.push_back(AcceptedUse{reference:CheckedComponent{handle,generation},instigator:*instigator,state_applied:false});
        Ok(())
    }
    pub(in crate::app) fn set_interaction_usable(&mut self,live:&LiveEntities,interactions:&mut super::super::interactions::Interactions,id:EntityId,enabled:bool)->Result<bool,String>{
        let Some((name,_))=live.get(id) else{return Ok(false);};
        let Some(&index)=self.shared_indices.get(&id) else{return Ok(false);};
        let Some(checked)=self.cue_dependency(index,b"arkCpntInteractable",[0x28,0])? else{return Ok(false);};
        if checked.handle.class_id==self.classes[58]{
            let component=self.static_usables.get_mut(checked.handle.slot).ok_or("static Usable Set absent")?;
            component.static_state.as_mut().ok_or("static Usable Set state absent")?.enabled=enabled;
        }else if self.is_pickable_class(checked.handle.class_id){
            self.pickable_mut(checked.handle).ok_or("Pickable Set Usability absent")?.interaction_enabled=enabled;
        }else if checked.handle.class_id!=self.classes[5]&&checked.handle.class_id!=self.classes[55]{
            return Err("Set Usability requires its actual hosted Interactable subtype".into());
        }
        if let Some(&actor)=interactions.actor_by_entity.get(name){interactions.actors[actor].enabled=enabled;}
        interactions.invalidate_selection();
        self.entities[index].entity_flags_d7=(self.entities[index].entity_flags_d7&!1)|u8::from(enabled);
        self.entity_flags_d4[index]|=4;
        Ok(true)
    }
    pub(in crate::app) fn process_usable_uses(&mut self,world:&mut bevy::prelude::World,live:&LiveEntities,events:&mut EntityEvents)->Result<(),String>{
        while let Some(request)=self.usable_uses.front(){
            let reference=request.reference;let instigator=request.instigator;
            let Some(component)=(if reference.handle.class_id==self.classes[58]{self.static_usables.get(reference.handle.slot)}else{self.usables.get(reference.handle.slot)})
                .filter(|c|c.owner.generation==reference.generation&&c.owner.flags&0x28==8) else{self.usable_uses.pop_front();continue;};
            let index=component.owner.entity.ok_or("accepted Usable owner absent")?;
            let id=self.entities[index].shared;
            if live.get(id).is_none(){self.usable_uses.pop_front();continue;}
            if !request.state_applied{
                let mut interactions=world.get_resource_mut::<super::super::interactions::Interactions>().ok_or("Usable callback backend absent")?;
                if let Some(&actor)=interactions.actor_by_entity.get(&self.entities[index].name){interactions.actors[actor].last_instigator=instigator;}
                interactions.request_usable_stage(&self.entities[index].name,None,false)?;
                self.usable_uses.front_mut().unwrap().state_applied=true;
            }
            world.get_resource_mut::<crate::rendering::bevy_backend::camera_scene::FxSimulation>().ok_or("Usable graph event owner absent")?.dispatch_used(id,instigator)?;
            self.usable_uses.pop_front();
        }
        use crate::gameplay::effects::graph::UsableAction;
        use crate::rendering::bevy_backend::camera_scene::FxSimulation;
        use super::super::interactions::Interactions;
        let Some(fx) = world.get_resource::<FxSimulation>() else { return Ok(()); };
        if !fx.usable_actions_pending() && !world.get_resource::<Interactions>()
            .is_some_and(|interactions| interactions.pending.iter().any(|change| change.stage.is_some())) {
            return Ok(());
        }
        world.resource_scope(|world, mut fx: bevy::prelude::Mut<FxSimulation>| {
            for (target, action) in fx.drain_usable_actions() {
                let Some((name, _)) = live.get(target) else { continue; };
                match action {
                    UsableAction::Stage { stage, instant } => {
                        world.resource_mut::<Interactions>().request_usable_stage(name, stage, instant)?;
                    }
                    UsableAction::SetAvailability(enabled) => {
                        self.set_interaction_usable(live, &mut world.resource_mut::<Interactions>(), target, enabled)?;
                    }
                    UsableAction::Remove { .. } => {
                        super::super::entity_functions::enqueue_graph_removal_with_live(world, live, target)?;
                    }
                    UsableAction::PhysicsBehavior(behavior) => {
                        self.change_graph_body_behavior(target, behavior, events)?;
                    }
                }
            }
            let Some(mut interactions) = world.get_resource_mut::<Interactions>() else { return Ok(()); };
            let interactions = &mut *interactions;
            for change in &mut interactions.pending {
                let Some((stage, started, instigator)) = change.stage else { continue; };
                let actor = interactions.actors.get(change.actor).ok_or("Usable stage event actor absent")?;
                if let Some(target) = live.find(&actor.entity) {
                    fx.dispatch_usable_stage(target, i32::try_from(stage).map_err(|_| "Usable stage exceeds source index")?, started, instigator)?;
                }
                change.stage = None;
            }
            Ok(())
        })
    }
    pub(super) fn stop_usable_events(&mut self,events:&mut EntityEvents,handle:ComponentHandle)->Result<(),String>{
        let component=self.usables.get(handle.slot).ok_or("Usable Stop absent")?;
        let index=component.owner.entity.ok_or("Usable Stop owner absent")?;
        events.unregister(0x96,Some(self.entities[index].shared),Listener::Component{owner:self.owner_id,handle,generation:component.owner.generation,handler:0x96});
        self.usable_uses.retain(|request|request.reference.handle!=handle||request.reference.generation!=component.owner.generation);
        Ok(())
    }
    pub(super) fn advance_static_usables(&mut self,world:&bevy::prelude::World)->Result<(),String>{
        if !self.static_usable_updates_pending{return Ok(());}
        let clock=world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>().ok_or("static Usable clock absent")?.clock()?;
        let mut registry=self.registry.take().ok_or("static Usable registry absent")?;
        let result=(||{
            let mut pending=false;
            for position in 0..self.static_usables.allocated_count(){
                let slot=self.static_usables.roster.indices[position];
                let component=self.static_usables.get_mut(slot).ok_or("static Usable roster absent")?;
                if component.owner.flags&0x28!=8{continue;}
                let state=component.static_state.as_mut().ok_or("static Usable state absent")?;
                let Some(remaining)=state.remaining.as_mut() else{continue;};
                let index=component.owner.entity.ok_or("static Usable update owner absent")?;
                *remaining-=clock.entity_delta(self.entities[index].timespace).ok_or("static Usable timespace invalid")?;
                if *remaining<=0.{
                    state.remaining=None;state.enabled=true;
                    self.entities[index].entity_flags_d7|=1;self.entity_flags_d4[index]|=4;
                    registry.base_update_remove(ComponentHandle{class_id:self.classes[58],slot},self)?;
                }else{pending=true;}
            }
            self.static_usable_updates_pending=pending;Ok(())
        })();
        self.registry=Some(registry);result
    }
    pub(in crate::app) fn write_static_usable_diagnostics(&self,world:&bevy::prelude::World,eye:[f32;3],report:&mut String){
        use std::fmt::Write;
        let interactions=world.get_resource::<super::super::interactions::Interactions>();
        let _=writeln!(report,"\nStatic usable owners: total={} cooldown_pending={} accepted_uses={}",self.static_usables.allocated_count(),self.static_usable_updates_pending,self.usable_uses.len());
        //These are the exact submitted F8 fixtures, observed only on demand.
        for name in ["script_rich_small_01_sndctrl","script_rich_small_gp_window_handle_135","script_invisible_window_locked_1",
            "script_door_latch_01_right_1","script_door_latch_blackmarket_apartment"]{
            let entity=self.entities.iter().find(|entity|entity.name==name);
            let live=world.get_resource::<LiveEntities>();
            let _=writeln!(report,"  captured_usable name={name} scene={:?}",entity.map(|entity|(entity.shared,entity.retired,entity.entity_flags,entity.entity_flags_d7,live.and_then(|live|live.stage(entity.shared)))));
            if let Some(entity)=entity{
                if let Some(&index)=self.shared_indices.get(&entity.shared){
                    let body=self.document_rigid(index).ok().flatten().and_then(|handle|self.rigid_component(handle).ok()).and_then(|component|component.body());
                    let _=writeln!(report,"    actual_collision_body={body:?} frame={:?}",entity.frame.pose);
                }
            }
            super::super::usable_animation::write_named_diagnostics(world,name,report);
        }
        for position in 0..self.static_usables.allocated_count(){
            let slot=self.static_usables.roster.indices[position];
            let Some(component)=self.static_usables.get(slot) else{continue;};
            let Some(index)=component.owner.entity else{continue;};
            let entity=&self.entities[index];
            let distance=(0..3).map(|i|(entity.frame.pose.position[i]-eye[i]).powi(2)).sum::<f32>().sqrt();
            if distance>8.{continue;}
            let Some(state)=component.static_state.as_ref() else{continue;};
            let _=writeln!(report,"  name={} id={:?} distance={} generation={} flags={:#x} enabled={} locked={} uses={} remaining={:?} fake_box={} frame={:?}",entity.name,entity.shared,distance,component.owner.generation,component.owner.flags,state.enabled,state.locked,state.uses,state.remaining,interactions.is_some_and(|owner|owner.actors.get(component.actor).is_some_and(|actor|actor.fake_box.is_some())),entity.frame.pose);
        }
    }

}
