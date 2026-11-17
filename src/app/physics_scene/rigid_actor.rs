//! Null-parent rigid movement interfaces and 753c10 publication on live owners.
use super::*;
use crate::app::model_motion::{ModelPose, ModelPoseUpdates};
use crate::gameplay::{
    collision_user_data::CollisionUserData,
    entity_lifetime::LiveEntities,
    game_clock::GameClock,
    physics_actor::{
        ActorMotionBody, ActorUpdatePhase, ResolvedMotion,
        basic_body::BasicBody,
        body_frame::body_origin_velocities,
        post_update::{BoundFrame, PostPhysicsWorld},
        pre_update::PrePhysicsWorld,
    },
    physics_world::update_queue::{ObjectReference, ObjectUpdateTask},
    reference_frame::{PublishedFrame, ReferenceFrame},
};

/// The shared render publisher accepts a rigid world delta from the actual
/// source frame. Both unbound body publication and808880 attachments use it.
pub(super) fn render_delta(
    initial: ReferenceFrame,
    current: ReferenceFrame,
) -> Result<[[f32; 4]; 3], String> {
    let mut inverse = initial.basis;
    if !crate::gameplay::reference_frame::invert_basis(&mut inverse) {
        return Err("rigid initial render basis is singular".into());
    }
    let basis = crate::gameplay::reference_frame::basis_product(inverse, current.basis);
    Ok(std::array::from_fn(|row| {
        let moved_origin = (0..3)
            .map(|axis| basis[axis][row] * initial.position[axis])
            .sum::<f32>();
        [
            basis[0][row],
            basis[1][row],
            basis[2][row],
            current.position[row] - moved_origin,
        ]
    }))
}

struct RigidPostHost<'a> {
    entities: &'a mut [PhysicsEntity],
    live: &'a LiveEntities,
    world: &'a WorldStorage,
    rigid: [(
        i32,
        &'a mut ComponentPool<crate::app::rigid_body::RigidBodyComponent>,
    ); 2],
    projectiles: (i32, &'a ComponentPool<crate::app::projectiles::Component>),
    triggers: (i32, &'a ComponentPool<crate::gameplay::trigger_body::TriggerComponent>),
    placements: &'a HashMap<usize, usize>,
    initial: &'a HashMap<usize, ReferenceFrame>,
    poses: Option<&'a mut ModelPoseUpdates>,
    clock: &'a GameClock,
    post: &'a ObjectUpdateTask<usize>,
    post_frame: i32,
    indices: &'a HashMap<crate::gameplay::entity_lifetime::EntityId, usize>,
    hierarchy: &'a super::document_hierarchy::DocumentHierarchy,
    joint_frames: Option<&'a super::super::joint_frames::JointFrames>,
    pre: &'a ObjectUpdateTask<usize>,
    pre_frame: i32,
    commands: &'a crate::gameplay::physics_world::commands::PhysicsCommands,
}
impl RigidPostHost<'_> {
    fn basic_body(
        &self,
        actors: &PhysicsActorLinks,
        actor: usize,
    ) -> Result<Option<BasicBody>, String> {
        let entity = self.actor_entity(actors, actor)?;
        if actors.actors[actor].current_body == actors.actors[actor].default_body {
            return Ok(Some(BasicBody::Embedded));
        }
        for &packed in self.entities[entity].components.entries.values() {
            let handle = ComponentHandle::from_packed(packed)?;
            // Trigger+20 and Projectile+20 have the same six movement
            // callbacks as the embedded actor (7792c0/627110). Select the
            // actual admitted interface, independently of a collision body.
            if handle.class_id == self.triggers.0 {
                let component = self.triggers.1.get(handle.slot)
                    .ok_or("trigger movement component absent")?;
                if component.owner.entity != Some(entity) {
                    return Err("trigger movement owner differs".into());
                }
                if component.movement_body == actors.actors[actor].current_body {
                    return Ok(Some(BasicBody::Trigger { body: component.movement_body }));
                }
                continue;
            }
            if handle.class_id != self.projectiles.0 {
                continue;
            }
            let component = self
                .projectiles
                .1
                .get(handle.slot)
                .ok_or("projectile movement component absent")?;
            if component.owner.entity != Some(entity) {
                return Err("projectile movement owner differs".into());
            }
            if component.movement_body == Some(actors.actors[actor].current_body) {
                return Ok(component.movement_body.map(|body| BasicBody::Projectile { body }));
            }
        }
        Ok(None)
    }
    fn entity(&self, index: usize) -> Result<&PhysicsEntity, String> {
        let entity = self
            .entities
            .get(index)
            .ok_or("rigid actor entity is absent")?;
        let (name, class) = self
            .live
            .get(entity.shared)
            .ok_or("rigid actor entity generation is stale")?;
        if name != entity.name || class != entity.class {
            return Err("rigid actor managed identity differs".into());
        }
        Ok(entity)
    }

    fn actor_entity(&self, actors: &PhysicsActorLinks, actor: usize) -> Result<usize, String> {
        let value = actors
            .actors
            .get_at(actor)
            .ok_or("rigid reference actor is absent")?;
        let entity = value
            .owner
            .entity
            .ok_or("rigid reference actor entity is absent")?;
        let record = self.entity(entity)?;
        let handle = record
            .components
            .reference_actor
            .ok_or("rigid reference actor handle is absent")?;
        //8762d0 retains the old joint index after clearing the parent. A
        // dormant index alone is not a live attachment or resource owner.
        if usize::from(handle.slot) != actor
            || value
                .binding
                .entity
                .map(|parent| self.entities[parent].shared)
                != self.live.parent(record.shared)?
        {
            return Err("rigid actor and managed hierarchy differ".into());
        }
        Ok(entity)
    }

    #[track_caller]
    fn body<'a>(
        &'a self,
        actors: &PhysicsActorLinks,
        actor: usize,
    ) -> Result<(&'a crate::app::rigid_body::RigidBodyComponent,
                 &'a crate::gameplay::collision_body::CollisionBody), String> {
        let caller = std::panic::Location::caller();
        self.resolve_body(actors, actor).map_err(|error| {
            self.context_error(actors, actor, &format!("movement virtual at {caller}"), error)
        })
    }

    fn context_error(&self, actors: &PhysicsActorLinks, actor: usize, operation: &str, error: String) -> String {
        let record = actors.actors.get_at(actor);
        let entity = record.and_then(|value| value.owner.entity).and_then(|index| self.entities.get(index));
        let body = record.and_then(|value| actors.bodies.get(value.current_body));
        let collision = body.and_then(|value| self.world.bodies.get(value.body_id));
        format!("{operation}: actor={actor}, entity={:?}, current={:?}, default={:?}, interface_body={:?}, collision_owner={:?}: {error}",
            entity.map(|value| (&value.name, value.shared)),
            record.map(|value| value.current_body), record.map(|value| value.default_body),
            body.map(|value| value.body_id), collision.map(|value| CollisionUserData::decode(value.user_data)))
    }

    fn resolve_body<'a>(
        &'a self,
        actors: &PhysicsActorLinks,
        actor: usize,
    ) -> Result<
        (
            &'a crate::app::rigid_body::RigidBodyComponent,
            &'a crate::gameplay::collision_body::CollisionBody,
        ),
        String,
    > {
        let entity = self.actor_entity(actors, actor)?;
        let value = actors.actors.get_at(actor).ok_or("rigid actor is absent")?;
        if value.current_body == value.default_body {
            return Err("rigid movement body is detached".into());
        }
        let link = actors
            .bodies
            .get(value.current_body)
            .ok_or("rigid movement interface is absent")?;
        if link.actor != Some(actor) {
            return Err("rigid movement reciprocal owner differs".into());
        }
        let body = self
            .world
            .bodies
            .get(link.body_id)
            .ok_or("rigid movement body generation is stale")?;
        // Movement virtuals receive their containing component, not a
        // collision-userdata pointer. Resolve the actual current interface
        // against admitted component interfaces (80a5f0/809ab0/808e30).
        for &packed in self.entities[entity].components.entries.values() {
            let handle = ComponentHandle::from_packed(packed)?;
            let Some((_, pool)) = self.rigid.iter().find(|(class, _)| *class == handle.class_id) else {
                continue;
            };
            let component = pool.get(handle.slot).ok_or("rigid movement component absent")?;
            if component.movement_body()? != value.current_body {
                continue;
            }
            if component.owner.entity != Some(entity) || component.body() != Some(body.id) {
                return Err("rigid movement component/body identity differs".into());
            }
            return Ok((component, body));
        }
        Err(format!("current movement interface has no admitted rigid owner: actor={actor}, entity={} {:?}, current={}, default={}, link_actor={:?}, body={:#x}, collision_owner={:?}",
            self.entities[entity].name, self.entities[entity].shared,
            value.current_body, value.default_body, link.actor, body.id,
            CollisionUserData::decode(body.user_data)))
    }

    ///80b9e0 publishes actor desired origin velocities through the existing
    ///18d020 queued owner; cached collision velocities remain that body's owner.
    fn publish_motion(&mut self, actors: &PhysicsActorLinks, actor: usize) -> Result<(), String> {
        let (component, body) = self.body(actors, actor)?;
        let body_id = body.id;
        let current = component.velocities;
        self.publish_motion_data(actors, actor, body_id, current)
    }

    fn publish_component_motion(
        &mut self,
        actors: &PhysicsActorLinks,
        actor: usize,
        family: usize,
        slot: u16,
    ) -> Result<(), String> {
        let (body_id, current) = self.component_body_data(actors, actor, family, slot)?;
        self.publish_motion_data(actors, actor, body_id, current)
    }

    fn publish_motion_data(
        &mut self,
        actors: &PhysicsActorLinks,
        actor: usize,
        body_id: u32,
        current: crate::gameplay::collision_user_data::velocity::CollisionVelocities,
    ) -> Result<(), String> {
        let entity = self.actor_entity(actors, actor)?;
        if self.clock.entity_delta(self.entities[entity].timespace)
            .ok_or("rigid timespace absent")? <= 0. {
            return Ok(());
        }
        let body = self.world.bodies.get(body_id).ok_or("rigid motion body generation stale")?;
        if body.flags & 1 != 0 {
            return Ok(());
        }
        let id = body.id;
        let flags = body.flags;
        let motion = actors.actors[actor].motion;
        let mut angular = current.requested_angular;
        let mut linear = current.requested_linear;
        if motion.valid[1] {
            angular[..3].copy_from_slice(&motion.velocities[1]);
        }
        if motion.valid[0] {
            let com = self
                .world
                .motions
                .get(body.motion_id)
                .ok_or("rigid motion absent")?
                .pose
                .center;
            let values = body_origin_velocities(
                com[..3].try_into().unwrap(),
                body.transform[3][..3].try_into().unwrap(),
                motion.velocities[0],
                angular[..3].try_into().unwrap(),
            );
            linear[..3].copy_from_slice(&values[0]);
        }
        if motion.valid[0] || motion.valid[1] || flags & 4 != 0 {
            if !motion.valid[0] && !motion.valid[1] {
                linear = [0.; 4];
                angular = [0.; 4];
            }
            if (linear[0] * linear[0] + linear[1] * linear[1]) + linear[2] * linear[2]
                < f32::from_bits(0x3089_705f)
            {
                linear = [0.; 4];
            }
            crate::gameplay::collision_user_data::velocity::publish_velocities(
                self, id, linear, angular, true,
            )?;
        }
        Ok(())
    }
    fn local_joint(&self, parent: usize, joint: i16) -> Result<ReferenceFrame, String> {
        if let Some(frame) = self.hierarchy.external_joints.get(&(parent, joint)) {
            return Ok(*frame);
        }
        let placement = self
            .hierarchy
            .bindings
            .iter()
            .find(|binding| binding.parent == parent && binding.joint == joint)
            .and_then(|binding| binding.parent_placement)
            .ok_or("bound body has no actual joint producer")?;
        self.joint_frames
            .ok_or("bound joint frame owner absent")?
            .frame(placement, joint, 3)?
            .ok_or_else(|| "bound body's real joint frame absent".into())
    }
    fn resolved_bound(
        &self,
        actors: &PhysicsActorLinks,
        actor: usize,
        delta: f32,
    ) -> Result<BoundFrame, String> {
        let value = actors
            .actors
            .get_at(actor)
            .ok_or("bound body reference actor absent")?;
        let parent = value
            .binding
            .entity
            .ok_or("bound body has no actual parent")?;
        let mut parent_frame = self.entity(parent)?.frame.pose;
        if value.binding.joint >= 0 {
            parent_frame = crate::gameplay::reference_frame::compose(
                parent_frame,
                self.local_joint(parent, value.binding.joint)?,
            );
        }
        let pose = crate::gameplay::reference_frame::bind(
            parent_frame,
            value.binding.offset,
            value.binding.orientation,
        );
        let velocities =
            crate::gameplay::reference_frame::velocities(value.frame.current, pose, delta);
        Ok(BoundFrame { pose, velocities })
    }
    fn publish_body(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<(), String> {
        self.publish_body_frame(actors, actor, false)
    }
}

impl RigidPostHost<'_> {
    fn publish_body_frame(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        notify_entity: bool,
    ) -> Result<(), String> {
        let (component, body) = self.body(actors, actor)?;
        let body_id = body.id;
        let linear = component.velocities.requested_linear[..3].try_into().unwrap();
        let angular = component.velocities.requested_angular[..3].try_into().unwrap();
        self.publish_body_data(actors, actor, body_id, linear, angular, notify_entity)
    }

    // Unlike the movement virtual 80a5f0, scheduled 80b7c0 receives a
    // component directly. Its interface may remain linked to an actor whose
    // current movement interface has since changed (751050 retains that link).
    fn publish_component_body_frame(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        family: usize,
        slot: u16,
    ) -> Result<(), String> {
        let (body_id, current) = self.component_body_data(actors, actor, family, slot)?;
        let linear = current.requested_linear[..3].try_into().unwrap();
        let angular = current.requested_angular[..3].try_into().unwrap();
        self.publish_body_data(actors, actor, body_id, linear, angular, true)
    }

    fn component_body_data(
        &self,
        actors: &PhysicsActorLinks,
        actor: usize,
        family: usize,
        slot: u16,
    ) -> Result<(u32, crate::gameplay::collision_user_data::velocity::CollisionVelocities), String> {
        let entity = self.actor_entity(actors, actor)?;
        let component = self.rigid[family].1.get(slot)
            .ok_or("scheduled rigid publication component absent")?;
        if component.owner.entity != Some(entity) {
            return Err("scheduled rigid publication entity differs".into());
        }
        let body_id = component.body().ok_or("scheduled rigid publication body absent")?;
        let link = actors.bodies.get(component.movement_body()?)
            .ok_or("scheduled rigid publication interface absent")?;
        if link.actor != Some(actor) || link.body_id != body_id {
            return Err("scheduled rigid publication interface identity differs".into());
        }
        Ok((body_id, component.velocities))
    }

    fn publish_body_data(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        body_id: u32,
        linear: [f32; 3],
        angular: [f32; 3],
        notify_entity: bool,
    ) -> Result<(), String> {
        let body = self.world.bodies.get(body_id)
            .ok_or("rigid publication body generation stale")?;
        let pose = ReferenceFrame {
            position: body.transform[3][..3].try_into().unwrap(),
            basis: std::array::from_fn(|axis| body.transform[axis][..3].try_into().unwrap()),
        };
        let motion = self
            .world
            .motions
            .get(body.motion_id)
            .ok_or("rigid publication motion is absent")?;
        // 80b7c0 reads component+140/+150: the requested vectors in its
        // +100 collision-data subobject, not the scaled +120/+130 vectors.
        let velocities = body_origin_velocities(
            pose.position,
            motion.pose.center[..3].try_into().unwrap(),
            linear,
            angular,
        );
        // 80a5f0 supplies false inside an admitted 753c10 task; phase6's
        // 951c80/4c9560 supply true to produce that task when the body moved.
        actors.publish_body_frame(actor, pose, velocities, notify_entity, self)
    }
}

impl crate::gameplay::collision_user_data::velocity::BodyVelocityWorld for RigidPostHost<'_> {
    type Reference = u32;
    fn body_id(&self, id: u32) -> Result<u32, String> {
        self.world
            .bodies
            .get(id)
            .map(|b| b.id)
            .ok_or_else(|| "velocity body generation stale".into())
    }
    fn velocity_scale(&self, id: u32) -> Result<f32, String> {
        self.world.motion_properties.body_velocity_scale(
            &self.world.bodies,
            &self.world.motions,
            id,
        )
    }
    fn set_velocity(&mut self, _: u32, _: [f32; 4], _: [f32; 4]) -> Result<(), String> {
        Err("pre-physics rigid velocity uses actual deferred command owner".into())
    }
    fn queue_velocity(
        &mut self,
        id: u32,
        linear: [f32; 4],
        angular: [f32; 4],
    ) -> Result<(), String> {
        self.commands.enqueue_velocity(id, linear, angular, false)
    }
    fn collision_velocities(
        &mut self,
        id: u32,
    ) -> Result<&mut crate::gameplay::collision_user_data::velocity::CollisionVelocities, String>
    {
        let body = self.world.bodies.get(id).ok_or("velocity body retired")?;
        let handle = CollisionUserData::decode(body.user_data)?.component;
        let pool = self
            .rigid
            .iter_mut()
            .find_map(|(class, pool)| (*class == handle.class_id).then_some(&mut **pool))
            .ok_or("velocity body class differs")?;
        let component = pool
            .get_mut(handle.slot)
            .ok_or("velocity body component absent")?;
        if component.body() != Some(id) {
            return Err("velocity body component identity differs".into());
        }
        Ok(&mut component.velocities)
    }
    fn next_attached_body(&self, id: u32) -> Result<u32, String> {
        Ok(self
            .world
            .bodies
            .get(id)
            .ok_or("velocity attached body retired")?
            .next_attached_body_id)
    }
    fn body_capacity(&self) -> usize {
        self.world.bodies.capacity()
    }
}
impl ActorMotionBody for RigidPostHost<'_> {
    fn accepts(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<bool, String> {
        self.actor_entity(actors, actor)?;
        if let Some(mut body) = self.basic_body(actors, actor)? {
            return body.accepts(actors, actor);
        }
        //8079d0 is the +8 motion predicate. +16 is the component's
        //motion type; -20 is its bound parent physics interface, not Wind.
        let body=self.body(actors,actor)?.1;
        if body.flags&1!=0{return Ok(false);}
        Ok(body.flags&4!=0||actors.actors[actor].binding.entity.is_none()
            ||body.next_attached_body_id==body.id)
    }
    fn apply(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        motion: ResolvedMotion,
    ) -> Result<u8, String> {
        self.actor_entity(actors, actor)?;
        if let Some(mut body) = self.basic_body(actors, actor)? {
            return body.apply(actors, actor, motion);
        }
        let (component, body) = self.body(actors, actor)?;
        //809ab0's normal-update branch needs no independent wake packet.
        if component.owner.flags & 0x30 < 0x20 {
            return Ok(1);
        }
        let linear = motion.velocities[0];
        let angular = motion.velocities[1];
        let moving = motion.valid[0]
            && ((linear[0] * linear[0] + linear[1] * linear[1]) + linear[2] * linear[2])
                >= f32::from_bits(0x3089_705f);
        let rotating = motion.valid[1]
            && angular
                .into_iter()
                .any(|value| value.abs() > f32::from_bits(0x3586_37bd));
        if !moving && !rotating {
            return Ok(0);
        }
        self.commands.enqueue_activation(body.id, false, false)?;
        self.publish_motion(actors, actor)?;
        Ok(1)
    }
}
impl PrePhysicsWorld for RigidPostHost<'_> {
    fn entity_delta(&mut self, _: &mut PhysicsActorLinks, entity: usize) -> Result<f32, String> {
        self.clock
            .entity_delta(self.entity(entity)?.timespace)
            .ok_or_else(|| "rigid entity timespace is absent".into())
    }
    fn entity_flags(&self, entity: usize) -> Result<u8, String> {
        Ok(self.entity(entity)?.entity_flags as u8)
    }
    fn bound_velocities(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        delta: f32,
    ) -> Result<[[f32; 3]; 2], String> {
        Ok(self.resolved_bound(actors, actor, delta)?.velocities)
    }
    fn pre_queue_frame(&self) -> i32 {
        self.pre_frame
    }
    fn enqueue_pre(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<(), String> {
        self.actor_entity(actors, actor)?;
        let generation = actors.actors[actor].owner.generation;
        self.pre
            .updates
            .enqueue(ObjectReference::capture(Some((generation, actor))))?;
        Ok(())
    }
    fn child_count(&self, entity: usize) -> Result<usize, String> {
        Ok(self.live.children(self.entity(entity)?.shared)?.len())
    }
    fn child_entity(&self, entity: usize, index: usize) -> Result<usize, String> {
        let parent = self.entity(entity)?;
        let id = *self
            .live
            .children(parent.shared)?
            .get(index)
            .ok_or("bound descendant index is absent")?;
        self.indices
            .get(&id)
            .copied()
            .ok_or_else(|| format!("bound descendant has no actual scene owner: parent={} {:?}, child={} {id:?}, child_index={index}",
                parent.name, parent.shared, self.live.get(id).map_or("<stale generation>", |(name, _)|name)))
    }
    fn actor_for_entity(
        &mut self,
        actors: &mut PhysicsActorLinks,
        entity: usize,
    ) -> Result<usize, String> {
        let handle = self
            .entity(entity)?
            .components
            .reference_actor
            .ok_or("bound descendant reference actor not admitted")?;
        let actor = actors
            .actors
            .get(handle.slot)
            .ok_or("bound descendant actor retired")?;
        if actor.owner.entity != Some(entity) {
            return Err("bound descendant actor owner differs".into());
        }
        Ok(handle.slot as usize)
    }
}
impl PostPhysicsWorld for RigidPostHost<'_> {
    fn publication_frame(&self) -> i32 {
        self.clock.frame_marker()
    }
    fn post_queue_frame(&self) -> i32 {
        self.post_frame
    }
    fn entity_frame(&self, entity: usize) -> Result<PublishedFrame, String> {
        Ok(self.entity(entity)?.frame)
    }
    fn publish_entity_frame(&mut self, entity: usize, frame: PublishedFrame) -> Result<(), String> {
        self.entity(entity)?;
        self.entities[entity].frame = frame;
        Ok(())
    }
    fn bound_frame(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        delta: f32,
    ) -> Result<BoundFrame, String> {
        self.resolved_bound(actors, actor, delta)
    }
    fn accepts_pose(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
    ) -> Result<bool, String> {
        if let Some(body) = self.basic_body(actors, actor)? {
            return body.accepts_pose(actors, actor);
        }
        //808e30 is the distinct +10 pose predicate, using body deactivation.
        Ok(self.body(actors,actor)?.1.flags&8!=0)
    }
    fn update_body_pose(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
    ) -> Result<(), String> {
        if let Some(body) = self.basic_body(actors, actor)? {
            body.update_pose(actors, actor)?;
            return Ok(());
        }
        self.publish_body(actors, actor)
    }
    fn body_handles_render_entity(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
    ) -> Result<bool, String> {
        let entity = self.actor_entity(actors, actor)?;
        if let Some(body) = self.basic_body(actors, actor)? {
            return body.handles_render_entity(actors, actor);
        }
        self.body(actors, actor)?;
        // 808880 first tests 878c60/878d10. Either actual entity flag returns
        // true immediately; pickup's existing owner has already projected the
        // hide to the render sources. 753c10 still publishes the entity frame
        // before this predicate and skips only additional render pose work.
        if self.entity(entity)?.entity_flags & 3 != 0 {
            return Ok(true);
        }
        if actors.actors[actor].binding.entity.is_none() {
            return Ok(false);
        }
        //808880 owns the bound render pose itself. Returning true without
        //publishing this frame would leave a carried object's mesh behind.
        let delta = self
            .clock
            .entity_delta(self.entity(entity)?.timespace)
            .ok_or("bound render timespace absent")?;
        let pose = self
            .resolved_bound(actors, actor, delta.max(f32::MIN_POSITIVE))?
            .pose;
        let Some(&placement) = self.placements.get(&entity) else {
            return Ok(true);
        };
        let initial = *self
            .initial
            .get(&entity)
            .ok_or("bound render initial frame absent")?;
        let delta = render_delta(initial, pose)?;
        let poses = self
            .poses
            .as_deref_mut()
            .ok_or("bound render publication during pre-physics")?;
        poses
            .0
            .try_reserve(1)
            .map_err(|error| format!("bound render publication: {error}"))?;
        poses.0.push(ModelPose { placement, delta });
        Ok(true)
    }
    fn render_entity(&self, entity: usize) -> Result<Option<usize>, String> {
        self.entity(entity)?;
        Ok(self.placements.contains_key(&entity).then_some(entity))
    }
    fn render_binding(
        &self,
        actors: &PhysicsActorLinks,
        actor: usize,
    ) -> Result<Option<usize>, String> {
        self.actor_entity(actors, actor)?;
        Ok(None) // The admitted live binding is actually null.
    }
    fn refresh_render_position(
        &mut self,
        _: &mut PhysicsActorLinks,
        _: usize,
    ) -> Result<(), String> {
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
        if scale != [1.; 3] {
            return Err("rigid entity scale change requires the actual render scale owner".into());
        }
        Ok(())
    }
    fn notify_render_pose(
        &mut self,
        _: &mut PhysicsActorLinks,
        render: usize,
    ) -> Result<(), String> {
        let current = self.entity(render)?.frame.pose;
        let initial = *self
            .initial
            .get(&render)
            .ok_or("rigid initial body frame is absent")?;
        // Stored basis axes are columns. This is the existing ModelPose world
        // delta, so authored mesh scale/shear and blueprint offsets survive.
        let delta = render_delta(initial, current)?;
        let poses = self
            .poses
            .as_deref_mut()
            .ok_or("render callback during actual pre-physics traversal")?;
        poses
            .0
            .try_reserve(1)
            .map_err(|error| format!("rigid render publication: {error}"))?;
        poses.0.push(ModelPose {
            placement: *self
                .placements
                .get(&render)
                .ok_or("rigid render placement is absent")?,
            delta,
        });
        Ok(())
    }
    fn defer_render_pose(&mut self, _: &mut PhysicsActorLinks, _: usize) -> Result<(), String> {
        Err("rigid deferred render owner is not admitted".into())
    }
    fn post_joint_frame(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        joint: i16,
    ) -> Result<ReferenceFrame, String> {
        let parent = actors
            .actors
            .get_at(actor)
            .and_then(|actor| actor.binding.entity)
            .ok_or("post joint binding parent absent")?;
        self.local_joint(parent, joint)
    }
    fn enqueue_post(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<(), String> {
        self.actor_entity(actors, actor)?;
        let generation = actors.actors[actor].owner.generation;
        self.post
            .updates
            .enqueue(ObjectReference::capture(Some((generation, actor))))?;
        Ok(())
    }
}

impl PhysicsScene {
    /// Deferred 18d020 adapter for gameplay producers outside an actor task.
    pub(in crate::app) fn queue_rigid_velocity(
        &mut self,
        body: u32,
        linear: [f32; 4],
        angular: [f32; 4],
    ) -> Result<(), String> {
        use crate::gameplay::collision_user_data::velocity::{
            BodyVelocityWorld, CollisionVelocities, publish_velocities,
        };
        struct Host<'a> {
            world: &'a crate::gameplay::physics_world::storage::WorldStorage,
            pools: [(
                i32,
                &'a mut ComponentPool<super::super::rigid_body::RigidBodyComponent>,
            ); 2],
            commands: &'a crate::gameplay::physics_world::commands::PhysicsCommands,
        }
        impl BodyVelocityWorld for Host<'_> {
            type Reference = u32;
            fn body_id(&self, id: u32) -> Result<u32, String> {
                self.world
                    .bodies
                    .get(id)
                    .map(|b| b.id)
                    .ok_or("queued rigid body stale".into())
            }
            fn velocity_scale(&self, id: u32) -> Result<f32, String> {
                self.world.motion_properties.body_velocity_scale(
                    &self.world.bodies,
                    &self.world.motions,
                    id,
                )
            }
            fn set_velocity(&mut self, _: u32, _: [f32; 4], _: [f32; 4]) -> Result<(), String> {
                Err("gameplay rigid velocity requires deferred publication".into())
            }
            fn queue_velocity(
                &mut self,
                id: u32,
                linear: [f32; 4],
                angular: [f32; 4],
            ) -> Result<(), String> {
                self.commands.enqueue_velocity(id, linear, angular, false)
            }
            fn collision_velocities(
                &mut self,
                id: u32,
            ) -> Result<&mut CollisionVelocities, String> {
                let body = self
                    .world
                    .bodies
                    .get(id)
                    .ok_or("queued rigid payload body stale")?;
                let handle = CollisionUserData::decode(body.user_data)?.component;
                let (_, pool) = self
                    .pools
                    .iter_mut()
                    .find(|(class, _)| *class == handle.class_id)
                    .ok_or("queued rigid payload class unsupported")?;
                let component = pool
                    .get_mut(handle.slot)
                    .ok_or("queued rigid component absent")?;
                if component.body() != Some(id) {
                    return Err("queued rigid body owner differs".into());
                }
                Ok(&mut component.velocities)
            }
            fn next_attached_body(&self, id: u32) -> Result<u32, String> {
                Ok(self
                    .world
                    .bodies
                    .get(id)
                    .ok_or("queued rigid ring body stale")?
                    .next_attached_body_id)
            }
            fn body_capacity(&self) -> usize {
                self.world.bodies.capacity()
            }
        }
        let shared = self.world.clone();
        let guard = shared.read().map_err(|_| "queued rigid world poisoned")?;
        let mut host = Host {
            world: &guard.storage,
            pools: [
                (self.classes[8], &mut self.rigid_bodies),
                (self.classes[12], &mut self.throwables),
            ],
            commands: &self.body_commands,
        };
        publish_velocities(&mut host, body, linear, angular, true)
    }
    ///752870/7527e0: one-evaluation priority-two requests on the real actor.
    pub(in crate::app) fn request_actor_velocity(
        &mut self,
        id: crate::gameplay::entity_lifetime::EntityId,
        linear: [f32; 3],
        angular: [f32; 3],
    ) -> Result<(), String> {
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("carry motion entity has no scene owner")?;
        if self.entities[index].retired {
            return Err("carry motion entity retired".into());
        }
        let handle = self.entities[index]
            .components
            .reference_actor
            .ok_or("carry motion reference actor absent")?;
        let actor = self
            .actors
            .actors
            .get(handle.slot)
            .ok_or("carry motion actor retired")?;
        if actor.owner.entity != Some(index) {
            return Err("carry motion actor owner differs".into());
        }
        if actor.requests.iter().any(|list| list.entries().len() >= 10) {
            return Err("carry motion request list full".into());
        }
        let actor_index = usize::from(handle.slot);
        for (list, velocity) in self.actors.actors[actor_index]
            .requests
            .iter_mut()
            .zip([linear, angular])
        {
            list.push(crate::gameplay::motion_requests::MotionRequestInput {
                velocity,
                priority: 2,
                remaining: 0.,
                lifetime: 2,
                flags: 0,
            });
        }
        let task = &self.pre_updates;
        self.actors.request_phase_update(
            actor_index,
            self.pre_frame,
            ActorUpdatePhase::PrePhysics,
            |actors, actor| {
                task.updates.enqueue(ObjectReference::capture(Some((
                    actors.actors[actor].owner.generation,
                    actor,
                ))))?;
                Ok(())
            },
        )?;
        Ok(())
    }
    ///7551c0 uses the same canonical actor/body hierarchy as753c10.
    pub(in crate::app) fn advance_rigid_pre_physics(
        &mut self,
        clock: &GameClock,
        live: &LiveEntities,
        joint_frames: Option<&super::super::joint_frames::JointFrames>,
    ) -> Result<(), String> {
        if !self.rigid_pre_started {
            self.pre_updates.frame = self.pre_frame;
            self.pre_updates.stamp = self.pre_frame.wrapping_add(1);
            self.pre_frame = self.pre_frame.wrapping_add(2);
            if !self.pre_updates.queue_is_empty() {
                self.pre_updates.publish_sentinels(1);
            }
            self.rigid_pre_started = true;
        }
        let queued = !self.pre_updates.queue_is_empty();
        let world = self
            .world
            .read()
            .map_err(|_| "rigid canonical world owner is poisoned")?;
        let mut host = RigidPostHost {
            entities: &mut self.entities,
            live,
            world: &world.storage,
            rigid: [
                (self.classes[8], &mut self.rigid_bodies),
                (self.classes[12], &mut self.throwables),
            ],
            projectiles: (self.classes[13], &self.projectiles),
            triggers: (self.classes[0], &self.triggers),
            placements: &self.rigid_placements,
            initial: &self.initial_rigid_frames,
            poses: None,
            clock,
            post: &self.post_updates,
            post_frame: self.post_frame,
            indices: &self.shared_indices,
            hierarchy: &self.document_hierarchy,
            joint_frames,
            pre: &self.pre_updates,
            pre_frame: self.pre_frame,
            commands: &self.body_commands,
        };
        if queued {
            self.actors.consume_pre_updates_reuse(
                &self.pre_updates,
                &mut host,
                &mut self.pre_path,
            )?;
        }
        //80b9e0 is phase5 work of the scheduled prefix, not an all-entity scan.
        for family in 0..host.rigid.len() {
            let count = usize::try_from(host.rigid[family].1.roster.sorted_start)
                .map_err(|_| "negative rigid update prefix")?;
            for at in 0..count {
                let slot = host.rigid[family].1.roster.indices[at];
                let component = host.rigid[family]
                    .1
                    .get(slot)
                    .ok_or("scheduled rigid component absent")?;
                if component.owner.flags & 0xf != 0xa {
                    continue;
                }
                let movement = component.movement_body()?;
                let actor = self
                    .actors
                    .bodies
                    .get(movement)
                    .and_then(|body| body.actor)
                    .ok_or("scheduled rigid actor absent")?;
                host.publish_component_motion(&self.actors, actor, family, slot)
                    .map_err(|error| host.context_error(&self.actors, actor,
                        &format!("phase5 scheduled rigid class={} slot={slot}", host.rigid[family].0), error))?;
            }
        }
        self.rigid_pre_started = false;
        Ok(())
    }

    pub(super) fn request_post_physics_actor(
        &mut self,
        actor: usize,
        generation: u32,
    ) -> Result<bool, String> {
        let value = self
            .actors
            .actors
            .get_at(actor)
            .ok_or("queued rigid actor is absent")?;
        if value.owner.generation != generation {
            return Err("queued rigid actor generation is stale".into());
        }
        let task = &self.post_updates;
        self.actors.request_phase_update(
            actor,
            self.post_frame,
            ActorUpdatePhase::PostPhysics,
            |actors, actor| {
                let generation = actors.actors[actor].owner.generation;
                task.updates
                    .enqueue(ObjectReference::capture(Some((generation, actor))))?;
                Ok(())
            },
        )
    }

    /// 951c80 and 4c9560 publish every scheduled rigid/Throwable body after
    /// physics, including Throwables with no pending contacts. 754ea0 then
    /// requests the ordinary actor traversal only when its entity differs.
    fn publish_scheduled_rigid_frames(
        &mut self,
        clock: &GameClock,
        live: &LiveEntities,
        joint_frames: Option<&super::super::joint_frames::JointFrames>,
    ) -> Result<(), String> {
        let world = self
            .world
            .read()
            .map_err(|_| "rigid post publication world owner is poisoned")?;
        let mut host = RigidPostHost {
            entities: &mut self.entities,
            live,
            world: &world.storage,
            rigid: [
                (self.classes[8], &mut self.rigid_bodies),
                (self.classes[12], &mut self.throwables),
            ],
            projectiles: (self.classes[13], &self.projectiles),
            triggers: (self.classes[0], &self.triggers),
            placements: &self.rigid_placements,
            initial: &self.initial_rigid_frames,
            poses: None,
            clock,
            post: &self.post_updates,
            post_frame: self.post_frame,
            indices: &self.shared_indices,
            hierarchy: &self.document_hierarchy,
            joint_frames,
            pre: &self.pre_updates,
            pre_frame: self.pre_frame,
            commands: &self.body_commands,
        };
        // The accepted frame owns this cursor: a fallible queue admission
        // cannot replay acknowledged components or lose the failed component.
        while self.rigid_post_publication_cursor[0] < host.rigid.len() {
            let family = self.rigid_post_publication_cursor[0];
            let count = usize::try_from(host.rigid[family].1.roster.sorted_start)
                .map_err(|_| "negative rigid post update prefix")?;
            while self.rigid_post_publication_cursor[1] < count {
                let at = self.rigid_post_publication_cursor[1];
                let slot = host.rigid[family].1.roster.indices[at];
                let component = host.rigid[family]
                    .1
                    .get(slot)
                    .ok_or("scheduled rigid post component absent")?;
                if component.owner.flags & 0xf == 0xa {
                    let movement = component.movement_body()?;
                    let actor = self
                        .actors
                        .bodies
                        .get(movement)
                        .and_then(|body| body.actor)
                        .ok_or("scheduled rigid post actor absent")?;
                    host.publish_component_body_frame(&mut self.actors, actor, family, slot)
                        .map_err(|error| host.context_error(&self.actors, actor,
                            &format!("phase6 scheduled rigid class={} slot={slot}", host.rigid[family].0), error))?;
                }
                self.rigid_post_publication_cursor[1] += 1;
            }
            self.rigid_post_publication_cursor[0] += 1;
            self.rigid_post_publication_cursor[1] = 0;
        }
        self.rigid_post_publication_cursor = [0; 2];
        Ok(())
    }

    pub(super) fn advance_rigid_actors(
        &mut self,
        clock: &GameClock,
        live: &LiveEntities,
        poses: &mut ModelPoseUpdates,
        joint_frames: Option<&super::super::joint_frames::JointFrames>,
    ) -> Result<(), String> {
        // 8af6a0: one actual host worker, snapshot enqueue frame then advance
        // by two before it can enqueue further post requests during traversal.
        if !self.rigid_post_started {
            self.publish_scheduled_rigid_frames(clock, live, joint_frames)?;
            self.post_updates.frame = self.post_frame;
            self.post_updates.stamp = self.post_frame.wrapping_add(1);
            self.post_frame = self.post_frame.wrapping_add(2);
            if !self.post_updates.queue_is_empty() {
                self.post_updates.publish_sentinels(1);
            }
            self.rigid_post_started = true;
        }
        // Host: an empty single-worker batch has no work or sentinel to drain.
        // Preserve its phase stamps without taking another physics-world lock.
        if self.post_updates.queue_is_empty() {
            self.rigid_post_started = false;
            return Ok(());
        }
        let world = self
            .world
            .read()
            .map_err(|_| "rigid canonical world owner is poisoned")?;
        let mut host = RigidPostHost {
            entities: &mut self.entities,
            live,
            world: &world.storage,
            rigid: [
                (self.classes[8], &mut self.rigid_bodies),
                (self.classes[12], &mut self.throwables),
            ],
            projectiles: (self.classes[13], &self.projectiles),
            triggers: (self.classes[0], &self.triggers),
            placements: &self.rigid_placements,
            initial: &self.initial_rigid_frames,
            poses: Some(poses),
            clock,
            post: &self.post_updates,
            post_frame: self.post_frame,
            indices: &self.shared_indices,
            hierarchy: &self.document_hierarchy,
            joint_frames,
            pre: &self.pre_updates,
            pre_frame: self.pre_frame,
            commands: &self.body_commands,
        };
        self.actors.consume_post_updates_reuse(
            &self.post_updates,
            &mut host,
            &mut self.rigid_post_path,
        )?;
        self.rigid_post_started = false;
        Ok(())
    }
}

/// Runs before model_motion::publish_model_poses; all GPU/query publication
/// remains in that existing owner, with no new meshes or per-draw allocations.
pub(in crate::app) fn update_rigid_actors(
    scene: Option<bevy::prelude::ResMut<PhysicsScene>>,
    clock: Option<bevy::prelude::Res<crate::gameplay::game_clock_runtime::ClockRuntime>>,
    live: Option<bevy::prelude::Res<LiveEntities>>,
    poses: Option<bevy::prelude::ResMut<ModelPoseUpdates>>,
    joint_frames: Option<bevy::prelude::Res<super::super::joint_frames::JointFrames>>,
) {
    let Some(mut scene) = scene else {
        return;
    };
    if scene.retirement.is_some() {
        return;
    }
    let result = (|| -> Result<(), String> {
        let clock = clock
            .as_ref()
            .ok_or("rigid actor game clock owner is absent")?;
        if !clock.advanced() {
            return Ok(());
        }
        let clock = clock.clock().map_err(str::to_owned)?;
        let live = live.as_ref().ok_or("rigid actor entity owner is absent")?;
        let mut poses = poses.ok_or("rigid actor render publication owner is absent")?;
        scene.advance_rigid_actors(clock, live, &mut poses, joint_frames.as_deref())
    })();
    if let Err(error) = result {
        bevy::log::error!("Rigid actor publication failed: {error}");
    }
}
