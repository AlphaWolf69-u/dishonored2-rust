//! Concrete Statistics431df0 and Timers778c40 lifecycle/receivers.
use super::*;
use crate::gameplay::{ai_world::{Slot,Worlds,managers::{Clock,STATISTICS_EVENTS}},
    entity_lifetime::{EntityId,LiveEntities},entity_events::{EntityEvents,Event,Listener,Payload},
    game_clock_runtime::ClockRuntime,pickable_component::CheckedComponent};
use bevy::prelude::World;
impl PhysicsScene{
    fn manager_entity(&self,handle:ComponentHandle)->Result<EntityId,String>{
        let index=self.owner(handle)?.entity.ok_or("AI manager has no actual owner")?;
        self.entities.get(index).filter(|entity|!entity.retired).map(|entity|entity.shared).ok_or("AI manager actor retired".into())
    }
    pub(super) fn initialize_statistics(&mut self,handle:ComponentHandle)->Result<(),String>{
        let component=self.statistics.get_mut(handle.slot).ok_or("AIStatistics payload absent")?;
        component.owner.mark_initialized();component.initialized=true;Ok(())
    }
    pub(super) fn start_statistics(&mut self,events:&mut EntityEvents,handle:ComponentHandle)->Result<(),String>{
        let component=self.statistics.get_mut(handle.slot).ok_or("AIStatistics payload absent")?;
        if component.started{return Ok(());}
        if !component.initialized{return Err("AIStatistics Start precedes Init".into());}
        for handler in STATISTICS_EVENTS{events.register(handler,None,Listener::AIStatistics{handle,generation:component.owner.generation,handler});}
        component.started=true;Ok(())
    }
    pub(super) fn stop_statistics(&mut self,events:&mut EntityEvents,handle:ComponentHandle)->Result<(),String>{
        let component=self.statistics.get_mut(handle.slot).ok_or("AIStatistics payload absent")?;
        if component.started{
            for handler in STATISTICS_EVENTS{events.unregister(handler,None,Listener::AIStatistics{handle,generation:component.owner.generation,handler});}
        }
        component.started=false;Ok(())
    }
    pub(super) fn deinitialize_statistics(&mut self,handle:ComponentHandle)->Result<(),String>{
        //The source inherits baseDeinit. Its recorded-body list survives a
        //Stop/Start and is reclaimed only with the actual component slot.
        self.statistics.get_mut(handle.slot).ok_or("AIStatistics payload absent")?.initialized=false;Ok(())
    }
    pub(super) fn initialize_timers(&mut self,world:Option<&mut World>,live:&LiveEntities,handle:ComponentHandle)->Result<(),String>{
        let id=self.manager_entity(handle)?;
        let class=live.get(id).ok_or("Timers owner generation stale")?.1;
        let component=self.timers.get(handle.slot).ok_or("Timers payload absent")?;
        if component.initialized{return Ok(());}
        let checked=CheckedComponent{handle,generation:component.owner.generation};
        if self.throwable_class_is(class,b"arkAIWorld")?{
            world.ok_or("Timers AIWorld Init requires its published owner")?
                .get_resource_mut::<Worlds>().ok_or("Timers AIWorld registry absent")?
                .bind_component(id,Slot::Timers,checked)?;
        }
        let component=self.timers.get_mut(handle.slot).unwrap();component.owner.mark_initialized();component.initialized=true;Ok(())
    }
    pub(super) fn start_timers(&mut self,live:&LiveEntities,handle:ComponentHandle)->Result<(),String>{
        let component=self.timers.get_mut(handle.slot).ok_or("Timers payload absent")?;
        if component.started{return Ok(());}
        if !component.initialized{return Err("Timers Start precedes Init".into());}
        component.restore_start(|id|live.get(id).is_some());component.started=true;Ok(())
    }
    pub(super) fn stop_timers(&mut self,handle:ComponentHandle)->Result<(),String>{
        self.timers.get_mut(handle.slot).ok_or("Timers payload absent")?.stop();Ok(())
    }
    pub(super) fn deinitialize_timers(&mut self,world:Option<&mut World>,live:&LiveEntities,handle:ComponentHandle)->Result<(),String>{
        let id=self.manager_entity(handle)?;let class=live.get(id).ok_or("Timers owner generation stale")?.1;
        let component=self.timers.get(handle.slot).ok_or("Timers payload absent")?;
        if !component.initialized{return Ok(());}
        let checked=CheckedComponent{handle,generation:component.owner.generation};
        if self.throwable_class_is(class,b"arkAIWorld")?{
            world.ok_or("Timers AIWorld Deinit requires its actual owner")?
                .get_resource_mut::<Worlds>().ok_or("Timers AIWorld registry absent")?
                .unbind_component(id,Slot::Timers,checked)?;
        }
        self.timers.get_mut(handle.slot).unwrap().initialized=false;Ok(())
    }
    ///778f40 is callable by the actual script/AI owner, not a second timer
    ///service. The native key is the full target generation plus timerID.
    pub(in crate::app) fn set_ai_timer(&mut self,checked:CheckedComponent,target:Option<EntityId>,timer:i32,duration:f32,clock:i32,
        registry:&mut ComponentRegistry)->Result<(),String>{
        if checked.handle.class_id!=self.classes[22]{return Err("timer manager class differs".into());}
        let component=self.timers.get_mut(checked.handle.slot).ok_or("Timers payload absent")?;
        if component.owner.generation!=checked.generation{return Err("Timers generation stale".into());}
        let resumed=component.set(target,timer,duration,Clock::from_source(clock))?;
        if resumed{registry.resume_update_flags(checked.handle,&mut component.owner.flags)?;}Ok(())
    }
    pub(in crate::app) fn update_timers(&mut self,world:&World,live:&LiveEntities,events:&mut EntityEvents,handle:ComponentHandle,
        registry:&mut ComponentRegistry)->Result<(),String>{
        let id=self.manager_entity(handle)?;
        let clock=world.get_resource::<ClockRuntime>().ok_or("Timers game-clock owner absent")?;
        let marker=clock.clock().map_err(str::to_owned)?.frame_marker();
        let component=self.timers.get_mut(handle.slot).ok_or("Timers payload absent")?;
        if !component.started||component.updated_frame==Some(marker){return Ok(());}
        let empty=component.update(|selection,target|{
            let group=match selection{Clock::Game3=>Some(3),Clock::Game2=>Some(2),Clock::Game4=>Some(4),_=>None};
            if let Some(group)=group{return clock.clock().map_err(str::to_owned)?.delta(group).ok_or("Timers game clock absent".into());}
            let target=if selection==Clock::Manager{id}else{target.ok_or("entity-clock timer has no actual target")?};
            clock.entity_delta(target).map_err(str::to_owned)
        },|target|live.get(target).is_some(),|target,id|events.post(Event{id:0xd1,target:Some(target),payload:Payload::Bytes(id.to_le_bytes().to_vec())}))?;
        if empty{registry.pause_update_flags(handle,&mut component.owner.flags)?;}
        component.updated_frame=Some(marker);Ok(())
    }
    ///All four431df0 subscriptions are real. Non-NPC subjects exit the same
    ///native predicate; an admitted NPC with an unported knowledge/decision
    ///reader is an explicit dependency error, never a fabricated statistic.
    pub(in crate::app) fn ai_statistics_event(&mut self,statistics:Option<&mut super::super::player_statistics::PlayerStatistics>,live:&LiveEntities,handle:ComponentHandle,generation:u32,event:&Event)->Result<(),String>{
        if handle.class_id!=self.classes[21]{return Err("AIStatistics listener class differs".into());}
        let component=self.statistics.get(handle.slot).ok_or("AIStatistics payload absent")?;
        if component.owner.generation!=generation||!component.started{return Ok(());}
        let manager=self.manager_entity(handle)?;
        if live.get(manager).is_none(){return Err("AIStatistics registered manager generation is stale".into());}
        let Some(subject)=event.target.filter(|&id|live.get(id).is_some())else{return Ok(());};
        match event.id{
            0x117|0x2b=>{
                if self.throwable_class_is(live.get(subject).unwrap().1,b"arkNPC")?{
                    return Err(format!("AIStatistics event {:x} needs the actual admitted NPC knowledge/decision owner",event.id));
                }Ok(())
            }
            0xa5=>{
                //431960 requires BOTH started AIDecisionCivilian and
                //AIKnowledge. Neither is implied by a Health component.
                let Some(&index)=self.shared_indices.get(&subject)else{return Ok(());};
                let mut civilian=false;let mut knowledge=false;
                for &word in self.entities[index].components.entries.values(){
                    let handle=ComponentHandle::from_packed(word)?;
                    if self.owner(handle)?.flags&0x0c!=8{continue;}
                    let class=*self.types.classes().hierarchy().by_id().get(handle.class_id as usize).ok_or("AIStatistics component class metadata absent")?;
                    civilian|=self.throwable_class_is(class,b"arkCpntAIDecisionCivilian")?;
                    knowledge|=self.throwable_class_is(class,b"arkCpntAIKnowledge")?;
                }
                if civilian&&knowledge{return Err("AIStatistics incapacity needs the actual civilian knowledge owner".into());}Ok(())
            }
            0x3b=>{
                let Payload::AIStatisticsBodyFound{body,kind}=&event.payload else{return Err("AIStatistics3B has no canonical body-found packet".into());};
                if *kind!=0{return Ok(());}
                let Some(body)=body.filter(|&id|live.get(id).is_some())else{return Ok(());};
                let owner=self.manager_entity(handle)?;
                if self.entity_time_binding(live,subject)?.0!=self.entity_time_binding(live,owner)?.0{return Ok(());}
                let Some(health)=self.health_component(live,body)?else{return Ok(());};
                let Some(attacker)=self.health_payload(health)?.attacker.filter(|&id|live.get(id).is_some())else{return Ok(());};
                if !self.throwable_class_is(live.get(attacker).unwrap().1,b"arkPlayer")?{return Ok(());}
                if self.statistics.get(handle.slot).unwrap().recorded.contains(&body){return Ok(());}
                statistics.ok_or("body-found statistic owner absent")?.increment(10)?;
                self.statistics.get_mut(handle.slot).unwrap().record(body);Ok(())
            }
            _=>Err(format!("AIStatistics has no source subscription {:x}",event.id)),
        }
    }
}

impl PhysicsScene {
    ///954f30 selects only779010's actual phase1 accepted prefix.
    pub(super) fn update_ai_timers(&mut self,world:&World,live:&LiveEntities,events:&mut EntityEvents)->Result<(),String>{
        let mut registry=self.registry.take().ok_or("Timers component registry borrowed")?;
        let result=(||{
            let count=self.timers.roster.sorted_start as usize;
            for cursor in 0..count{
                let handle=ComponentHandle{class_id:self.classes[22],slot:self.timers.roster.indices[cursor]};
                self.update_timers(world,live,events,handle,&mut registry)?;
            }
            Ok(())
        })();
        self.registry=Some(registry);result
    }
}
///The timer shares the existing stage2 scope; stage3 follows entity Think.
pub(in crate::app) fn before_actors(world:&mut World)->Result<(),String>{super::grenades::before_actors(world)}
pub(in crate::app) fn update(world:&mut World)->Result<(),String>{
    super::grenades::update(world)
}
