//! Actual dynamic constructors8aecc0/874c50/784db0, sharing the level's
//! component factories, managed generations and canonical physics world.
use super::*;
use crate::{assets::runtime_entity::{Definition,Kind},gameplay::{entity_lifetime::{LiveEntities,EntityId},entity_events::EntityEvents,component_state::ComponentLifecycleWorld}};
use super::super::runtime_entities::{BodyDraft,BodyResources,SpawnOptions};
impl PhysicsScene{
    pub(in crate::app) fn publish_runtime_resources(&mut self,drafts:Vec<BodyDraft>)->Result<Vec<BodyResources>,String>{
        let mut query=self.world.write().map_err(|_|"runtime physics world is poisoned")?;
        if !query.storage.materials.library.added.is_empty(){return Err("runtime material publication requires its actual registered observer host".into());}
        drafts.into_iter().map(|body|Ok(BodyResources{component:body.component,resources:Arc::new(body.draft.publish(&mut query.storage.materials,|_,_,_,_|unreachable!("material subscriber list was checked empty"))?)})).collect()
    }
    pub(in crate::app) fn allocate_runtime_actor(&mut self,live:&mut LiveEntities,definition:&Definition,options:SpawnOptions)->Result<EntityId,String>{
        if self.registry.is_none(){return Err("runtime entity factory registry is borrowed".into());}
        let tag=self.types.intern_name(Some(b"MAIN"))?;
        let id=live.allocate_managed_auto_name(&definition.entity,definition.class_index,0,tag)?;
        let name=live.get(id).ok_or("new managed actor disappeared")?.0.to_owned();
        let index=self.free_entities.pop().unwrap_or(self.entities.len());
        let entity=PhysicsEntity{name,shared:id,components:EntityComponents::new(index),class:definition.class_index,environment:None,flags:0,
            entity_flags:0x001e8008|if options.hidden_disabled{3}else{0},entity_flags_d7:0,priority:0,
            frame:crate::gameplay::reference_frame::PublishedFrame{pose:options.frame,scale:[options.scale;3],velocities:[[0.;3];2]},timespace:0,retired:false,external:false};
        if index==self.entities.len(){self.entities.push(entity);self.entity_flags_d4.push(0);}else{
            if !self.entities[index].retired||!self.entities[index].components.entries.values().is_empty(){return Err("free runtime entity header is still owned".into());}
            self.entities[index]=entity;self.entity_flags_d4[index]=0;
        }
        self.shared_indices.insert(id,index);Ok(id)
    }
    pub(in crate::app) fn admit_runtime_actor(&mut self,world:&mut bevy::prelude::World,live:&mut LiveEntities,events:&mut EntityEvents,id:EntityId,
        definition:Arc<Definition>,resources:&[BodyResources],projectile:Option<crate::gameplay::projectiles::SpawnRequest>)->Result<(),String>{
        let index=*self.shared_indices.get(&id).ok_or("runtime managed actor is absent")?;
        let mut registry=self.registry.take().ok_or("runtime component registry is borrowed")?;
        let result=(||{
            for(component_index,component)in definition.components.iter().enumerate(){
                if let Kind::Unhosted{class}=&component.kind{
                    static AI:std::sync::Once=std::sync::Once::new();static AIM:std::sync::Once=std::sync::Once::new();static OBS:std::sync::Once=std::sync::Once::new();
                    let once=match *class{"arkCpntAIAttentionSource"=>&AI,"arkCpntAimAssist"=>&AIM,"arkCpntObservable"=>&OBS,_=>return Err("runtime unhosted capability is not source admitted".into())};
                    once.call_once(||eprintln!("RUNTIME_ENTITY_UNHOSTED class={class} consumers=absent state=not-constructed"));continue;
                }
                let declaration=self.types.declaration(component.factory,Arc::as_ptr(&definition) as usize+component_index,(*component.scheduling).clone())?;
                let immediate=declaration.scheduling.starts_on_spawn();
                let handle=match &component.kind{
                    Kind::Rigid(value)=>{
                        let bodies=&mut self.actors.bodies;let free=&mut self.free_rigid_movement;
                        ComponentFactory{declaration:&declaration,pool:&mut self.rigid_bodies,registry:&mut registry,grow_one_at_a_time:false}.create(&mut self.entities[index].components,immediate,
                            |_,_,_|super::super::rigid_body::RigidBodyComponent::new(value.clone(),bodies,free),|_,_,_|Ok(()))?
                    },
                    Kind::Throwable(value)=>{
                        let bodies=&mut self.actors.bodies;let free=&mut self.free_rigid_movement;
                        ComponentFactory{declaration:&declaration,pool:&mut self.throwables,registry:&mut registry,grow_one_at_a_time:false}.create(&mut self.entities[index].components,immediate,
                            |_,_,_|super::super::rigid_body::RigidBodyComponent::new_throwable(value.clone(),bodies,free),|_,_,_|Ok(()))?
                    },
                    Kind::Health(value)=>{
                        if component.class_index!=self.types.factory(self.factories[10])?.class{return Err("runtime derived Health has no actual payload pool".into());}
                        ComponentFactory{declaration:&declaration,pool:&mut self.health,registry:&mut registry,grow_one_at_a_time:false}.create(&mut self.entities[index].components,immediate,
                            |_,_,_|Ok(damage::HealthComponent{owner:Default::default(),receiver:value.receiver.clone(),maximum:value.maximum,current:1.,attacker:None}),|_,_,_|Ok(()))?
                    },
                    Kind::Breakable(value)=>ComponentFactory{declaration:&declaration,pool:&mut self.breakables,registry:&mut registry,grow_one_at_a_time:false}.create(&mut self.entities[index].components,immediate,|_,_,_|Ok(breakable::BreakableComponent::new(value.clone())),|_,_,_|Ok(()))?,
                    Kind::Projectile(_)=>{
                        let prepared=crate::assets::projectiles::Prepared::from_entity(definition.clone())?;
                        ComponentFactory{declaration:&declaration,pool:&mut self.projectiles,registry:&mut registry,grow_one_at_a_time:false}.create(&mut self.entities[index].components,immediate,|_,_,_|Ok(super::super::projectiles::Component::new(prepared)),|_,_,_|Ok(()))?
                    },
                    Kind::Pickable(value)=>ComponentFactory{declaration:&declaration,pool:&mut self.pickables,registry:&mut registry,grow_one_at_a_time:false}.create(&mut self.entities[index].components,immediate,|_,_,_|Ok(crate::gameplay::pickable_component::PickableComponent::new(value.clone())),|_,_,_|Ok(()))?,
                    Kind::Audio(value)=>ComponentFactory{declaration:&declaration,pool:&mut self.audios,registry:&mut registry,grow_one_at_a_time:false}.create(&mut self.entities[index].components,immediate,|_,_,_|{
                        let mut audio=crate::gameplay::sound::component::AudioComponent::default();audio.runtime_definition=Some(value.clone());Ok(audio)
                    },|_,_,_|Ok(()))?,
                    Kind::ReferenceFrame=>Some(self.reference_actor(index,&mut registry)?),
                    Kind::AnimBasic=>return Err("runtime AnimBasic component requires its actual typed component pool".into()),
                    Kind::Unhosted{..}=>unreachable!(),
                }.ok_or("runtime component creation was disabled")?;
                if self.is_rigid_class(handle.class_id){
                    let resource=resources.iter().find(|resource|resource.component==component_index).ok_or("runtime rigid prepared resource is absent")?;
                    self.rigid_component_mut(handle)?.bind_prepared_resources(None,resource.resources.clone_refs())?;
                }
                if handle.class_id==self.classes[6]{let generation=self.pickables.get(handle.slot).unwrap().owner.generation;self.readable_components.insert(self.entities[index].name.clone(),crate::gameplay::pickable_component::CheckedComponent{handle,generation});}
                self.declarations.insert(handle.packed(),declaration);
            }
            self.reference_actor(index,&mut registry)?;
            if let Some(request)=projectile{self.configure_projectile(id,request)?;}
            live.initialized(id)?;live.spawned(id)?;
            registry.initialize_and_start(&mut interaction_components::Startup{scene:self,live,events})?;
            let _=world;Ok(())
        })();self.registry=Some(registry);result
    }
}
