//! The84a480/84aa20/84a6f0 state uses the same Midnight and actual equipped
//! component as other player actions. Fuse state belongs to Grenade490600.
use super::{Frame, StateMachine, StateId};
use crate::gameplay::{entity_lifetime::EntityId, player::weapons::throwable::{Effect,GrenadeState,Scene,StateHost,WeaponRuntime}};
struct Host<'a,'b> { frame:&'a mut Frame<'b>, handle:u32, sliding:bool, quitting:bool }
impl Host<'_,'_>{
    fn runtime(&self,id:EntityId)->Option<&WeaponRuntime>{
        self.frame.throwable.iter().chain(self.frame.holstered_throwables.values()).find(|runtime|runtime.entity==id)
    }
    fn runtime_mut(&mut self,id:EntityId)->Option<&mut WeaponRuntime>{
        self.frame.throwable.iter_mut().chain(self.frame.holstered_throwables.values_mut()).find(|runtime|runtime.entity==id)
    }
}
impl StateHost for Host<'_,'_>{
    fn current_weapon(&self)->Option<EntityId>{
        (self.frame.equipment_archetypes[0]==Some(4)).then(||self.frame.throwable.as_ref().map(|runtime|runtime.entity)).flatten()
    }
    fn weapon_alive(&self,id:EntityId)->bool{self.runtime(id).is_some()}
    fn grenade_state(&self,id:EntityId)->Result<Option<GrenadeState>,String>{Ok(self.runtime(id).and_then(|runtime|runtime.grenade_state))}
    fn button_down(&self)->bool{
        self.frame.controller.mapping.iter().find(|mapping|mapping.state==StateId::THROW_WEAPON)
            .is_some_and(|mapping|self.frame.controller.down(mapping.action))
    }
    fn scene_finished(&self)->bool{self.handle==0||self.frame.midnight.finished(self.handle)}
    fn begin_scene(&mut self,id:EntityId,scene:Scene)->Result<(),String>{
        let equipment=self.frame.equipment_changes.preferred(0).cloned().ok_or("ThrowWeapon has no actual selected equipment")?;
        let pitch=self.frame.camera_axis.x_axis;
        let pitch=(-pitch.z).atan2(pitch.x.hypot(pitch.y)).to_degrees();
        let player_z=self.frame.origin.z;
        let (path,rate)={
            let runtime=self.runtime_mut(id).ok_or("ThrowWeapon component generation changed")?;
            match scene{
                Scene::CookingIn=>runtime.begin_cooking_in(),
                Scene::CookingIdle=>runtime.push_effect(Effect::Arm{in_hand:true,bypass_hit_floor:true}),
                Scene::Fire=>{
                    runtime.push_effect(Effect::Arm{in_hand:true,bypass_hit_floor:false});
                    runtime.cooking_duration=0.;
                    runtime.low_throw=runtime.source.as_ref().is_some_and(|source|
                        runtime.definition.low_throw(&equipment,pitch,player_z,source.aim.z));
                    if let Some(source)=&runtime.source{
                        runtime.push_effect(Effect::FiredNotice{attacker:source.attacker,position:source.weapon_position});
                    }
                }
                _=>{},
            }
            let rate=match scene{Scene::Fire=>runtime.definition.fire_playrate_attribute.clone(),
                Scene::Reload=>runtime.definition.reload_playrate_attribute.clone(),_=>None};
            (runtime.scene(&equipment,scene,false).map(str::to_owned),rate)
        };
        //Low-throw Fire/Reload/Empty take precedence over slide variants.
        let path=if self.sliding && !matches!(scene,Scene::Fire|Scene::Reload|Scene::Empty)
            || self.sliding && self.runtime(id).is_some_and(|runtime|!runtime.low_throw){
            self.runtime(id).and_then(|runtime|runtime.scene(&equipment,scene,true)).map(str::to_owned)
        }else{path};
        let rate=rate.as_deref().map_or(1.,|tag|self.frame.attributes.get(tag));
        self.handle=StateMachine::play(path.as_ref(),rate,0xb,self.frame);
        let handle=self.handle;
        if let Some(runtime)=self.runtime_mut(id){runtime.active_scene=handle;}
        Ok(())
    }
    fn stop_scene(&mut self){
        self.frame.midnight.stop(self.handle);
        for runtime in self.frame.throwable.iter_mut().chain(self.frame.holstered_throwables.values_mut()){
            if runtime.active_scene==self.handle{runtime.active_scene=0;}
        }
        self.handle=0;
    }
    fn has_remaining_ammunition(&self,id:EntityId)->bool{self.runtime(id).is_some_and(|runtime|runtime.ammunition().is_some()&&runtime.total_ammo>0)}
    fn update_cooking(&mut self,id:EntityId)->Result<(),String>{
        let runtime=self.runtime_mut(id).ok_or("cooking WeaponThrowable is absent")?;
        let dt=runtime.entity_delta;
        if let Some(sound)=runtime.update_cooking(dt).map(str::to_owned){runtime.push_effect(Effect::Sound(sound));}Ok(())
    }
    fn cancel_cooking(&mut self,id:EntityId)->Result<(),String>{
        let runtime=self.runtime_mut(id).ok_or("cancelled WeaponThrowable is absent")?;
        runtime.push_effect(Effect::Disarm);
        if let Some(sound)=runtime.cancel_cooking().map(str::to_owned){runtime.push_effect(Effect::Sound(sound));}Ok(())
    }
    fn disarm(&mut self,id:EntityId)->Result<(),String>{self.runtime_mut(id).ok_or("WeaponThrowable is absent")?.push_effect(Effect::Disarm);Ok(())}
    fn exploded_cleanup(&mut self,id:EntityId)->Result<(),String>{
        let runtime=self.runtime_mut(id).ok_or("exploded WeaponThrowable is absent")?;
        if let Some(ammo)=runtime.ammunition(){runtime.push_effect(Effect::ConsumeAmmo(ammo.item.clone()));}
        runtime.magazine=(runtime.magazine-1).max(0);runtime.total_ammo=runtime.total_ammo.saturating_sub(1);
        runtime.push_effect(Effect::Reset);Ok(())
    }
    fn finish_weapon_action(&mut self,id:EntityId)->Result<(),String>{
        //63bfa0 clears the actual remaining magazine and invokes OnAmmoChanged.
        let runtime=self.runtime_mut(id).ok_or("WeaponThrowable is absent")?;runtime.magazine=0;runtime.active_scene=0;Ok(())
    }
    fn quit(&mut self){self.quitting=true;}
}
impl StateMachine{
    pub(super) fn play_throwable_reload(&mut self,index:usize,frame:&mut Frame){
        let sliding=self.is_active(StateId::SLIDING);
        let equipment=frame.equipment_changes.preferred(0);
        let path=equipment.and_then(|equipment|{
            (if sliding{equipment.scenes.get("m_reloadMidnightSlide")}else{None})
                .or_else(||equipment.scenes.get("m_reloadMidnight"))
        }).cloned();
        let rate=frame.throwable.as_ref().and_then(|runtime|runtime.definition.reload_playrate_attribute.as_deref())
            .map_or(1.,|tag|frame.attributes.get(tag));
        let handle=Self::play(path.as_ref(),rate,0xa,frame);
        self.states[index].midnight=handle;
        if let Some(runtime)=frame.throwable.as_mut(){
            runtime.active_scene=handle;
            if path.is_none(){runtime.magazine=i32::from(runtime.total_ammo>0);}
        }
    }
    pub(super) fn run_throw_weapon(&mut self,index:usize,frame:&mut Frame,phase:u8){
        let sliding=self.is_active(StateId::SLIDING);
        let mut state=std::mem::take(&mut self.states[index].throw_weapon);
        let mut host=Host{frame,handle:self.states[index].midnight,sliding,quitting:false};
        let result=match phase{0=>state.activate(&mut host),1=>state.update(&mut host),_=>state.deactivate(&mut host)};
        let handle=host.handle;let quit=host.quitting;
        if let Err(error)=result{host.frame.midnight.last_start_error=Some(error);host.quitting=true;}
        let quit=quit||host.quitting;
        self.states[index].midnight=handle;self.states[index].throw_weapon=state;
        if quit{self.quit(StateId::THROW_WEAPON);}
    }
}
