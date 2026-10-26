//! Level-owned physics actor/body links and transform scheduling decisions.
//! Native pointers become stable indices; body poses stay in BodyStorage.
use super::component_state::{ComponentInstance, ComponentOwner};
pub mod basic_body;
pub mod post_update;
pub mod pre_update;
use super::motion_requests::MotionRequestList;

#[derive(Clone, Copy, Debug, Default)]
pub struct ResolvedMotion {
    pub velocities: [[f32; 3]; 2],
    pub valid: [bool; 2],
    pub dirty: [bool; 2],
}

pub struct PhysicsActor {
    pub owner: ComponentOwner,
    pub frame: super::reference_frame::ReferenceFrameCache,
    pub default_body: usize,
    pub current_body: usize,
    pub parent: Option<usize>,
    pub binding: ActorBinding,
    pub children: Vec<usize>,
    pub flags: u8,
    pub last_queued_frame: i32,
    pub last_post_frame: i32,
    pub requests: [MotionRequestList; 2],
    pub motion: ResolvedMotion,
}

/// The actor retains the bound entity even when that entity has no cached
/// physics actor. `PhysicsActor::parent` is the resolved actor cache used by
/// scheduler ancestor tests, not a replacement for this binding.
pub struct ActorBinding {
    pub entity: Option<usize>,
    pub joint: i16,
    pub offset: [f32; 3],
    pub orientation: [f32; 4],
    pub phase: u8,
}
impl Default for ActorBinding {
    /// 74f5e0 fields +3c8..3ec and +40a.
    fn default() -> Self {
        Self {
            entity: None,
            joint: -1,
            offset: [0.; 3],
            orientation: [0., 0., 0., 1.],
            phase: 0,
        }
    }
}

impl ComponentInstance for PhysicsActor {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActorUpdatePhase {
    PrePhysics,
    PostPhysics,
}
impl PhysicsActor {
    /// 751d00's entity-transform notification. This replaces the current pose
    /// without publishing a frame marker or changing scale/prediction caches.
    pub fn set_transform(
        &mut self,
        pose: super::reference_frame::ReferenceFrame,
        clear_motion: bool,
    ) {
        self.frame.current = pose;
        self.motion.dirty = [true; 2];
        if clear_motion {
            self.frame.published_velocities = [[0.; 3]; 2];
            self.motion.valid = [false; 2];
        }
    }

    /// 74f5e0's actor defaults. The enclosing arena supplies the embedded
    /// body's stable handle; owner admission and generation assignment happen
    /// separately through the common component manager, as in the game.
    pub fn new(default_body: usize) -> Self {
        Self {
            owner: ComponentOwner::default(),
            frame: Default::default(),
            default_body,
            current_body: default_body,
            parent: None,
            binding: ActorBinding::default(),
            children: Vec::new(),
            flags: 2,
            last_queued_frame: -1,
            last_post_frame: -1,
            requests: std::array::from_fn(|_| MotionRequestList::default()),
            motion: ResolvedMotion {
                velocities: [[0.; 3]; 2],
                valid: [false; 2],
                dirty: [true; 2],
            },
        }
    }
    /// Complete751140's boolean result; unused upper return bits are omitted.
    pub fn has_enabled_motion(&self) -> bool {
        self.requests
            .iter()
            .any(MotionRequestList::has_enabled_motion)
    }
}

pub struct ActorBody {
    pub actor: Option<usize>,
    pub body_id: u32,
}

pub struct PhysicsActorLinks {
    /// The same typed pool used by declaration construction and owner linking.
    pub actors: super::component_state::pool::ComponentPool<PhysicsActor>,
    pub bodies: Vec<ActorBody>,
}

#[cfg(test)]
pub(crate) fn test_actor_pool(
    values: Vec<PhysicsActor>,
) -> super::component_state::pool::ComponentPool<PhysicsActor> {
    let mut pool = super::component_state::pool::ComponentPool::new(0, 0x420).unwrap();
    for value in values {
        pool.allocate(|_| Ok(value)).unwrap();
    }
    pool
}

pub trait ActorMotionBody {
    fn accepts(&mut self, actors: &mut PhysicsActorLinks, actor: usize) -> Result<bool, String>;
    fn apply(
        &mut self,
        actors: &mut PhysicsActorLinks,
        actor: usize,
        motion: ResolvedMotion,
    ) -> Result<u8, String>;
}

impl PhysicsActorLinks {
    /// Complete753a90. A body accepting velocity requests is a required virtual
    /// boundary. A rejecting body skips application but retains resolved requests
    /// and completes caching, as the machine-code warning branch does.
    pub fn resolve_motion(
        &mut self,
        actor: usize,
        delta: f32,
        body: &mut impl ActorMotionBody,
    ) -> Result<bool, String> {
        let record = self
            .actors
            .get_at_mut(actor)
            .ok_or("motion actor is missing")?;
        if record.flags & 4 == 0 {
            for axis in 0..2 {
                record.requests[axis].advance(delta);
                if let Some(request) = record.requests[axis].selected() {
                    record.motion.velocities[axis] = request.input.velocity;
                    record.motion.valid[axis] = true;
                } else {
                    record.motion.valid[axis] = false;
                }
                record.motion.dirty[axis] = true;
            }
            if record.motion.valid.iter().any(|&valid| valid) {
                if body.accepts(self, actor)? {
                    let motion = self.actors[actor].motion;
                    let changed = body.apply(self, actor, motion)?;
                    self.actors[actor].flags = (self.actors[actor].flags & !1) | (changed & 1);
                }
            }
            self.actors[actor].flags = (self.actors[actor].flags & !8) | 4;
        }
        Ok(self.actors[actor].motion.valid.iter().any(|&valid| valid))
    }
    /// 751980 checks ancestors, excluding the supplied actor itself.
    pub fn ancestor_has_update(&self, actor: usize, frame: i32) -> Result<bool, String> {
        self.ancestor_has_phase_update(actor, frame, ActorUpdatePhase::PrePhysics)
    }
    /// 751980/751930 use the same parent walk with independent phase stamps.
    pub fn ancestor_has_phase_update(
        &self,
        actor: usize,
        frame: i32,
        phase: ActorUpdatePhase,
    ) -> Result<bool, String> {
        let mut parent = self
            .actors
            .get_at(actor)
            .ok_or("physics actor is missing")?
            .parent;
        for _ in 0..self.actors.allocated_count() {
            let Some(index) = parent else {
                return Ok(false);
            };
            let actor = self
                .actors
                .get_at(index)
                .ok_or("physics actor parent is missing")?;
            let stamp = match phase {
                ActorUpdatePhase::PrePhysics => actor.last_queued_frame,
                ActorUpdatePhase::PostPhysics => actor.last_post_frame,
            };
            if frame <= stamp {
                return Ok(true);
            }
            parent = actor.parent;
        }
        Err("cyclic physics actor parents".into())
    }

    /// Complete755030/7551c0 task admission, using the real bounded queue and
    /// retained reference generations. The phase's transform/body producer is
    /// required; dispatch may mutate hierarchy before the next queued entry.
    pub fn consume_updates(
        &mut self,
        task: &super::physics_world::update_queue::ObjectUpdateTask<usize>,
        phase: ActorUpdatePhase,
        mut update: impl FnMut(&mut Self, usize, ActorUpdatePhase) -> Result<(), String>,
    ) -> Result<(), String> {
        task.drain(|reference, frame, stamp| {
            let Some(index) =
                reference.resolve(|i| self.actors.get_at(i).map(|a| a.owner.generation))
            else {
                return Ok(());
            };
            if self.ancestor_has_phase_update(index, frame, phase)? {
                return Ok(());
            }
            let actor = &mut self.actors[index];
            let marker = match phase {
                ActorUpdatePhase::PrePhysics => &mut actor.last_queued_frame,
                ActorUpdatePhase::PostPhysics => &mut actor.last_post_frame,
            };
            // Mutable owner access supplies exclusivity for source1581e0.
            if std::mem::replace(marker, stamp) != stamp {
                update(self, index, phase)?;
            }
            Ok(())
        })
    }

    /// 752260's frame/ancestor gate. The required callback is the actual world
    /// update queue (8a60e0), whose allocation and processing remain separate.
    pub fn request_update(
        &mut self,
        actor: usize,
        frame: i32,
        enqueue: impl FnMut(&mut Self, usize) -> Result<(), String>,
    ) -> Result<bool, String> {
        self.request_phase_update(actor, frame, ActorUpdatePhase::PrePhysics, enqueue)
    }

    /// 752260/7521f0 differ only in their phase's frame and world queue.
    pub fn request_phase_update(
        &mut self,
        actor: usize,
        frame: i32,
        phase: ActorUpdatePhase,
        mut enqueue: impl FnMut(&mut Self, usize) -> Result<(), String>,
    ) -> Result<bool, String> {
        let record = self
            .actors
            .get_at(actor)
            .ok_or("physics actor is missing")?;
        let previous = match phase {
            ActorUpdatePhase::PrePhysics => record.last_queued_frame,
            ActorUpdatePhase::PostPhysics => record.last_post_frame,
        };
        if frame <= previous || self.ancestor_has_phase_update(actor, frame, phase)? {
            return Ok(false);
        }
        match phase {
            ActorUpdatePhase::PrePhysics => self.actors[actor].last_queued_frame = frame,
            ActorUpdatePhase::PostPhysics => self.actors[actor].last_post_frame = frame,
        }
        enqueue(self, actor)?;
        Ok(true)
    }

    fn alternate_body_in_children(&self, actor: usize) -> Result<bool, String> {
        for &child in &self.actors[actor].children {
            if self
                .actors
                .get_at(child)
                .ok_or("physics actor child is missing")?
                .flags
                & 0x20
                != 0
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// 751e80 propagates the alternate-body hierarchy bit until it is unchanged.
    pub fn propagate_body_flag(&mut self, first: usize, mut value: bool) -> Result<(), String> {
        let mut current = Some(first);
        for _ in 0..self.actors.allocated_count() {
            let Some(index) = current else {
                return Ok(());
            };
            let actor = self
                .actors
                .get_at(index)
                .ok_or("physics actor is missing")?;
            value = value
                || actor.current_body != actor.default_body
                || self.alternate_body_in_children(index)?;
            let actor = &mut self.actors[index];
            if (actor.flags & 0x20 != 0) == value {
                return Ok(());
            }
            actor.flags = (actor.flags & !0x20) | (u8::from(value) << 5);
            current = actor.parent;
            if current.is_none() {
                return Ok(());
            }
        }
        Err("cyclic physics actor flag propagation".into())
    }

    /// 751050 changes the current body and its reciprocal owner. It does not
    /// clear the old body's owner; the source retains that link as well.
    pub fn attach_body(
        &mut self,
        actor: usize,
        body: usize,
        frame: i32,
        enqueue: impl FnMut(&mut Self, usize) -> Result<(), String>,
    ) -> Result<(), String> {
        self.actors
            .get_at(actor)
            .ok_or("physics actor is missing")?;
        self.bodies
            .get(body)
            .ok_or("physics actor body is missing")?;
        self.actors[actor].current_body = body;
        self.bodies[body].actor = Some(actor);
        if self.actors[actor].has_enabled_motion() {
            self.request_update(actor, frame, enqueue)?;
        }
        let value = self.actors[actor].current_body != self.actors[actor].default_body
            || self.alternate_body_in_children(actor)?;
        let record = &mut self.actors[actor];
        if (record.flags & 0x20 != 0) != value {
            record.flags = (record.flags & !0x20) | (u8::from(value) << 5);
            if let Some(parent) = record.parent {
                self.propagate_body_flag(parent, value)?;
            }
        }
        Ok(())
    }

    /// 751f60 clears the supplied body's reciprocal link and restores the
    /// actor's embedded body, even when the supplied body wasn't current.
    /// Unlike attachment, this does not request a transform update.
    pub fn detach_body(&mut self, actor: usize, body: usize) -> Result<(), String> {
        self.actors
            .get_at(actor)
            .ok_or("physics actor is missing")?;
        self.bodies
            .get_mut(body)
            .ok_or("physics actor body is missing")?
            .actor = None;
        self.actors[actor].current_body = self.actors[actor].default_body;
        let value = self.alternate_body_in_children(actor)?;
        let record = &mut self.actors[actor];
        if (record.flags & 0x20 != 0) != value {
            record.flags = (record.flags & !0x20) | (u8::from(value) << 5);
            if let Some(parent) = record.parent {
                self.propagate_body_flag(parent, value)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "physics_actor/construction_tests.rs"]
mod construction_tests;
#[cfg(test)]
#[path = "physics_actor/factory_tests.rs"]
mod factory_tests;
#[cfg(test)]
#[path = "physics_actor/motion_tests.rs"]
mod motion_tests;
#[cfg(test)]
#[path = "physics_actor/task_tests.rs"]
mod task_tests;
#[cfg(test)]
#[path = "physics_actor/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "physics_actor/transform_tests.rs"]
mod transform_tests;
