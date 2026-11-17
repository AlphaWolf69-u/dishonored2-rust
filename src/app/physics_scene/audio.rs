//! Actual arkCpntAudio owner context (708960/708250/708b00, 877eb0/8abb10).
use super::*;
use crate::gameplay::{
    entity_lifetime::LiveEntities, game_clock::GameClock, reference_frame::ReferenceFrame,
    sound::component::AudioComponent,
};

impl PhysicsScene {
    /// 708fc0 -> 7096b0: A9 belongs to Start, not resource initialization.
    pub(in crate::app) fn start_audio_events(
        &mut self, live:&LiveEntities,
        events:&mut crate::gameplay::entity_events::EntityEvents, handle:ComponentHandle,
    )->Result<(),String>{
        let id=self.audio_message_identity(live,handle)?;
        let component=self.audios.get_mut(handle.slot).ok_or("audio Start payload absent")?;
        if !component.started{return Err("audio subscriptions require accepted Start".into());}
        if !component.audio_events_registered {
            events.register(0xa9,Some(id),crate::gameplay::entity_events::Listener::Component{
                owner:self.owner_id,handle,generation:component.owner.generation,handler:0xa9,
            });
            component.audio_events_registered=true;
        }
        Ok(())
    }

    /// 709020 -> 709c00. Accepted posts are drained by the host before this call.
    pub(in crate::app) fn stop_audio_events(
        &mut self,events:&mut crate::gameplay::entity_events::EntityEvents,handle:ComponentHandle,
    )->Result<(),String>{
        if handle.class_id!=self.classes[4]{return Err("audio Stop class differs".into());}
        let component=self.audios.get_mut(handle.slot).ok_or("audio Stop payload absent")?;
        if !component.posts.is_empty(){return Err("audio Stop has unacknowledged animation posts".into());}
        if component.audio_events_registered {
            let index=component.owner.entity.ok_or("audio Stop owner absent")?;
            let id=self.entities.get(index).ok_or("audio Stop entity absent")?.shared;
            events.unregister(0xa9,Some(id),crate::gameplay::entity_events::Listener::Component{
                owner:self.owner_id,handle,generation:component.owner.generation,handler:0xa9,
            });
            component.audio_events_registered=false;
        }
        component.started=false;
        Ok(())
    }

    fn audio_message_identity(&self,live:&LiveEntities,handle:ComponentHandle)
        ->Result<crate::gameplay::entity_lifetime::EntityId,String>{
        if handle.class_id!=self.classes[4]{return Err("audio message class differs".into());}
        let component=self.audios.get(handle.slot).ok_or("audio message component absent")?;
        let index=component.owner.entity.ok_or("audio message owner absent")?;
        let entity=self.entities.get(index).ok_or("audio message entity absent")?;
        if entity.retired || live.get(entity.shared).is_none()
            || self.shared_indices.get(&entity.shared)!=Some(&index)
            || !entity.components.entries.values().contains(&handle.packed()) {
            return Err("audio message owner is not the live linked component".into());
        }
        Ok(entity.shared)
    }

    /// Reserve against the actual A9 receiver; no receiver is a native no-op.
    pub(in crate::app) fn admit_animation_audio(
        &mut self,live:&LiveEntities,entity:&str,scope:std::sync::Arc<str>,
    )->Result<Option<(ComponentHandle,u32)>,String>{
        let Some(id)=live.find(entity) else{return Ok(None)};
        let Some(&index)=self.shared_indices.get(&id) else{return Ok(None)};
        let mut selected=None;
        for &packed in self.entities[index].components.entries.values() {
            let handle=ComponentHandle::from_packed(packed)?;
            if handle.class_id==self.classes[4] {selected=Some(handle);break;}
        }
        let Some(handle)=selected else{return Ok(None)};
        self.audio_message_identity(live,handle)?;
        let component=self.audios.get_mut(handle.slot).ok_or("animation audio component absent")?;
        if !component.started || !component.audio_events_registered{return Ok(None)};
        if !component.animation_scopes.iter().any(|existing|existing==&scope){
            component.animation_scopes.push(scope);
        }
        Ok(Some((handle,component.owner.generation)))
    }

    /// Register the demanded native joint slot before any event can fire.
    pub(in crate::app) fn admit_animation_audio_joint(
        &mut self,handle:ComponentHandle,generation:u32,joint:i16,placement:usize,max_distance:f32,
    )->Result<(),String>{
        if handle.class_id!=self.classes[4]{return Err("animation joint audio class differs".into());}
        let component=self.audios.get_mut(handle.slot).ok_or("animation joint audio component absent")?;
        if component.owner.generation!=generation || !component.started {
            return Err("animation joint audio generation or Start differs".into());
        }
        if joint < -1{return Err("animation sound joint is below default slot".into());}
        if !max_distance.is_finite() || max_distance<0. {return Err("animation sound distance is invalid".into());}
        if joint==-1{return Ok(())}
        if let Some(existing)=component.joints.iter_mut().find(|existing|existing.joint==joint){
            if existing.placement!=placement{return Err("animation sound joint belongs to another placement".into());}
            existing.max_distance=existing.max_distance.max(max_distance);
        } else {
            component.joints.push(crate::gameplay::sound::component::JointObject{joint,placement,max_distance,allocated:false});
        }
        Ok(())
    }

    /// A9 posts already accepted by the message owner remain drainable during Stop.
    pub(in crate::app) fn drain_audio_posts(&mut self,world:&mut bevy::prelude::World)->Result<(),String>{
        for index in 0..self.audios.chunk_count()*self.audios.items_per_chunk {
            let Some(component)=self.audios.get_at_mut(index) else {continue};
            if component.posts.is_empty(){continue;}
            let handle=ComponentHandle{class_id:self.classes[4],slot:u16::try_from(index)
                .map_err(|_|"audio post slot exceeds pool identity")?};
            super::super::audio::drain_component_posts(world,handle,component.owner.generation,component)?;
        }
        Ok(())
    }

    /// 875ba0 -> 70a030 retains the real declaration through 708810 dispatch.
    pub(super) fn audio_message(&mut self,live:&LiveEntities,
        listener:crate::gameplay::entity_events::Listener,
        event:&crate::gameplay::entity_events::Event,
    )->Result<(),String>{
        use crate::gameplay::entity_events::{Listener,Payload};
        let Listener::Component{owner,handle,generation,handler}=listener
            else{return Err("audio message listener differs".into())};
        if owner!=self.owner_id || handler!=0xa9 || event.id!=handler {
            return Err("audio message subscription differs".into());
        }
        let id=self.audio_message_identity(live,handle)?;
        if event.target!=Some(id){return Err("audio message target differs".into());}
        let frame=self.entity_frame(live,id)?.ok_or("audio message reference frame absent")?;
        let component=self.audios.get_mut(handle.slot).ok_or("audio message payload absent")?;
        if component.owner.generation!=generation || !component.started || !component.audio_events_registered {
            return Err("audio message generation or Start ownership differs".into());
        }
        let Payload::AnimationSound{joint,event,placement}=&event.payload
            else{return Err("A9 has no animation sound payload".into())};
        if *joint >= 0 {
            let slot=component.joints.iter_mut().find(|slot|slot.joint==*joint&&slot.placement==*placement)
                .ok_or("A9 actual joint was not admitted by its animator")?;
            slot.allocated=true;
            slot.max_distance=slot.max_distance.max(event.max_distance);
        } else { component.allocated=true; }
        component.posts.push_back(crate::gameplay::sound::component::AnimationPost{
            entity:id,frame,joint:*joint,event:event.clone(),placement:*placement,
        });
        Ok(())
    }
    ///6dfb40/6dfa50: derive game-side hearing distance from the actual
    ///source identity/class and component admission, never an emitter name.
    pub(in crate::app) fn contact_sound_distance(
        &self, live:&LiveEntities, owner:crate::gameplay::entity_lifetime::EntityId,
        descriptor:&crate::assets::wwise::game_properties::GameSoundProperties,
        distance:f32,
    )->Result<Option<(f32,bool)>,String>{
        let result=(||{
        if distance<=0. || live.get(owner).is_none(){return Ok(None);}
        let npc=|id|->Result<bool,String>{
            let Some((_,mut class))=live.get(id) else{return Ok(false)};
            loop{
                let metadata=self.types.classes().metadata(class).ok_or("sound source class absent")?;
                if self.types.classes().name_text(metadata.name)==Some(b"arkNPC"){return Ok(true);}
                match self.types.classes().hierarchy().node(class).ok_or("sound source hierarchy absent")?.base{
                    Some(base)=>class=base,None=>return Ok(false),
                }
            }
        };
        let source_npc=npc(owner)?;
        let owner_npc=live.owner(owner)?.map(&npc).transpose()?.unwrap_or(false);
        // NPCs are not admitted by this scene's entity factory. Requiring
        //their real attribute owner protects that boundary when it expands.
        if (source_npc && matches!(descriptor.context,15|16))
            || (!source_npc && owner_npc && descriptor.context==12){
            return Err("sound stimulus requires its actual NPC attribute and difficulty owner".into());
        }
        let limit=descriptor.propagation_distance;
        let distance=if distance<=limit{distance}else{limit};
        let emit_general=if descriptor.context==20 && !source_npc && !owner_npc{
            let index=*self.shared_indices.get(&owner).ok_or("sound stimulus scene identity absent")?;
            match self.removal_tampering.get(index).copied().flatten(){
                Some(false)=>false, //877720(mode1) suppresses29 only, afterC6.
                Some(true)=>return Err("sound stimulus requires its actual AITampering enabled state".into()),
                None=>return Err("sound stimulus component admission is not certified".into()),
            }
        }else{true};
        Ok(Some((distance,emit_general)))
        })();
        result.map_err(|error:String|{
            let identity=live.get(owner);
            let class=identity.and_then(|(_,class)|self.types.classes().metadata(class))
                .and_then(|metadata|self.types.classes().name_text(metadata.name))
                .map(String::from_utf8_lossy);
            let header=self.shared_indices.get(&owner).and_then(|&index|self.entities.get(index)
                .map(|entity|(index,entity.external,entity.retired,
                    self.removal_tampering.get(index).copied().flatten(),entity.components.entries.values())));
            format!("sound stimulus owner={owner:?} source={identity:?} class={class:?} header={header:?} context={} distance={distance}: {error}",descriptor.context)
        })
    }
    pub(in crate::app) fn audio_instance_context(
        &self,
        handle: ComponentHandle,
        generation: u32,
        live: &LiveEntities,
        clock: &GameClock,
    ) -> Result<Option<(ReferenceFrame, f32)>, String> {
        if handle.class_id != self.classes[4] {
            return Err("audio context class differs".into());
        }
        let audio = self
            .audios
            .get(handle.slot)
            .ok_or("audio context component is absent")?;
        if !audio.started || audio.owner.generation != generation {
            return Ok(None);
        }
        let Some(index) = audio.owner.entity else {
            return Ok(None);
        };
        let entity = self
            .entities
            .get(index)
            .ok_or("audio context entity is absent")?;
        if entity.retired || self.shared_indices.get(&entity.shared) != Some(&index) {
            return Ok(None);
        }
        if !entity
            .components
            .entries
            .values()
            .contains(&handle.packed())
        {
            return Err("audio component is not linked to its actual owner".into());
        }
        let Some(frame) = self.entity_frame(live, entity.shared)? else {
            return Ok(None);
        };
        let rate = clock
            .entity_rates(entity.timespace)
            .ok_or("audio owner timespace is invalid")?[1];
        Ok(Some((frame, rate)))
    }

    /// Resolve immutable owner context before borrowing the component mutably;
    /// no temporary entity names, handle vectors, or second object registry.
    pub(in crate::app) fn visit_audio_instances(
        &mut self,
        live: &LiveEntities,
        clock: &GameClock,
        mut visit: impl FnMut(
            ComponentHandle,
            &entities::audio::AudioBinding,
            &mut AudioComponent,
            ReferenceFrame,
            f32,
        ) -> Result<(), String>,
    ) -> Result<(), String> {
        for index in 0..self.audios.chunk_count() * self.audios.items_per_chunk {
            let Some(audio) = self.audios.get_at(index) else {
                continue;
            };
            if !audio.started {
                continue;
            }
            let handle = ComponentHandle {
                class_id: self.classes[4],
                slot: u16::try_from(index).map_err(|_| "audio slot exceeds pool identity")?,
            };
            let Some((frame, rate)) =
                self.audio_instance_context(handle, audio.owner.generation, live, clock)?
            else {
                continue;
            };
            let runtime = audio.runtime_definition.clone();
            let declaration = audio.owner.declaration;
            let binding = match runtime.as_deref() {
                Some(binding) => binding,
                None => self
                    .audio_bindings
                    .get(declaration.ok_or("audio instance declaration is absent")?)
                    .ok_or("audio instance definition is absent")?,
            };
            let audio = self
                .audios
                .get_at_mut(index)
                .ok_or("audio instance disappeared")?;
            visit(handle, binding, audio, frame, rate)?;
        }
        Ok(())
    }
}
