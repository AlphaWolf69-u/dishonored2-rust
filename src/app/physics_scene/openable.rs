//!74a4a0/74c1f0/74cd90: actual Openable state and animated-part ownership.
//!The hinge drives the existing bound rigid/reference actor. Animation,
//!rendering, audio and child retirement keep their canonical owners.
use super::*;
use crate::{assets::{openable::{Definition,InitialState},runtime_entity::Definition as EntityDefinition},
    gameplay::{component_state::ComponentInstance,entity_events::{EntityEvents,Event,Payload,Listener},
        entity_lifetime::{EntityId,LiveEntities},pickable_component::CheckedComponent,
        player::animation::{AnimBasicCommand,PlayMode},reference_frame::ReferenceFrame}};

#[derive(Clone,Copy,PartialEq,Eq)]
enum State { Closed, Animated{opened:bool,direction:bool}, Opened{direction:bool} }
struct UseRequest { instigator:EntityId, local_done:bool, local_occlusion_done:bool, linked_done:bool, linked_occlusion_done:bool }
pub(super) struct Component {
    pub owner:ComponentOwner,
    definition:Arc<Definition>,
    animated_definition:Arc<EntityDefinition>,
    child:Option<EntityId>,
    linked:Option<CheckedComponent>,
    aliases:[Option<usize>;8],
    state:State,
    initial:InitialState,
    locked:bool,
    blocked:bool,
    blockers:Vec<EntityId>,
    blocker_listeners:Vec<(EntityId,u16)>,
    needed_keys:Vec<Arc<str>>,
    started:bool,
    command_pending:bool,
    uses:std::collections::VecDeque<UseRequest>,
}
impl ComponentInstance for Component {
    fn owner(&self)->&ComponentOwner{&self.owner}
    fn owner_mut(&mut self)->&mut ComponentOwner{&mut self.owner}
}
impl Component {
    fn new(definition:Arc<Definition>,animated_definition:Arc<EntityDefinition>)->Self{
        let mut owner=ComponentOwner::default();owner.request_update(1); //74a4a0.
        Self{owner,initial:definition.initial,locked:definition.locked,definition,
            animated_definition,child:None,linked:None,aliases:[None;8],state:State::Closed,
            blocked:false,blockers:Vec::new(),blocker_listeners:Vec::new(),needed_keys:Vec::new(),started:false,command_pending:false,uses:Default::default()}
    }
}
pub(in crate::app) struct Prepared {
    pub actor:EntityId,
    pub definition:Arc<Definition>,
    pub animated:Arc<EntityDefinition>,
    pub scheduling:DeclarationScheduling,
    pub scope:Arc<str>,
    pub handle:Option<CheckedComponent>,
    pub requested:bool,
}
impl PhysicsScene {
    ///74b1c0 returns no interaction while Animated. Its Closed and Opened
    ///records are the real declaration values/UIDs, not a Usable alias.
    pub(in crate::app) fn interaction_openable(&self,live:&LiveEntities,id:EntityId)
        ->Result<Option<(CheckedComponent,usize,&crate::gameplay::interaction_definition::InteractionDef,&[String;2])>,String>{
        let Some(checked)=self.openable_handle(live,id,true)? else{return Ok(None);};
        let component=self.checked_openable(checked)?;
        let entity=&self.entities[component.owner.entity.ok_or("Openable interaction entity absent")?];
        //77c510 rejects actual Hide;77c530/77c640 publish usableByPlayer.
        if entity.entity_flags&1!=0||entity.entity_flags_d7&1==0{return Ok(None);}
        let slot=match component.state{State::Animated{..}=>return Ok(None),State::Opened{..}=>1,
            State::Closed=>if component.locked{2}else if component.blocked{3}else{0}};
        Ok(Some((checked,slot,&component.definition.usages[slot],&component.definition.usage_labels[slot])))
    }
    pub(super) fn dispatch_openable_message(&mut self,live:&LiveEntities,listener:Listener,event:&Event,
        commands:&mut crate::gameplay::entity_events::Commands)->Result<(),String>{
        let Listener::Component{owner,handle,generation,handler}=listener else{return Err("Openable listener kind differs".into());};
        if owner!=self.owner_id||handle.class_id!=self.classes[55]||handler!=event.id||!matches!(handler,0x96|0xa1){
            return Err("Openable Use subscription differs".into());
        }
        let component=self.openables.get_mut(handle.slot).ok_or("Openable Use component absent")?;
        let id=self.entities[component.owner.entity.ok_or("Openable Use entity absent")?].shared;
        if component.owner.generation!=generation||!component.started||live.get(id).is_none(){
            return Err("Openable Use actual generation/target is stale".into());
        }
        if let Some(blocker)=event.target.filter(|&target|target!=id){
            //74bd80/74b550→74b390 removes a real blocker when that
            //blocker's Interactable96 or BreakableA1 notification is dispatched.
            let position=component.blockers.iter().position(|&id|id==blocker).ok_or("Openable blocker callback target differs")?;
            component.blockers.swap_remove(position);
            component.blocked=!component.blockers.is_empty();
            component.blocker_listeners.retain(|&(target,handler)|{
                if target!=blocker{return true;}
                commands.unregister(handler,Some(target),Listener::Component{owner,handle,generation,handler});false
            });
            self.entity_flags_d4[component.owner.entity.unwrap()]|=4;
            return Ok(());
        }
        if event.target!=Some(id){return Err("Openable Use target differs".into());}
        let Payload::Use{instigator:Some(instigator)}=&event.payload else{return Err("Openable96 has no actual instigator".into());};
        let instigator=*instigator;
        live.get(instigator).ok_or("Openable96 instigator generation stale")?;
        component.uses.push_back(UseRequest{instigator,local_done:false,local_occlusion_done:false,linked_done:false,linked_occlusion_done:false});
        self.openable_uses_pending=true;
        Ok(())
    }
    ///Bevy callback borrowing bridge: execute accepted96 calls at the same
    ///message barrier once the actual inventory/acoustic owners are available.
    ///Each side retains its cursor so a linked-door error cannot toggle the
    ///already accepted first leaf a second time during cleanup/retry.
    pub(in crate::app) fn process_openable_uses(&mut self,world:&mut bevy::prelude::World,
        live:&LiveEntities,events:&mut EntityEvents)->Result<(),String>{
        if !self.openable_uses_pending{return Ok(());}
        for position in 0..self.openables.allocated_count(){
            let slot=self.openables.roster.indices[position];
            let handle=ComponentHandle{class_id:self.classes[55],slot};
            loop{
                let Some(request)=self.openables.get(slot).and_then(|component|component.uses.front()) else{break;};
                let instigator=request.instigator;
                if !request.local_done{
                    self.use_openable(world,live,events,handle,instigator)?;
                    self.openables.get_mut(slot).unwrap().uses.front_mut().unwrap().local_done=true;
                }
                if !self.openables.get(slot).unwrap().uses.front().unwrap().local_occlusion_done{
                    self.update_openable_occlusion(world,live,handle)?;
                    self.openables.get_mut(slot).unwrap().uses.front_mut().unwrap().local_occlusion_done=true;
                }
                let component=self.openables.get(slot).unwrap();
                if !component.uses.front().unwrap().linked_done{
                    if let Some(linked)=component.linked{
                        if self.checked_openable(linked)?.started{self.use_openable(world,live,events,linked.handle,instigator)?;}
                    }
                    self.openables.get_mut(slot).unwrap().uses.front_mut().unwrap().linked_done=true;
                }
                if !self.openables.get(slot).unwrap().uses.front().unwrap().linked_occlusion_done{
                    if let Some(linked)=self.openables.get(slot).unwrap().linked{
                        if self.checked_openable(linked)?.started{self.update_openable_occlusion(world,live,linked.handle)?;}
                    }
                    self.openables.get_mut(slot).unwrap().uses.front_mut().unwrap().linked_occlusion_done=true;
                }
                self.openables.get_mut(slot).unwrap().uses.pop_front();
            }
        }
        self.openable_uses_pending=false;Ok(())
    }
    fn use_openable(&mut self,world:&mut bevy::prelude::World,live:&LiveEntities,events:&mut EntityEvents,
        handle:ComponentHandle,instigator:EntityId)->Result<(),String>{
        let (_,class)=live.get(instigator).ok_or("Openable Use instigator stale")?;
        if !self.throwable_class_is(class,b"arkPlayer")?{
            return Err("Openable NPC Use needs the actual NPC door-action owner74dd50".into());
        }
        let component=self.openables.get(handle.slot).ok_or("Openable Use payload absent")?;
        let index=component.owner.entity.ok_or("Openable Use entity absent")?;
        let id=self.entities[index].shared;
        let child=component.child.ok_or("Openable Use child absent")?;
        let has_key=if component.locked{
            let inventory=world.get_resource::<super::super::interaction_focus::DiscoveredDocuments>()
                .ok_or("Openable Player Use requires its actual inventory owner")?;
            component.needed_keys.iter().any(|key|inventory.quantity(key)>0)
        }else{false};
        let (opened,direction,alias)=if let State::Opened{direction}=component.state{(false,direction,if direction{2}else{3})}
        else{
            let child_frame=self.entity_frame(live,child)?.ok_or("Openable child frame absent")?;
            let actor_frame=self.entity_frame(live,instigator)?.ok_or("Openable instigator frame absent")?;
            let direction=(0..3).map(|axis|(child_frame.position[axis]-actor_frame.position[axis])*child_frame.basis[0][axis]).sum::<f32>()>=0.;
            let opening=!component.locked&&!component.blocked;
            (opening,direction,if opening{usize::from(direction)}else{4+usize::from(direction)})
        };
        if has_key{
            let component=self.openables.get_mut(handle.slot).unwrap();component.locked=false;
            //74c5c0 unlocks and returns to Closed on this press; it does not
            //invent an opening or play the locked animation after success.
            component.state=State::Closed;
            if let Some(event)=component.definition.unlocked_event.clone(){
                events.post(Event{id:0xa9,target:Some(id),payload:Payload::AnimationSound{joint:-1,event,
                    placement:crate::gameplay::sound::component::AnimationSource::Runtime(id)}});
            }
        }else{
            self.openables.get_mut(handle.slot).unwrap().state=State::Animated{opened,direction};
            self.openable_updates_pending=true;
            self.post_openable_alias(handle,alias,PlayMode::AndStay)?;
        }
        self.entity_flags_d4[index]|=4;
        if let Some(&child_index)=self.shared_indices.get(&child){self.entity_flags_d4[child_index]|=4;}
        Ok(())
    }
    ///74e7a0 only finishes a transition after actual53e770 input0 playback
    ///ends. A just-posted command must be sampled before that query.
    pub(in crate::app) fn advance_openables(&mut self,world:&mut bevy::prelude::World,live:&LiveEntities)->Result<(),String>{
        if !self.openable_updates_pending{return Ok(());}
        let mut pending=false;
        for position in 0..self.openables.allocated_count(){
            let slot=self.openables.roster.indices[position];
            let component=self.openables.get(slot).ok_or("Openable roster payload absent")?;
            if !component.started{continue;}
            let child=component.child.ok_or("Openable running child absent")?;
            if component.command_pending{
                self.publish_animation_binding_joints(child)?;
                self.openables.get_mut(slot).unwrap().command_pending=false;
            }
            let state=self.openables.get(slot).unwrap().state;
            if let State::Animated{opened,direction}=state{
                if !self.animation_playing(child)?{
                    self.openables.get_mut(slot).unwrap().state=if opened{State::Opened{direction}}else{State::Closed};
                }else{pending=true;}
                self.update_openable_occlusion(world,live,ComponentHandle{class_id:self.classes[55],slot})?;
            }
        }
        self.openable_updates_pending=pending;Ok(())
    }
    fn openables_active(&self)->bool{
        self.openable_updates_pending
    }
    pub(in crate::app) fn prepare_placed_openables(&mut self,catalog:&archive::Catalog,document:&entities::Document,
        models:&[entities::PlacedModel],live:&mut LiveEntities,_registry:&mut ComponentRegistry)->Result<(),String>{
        let mut children=HashMap::<String,Arc<EntityDefinition>>::new();
        for binding in crate::assets::openable::bindings(catalog,document)?{
            let actor=if let Some(entity)=self.entities.iter().find(|entity|!entity.retired&&entity.name==binding.entity){entity.shared}
            else{
                let authored=document.definitions.iter().find(|entry|entry.kind=="entity"&&entry.id==binding.entity)
                    .ok_or("Openable authored entity absent")?;
                let class=entities::scene_motion::atom(&authored.fields,"class").ok_or("Openable authored class absent")?;
                let class=self.types.classes().hierarchy().registration_order().iter().copied().find(|&index|
                    self.types.classes().metadata(index).and_then(|metadata|self.types.classes().name_text(metadata.name))==Some(class.as_bytes()))
                    .ok_or("Openable authored class is not registered")?;
                let frame=entities::spawn_frame(authored)?;
                let scale=models.iter().find(|model|model.id==binding.entity).map_or([1.;3],|model|model.scale);
                let id=self.allocate_entity_header(live,&binding.entity,class,0,super::super::runtime_entities::SpawnOptions{
                    frame,scale:None,hidden_disabled:false},scale,Some(Arc::from(binding.entity.as_str())),true,false)?;
                let index=self.shared_indices[&id];
                if entities::entity_hidden(authored){self.entities[index].entity_flags|=1;}
                //The enclosing Scene.load registry owns the real common
                //initialized/spawned pass after every CPU header is admitted.
                id
            };
            let animated=if let Some(child)=children.get(&binding.definition.animated_part){child.clone()}else{
                let child=EntityDefinition::acquire(catalog,&binding.definition.animated_part,None)?;
                children.insert(binding.definition.animated_part.clone(),child.clone());child
            };
            let scope=Arc::from(format!("openable:{}:{actor:?}",self.owner_id));
            self.prepared_openables.push(Prepared{actor,definition:binding.definition,animated,scheduling:binding.scheduling,
                scope,handle:None,requested:false});
        }
        Ok(())
    }
    pub(in crate::app) fn take_prepared_openables(&mut self)->Vec<Prepared>{std::mem::take(&mut self.prepared_openables)}
    pub(in crate::app) fn admit_prepared_openables(&mut self,world:&mut bevy::prelude::World,live:&mut LiveEntities,
        events:&mut EntityEvents,plans:&mut [Prepared])->Result<(),String>{
        let mut registry=self.registry.take().ok_or("Openable admission registry borrowed")?;
        let result=(||{
            //74c1f0 may resolve its linked sibling during Initialize. Admit
            //all real parent headers first, then drain one shared source queue.
            for plan in plans{
                live.get(plan.actor).ok_or("Openable admission parent generation stale")?;
                if plan.handle.is_some(){continue;}
                let index=*self.shared_indices.get(&plan.actor).ok_or("Openable admission header absent")?;
                let declaration=self.types.declaration(self.factories[55],Arc::as_ptr(&plan.definition) as usize,plan.scheduling.clone())?;
                let handle=self.create_openable(index,plan.definition.clone(),plan.animated.clone(),&declaration,&mut registry,
                    declaration.scheduling.starts_on_spawn())?.ok_or("Openable construction disabled")?;
                self.declarations.insert(handle.packed(),declaration);
                plan.handle=Some(CheckedComponent{handle,generation:self.owner(handle)?.generation});
            }
            self.run_runtime_startup(world,live,events,&mut registry)
        })();
        self.registry=Some(registry);result
    }
    pub(super) fn create_openable(&mut self,index:usize,definition:Arc<Definition>,animated:Arc<EntityDefinition>,
        declaration:&ComponentDeclaration,registry:&mut ComponentRegistry,immediate:bool)->Result<Option<ComponentHandle>,String>{
        ComponentFactory{declaration,pool:&mut self.openables,registry,grow_one_at_a_time:false}
            .create(&mut self.entities[index].components,immediate,|_,_,_|Ok(Component::new(definition,animated)),|_,_,_|Ok(()))
    }
    pub(super) fn openable_handle(&self,live:&LiveEntities,id:EntityId,started:bool)->Result<Option<CheckedComponent>,String>{
        live.get(id).ok_or("Openable managed generation is stale")?;
        let Some(&index)=self.shared_indices.get(&id) else{return Ok(None);};
        self.cue_dependency(index,b"arkCpntOpenable",if started{[0x28,0]}else{[0,0]})
    }
    fn checked_openable(&self,checked:CheckedComponent)->Result<&Component,String>{
        let component=self.openables.get(checked.handle.slot).ok_or("Openable component absent")?;
        if checked.handle.class_id!=self.classes[55]||component.owner.generation!=checked.generation{
            return Err("Openable checked component generation is stale".into());
        }
        Ok(component)
    }
    pub(super) fn initialize_openable(&mut self,world:&mut bevy::prelude::World,live:&mut LiveEntities,
        events:&mut EntityEvents,registry:&mut ComponentRegistry,handle:ComponentHandle)->Result<(),String>{
        let component=self.openables.get(handle.slot).ok_or("Openable Initialize payload absent")?;
        let index=component.owner.entity.ok_or("Openable Initialize entity absent")?;
        let parent=self.entities[index].shared;
        let definition=component.definition.clone();
        if component.child.is_none(){
            let animated=component.animated_definition.clone();
            let frame=self.entities[index].frame.pose;
            //74c1f0→10fae0 negates14228a750(pi/2), retains the actual up
            //axis and uses111590's compact matrix convention. Reuse that
            //same quaternion/basis owner; no host-axis normalization.
            let rotation=crate::gameplay::reference_frame::axis_angle_basis(frame.basis[2],-std::f32::consts::FRAC_PI_2);
            let frame=ReferenceFrame{position:frame.position,basis:crate::gameplay::reference_frame::basis_product(frame.basis,rotation)};
            let child=super::super::runtime_entities::spawn_component_child_in_registry(world,self,live,events,registry,
                animated,super::super::runtime_entities::SpawnOptions{frame,scale:None,hidden_disabled:false},None,parent)?;
            //Journal immediately; a later failure must never spawn twice.
            self.openables.get_mut(handle.slot).unwrap().child=Some(child);
            live.set_owner(child,Some(parent))?;
            live.set_ai_world_index(child,live.ai_world_index(parent)?)?;
        }
        let linked=definition.linked.as_deref().and_then(|name|live.find(name))
            .map(|id|self.openable_handle(live,id,false)).transpose()?.flatten();
        let mut initial=definition.initial;
        if let Some(other)=linked{
            let other=self.checked_openable(other)?;
            //74c1f0 mirrors initial CW/CCW across the linked door. The source
            //uses the other declaration before Initialize and its actual
            //stored default afterwards; it does not copy its world angle.
            let other_initial=if other.owner.flags&3<2{other.definition.initial}else{other.initial};
            if other.owner.flags&3>=2||initial==InitialState::Closed{
                initial=match other_initial{InitialState::Opened=>InitialState::OpenedCounterClockwise,
                    InitialState::OpenedCounterClockwise=>InitialState::Opened,InitialState::Closed=>InitialState::Closed};
            }
        }
        let component=self.openables.get_mut(handle.slot).unwrap();
        component.linked=linked;component.initial=initial;component.owner.mark_initialized();
        Ok(())
    }
    pub(super) fn start_openable(&mut self,world:&mut bevy::prelude::World,live:&mut LiveEntities,
        events:&mut EntityEvents,registry:&mut ComponentRegistry,handle:ComponentHandle)->Result<(),String>{
        let component=self.openables.get(handle.slot).ok_or("Openable Start payload absent")?;
        if component.started{return Ok(());}
        let index=component.owner.entity.ok_or("Openable Start entity absent")?;
        let parent=self.entities[index].shared;
        let child=component.child.ok_or("Openable animated child absent")?;
        let definition=component.definition.clone();
        let rig=self.initialized_animation_rig(child)?.ok_or("Openable child has no actual initialized AnimBase rig")?;
        let group=super::super::world_models::descriptor(world,parent).map(|model|model.placement.resource_path.clone())
            .or_else(||world.get_resource::<super::super::RenderSourcePlacements>().and_then(|placements|
                placements.0.iter().find(|model|model.id==self.entities[index].name).map(|model|model.resource_path.clone())))
            .unwrap_or_else(||"_info".to_owned());
        let tag=rig.rig.optional_tag_quaternion_in_group(&group,&definition.attachment_tag)?;
        let (joint,offset,orientation)=if let Some((joint,offset,orientation))=tag{
            (i16::try_from(joint).map_err(|_|"Openable hinge joint exceeds source index")?,offset.to_array(),orientation.to_array())
        }else if let Some(joint)=rig.skeleton.joint_names.iter().position(|name|name==&definition.attachment_tag){
            (i16::try_from(joint).map_err(|_|"Openable hinge joint exceeds source index")?,[0.;3],[0.,0.,0.,1.])
        }else{
            let child_frame=self.entities[*self.shared_indices.get(&child).ok_or("Openable child header absent")?].frame.pose;
            let (offset,orientation)=crate::gameplay::reference_frame::relative_binding(child_frame,self.entities[index].frame.pose);
            (-1,offset,orientation)
        };
        let aliases=definition.animations.iter().map(|alias|self.animation_alias(child,alias)).collect::<Result<Vec<_>,_>>()?;
        self.bind_entity_in_registry(live,parent,child,super::throwable::BindingPose{joint,offset,orientation,parent_placement:None},events,registry)?;
        //74cd90 retains each successfully resolved managed blocker; the
        //presence of that roster, not a guessed entity context, blocks use.
        let linked_definition=self.openables.get(handle.slot).unwrap().linked.map(|linked|
            self.checked_openable(linked).map(|component|component.definition.clone())).transpose()?;
        let mut needed_keys=Vec::new();
        for key in definition.needed_keys.iter().chain(linked_definition.as_ref().into_iter().flat_map(|definition|definition.needed_keys.iter())){
            if !needed_keys.contains(key){needed_keys.push(key.clone());}
        }
        let linked_locked=self.openables.get(handle.slot).unwrap().linked.map(|linked|self.checked_openable(linked)
            .map(|other|other.owner.flags&(1<<30)==0&&other.definition.locked)).transpose()?.unwrap_or(false);
        let mut blockers=Vec::new();
        for name in definition.blockers.iter().chain(linked_definition.as_ref().into_iter().flat_map(|definition|definition.blockers.iter())){
            if let Some(id)=live.find(name){if !blockers.contains(&id){blockers.push(id);}}
        }
        let mut blocker_listeners=Vec::new();
        for &id in &blockers{
            let Some(&index)=self.shared_indices.get(&id) else{
                return Err(format!("Openable blocker {} has no actual scene component owner",live.get(id).unwrap().0));
            };
            if self.cue_dependency(index,b"arkCpntBreakable",[0,0])?.is_some(){blocker_listeners.push((id,0xa1));}
            if self.cue_dependency(index,b"arkCpntInteractable",[0,0])?.is_some(){blocker_listeners.push((id,0x96));}
        }
        let blocked=!blockers.is_empty();
        let component=self.openables.get_mut(handle.slot).unwrap();
        component.aliases=aliases.try_into().map_err(|_|"Openable alias count")?;
        component.blocked=blocked;
        component.blockers=blockers;component.blocker_listeners=blocker_listeners;
        component.needed_keys=needed_keys;
        if component.owner.flags&(1<<30)==0{
            component.locked=definition.locked||linked_locked;
            component.state=if component.initial==InitialState::Closed||component.locked||blocked{State::Closed}
                else{State::Opened{direction:component.initial==InitialState::OpenedCounterClockwise}};
        }
        let initial=match component.state{State::Opened{direction}=>Some(usize::from(direction)),_=>None};
        component.started=true;
        self.entities[index].entity_flags_d7=(self.entities[index].entity_flags_d7&!1)|u8::from(definition.usable_by_player);
        events.register(0x96,Some(parent),Listener::Component{owner:self.owner_id,handle,generation:component.owner.generation,handler:0x96});
        for &(target,handler) in &component.blocker_listeners{
            events.register(handler,Some(target),Listener::Component{owner:self.owner_id,handle,generation:component.owner.generation,handler});
        }
        if let Some(alias)=initial{self.post_openable_alias(handle,alias,PlayMode::StayOnLastFrame)?;}
        self.publish_animation_binding_joints(child)?;
        self.update_openable_occlusion(world,live,handle)?;
        if initial.is_some(){
            //74cd90 explicitly disables the already-open startup leaf's
            //volume after its Opened state initialized it.
            if let Some(volume)=self.cue_dependency(index,b"arkCpntSoundVolumeControl",[0x28,0])?{
                let acoustic=world.get_resource::<super::super::audio::level::LevelAudio>().ok_or("Openable acoustic world absent")?.acoustic_world();
                let mut acoustic=acoustic.lock().map_err(|_|"Openable acoustic world poisoned")?;
                let value=self.sound_volumes.get_mut(volume.handle.slot).ok_or("Openable SoundVolume payload absent")?;
                value.state.set_enabled(value.definition.as_deref().ok_or("Openable SoundVolume declaration absent")?,&mut acoustic,false)?;
            }
        }
        Ok(())
    }
    fn post_openable_alias(&mut self,handle:ComponentHandle,alias:usize,mode:PlayMode)->Result<(),String>{
        let component=self.openables.get(handle.slot).ok_or("Openable animation owner absent")?;
        let child=component.child.ok_or("Openable animated child absent")?;
        if let Some(index)=component.aliases[alias]{
            if !self.post_initialized_runtime_animation(child,AnimBasicCommand{idx:i32::try_from(index).map_err(|_|"Openable alias exceeds source index")?,
                weight:1.,mode,rate:1.,start:0.,play_on_end:false})?{return Err("Openable animation rejected by its initialized child".into());}
            self.openables.get_mut(handle.slot).unwrap().command_pending=true;
            self.openable_updates_pending=true;
        }
        Ok(())
    }
    fn update_openable_occlusion(&mut self,world:&mut bevy::prelude::World,live:&LiveEntities,handle:ComponentHandle)->Result<(),String>{
        let component=self.openables.get(handle.slot).ok_or("Openable occlusion owner absent")?;
        let index=component.owner.entity.ok_or("Openable occlusion entity absent")?;
        let Some(volume)=self.cue_dependency(index,b"arkCpntSoundVolumeControl",[0x28,0])? else{return Ok(());};
        let acoustic=world.get_resource::<super::super::audio::level::LevelAudio>().ok_or("Openable acoustic world absent")?.acoustic_world();
        let mut acoustic=acoustic.lock().map_err(|_|"Openable acoustic world poisoned")?;
        let state=component.state;
        let strength=match state{
            State::Closed=>0.,State::Opened{..}=>1.,
            State::Animated{..}=>{
                //74e7a0 uses the actual two published horizontal axes,
                //normalizing each independently before abs(dot).
                let parent=self.entities[index].frame.pose.basis[0];
                let child=component.child.ok_or("Openable occlusion child absent")?;
                let child=self.entity_frame(live,child)?.ok_or("Openable occlusion child frame absent")?.basis[0];
                let epsilon=f32::from_bits(0x0080_0000);
                let a=(parent[0]*parent[0]+parent[1]*parent[1]).max(epsilon).sqrt().recip();
                let b=(child[0]*child[0]+child[1]*child[1]).max(epsilon).sqrt().recip();
                1.-((parent[0]*a)*(child[0]*b)+(parent[1]*a)*(child[1]*b)).clamp(-1.,1.).abs()
            }
        };
        let value=self.sound_volumes.get_mut(volume.handle.slot).ok_or("Openable SoundVolume payload absent")?;
        live.get(self.entities[index].shared).ok_or("Openable acoustic actor generation stale")?;
        let definition=value.definition.as_deref().ok_or("Openable SoundVolume declaration absent")?;
        if matches!(state,State::Opened{..}){value.state.set_enabled(definition,&mut acoustic,true)?;}
        if value.state.set_strength(definition,&mut acoustic,strength,true)?{self.entity_flags_d4[index]|=4;}
        Ok(())
    }
    pub(super) fn stop_openable(&mut self,events:&mut EntityEvents,handle:ComponentHandle)->Result<(),String>{
        let component=self.openables.get_mut(handle.slot).ok_or("Openable Stop payload absent")?;
        if !component.uses.is_empty(){return Err("Openable Stop precedes accepted Use journal delivery".into());}
        if component.started{
            let index=component.owner.entity.ok_or("Openable Stop entity absent")?;
            self.entities[index].entity_flags_d7&=!1; //77c580→77c640(false).
            let entity=self.entities[index].shared;
            events.unregister(0x96,Some(entity),Listener::Component{owner:self.owner_id,handle,generation:component.owner.generation,handler:0x96});
            for (target,handler) in component.blocker_listeners.drain(..){
                events.unregister(handler,Some(target),Listener::Component{owner:self.owner_id,handle,generation:component.owner.generation,handler});
            }
        }
        component.blockers=Vec::new();component.blocker_listeners=Vec::new();component.needed_keys=Vec::new();component.blocked=false;
        component.started=false;component.command_pending=false;Ok(())
    }
    pub(super) fn deinitialize_openable(&mut self,world:&mut bevy::prelude::World,live:&LiveEntities,handle:ComponentHandle)->Result<(),String>{
        let component=self.openables.get_mut(handle.slot).ok_or("Openable Deinitialize payload absent")?;
        if let Some(child)=component.child{
            if live.get(child).is_some(){super::super::entity_functions::enqueue_removal_with_live(world,live,child,0.)?;}
            component.child=None;
        }
        component.linked=None;Ok(())
    }
}

pub(in crate::app) fn update(world:&mut bevy::prelude::World){
    if !world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>().is_some_and(|clock|clock.advanced())
        ||!world.get_resource::<PhysicsScene>().is_some_and(|scene|scene.openables_active()){return;}
    if let Err(error)=super::super::runtime_entities::with_scene(world,|world,scene,live,_|scene.advance_openables(world,live)){
        bevy::log::error!("Openable update failed: {error}");
        crate::platform::client_diagnostics::record(&format!("Openable update failed: {error}"));
        world.write_message(bevy::app::AppExit::error());
    }
}

impl PhysicsScene {
    ///Read only inside the existing F8 callback. No retained diagnostic state.
    pub(in crate::app) fn write_openable_diagnostics(&self, world:&bevy::prelude::World,
        eye:[f32;3], report:&mut String) {
        use std::fmt::Write;
        let focus=world.get_resource::<super::super::interaction_focus::FocusedInteraction>()
            .and_then(|owner|owner.current.as_ref()).map(|prompt|prompt.entity.as_str());
        let live=world.get_resource::<LiveEntities>();
        let _=writeln!(report,"\nOpenable owners (focused first, nearest8): focus={focus:?} live_owner={} pending_uses={} pending_updates={} prepared_plans={}",
            live.is_some(),self.openable_uses_pending,self.openable_updates_pending,self.prepared_openables.len());
        let mut nearest=Vec::new();
        for &slot in self.openables.roster.indices.iter().take(self.openables.allocated_count()) {
            let Some(component)=self.openables.get(slot) else{continue};
            let Some(entity)=component.owner.entity.and_then(|index|self.entities.get(index)) else{continue};
            let distance=entity.frame.pose.position.iter().zip(eye).map(|(a,b)|(a-b)*(a-b)).sum::<f32>();
            nearest.push((focus!=Some(entity.name.as_str()),distance,slot));
        }
        nearest.sort_by(|a,b|a.0.cmp(&b.0).then_with(||a.1.total_cmp(&b.1)));
        for (_,distance,slot) in nearest.into_iter().take(8) {
            let component=self.openables.get(slot).expect("same immutable F8 roster");
            let index=component.owner.entity.expect("same immutable F8 owner");
            let entity=&self.entities[index];
            let state=match component.state{State::Closed=>("Closed",None),State::Animated{opened,direction}=>("Animated",Some((opened,direction))),State::Opened{direction}=>("Opened",Some((true,direction)))};
            let _=writeln!(report,"  name={} id={:?} live={:?} distance={} handle={:?} generation={} owner_flags={:#x} started={} initial={:?} declared_initial={:?} state={state:?} locked={} blocked={} hidden={} usable_by_player={} child={:?} linked={:?} command_pending={} accepted_uses={}",
                entity.name,entity.shared,live.map(|live|live.get(entity.shared).is_some()),distance.sqrt(),
                ComponentHandle{class_id:self.classes[55],slot},component.owner.generation,component.owner.flags,component.started,
                component.initial,component.definition.initial,component.locked,component.blocked,entity.entity_flags&1!=0,
                entity.entity_flags_d7&1!=0,component.child,component.linked,component.command_pending,component.uses.len());
            let _=writeln!(report,"    blockers={:?} needed_keys={:?} aliases={:?} declaration_aliases={:?} actor_frame={:?}",
                component.blockers,component.needed_keys,component.aliases,component.definition.animations,entity.frame);
            if let Some(request)=component.uses.front(){let _=writeln!(report,"    first_accepted_use instigator={:?} local={} occlusion={} linked={} linked_occlusion={}",request.instigator,request.local_done,request.local_occlusion_done,request.linked_done,request.linked_occlusion_done);}
            if let Some(child)=component.child {
                let child_index=self.shared_indices.get(&child).copied();
                let _=writeln!(report,"    child_frame={:?}",child_index.and_then(|index|self.entities.get(index)).map(|entity|entity.frame));
                self.write_runtime_animation_diagnostics(child,report);
                if let Some(models)=world.get_resource::<super::super::world_models::WorldModels>() {
                    let _=writeln!(report,"    renderer_parent_pose={:?} renderer_child_pose={:?}",models.publication_pose(entity.shared),models.publication_pose(child));
                }
            }
            let bindings=self.document_hierarchy.bindings.iter().filter(|binding|binding.child==index);
            for binding in bindings.take(4){let _=writeln!(report,"    binding parent={} joint={} parent_placement={:?}",binding.parent,binding.joint,binding.parent_placement);}
        }
    }
}
