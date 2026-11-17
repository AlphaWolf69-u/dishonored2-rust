//!755350/757b70: the actual Retargeting owner on an animated prop.
//!An empty slave list is the native constructed state. Ordinary door tag
//!bindings belong to752900, not to a manufactured retargeting slave.
use super::*;
use crate::gameplay::{component_state::ComponentInstance,pickable_component::CheckedComponent};

pub(super) struct Component {
    pub owner:ComponentOwner,
    physics:Option<CheckedComponent>,
    animation:Option<CheckedComponent>,
}
impl ComponentInstance for Component {
    fn owner(&self)->&ComponentOwner{&self.owner}
    fn owner_mut(&mut self)->&mut ComponentOwner{&mut self.owner}
}
impl Component {
    fn new()->Self{Self{owner:Default::default(),physics:None,animation:None}}
}
impl PhysicsScene {
    pub(super) fn create_retargeting(&mut self,index:usize,declaration:&ComponentDeclaration,
        registry:&mut ComponentRegistry,immediate:bool)->Result<Option<ComponentHandle>,String>{
        ComponentFactory{declaration,pool:&mut self.retargeters,registry,grow_one_at_a_time:false}
            .create(&mut self.entities[index].components,immediate,|_,_,_|Ok(Component::new()),|_,_,_|Ok(()))
    }
    pub(super) fn initialize_retargeting(&mut self,handle:ComponentHandle)->Result<(),String>{
        let index=self.owner(handle)?.entity.ok_or("Retargeting entity absent")?;
        //757b70 looks up real PhysicsSystemEvolved and AnimBase with mode0.
        //A door child has no physical slave system; that null branch is real.
        let physics=self.cue_dependency(index,b"arkCpntPhysicsSystemEvolved",[0,0])?;
        let animation=self.cue_dependency(index,b"arkCpntAnimBase",[0,0])?;
        let component=self.retargeters.get_mut(handle.slot).ok_or("Retargeting payload absent")?;
        component.physics=physics;component.animation=animation;
        component.owner.mark_initialized();
        Ok(())
    }
    pub(super) fn deinitialize_retargeting(&mut self,handle:ComponentHandle)->Result<(),String>{
        let component=self.retargeters.get_mut(handle.slot).ok_or("Retargeting payload absent")?;
        component.physics=None;component.animation=None;
        Ok(())
    }
}
