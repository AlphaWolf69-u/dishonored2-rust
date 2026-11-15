//! Null-parent rigid movement interfaces and 753c10 publication on live owners.
use super::*;
use crate::gameplay::{
    collision_user_data::CollisionUserData,
    entity_lifetime::LiveEntities,
    game_clock::GameClock,
    physics_actor::{ActorMotionBody, ActorUpdatePhase, ResolvedMotion,
        basic_body::BasicBody, body_frame::body_origin_velocities,
        post_update::{BoundFrame, PostPhysicsWorld}, pre_update::PrePhysicsWorld},
    physics_world::update_queue::{ObjectReference, ObjectUpdateTask},
    reference_frame::{PublishedFrame, ReferenceFrame},
};
use crate::app::model_motion::{ModelPose, ModelPoseUpdates};

/// The shared render publisher accepts a rigid world delta from the actual
/// source frame. Both unbound body publication and808880 attachments use it.
pub(super) fn render_delta(initial: ReferenceFrame, current: ReferenceFrame) -> Result<[[f32; 4]; 3], String> {
    let mut inverse = initial.basis;
    if !crate::gameplay::reference_frame::invert_basis(&mut inverse) {
        return Err("rigid initial render basis is singular".into());
    }
    let basis = crate::gameplay::reference_frame::basis_product(inverse, current.basis);
    Ok(std::array::from_fn(|row| {
        let moved_origin = (0..3).map(|axis| basis[axis][row] * initial.position[axis]).sum::<f32>();
        [basis[0][row], basis[1][row], basis[2][row], current.position[row] - moved_origin]
    }))
}

struct RigidPostHost<'a> {
    entities: &'a mut [PhysicsEntity],
    live: &'a LiveEntities,
    world: &'a WorldStorage,
    rigid: &'a ComponentPool<crate::app::rigid_body::RigidBodyComponent>,
    rigid_class: i32,
    placements: &'a HashMap<usize, usize>,
    initial: &'a HashMap<usize, ReferenceFrame>,
    poses: &'a mut ModelPoseUpdates,
    clock: &'a GameClock,
    post: &'a ObjectUpdateTask<usize>,
    post_frame: i32,
}
impl RigidPostHost<'_> {
    fn entity(&self, index: usize) -> Result<&PhysicsEntity, String> {
        let entity = self.entities.get(index).ok_or("rigid actor entity is absent")?;
        let (name, class) = self.live.get(entity.shared).ok_or("rigid actor entity generation is stale")?;
        if name != entity.name || class != entity.class {
            return Err("rigid actor managed identity differs".into());
        }
        if self.live.parent(entity.shared)?.is_some() || !self.live.children(entity.shared)?.is_empty() {
            return Err("rigid actor requires the actual bound hierarchy owner".into());
        }
        if !self.initial.contains_key(&index) { return Err("rigid actor has no initial body frame".into()); }
        Ok(entity)
    }

    fn actor_entity(&self, actors: &PhysicsActorLinks, actor: usize) -> Result<usize, String> {
        let value = actors.actors.get_at(actor).ok_or("rigid reference actor is absent")?;
        let entity = value.owner.entity.ok_or("rigid reference actor entity is absent")?;
        let record = self.entity(entity)?;
        let handle = record.components.reference_actor.ok_or("rigid reference actor handle is absent")?;
        //8762d0 retains the old joint index after clearing the parent. A
        // dormant index alone is not a live attachment or resource owner.
        if usize::from(handle.slot) != actor || value.parent.is_some() || value.binding.entity.is_some()
            || !value.children.is_empty() {
            return Err("rigid actor has an unhosted binding or descendant".into());
        }
        // +18 dispatch needs the real requested-velocity/command owner. The
        // admitted static/keyframed document branch has no such requests.
        if value.has_enabled_motion() || value.motion.valid.into_iter().any(|valid| valid) {
            return Err("rigid actor desired-motion publication is not admitted".into());
        }
        Ok(entity)
    }

    fn body<'a>(&'a self, actors: &PhysicsActorLinks, actor: usize)
        -> Result<(&'a crate::app::rigid_body::RigidBodyComponent, &'a crate::gameplay::collision_body::CollisionBody), String> {
        let entity = self.actor_entity(actors, actor)?;
        let value = actors.actors.get_at(actor).ok_or("rigid actor is absent")?;
        if value.current_body == value.default_body { return Err("rigid movement body is detached".into()); }
        let link = actors.bodies.get(value.current_body).ok_or("rigid movement interface is absent")?;
        if link.actor != Some(actor) { return Err("rigid movement reciprocal owner differs".into()); }
        let body = self.world.bodies.get(link.body_id).ok_or("rigid movement body generation is stale")?;
        let handle = CollisionUserData::decode(body.user_data)?.component;
        if handle.class_id != self.rigid_class { return Err("rigid collision owner class differs".into()); }
        let component = self.rigid.get(handle.slot).ok_or("rigid component is absent")?;
        if component.owner.entity != Some(entity) || component.body() != Some(body.id)
            || component.movement_body()? != value.current_body {
            return Err("rigid collision and movement owners differ".into());
        }
        Ok((component, body))
    }

    fn publish_body(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<(), String> {
        let (component, body) = self.body(actors, actor)?;
        let pose = ReferenceFrame { position: body.transform[3][..3].try_into().unwrap(),
            basis: std::array::from_fn(|axis| body.transform[axis][..3].try_into().unwrap()) };
        let motion = self.world.motions.get(body.motion_id).ok_or("rigid publication motion is absent")?;
        // 80b7c0 reads component+140/+150: the requested vectors in its
        // +100 collision-data subobject, not the scaled +120/+130 vectors.
        let velocities = body_origin_velocities(pose.position, motion.pose.center[..3].try_into().unwrap(),
            component.velocities.requested_linear[..3].try_into().unwrap(),
            component.velocities.requested_angular[..3].try_into().unwrap());
        // 80a5f0 calls 80b7c0 with notify=false: this is already inside the
        // admitted 753c10 task, rather than a second independent queue producer.
        actors.publish_body_frame(actor, pose, velocities, false, self)
    }
}

impl ActorMotionBody for RigidPostHost<'_> {
    fn accepts(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<bool, String> {
        self.actor_entity(actors, actor)?;
        if actors.actors[actor].current_body == actors.actors[actor].default_body {
            BasicBody::Embedded.accepts(actors, actor)
        } else { Ok(self.body(actors, actor)?.1.flags & 8 != 0) } // 808e30
    }
    fn apply(&mut self, _: &mut PhysicsActorLinks, _: usize, _: ResolvedMotion) -> Result<u8, String> {
        Err("rigid desired-motion requires its actual queued velocity command owner".into())
    }
}
impl PrePhysicsWorld for RigidPostHost<'_> {
    fn entity_delta(&mut self, _: &mut PhysicsActorLinks, entity: usize) -> Result<f32, String> {
        self.clock.entity_delta(self.entity(entity)?.timespace).ok_or_else(|| "rigid entity timespace is absent".into())
    }
    fn entity_flags(&self, entity: usize) -> Result<u8, String> { Ok(self.entity(entity)?.entity_flags as u8) }
    fn bound_velocities(&mut self, _: &mut PhysicsActorLinks, _: usize, _: f32) -> Result<[[f32; 3]; 2], String> {
        Err("rigid bound velocity owner is not admitted".into())
    }
    fn pre_queue_frame(&self) -> i32 { self.post_frame }
    fn enqueue_pre(&mut self, _: &mut PhysicsActorLinks, _: usize) -> Result<(), String> {
        Err("rigid desired-motion pre-physics owner is not admitted".into())
    }
    fn child_count(&self, entity: usize) -> Result<usize, String> {
        Ok(self.live.children(self.entity(entity)?.shared)?.len())
    }
    fn child_entity(&self, _: usize, _: usize) -> Result<usize, String> {
        Err("rigid descendant publication is not admitted".into())
    }
    fn actor_for_entity(&mut self, _: &mut PhysicsActorLinks, _: usize) -> Result<usize, String> {
        Err("rigid descendant actor factory is not admitted".into())
    }
}
impl PostPhysicsWorld for RigidPostHost<'_> {
    fn publication_frame(&self) -> i32 { self.clock.frame_marker() }
    fn post_queue_frame(&self) -> i32 { self.post_frame }
    fn entity_frame(&self, entity: usize) -> Result<PublishedFrame, String> { Ok(self.entity(entity)?.frame) }
    fn publish_entity_frame(&mut self, entity: usize, frame: PublishedFrame) -> Result<(), String> {
        self.entity(entity)?;
        self.entities[entity].frame = frame;
        Ok(())
    }
    fn bound_frame(&mut self, _: &mut PhysicsActorLinks, _: usize, _: f32) -> Result<BoundFrame, String> {
        Err("rigid bound pose owner is not admitted".into())
    }
    fn accepts_pose(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<bool, String> {
        self.accepts(actors, actor)
    }
    fn update_body_pose(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<(), String> {
        self.publish_body(actors, actor)
    }
    fn body_handles_render_entity(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<bool, String> {
        let entity = self.actor_entity(actors, actor)?;
        if actors.actors[actor].current_body == actors.actors[actor].default_body {
            return BasicBody::Embedded.handles_render_entity(actors, actor);
        }
        self.body(actors, actor)?;
        // 808880 first tests 878c60/878d10. Either actual entity flag returns
        // true immediately; pickup's existing owner has already projected the
        // hide to the render sources. 753c10 still publishes the entity frame
        // before this predicate and skips only additional render pose work.
        // With both flags clear, this admitted null binding returns false.
        Ok(self.entity(entity)?.entity_flags & 3 != 0)
    }
    fn render_entity(&self, entity: usize) -> Result<Option<usize>, String> {
        self.entity(entity)?;
        Ok(self.placements.contains_key(&entity).then_some(entity))
    }
    fn render_binding(&self, actors: &PhysicsActorLinks, actor: usize) -> Result<Option<usize>, String> {
        self.actor_entity(actors, actor)?;
        Ok(None) // The admitted live binding is actually null.
    }
    fn refresh_render_position(&mut self, _: &mut PhysicsActorLinks, _: usize) -> Result<(), String> {
        Err("rigid render binding position owner is not admitted".into())
    }
    fn refresh_render_basis(&mut self, _: &mut PhysicsActorLinks, _: usize) -> Result<(), String> {
        Err("rigid render binding basis owner is not admitted".into())
    }
    fn set_render_pose(&mut self, render: usize, pose: ReferenceFrame) -> Result<(), String> {
        let frame = self.entity(render)?.frame;
        if frame.pose.position != pose.position || frame.pose.basis != pose.basis {
            return Err("rigid render pose differs from its canonical entity publication".into());
        }
        // The immediate render update is committed by notify_render_pose through
        // the existing colour/shadow/query publication owner below.
        Ok(())
    }
    fn set_render_scale(&mut self, render: usize, scale: [f32; 3]) -> Result<(), String> {
        self.entity(render)?;
        // 18df90 initializes entity scale to one. Authored RenderModel scale
        // remains separate in the placement and is preserved by rigid deltas.
        if scale != [1.; 3] { return Err("rigid entity scale change requires the actual render scale owner".into()); }
        Ok(())
    }
    fn notify_render_pose(&mut self, _: &mut PhysicsActorLinks, render: usize) -> Result<(), String> {
        let current = self.entity(render)?.frame.pose;
        let initial = *self.initial.get(&render).ok_or("rigid initial body frame is absent")?;
        // Stored basis axes are columns. This is the existing ModelPose world
        // delta, so authored mesh scale/shear and blueprint offsets survive.
        let delta = render_delta(initial, current)?;
        self.poses.0.try_reserve(1).map_err(|error| format!("rigid render publication: {error}"))?;
        self.poses.0.push(ModelPose { placement: *self.placements.get(&render).ok_or("rigid render placement is absent")?, delta });
        Ok(())
    }
    fn defer_render_pose(&mut self, _: &mut PhysicsActorLinks, _: usize) -> Result<(), String> {
        Err("rigid deferred render owner is not admitted".into())
    }
    fn post_joint_frame(&mut self, _: &mut PhysicsActorLinks, _: usize, _: i16) -> Result<ReferenceFrame, String> {
        Err("rigid joint binding owner is not admitted".into())
    }
    fn enqueue_post(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<(), String> {
        self.actor_entity(actors, actor)?;
        let generation = actors.actors[actor].owner.generation;
        self.post.updates.enqueue(ObjectReference::capture(Some((generation, actor))))?;
        Ok(())
    }
}

impl PhysicsScene {
    pub(super) fn request_post_physics_actor(&mut self, actor: usize, generation: u32) -> Result<bool, String> {
        let value = self.actors.actors.get_at(actor).ok_or("queued rigid actor is absent")?;
        if value.owner.generation != generation { return Err("queued rigid actor generation is stale".into()); }
        let task = &self.post_updates;
        self.actors.request_phase_update(actor, self.post_frame, ActorUpdatePhase::PostPhysics, |actors, actor| {
            let generation = actors.actors[actor].owner.generation;
            task.updates.enqueue(ObjectReference::capture(Some((generation, actor))))?;
            Ok(())
        })
    }

    pub(super) fn advance_rigid_actors(&mut self, clock: &GameClock, live: &LiveEntities,
        poses: &mut ModelPoseUpdates) -> Result<(), String> {
        // 8af6a0: one actual host worker, snapshot enqueue frame then advance
        // by two before it can enqueue further post requests during traversal.
        self.post_updates.frame = self.post_frame;
        self.post_updates.stamp = self.post_frame.wrapping_add(1);
        self.post_frame = self.post_frame.wrapping_add(2);
        // Host: an empty single-worker batch has no work or sentinel to drain.
        // Preserve its phase stamps without taking the physics-world lock.
        if self.post_updates.queue_is_empty() { return Ok(()); }
        self.post_updates.publish_sentinels(1);
        let world = self.world.read().map_err(|_| "rigid canonical world owner is poisoned")?;
        let mut host = RigidPostHost { entities: &mut self.entities, live, world: &world.storage,
            rigid: &self.rigid_bodies, rigid_class: self.classes[8], placements: &self.rigid_placements,
            initial: &self.initial_rigid_frames, poses, clock, post: &self.post_updates, post_frame: self.post_frame };
        self.actors.consume_post_updates_reuse(&self.post_updates, &mut host, &mut self.rigid_post_path)
    }
}

/// Runs before model_motion::publish_model_poses; all GPU/query publication
/// remains in that existing owner, with no new meshes or per-draw allocations.
pub(in crate::app) fn update_rigid_actors(
    scene: Option<bevy::prelude::ResMut<PhysicsScene>>,
    clock: Option<bevy::prelude::Res<crate::gameplay::game_clock_runtime::ClockRuntime>>,
    live: Option<bevy::prelude::Res<LiveEntities>>,
    poses: Option<bevy::prelude::ResMut<ModelPoseUpdates>>,
) {
    let Some(mut scene) = scene else { return; };
    if scene.retirement.is_some() { return; }
    let result = (|| -> Result<(), String> {
        let clock = clock.as_ref().ok_or("rigid actor game clock owner is absent")?;
        if !clock.advanced() { return Ok(()); }
        let clock = clock.clock().map_err(str::to_owned)?;
        let live = live.as_ref().ok_or("rigid actor entity owner is absent")?;
        let mut poses = poses.ok_or("rigid actor render publication owner is absent")?;
        scene.advance_rigid_actors(clock, live, &mut poses)
    })();
    if let Err(error) = result { bevy::log::error!("Rigid actor publication failed: {error}"); }
}
