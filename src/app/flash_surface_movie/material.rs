//! Per-model diffuseMap and Color overrides (2369c0/236900 ->343990).
//! Immutable source materials stay shared; only a touched model's material
//! instances retain the movie target. Stop restores the exact original handle.
use super::*;
use crate::gameplay::entity_lifetime::{EntityId,LiveEntities};
use super::super::{MaterialHandle,ProbeMaterial,DecalMaterial,RenderModelSurface};

struct Material {original:MaterialHandle,bound:MaterialHandle,streaming:Vec<(usize,crate::rendering::bevy_backend::surface_gpu::ImageRole)>}
struct Draw {entity:Entity,material:usize}
struct Queued {index:usize,material:usize}
#[derive(Default)]
pub(super) struct Bindings {
    materials:Vec<Material>,draws:Vec<Draw>,queued:Vec<Queued>,
    placements:Option<Vec<usize>>,queue_identity:Option<(usize,usize)>,
    runtime_draws:Vec<Entity>,
    target:Option<bevy::asset::AssetId<Image>>,
}
impl Bindings {
    pub(super) fn bound(&self)->bool{!self.materials.is_empty()}
    pub(super) fn set_target(&mut self,world:&mut World,target:&Handle<Image>,activate:bool)->Result<(),String>{
        if self.target==Some(target.id()){return Ok(())}
        for material in &self.materials {match &material.bound{
            MaterialHandle::Probe(handle)=>{let mut assets=world.resource_mut::<Assets<ProbeMaterial>>();
                let mut value=assets.get_mut(handle).ok_or("world movie model material disappeared")?;
                value.base.base_color_texture=Some(target.clone());value.extension.diffuse0=Some(target.clone());
                if activate {value.base.base_color=Color::WHITE;}}
            MaterialHandle::Decal(handle)=>{let mut assets=world.resource_mut::<Assets<DecalMaterial>>();
                let mut value=assets.get_mut(handle).ok_or("world movie model decal disappeared")?;
                value.base.base_color_texture=Some(target.clone());value.extension.diffuse0=Some(target.clone());
                if activate {value.base.base_color=Color::WHITE;}}
            _=>return Err("world movie model parameter type changed".into()),
        }}
        self.target=Some(target.id());Ok(())
    }
    pub(super) fn bind(&mut self,world:&mut World,id:EntityId,target:&Handle<Image>,activate:bool)->Result<(),String>{
        self.set_target(world,target,activate)?;
        if self.placements.is_none(){
            //Registry Stop may pump retirement while its canonical live table
            //is borrowed by the host. New bindings stay pending for the next
            //ordinary publication; never create another entity lookup owner.
            let Some(live)=world.get_resource::<LiveEntities>() else{return Ok(())};
            let(name,_)=live.get(id).ok_or("world movie model generation stale")?;
            self.placements=Some(world.get_resource::<super::super::RenderSourcePlacements>()
                .map(|sources|sources.0.iter().enumerate().filter_map(|(index,source)|source.id.eq_ignore_ascii_case(name).then_some(index)).collect()).unwrap_or_default());
        }
        let identity=world.get_resource::<super::super::SceneRenderQueue>().map(|queue|(queue.instances.as_ptr() as usize,queue.instances.len()));
        if identity!=self.queue_identity{
            let requests=world.get_resource::<super::super::SceneRenderQueue>().map(|queue|queue.instances.iter().enumerate().filter_map(|(index,instance)|{
                let placement=match instance.source?.surface{RenderModelSurface::Bmodel{placement,..}|RenderModelSurface::Bmd6{placement,..}=>placement,_=>return None};
                if !self.placements.as_ref()?.contains(&placement)||self.queued.iter().any(|draw|draw.index==index){return None}
                Some((index,instance.entity,instance.material.clone()))
            }).collect::<Vec<_>>()).unwrap_or_default();
            for(index,entity,original)in requests{
                let material=self.material(world,original,target,activate)?;
                world.resource_mut::<super::super::SceneRenderQueue>().instances[index].material=self.materials[material].bound.clone();
                self.queued.push(Queued{index,material});
                if let Some(entity)=entity{self.set_draw(world,entity,material)?;}
            }
            self.queue_identity=identity;
        }
        //Runtime actors own their actual draw list. Borrow those few model
        //draws; never scan all managed entities or create another model cache.
        self.runtime_draws.clear();
        if let Some(draws)=world.get_resource::<super::super::world_models::WorldModels>().and_then(|models|models.publication_draws(id)){self.runtime_draws.extend(draws);}
        for index in 0..self.runtime_draws.len() {
            let entity=self.runtime_draws[index];
            if self.draws.iter().any(|draw|draw.entity==entity){continue}
            let original=if let Some(material)=world.get::<MeshMaterial3d<ProbeMaterial>>(entity){MaterialHandle::Probe(material.0.clone())}
                else if let Some(material)=world.get::<MeshMaterial3d<DecalMaterial>>(entity){MaterialHandle::Decal(material.0.clone())}
                else{return Err("world movie model has no supported diffuseMap material owner".into())};
            let material=self.material(world,original,target,activate)?;self.set_draw(world,entity,material)?;
        }
        Ok(())
    }
    fn material(&mut self,world:&mut World,original:MaterialHandle,target:&Handle<Image>,activate:bool)->Result<usize,String>{
        if let Some(index)=self.materials.iter().position(|material|material.original==original){return Ok(index)}
        let bound=match &original {
            MaterialHandle::Probe(handle)=>{
                let mut assets=world.resource_mut::<Assets<ProbeMaterial>>();
                let mut material=assets.get(handle).ok_or("world movie source material not resident")?.clone();
                material.base.base_color_texture=Some(target.clone());if activate {material.base.base_color=Color::WHITE;}
                material.extension.diffuse0=Some(target.clone());
                MaterialHandle::Probe(assets.add(material))
            }
            MaterialHandle::Decal(handle)=>{
                let mut assets=world.resource_mut::<Assets<DecalMaterial>>();
                let mut material=assets.get(handle).ok_or("world movie source decal material not resident")?.clone();
                material.base.base_color_texture=Some(target.clone());if activate {material.base.base_color=Color::WHITE;}
                material.extension.diffuse0=Some(target.clone());
                MaterialHandle::Decal(assets.add(material))
            }
            _=>return Err("world movie diffuseMap override requires an admitted surface material".into()),
        };
        let streaming=if let Some(mut queue)=world.get_resource_mut::<super::super::SceneRenderQueue>(){
            let mut bindings=Vec::new();
            for(slot,values)in queue.diffuse_image_materials.iter().enumerate(){for(material,role)in values{
                if *material==original&&role.0!=0{bindings.push((slot,*role));}
            }}
            let mut slots=Vec::new();
            for &(slot,role) in &bindings{
                queue.diffuse_image_materials[slot].push((bound.clone(),role));
                if !slots.contains(&slot){slots.push(slot);}
            }
            queue.material_images.insert(bound.clone(),slots);
            if let Some(sort)=queue.material_stage_sorts.get(&original).cloned(){queue.material_stage_sorts.insert(bound.clone(),sort);}
            bindings
        }else{Vec::new()};
        let index=self.materials.len();self.materials.push(Material{original,bound,streaming});Ok(index)
    }
    fn set_draw(&mut self,world:&mut World,entity:Entity,material:usize)->Result<(),String>{
        let mut draw=world.get_entity_mut(entity).map_err(|_|"world movie draw generation absent")?;
        match &self.materials[material].bound {
            MaterialHandle::Probe(handle)=>{draw.insert(MeshMaterial3d(handle.clone()));}
            MaterialHandle::Decal(handle)=>{draw.insert(MeshMaterial3d(handle.clone()));}
            _=>return Err("world movie bound material type changed".into()),
        }
        self.draws.push(Draw{entity,material});Ok(())
    }
    pub(super) fn restore(&mut self,world:&mut World)->Result<(),String>{
        //A queued model can allocate its draw after the initial override.
        //Include that actual publication even if Stop precedes the next poll.
        let published=world.get_resource::<super::super::SceneRenderQueue>().map(|queue|self.queued.iter().filter_map(|draw|{
            queue.instances.get(draw.index)?.entity.map(|entity|(entity,draw.material))
        }).collect::<Vec<_>>()).unwrap_or_default();
        for(entity,material)in published {if !self.draws.iter().any(|draw|draw.entity==entity){self.draws.push(Draw{entity,material});}}
        if let Some(mut queue)=world.get_resource_mut::<super::super::SceneRenderQueue>(){
            for draw in &self.queued {if let Some(instance)=queue.instances.get_mut(draw.index){
                let material=&self.materials[draw.material];
                if instance.material==material.bound {instance.material=material.original.clone();}
            }}
            for material in &self.materials{
                for &(slot,role) in &material.streaming {if let Some(bindings)=queue.diffuse_image_materials.get_mut(slot){bindings.retain(|(bound,bound_role)|*bound!=material.bound||*bound_role!=role);}}
                queue.material_images.remove(&material.bound);queue.material_stage_sorts.remove(&material.bound);
            }
        }
        for draw in &self.draws{
            let material=&self.materials[draw.material];
            if let Ok(mut entity)=world.get_entity_mut(draw.entity){match(&material.original,&material.bound){
                (MaterialHandle::Probe(original),MaterialHandle::Probe(bound))=>{
                    if entity.get::<MeshMaterial3d<ProbeMaterial>>().is_some_and(|value|value.0==*bound){entity.insert(MeshMaterial3d(original.clone()));}
                }
                (MaterialHandle::Decal(original),MaterialHandle::Decal(bound))=>{
                    if entity.get::<MeshMaterial3d<DecalMaterial>>().is_some_and(|value|value.0==*bound){entity.insert(MeshMaterial3d(original.clone()));}
                }
                _=>return Err("world movie material restoration type mismatch".into()),
            }}
        }
        self.draws.clear();self.queued.clear();self.materials.clear();self.runtime_draws.clear();self.placements=None;self.queue_identity=None;self.target=None;Ok(())
    }
}
