//! Actual arkCpntAnimBase owner:53b090/53e610/53edb0/53ee90/53e9c0.
//! Model/clips are the descriptor's shared BasicRig; playback and exported
//! transforms belong to the actual component generation, never the renderer.
use super::*;
use crate::{
    assets::animation_rig::BasicRig,
    gameplay::{
        component_state::ComponentInstance,
        entity_lifetime::EntityId,
        game_clock::GameClock,
        player::animation::{AnimBasicCommand, AnimBasicNode, PlayMode},
    },
};

pub(super) struct AnimBaseComponent {
    pub owner: ComponentOwner,
    rig: Option<Arc<BasicRig>>,
    default_alias: Option<String>,
    node: Option<AnimBasicNode>,
    pending: Option<AnimBasicCommand>,
    matrices: Vec<[[f32; 4]; 3]>,
    ready: Vec<bool>,
    dirty: bool,
    running: bool,
    frame: Option<i32>,
    audio_scope: Option<Arc<str>>,
    audio_ready: bool,
    sounds: std::collections::VecDeque<(i16,Arc<crate::assets::wwise::SoundEventDeclaration>)>,
}
impl ComponentInstance for AnimBaseComponent {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}
impl AnimBaseComponent {
    pub(super) fn new(rig: Option<Arc<BasicRig>>, default_alias: Option<String>) -> Self {
        Self {
            owner: Default::default(),
            rig,
            default_alias,
            node: None,
            pending: None,
            matrices: Vec::new(),
            ready: Vec::new(),
            dirty: false,
            running: false,
            frame: None,
            audio_scope: None,
            audio_ready: false,
            sounds: Default::default(),
        }
    }
    fn sample(&mut self, delta: f32) -> Result<(), String> {
        let Some(rig) = self.rig.as_ref() else {
            return Ok(());
        };
        let node = self
            .node
            .as_mut()
            .ok_or("AnimBase executor is not initialized")?;
        if !self.dirty && self.pending.is_none() && node.is_settled() {
            return Ok(());
        }
        let previous=rig.has_sounds.then(||node.sound_witnesses(&rig.animations));
        node.update(delta, self.pending.take(), &rig.animations);
        let pose = node.evaluate(&rig.animations, &rig.skeleton.rest)?;
        let joints = pose
            .as_ref()
            .map_or(rig.skeleton.rest.as_slice(), |pose| pose.joints.as_slice());
        crate::edge_skeleton::pose_matrices_into(
            joints,
            &rig.skeleton.hierarchy,
            &rig.root,
            &mut self.matrices,
            &mut self.ready,
        )?;
        if let Some(previous)=previous {
            node.visit_sound_crossings(&previous,&rig.animations,|item,old,new|{
                for sound in rig.sounds[item].iter(){
                    let key=i32::from(sound.frame);
                    if if old<=new{old<key&&key<=new}else{old<key||key<=new}{
                        self.sounds.push_back((sound.joint,sound.event.clone()));
                    }
                }
            });
        }
        self.dirty = false;
        Ok(())
    }
}
impl PhysicsScene {
    pub(super) fn initialize_animation(&mut self, handle: ComponentHandle) -> Result<(), String> {
        let animation = self
            .animations
            .get_mut(handle.slot)
            .ok_or("AnimBase payload absent")?;
        //53e610 tolerates absent model/skeleton. Its actual header still exists.
        animation.node = animation.rig.as_ref().map(|rig| rig.template.clone());
        animation.dirty = animation.rig.is_some();
        animation.frame = None;
        animation.sample(0.)?;
        animation.owner.mark_initialized();
        Ok(())
    }
    pub(super) fn start_animation(&mut self, handle: ComponentHandle) -> Result<(), String> {
        self.animation_audio_pending=true;
        let animation = self
            .animations
            .get_mut(handle.slot)
            .ok_or("AnimBase payload absent")?;
        animation.running = true;
        animation.audio_ready=false;
        animation.audio_scope=None;
        if let (Some(alias), Some(node)) = (&animation.default_alias, &animation.node) {
            //53edb0 ->53efa0: failed alias lookup posts nothing, not clip zero.
            if let Some(index) = node.index_of(alias) {
                animation.pending = Some(AnimBasicCommand {
                    idx: index as i32,
                    weight: 1.,
                    mode: PlayMode::StayOnFirstFrame,
                    rate: 1.,
                    start: 0.,
                    play_on_end: false,
                });
                animation.dirty = true;
            }
        }
        animation.sample(0.)
    }
    pub(super) fn stop_animation(&mut self, handle: ComponentHandle,
        events:&mut crate::gameplay::entity_events::EntityEvents) -> Result<(), String> {
        let animation = self
            .animations
            .get_mut(handle.slot)
            .ok_or("AnimBase payload absent")?;
        animation.running = false;
        animation.pending = None;
        animation.frame = None;
        // Accepted A9 records stay owned until the ordinary event FIFO receives
        // them; Audio's existing Stop fence drains that same recipient journal.
        let id=self.entities[animation.owner.entity.ok_or("AnimBase Stop owner absent")?].shared;
        while let Some((joint,event))=animation.sounds.pop_front(){
            events.post(crate::gameplay::entity_events::Event{id:0xa9,target:Some(id),payload:
                crate::gameplay::entity_events::Payload::AnimationSound{joint,event,
                    placement:crate::gameplay::sound::component::AnimationSource::Runtime(id)}});
        }
        Ok(())
    }
    pub(super) fn retire_animation(&mut self, handle: ComponentHandle) -> Result<(), String> {
        let animation = self
            .animations
            .get_mut(handle.slot)
            .ok_or("AnimBase payload absent")?;
        if !animation.sounds.is_empty(){return Err("AnimBase retirement has accepted sound records".into());}
        animation.running = false;
        animation.pending = None;
        animation.node = None;
        animation.rig = None;
        animation.default_alias = None;
        animation.matrices = Vec::new();
        animation.ready = Vec::new();
        animation.frame = None;
        animation.audio_scope=None;
        animation.audio_ready=false;
        Ok(())
    }
    fn animation_handle(&self, id: EntityId) -> Result<Option<ComponentHandle>, String> {
        self.animation_handle_with_flags(id,[0x28,0])
    }
    fn animation_handle_with_flags(&self, id:EntityId, flags:[u32;2])->Result<Option<ComponentHandle>,String> {
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(None);
        };
        if self.entities[index].retired {
            return Ok(None);
        }
        self.entities[index]
            .components
            .find_component([self.classes[15]; 2], flags, |handle| {
                let component = self
                    .animations
                    .get(handle.slot)
                    .ok_or("AnimBase component absent")?;
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: component.owner.flags,
                        start_phases: self
                            .declarations
                            .get(&handle.packed())
                            .map(|decl| decl.scheduling.start_phases),
                    },
                )
            })
    }
    /// Borrow the same descriptor rig retained by the actual AnimBase owner.
    /// Joint-name consumers must not prepare a second rig or pose cache.
    pub(in crate::app) fn runtime_animation_rig(
        &self,
        id: EntityId,
    ) -> Result<Option<&Arc<BasicRig>>, String> {
        let Some(handle) = self.animation_handle(id)? else {
            return Ok(None);
        };
        Ok(self
            .animations
            .get(handle.slot)
            .ok_or("AnimBase component absent")?
            .rig
            .as_ref())
    }
    pub(in crate::app) fn runtime_animation_matrices(
        &self,
        id: EntityId,
    ) -> Result<Option<&[[[f32; 4]; 3]]>, String> {
        let Some(handle) = self.animation_handle(id)? else {
            return Ok(None);
        };
        let animation = self
            .animations
            .get(handle.slot)
            .ok_or("AnimBase component absent")?;
        Ok(animation
            .rig
            .as_ref()
            .map(|_| animation.matrices.as_slice()))
    }
    ///53eef0's actual input0 command producer; last posted input wins as in6e1fe0.
    pub(in crate::app) fn initialized_animation_rig(&self,id:EntityId)->Result<Option<&Arc<BasicRig>>,String>{
        let Some(handle)=self.animation_handle_with_flags(id,[0x20,0])? else{return Ok(None);};
        Ok(self.animations.get(handle.slot).ok_or("AnimBase payload absent")?.rig.as_ref())
    }
    ///53c5b0 preserves the invalid-alias result; absent names never mean clip0.
    pub(in crate::app) fn animation_alias(&self,id:EntityId,alias:&str)->Result<Option<usize>,String>{
        let Some(handle)=self.animation_handle_with_flags(id,[0x20,0])? else{return Ok(None);};
        Ok(self.animations.get(handle.slot).ok_or("AnimBase payload absent")?.node.as_ref().and_then(|node|node.index_of(alias)))
    }
    ///53e770's input0 state, used by74e7a0 rather than a guessed door timer.
    pub(in crate::app) fn animation_playing(&self,id:EntityId)->Result<bool,String>{
        let Some(handle)=self.animation_handle_with_flags(id,[0x20,0])? else{return Ok(false);};
        Ok(self.animations.get(handle.slot).ok_or("AnimBase payload absent")?.node.as_ref()
            .is_some_and(|node|node.input_playing()))
    }
    ///An initialized held frame is observable before the gameplay clock moves.
    ///Publish only joints demanded by actual bindings, using the same sampled
    ///AnimBase matrices; no additional rig or pose owner is created.
    pub(in crate::app) fn publish_animation_binding_joints(&mut self,id:EntityId)->Result<(),String>{
        let Some(handle)=self.animation_handle_with_flags(id,[0x20,0])? else{return Ok(());};
        let parent=*self.shared_indices.get(&id).ok_or("AnimBase binding header absent")?;
        let animation=self.animations.get_mut(handle.slot).ok_or("AnimBase payload absent")?;
        animation.sample(0.)?;
        let mut changed=false;
        for binding in self.document_hierarchy.bindings.iter().filter(|binding|binding.parent==parent&&binding.joint>=0){
            let matrix=animation.matrices.get(binding.joint as usize).ok_or("AnimBase bound joint matrix absent")?;
            let frame=crate::gameplay::reference_frame::ReferenceFrame{
                position:[matrix[0][3],matrix[1][3],matrix[2][3]],
                basis:std::array::from_fn(|column|std::array::from_fn(|row|matrix[row][column])),
            };
            let current=self.document_hierarchy.external_joints.entry((parent,binding.joint));
            match current{
                std::collections::hash_map::Entry::Occupied(mut current)=>{
                    if current.get().position!=frame.position||current.get().basis!=frame.basis{current.insert(frame);changed=true;}
                },
                std::collections::hash_map::Entry::Vacant(current)=>{current.insert(frame);changed=true;}
            }
        }
        if changed{
            if let Some(handle)=self.entities[parent].components.reference_actor{
                let generation=self.actors.actors.get(handle.slot).ok_or("AnimBase reference actor absent")?.owner.generation;
                self.request_post_physics_actor(usize::from(handle.slot),generation)?;
            }
        }
        Ok(())
    }
    ///4911b0 requests mode1 (initialized), before all sibling Starts complete.
    pub(in crate::app) fn post_initialized_runtime_animation(&mut self,id:EntityId,
        command:AnimBasicCommand)->Result<bool,String> {
        let Some(handle)=self.animation_handle_with_flags(id,[0x20,0])? else{return Ok(false);};
        let animation=self.animations.get_mut(handle.slot).ok_or("AnimBase component absent")?;
        if animation.node.is_none(){return Ok(false);}
        animation.pending=Some(command);animation.dirty=true;Ok(true)
    }
    pub(in crate::app) fn post_runtime_animation(
        &mut self,
        id: EntityId,
        command: AnimBasicCommand,
    ) -> Result<bool, String> {
        let Some(handle) = self.animation_handle(id)? else {
            return Ok(false);
        };
        let animation = self
            .animations
            .get_mut(handle.slot)
            .ok_or("AnimBase component absent")?;
        if animation.node.is_none() {
            return Ok(false);
        }
        animation.pending = Some(command);
        animation.dirty = true;
        Ok(true)
    }
    fn update_runtime_animations(&mut self, clock: &GameClock,
        mut frames:Option<&mut super::super::joint_frames::JointFrames>,
        mut events:Option<&mut crate::gameplay::entity_events::EntityEvents>) -> Result<(), String> {
        for position in 0..self.animations.allocated_count() {
            let slot = self.animations.roster.indices[position];
            let animation = self
                .animations
                .get_mut(slot)
                .ok_or("AnimBase roster slot absent")?;
            if !animation.running || animation.frame == Some(clock.frame_marker()) {
                continue;
            }
            if animation.rig.as_ref().is_some_and(|rig|rig.has_sounds) && !animation.audio_ready {continue;}
            let entity = animation
                .owner
                .entity
                .ok_or("AnimBase has no entity owner")?;
            let timespace = self
                .entities
                .get(entity)
                .ok_or("AnimBase entity absent")?
                .timespace;
            let delta = clock
                .entity_delta(timespace)
                .ok_or("AnimBase entity timespace invalid")?;
            animation.sample(delta)?;
            animation.frame = Some(clock.frame_marker());
            let id=self.entities[entity].shared;
            if let Some(frames)=frames.as_deref_mut(){
                frames.publish_audio(crate::gameplay::sound::component::AnimationSource::Runtime(id),&animation.matrices);
            } else if animation.rig.as_ref().is_some_and(|rig|!rig.sound_joints.is_empty()){
                return Err("AnimBase demanded audio pose owner absent".into());
            }
            if !animation.sounds.is_empty() && events.is_none(){return Err("AnimBase accepted sound FIFO absent".into());}
            while let Some((joint,event))=animation.sounds.pop_front(){
                events.as_deref_mut().expect("accepted sound owner validated").post(crate::gameplay::entity_events::Event{id:0xa9,target:Some(id),payload:
                    crate::gameplay::entity_events::Payload::AnimationSound{joint,event,
                        placement:crate::gameplay::sound::component::AnimationSource::Runtime(id)}});
            }
            self.publish_animation_binding_joints(id)?;
        }
        Ok(())
    }
}
/// Reuse the actual Audio reservation/joint owner. Only Start re-arms this
/// admission pass; settled actors do not scan the roster every frame.
pub(in crate::app) fn prepare_audio(world:&mut bevy::prelude::World)->Result<(),String>{
    if !world.get_resource::<PhysicsScene>().is_some_and(|scene|scene.animation_audio_pending){return Ok(())}
    world.resource_scope(|world,mut scene:bevy::prelude::Mut<PhysicsScene>|{
        let mut pending=false;
        for position in 0..scene.animations.allocated_count(){
            let slot=scene.animations.roster.indices[position];
            let animation=scene.animations.get(slot).ok_or("AnimBase admission slot absent")?;
            if !animation.running || animation.audio_ready{continue;}
            let Some(rig)=animation.rig.clone()else{scene.animations.get_mut(slot).unwrap().audio_ready=true;continue;};
            if !rig.has_sounds{scene.animations.get_mut(slot).unwrap().audio_ready=true;continue;}
            let index=animation.owner.entity.ok_or("AnimBase Audio owner absent")?;
            let id=scene.entities[index].shared;
            let source=crate::gameplay::sound::component::AnimationSource::Runtime(id);
            if let Some(scope)=animation.audio_scope.clone(){
                let ready=super::super::audio::events_ready(world,&scope)?;
                scene.animations.get_mut(slot).unwrap().audio_ready=ready;
                pending|=!ready;continue;
            }
            let scope:Arc<str>=format!("animbasic-runtime:{id:?}:{}",animation.owner.generation).into();
            let recipient={
                let live=world.get_resource::<crate::gameplay::entity_lifetime::LiveEntities>().ok_or("AnimBase Audio identities absent")?;
                let name=live.get(id).ok_or("AnimBase Audio actor retired")?.0;
                scene.admit_animation_audio(live,name,scope.clone())?
            };
            let Some((handle,generation))=recipient else{scene.animations.get_mut(slot).unwrap().audio_ready=true;continue;};
            super::super::audio::reserve_events(world,scope.clone(),rig.sounds.iter().flat_map(|sounds|sounds.iter().map(|sound|sound.event.clone())))?;
            if !rig.sound_joints.is_empty(){
                let scale=scene.entities[index].frame.scale;
                let mut frames=world.get_resource_mut::<super::super::joint_frames::JointFrames>().ok_or("AnimBase Audio pose owner absent")?;
                for &(joint,initial,range) in rig.sound_joints.iter(){
                    scene.admit_animation_audio_joint(handle,generation,joint,source,range)?;
                    frames.admit_audio(source,scale,joint,initial)?;
                }
                frames.publish_audio(source,&scene.animations.get(slot).unwrap().matrices);
            }
            let ready=super::super::audio::events_ready(world,&scope)?;
            let animation=scene.animations.get_mut(slot).unwrap();
            animation.audio_scope=Some(scope);animation.audio_ready=ready;pending|=!ready;
        }
        scene.animation_audio_pending=pending;Ok(())
    })
}
pub(in crate::app) fn audio_pending(world:&bevy::prelude::World)->bool{
    world.get_resource::<PhysicsScene>().is_some_and(|scene|scene.animation_audio_pending)
}
pub(in crate::app) fn update(
    scene: Option<bevy::prelude::ResMut<PhysicsScene>>,
    clock: Option<bevy::prelude::Res<crate::gameplay::game_clock_runtime::ClockRuntime>>,
    mut frames: Option<bevy::prelude::ResMut<super::super::joint_frames::JointFrames>>,
    mut events: Option<bevy::prelude::ResMut<crate::gameplay::entity_events::EntityEvents>>,
) {
    let Some(mut scene) = scene else {
        return;
    };
    if scene.retirement.is_some() {
        return;
    }
    let Some(clock) = clock else {
        return;
    };
    if !clock.advanced() {
        return;
    }
    let result = clock
        .clock()
        .map_err(str::to_owned)
        .and_then(|clock| scene.update_runtime_animations(clock,frames.as_deref_mut(),events.as_deref_mut()));
    if let Err(error) = result {
        bevy::log::error!("Runtime animation failed: {error}");
    }
}

impl PhysicsScene {
    pub(in crate::app) fn write_runtime_animation_diagnostics(&self,id:EntityId,report:&mut String) {
        use std::fmt::Write;
        let handle=match self.animation_handle_with_flags(id,[0,0]){
            Ok(Some(handle))=>handle,Ok(None)=>{let _=writeln!(report,"    AnimBase actual owner absent for {id:?}");return;},
            Err(error)=>{let _=writeln!(report,"    AnimBase lookup error for {id:?}: {error}");return;}
        };
        let Some(animation)=self.animations.get(handle.slot) else{let _=writeln!(report,"    AnimBase payload absent: {handle:?}");return;};
        let _=writeln!(report,"    AnimBase handle={handle:?} generation={} flags={:#x} rig={} node={} running={} dirty={} frame={:?} default_alias={:?} pending={:?} audio_ready={} audio_scope={:?} queued_sounds={} clip={:?} input_playing={:?} settled={:?} matrices={} pose_hash={:016x} first_matrix={:?}",
            animation.owner.generation,animation.owner.flags,animation.rig.is_some(),animation.node.is_some(),animation.running,
            animation.dirty,animation.frame,animation.default_alias,animation.pending,animation.audio_ready,animation.audio_scope,animation.sounds.len(),
            animation.node.as_ref().and_then(|node|node.current_clip()),animation.node.as_ref().map(|node|node.input_playing()),animation.node.as_ref().map(|node|node.is_settled()),
            animation.matrices.len(),super::super::diagnostics::pose_fingerprint(&animation.matrices),animation.matrices.first());
    }
}
