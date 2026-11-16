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
        let animation = self
            .animations
            .get_mut(handle.slot)
            .ok_or("AnimBase payload absent")?;
        animation.running = true;
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
    pub(super) fn stop_animation(&mut self, handle: ComponentHandle) -> Result<(), String> {
        let animation = self
            .animations
            .get_mut(handle.slot)
            .ok_or("AnimBase payload absent")?;
        animation.running = false;
        animation.pending = None;
        animation.frame = None;
        Ok(())
    }
    pub(super) fn retire_animation(&mut self, handle: ComponentHandle) -> Result<(), String> {
        let animation = self
            .animations
            .get_mut(handle.slot)
            .ok_or("AnimBase payload absent")?;
        animation.running = false;
        animation.pending = None;
        animation.node = None;
        animation.rig = None;
        animation.default_alias = None;
        animation.matrices = Vec::new();
        animation.ready = Vec::new();
        animation.frame = None;
        Ok(())
    }
    fn animation_handle(&self, id: EntityId) -> Result<Option<ComponentHandle>, String> {
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(None);
        };
        if self.entities[index].retired {
            return Ok(None);
        }
        self.entities[index]
            .components
            .find_component([self.classes[15]; 2], [0x28, 0], |handle| {
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
    fn update_runtime_animations(&mut self, clock: &GameClock) -> Result<(), String> {
        for position in 0..self.animations.allocated_count() {
            let slot = self.animations.roster.indices[position];
            let animation = self
                .animations
                .get_mut(slot)
                .ok_or("AnimBase roster slot absent")?;
            if !animation.running || animation.frame == Some(clock.frame_marker()) {
                continue;
            }
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
        }
        Ok(())
    }
}
pub(in crate::app) fn update(
    scene: Option<bevy::prelude::ResMut<PhysicsScene>>,
    clock: Option<bevy::prelude::Res<crate::gameplay::game_clock_runtime::ClockRuntime>>,
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
        .and_then(|clock| scene.update_runtime_animations(clock));
    if let Err(error) = result {
        bevy::log::error!("Runtime animation failed: {error}");
    }
}
