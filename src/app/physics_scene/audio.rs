//! Actual arkCpntAudio owner context (708960/708250/708b00, 877eb0/8abb10).
use super::*;
use crate::gameplay::{
    entity_lifetime::LiveEntities, game_clock::GameClock, reference_frame::ReferenceFrame,
    sound::component::AudioComponent,
};

impl PhysicsScene {
    ///6dfb40/6dfa50: derive game-side hearing distance from the actual
    ///source identity/class and component admission, never an emitter name.
    pub(in crate::app) fn contact_sound_distance(
        &self, live:&LiveEntities, owner:crate::gameplay::entity_lifetime::EntityId,
        descriptor:&crate::assets::wwise::game_properties::GameSoundProperties,
        distance:f32,
    )->Result<Option<(f32,bool)>,String>{
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
