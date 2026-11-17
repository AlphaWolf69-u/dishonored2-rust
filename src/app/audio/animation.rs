//! AnimBasic A9 delivery through the actual Audio component and SoundEvent FIFO.
//! 875ba0 ->70a030->708810->709110. Source709110 is synchronous;
//! this host journal bridges its accepted post to the existing output consumer.
use super::*;
use crate::gameplay::{component_state::entity::ComponentHandle,
    entity_lifetime::{EntityId, LiveEntities}, sound::component::AudioComponent};
use crate::assets::wwise::SoundEventDeclaration;

pub(super) struct Cookie {
    pub owner: EntityId,
    pub emitter: Emitter,
    pub event: Arc<SoundEventDeclaration>,
    pub position: Vec3,
    pub stimulus_sent: bool,
}

/// Resolve the actual recipient before acquiring voices/media. No recipient is
///the genuine unsubscribed875ba0 branch, not an invented audio component.
pub(in crate::app) fn prepare_usable_animation_audio(
    world: &mut World, entity: &str, placement: usize,
    declarations: impl IntoIterator<Item = Arc<SoundEventDeclaration>>,
    joints: &[(i16, super::super::physics_skinning::Affine, f32)], scale: [f32; 3],
    admitted_scope: &mut Option<Arc<str>>,
) -> Result<Option<bool>, String> {
    if let Some(scope) = admitted_scope { return super::events_ready(world, scope).map(Some); }
    if !world.contains_resource::<super::super::physics_scene::PhysicsScene>() {
        return Err("AnimBasic sound preparation requires actual component ownership".into());
    }
    let live = world.get_resource::<LiveEntities>().ok_or("AnimBasic sound identities absent")?;
    let id = live.find(entity).ok_or("AnimBasic sound actor generation absent")?;
    let scope: Arc<str> = format!("animbasic:{id:?}:{placement}").into();
    let recipient = world.resource_scope(|world, mut scene: Mut<super::super::physics_scene::PhysicsScene>| {
        let live = world.get_resource::<LiveEntities>().ok_or("AnimBasic sound identities absent")?;
        scene.admit_animation_audio(live, entity, scope.clone())
    })?;
    let Some((handle, generation)) = recipient else { return Ok(None); };
    let declarations: Vec<_> = declarations.into_iter().collect();
    if declarations.iter().any(|event| event.game_descriptor.is_some())
        && !world.contains_resource::<crate::rendering::bevy_backend::world_sounds::WorldSounds>() {
        return Err("AnimBasic sound stimulus owner absent".into());
    }
    super::reserve_events(world, scope.clone(), declarations)?;
    if !joints.is_empty() {
        world.resource_scope(|world, mut scene: Mut<super::super::physics_scene::PhysicsScene>| {
            let mut frames = world.get_resource_mut::<super::super::joint_frames::JointFrames>()
                .ok_or("AnimBasic sound joint-frame owner absent")?;
            for &(joint, initial, range) in joints {
                scene.admit_animation_audio_joint(handle, generation, joint, placement, range)?;
                frames.admit_audio(placement, scale, joint, initial)?;
            }
            Ok::<_, String>(())
        })?;
    }
    let ready = super::events_ready(world, &scope)?;
    *admitted_scope = Some(scope);
    Ok(Some(ready))
}

pub(super) fn prepare(world: &mut World) -> Result<(), bevy::ecs::error::BevyError> {
    super::super::usable_animation::prepare_audio(world).map_err(Into::into)
}

/// Ordinary publication already borrows Scene, its actual pose bank and the
///output FIFO. Reuse that borrow instead of moving the large PhysicsScene
///through another exclusive resource scope each frame.
pub(super) fn drain_published_posts(handle: ComponentHandle, audio: &mut AudioComponent,
    frames: Option<&super::super::joint_frames::JointFrames>, scene: &mut spatial::Scene,
    events: &mut MessageWriter<SoundEvent>, completions: &mut SoundCompletions,
) -> Result<(), String> {
    while let Some(post) = audio.posts.front() {
        let emitter = if post.joint < 0 {
            Emitter::Component { handle: handle.packed(), generation: audio.owner.generation }
        } else {
            Emitter::ComponentJoint { handle: handle.packed(), generation: audio.owner.generation, joint: post.joint }
        };
        let frame = post_frame(post, frames)?;
        let position = Vec3::from_array(frame.position);
        scene.publish_frame(emitter, spatial::Position { position, front: Some(Vec3::from_array(frame.basis[0])) });
        let message = events.write(SoundEvent(Request::PostAt { event: post.event.id, emitter,
            position, front: Some(Vec3::from_array(frame.basis[0])), parameter: None }));
        completions.component_posts.insert(message.id, Cookie { owner: post.entity, emitter,
            event: post.event.clone(), position, stimulus_sent: false });
        audio.posts.pop_front();
    }
    Ok(())
}

fn post_frame(post: &crate::gameplay::sound::component::AnimationPost,
    frames: Option<&super::super::joint_frames::JointFrames>)
    -> Result<crate::gameplay::reference_frame::ReferenceFrame, String> {
    if post.joint < 0 { return Ok(post.frame); }
    frames.ok_or("Audio A9 joint frames absent")?
        .bound_frame(post.placement, post.joint, 3, [0.;3], [0.,0.,0.,1.], post.frame)?
        .ok_or_else(||"Audio A9 actual joint pose absent".into())
}

/// Called both by ordinary publication and by Stop/Deinit before object
///retirement. It does not depend on a listener, playing state, or LevelAudio
///update: outgoing/loading/paused accepted posts cannot strand their leases.
pub(in crate::app) fn drain_component_posts(world: &mut World, handle: ComponentHandle,
    generation: u32, audio: &mut AudioComponent) -> Result<(), String> {
    if audio.owner.generation != generation { return Err("Audio A9 generation differs".into()); }
    while let Some(post) = audio.posts.front() {
        let emitter = if post.joint < 0 {
            Emitter::Component { handle: handle.packed(), generation }
        } else {
            Emitter::ComponentJoint { handle: handle.packed(), generation, joint: post.joint }
        };
        let frame = post_frame(post, world.get_resource::<super::super::joint_frames::JointFrames>())?;
        let position = Vec3::from_array(frame.position);
        // The ordinary LevelAudio path supplies the SAME acoustic world's
        //propagation. This independent frame handoff also works during Stop.
        world.get_resource_mut::<spatial::Scene>().ok_or("Audio spatial owner absent")?
            .publish_frame(emitter, spatial::Position { position, front: Some(Vec3::from_array(frame.basis[0])) });
        if !world.contains_resource::<Messages<SoundEvent>>() || !world.contains_resource::<SoundCompletions>() {
            return Err("Audio A9 output FIFO absent".into());
        }
        let cookie = Cookie { owner: post.entity, emitter, event: post.event.clone(), position, stimulus_sent: false };
        let message = world.write_message(SoundEvent(Request::PostAt {
            event: post.event.id, emitter, position, front: Some(Vec3::from_array(frame.basis[0])), parameter: None,
        })).ok_or("Audio A9 output FIFO rejected post")?;
        world.resource_mut::<SoundCompletions>().component_posts.insert(message.id, cookie);
        if post.joint < 0 { audio.allocated = true; }
        audio.posts.pop_front();
    }
    Ok(())
}

///708620: called only after all real output objects/tickets acknowledged
///return. Demand frames and source reservations cannot outlive their owner.
pub(in crate::app) fn release_component_animation(world: &mut World, audio: &mut AudioComponent) -> Result<(), String> {
    if !audio.posts.is_empty() { return Err("Audio A9 accepted posts have not transferred".into()); }
    if !audio.joints.is_empty() {
        let mut frames = world.get_resource_mut::<super::super::joint_frames::JointFrames>()
            .ok_or("retiring Audio joint-frame owner absent")?;
        for joint in &audio.joints { frames.release_audio(joint.placement, joint.joint); }
    }
    for scope in &audio.animation_scopes { super::release_events(world, scope)?; }
    audio.animation_scopes.clear();
    audio.joints.clear();
    Ok(())
}
