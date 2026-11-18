//! Actual source-journal/ticket observations; never requests or extends FX life.
use super::*;
use crate::gameplay::pickable_component::CheckedComponent;

struct Metadata {
    component: CheckedComponent,
    first: usize,
    through: usize,
    definition: std::sync::Weak<crate::assets::breakable::Definition>,
    expected: std::collections::HashMap<String,usize>,
}
impl Metadata {
    fn advance(&mut self, through: usize, definition: &crate::assets::breakable::Definition) -> Result<(),String> {
        if through>definition.steps.len(){return Err("contact Breakable step exceeds actual declaration".into())}
        if through<=self.through{return Ok(())}
        for step in &definition.steps[self.through..through]{
            for path in step.owner_fx.declaration.iter().chain(step.chunks.iter().filter_map(|chunk|chunk.fx.declaration.as_ref())){
                *self.expected.entry(path.clone()).or_default()+=1;
            }
        }
        self.through=through;Ok(())
    }
}
struct Tail {
    observation: WorldEffectObservation,
    requires_particles: bool,
    requires_sound: bool,
}
pub(super) struct ContactEffects { metadata: Vec<Metadata>, tails: Vec<Tail> }
impl ContactEffects {
    pub(super) fn baseline(world: &World, id: EntityId) -> Result<Self, String> {
        let mut metadata=Vec::new();
        world.resource::<super::super::physics_scene::PhysicsScene>().visit_breakable_progress(id,|component,step,definition|{
            metadata.push(Metadata{component,first:step,through:step,definition:std::sync::Arc::downgrade(definition),expected:Default::default()});
        })?;
        Ok(Self{metadata,tails:Vec::new()})
    }
    pub(super) fn update(&mut self, world: &mut World, id: EntityId) -> Result<bool,String> {
        let mut metadata_error=None;
        world.resource::<super::super::physics_scene::PhysicsScene>().visit_breakable_progress(id,|component,step,definition|{
            if let Some(row)=self.metadata.iter_mut().find(|row|row.component.handle==component.handle&&row.component.generation==component.generation){
                if let Err(error)=row.advance(step,definition){metadata_error=Some(error);}
            }
        })?;
        if let Some(error)=metadata_error.take(){return Err(error)}
        super::super::breakables::visit_tails(world,id,|component,step,ticket|{
            let Some(row)=self.metadata.iter_mut().find(|row|row.component.handle==component.handle&&row.component.generation==component.generation&&step>=row.first)else{return};
            if step+1>row.through{
                let Some(definition)=row.definition.upgrade()else{metadata_error=Some("contact source declaration retired before actual step metadata could be borrowed".into());return};
                if let Err(error)=row.advance(step+1,&definition){metadata_error=Some(error);return}
                // The temporary metadata borrow ends here; no strong source
                // declaration credit survives in the diagnostic Scope.
            }
            if self.tails.iter().any(|tail|tail.observation.ticket.same_request(ticket)){return}
            self.tails.push(Tail{observation:WorldEffectObservation{ticket:ticket.clone(),identity:None,publication:None,
                started:None,started_app_frame:None,particles:false,retired:false,emitters:HashSet::new(),
                expected_events:HashSet::new(),admissions:HashSet::new(),processed:HashSet::new(),last:None},
                requires_particles:false,requires_sound:false});
        });
        if let Some(error)=metadata_error{return Err(error)}
        for (index,tail) in self.tails.iter_mut().enumerate(){
            // Use the actual published prepared root descriptor. Conditional
            // location groups are not forced to run by this observer.
            if tail.observation.identity.is_none(){tail.observation.identity=super::super::usable_effects::ticket_request_identity(world,&tail.observation.ticket);}
            if let Some((raw,path))=&tail.observation.identity{
                if let Some(dynamic)=world.get_resource::<crate::rendering::bevy_backend::world_effects::DynamicEffects>(){
                    if let Some(entry)=dynamic.0.iter().find(|entry|entry.owner==format!("entity:{raw}")){
                        if let Some(definition)=entry.sources.effects.get(path.as_ref()){
                            let definition=definition.as_ref().map_err(Clone::clone)?;
                            for group in &definition.groups{
                                if group.asset.location!=crate::assets::fx::EffectGroupLocation::Everywhere{continue}
                                tail.requires_particles|=!group.particles.is_empty();
                                // Game-defined events intentionally notify without
                                // middleware playback; they are not missing voices.
                                tail.requires_sound|=group.sounds.iter().filter_map(|sound|sound.start.as_ref())
                                    .any(|event|!event.declaration.game_defined);
                            }
                        }
                    }
                }
            }
            if super::Observation::world_effect_progress(world,&mut tail.observation,u8::try_from(index+1).map_err(|_|"contact destruction FX count exceeds diagnostic contact IDs")?)?{
                if tail.requires_particles&&!tail.observation.particles{return Err("contact destruction FX retired without its authored unconditional particle chains".into())}
                if tail.requires_sound&&(tail.observation.admissions.is_empty()||!tail.observation.processed.is_superset(&tail.observation.admissions)){
                    return Err(format!("contact destruction FX retired without actual sound admission/SourceReady processing: admissions={:?} processed={:?}; no audibility claim",tail.observation.admissions,tail.observation.processed));
                }
            }
        }
        let mut expected=std::collections::HashMap::<&str,usize>::new();
        for row in &self.metadata{
            for (path,count) in &row.expected{*expected.entry(path.as_str()).or_default()+=count;}
        }
        for (path,count) in expected{
            let observed=self.tails.iter().filter(|tail|tail.observation.identity.as_ref().is_some_and(|(_,actual)|actual.as_ref()==path)).count();
            if observed<count{return Ok(false)}
        }
        Ok(self.tails.iter().all(|tail|tail.observation.retired))
    }
    pub(super) fn report(&self,id:EntityId){
        eprintln!("ACTION_SMOKE_PASS check=contact_authored_destruction_FX entity={id:?} actual_source_components={} advanced_steps={} actual_tickets={} naturally_retired={} real_particle_tickets={} admitted_sounds={} processed_sounds={} visible_or_audible_output_not_claimed=true",
            self.metadata.len(),self.metadata.iter().map(|row|row.through-row.first).sum::<usize>(),self.tails.len(),
            self.tails.iter().filter(|tail|tail.observation.retired).count(),self.tails.iter().filter(|tail|tail.observation.particles).count(),
            self.tails.iter().map(|tail|tail.observation.admissions.len()).sum::<usize>(),self.tails.iter().map(|tail|tail.observation.processed.len()).sum::<usize>());
    }
}
