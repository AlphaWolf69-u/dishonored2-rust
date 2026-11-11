//! Havok FX (hkfx, the 2016_1_0 drop linked into the game) as
//! arkHavokParticleStageSimulator uses it: an hkfxPipeline (constructor
//! 140ff1be0) of bodies and effects stepped by the physics world.
//!
//! The pipeline's task graph and CPU backend (140ff4380) run effects in a
//! fixed order; this port runs the same order serially. Its effects are the
//! ones the particle simulator adds (1401dfc20); the rest of hkfx (Goo,
//! Smoke, vorticity, tracers) is not linked into any installed content.
#[path = "havok_fx/allocator.rs"]
pub mod allocator;
#[path = "havok_fx/integrate.rs"]
pub mod integrate;
#[path = "havok_fx/packed.rs"]
pub mod packed;
#[path = "havok_fx/physics_collide.rs"]
pub mod physics_collide;
#[path = "havok_fx/spatial_partition.rs"]
pub mod spatial_partition;
#[path = "havok_fx/solve_contacts.rs"]
pub mod solve_contacts;
#[path = "havok_fx/world_collide.rs"]
pub mod world_collide;

use allocator::BodyAllocator;

/// 0x20 bytes per body: LinearMotion's "PositionAndAge" and
/// "VelocityAndScale" channels, or AngularMotion's "Orientation" and
/// "Velocity".
pub type BodyChannels = [[f32; 4]; 2];

pub struct Pipeline {
    pub allocator: BodyAllocator,
    /// hkfxEffects::LinearMotion (140ff8620).
    pub linear: Vec<BodyChannels>,
    /// hkfxEffects::AngularMotion (140ff8840), when the simulator adds it.
    pub angular: Option<Vec<BodyChannels>>,
    /// +0x70/+0x80: 140ff1be0's +-4096 box around the origin.
    pub domain: [[f32; 4]; 2],
    /// +0x90: an optional plane the bodies are locked to (zero: none).
    pub plane: [f32; 4],
    /// +0x30/+0x34: the constructor's body radius.
    pub radius: f32,
    /// +0x58: bodies allocated since the owner last took them.
    pub allocated_since: Vec<i32>,
    /// +0xd8: bodies queued for release at the next step (140ff2550).
    trash: Vec<i32>,
    /// +0x38 step count, +0x3c simulated time.
    pub steps: i32,
    pub time: f32,
}

impl Pipeline {
    /// 140ff1be0 with the allocator 140ff8410 and the motion effects'
    /// slot 6 sizing their channels to the capacity.
    pub fn new(capacity: usize, radius: f32, angular: bool) -> Self {
        let extent = f32::from_bits(0x4580_0000);
        Self {
            allocator: BodyAllocator::new(capacity),
            linear: vec![[[0.; 4]; 2]; capacity],
            angular: angular.then(|| vec![[[0.; 4]; 2]; capacity]),
            domain: [[-extent; 4], [extent; 4]],
            plane: [0.; 4],
            radius,
            allocated_since: Vec::new(),
            trash: Vec::new(),
            steps: 0,
            time: 0.,
        }
    }

    /// LinearMotion slot 8 (140ff8720) and AngularMotion slot 8 (140ff8900).
    fn initialize(&mut self, bodies: &[i32]) {
        for &body in bodies {
            self.linear[body as usize] = [[0.; 4], [0., 0., 0., 1.]];
            if let Some(angular) = &mut self.angular {
                angular[body as usize] = [[0., 0., 0., 1.], [0.; 4]];
            }
        }
    }

    /// LinearMotion slot 9 (140ff8780); AngularMotion has no release hook.
    fn clear(&mut self, bodies: &[i32]) {
        for &body in bodies {
            self.linear[body as usize] = [[0.; 4]; 2];
        }
    }

    /// 140ff2200 after the trash flush (140ff2620): stolen bodies are
    /// released by every effect first (slot 9), then all are initialized
    /// (slot 8). `extra` runs the other effects' slot 8/9 hooks.
    pub fn allocate(
        &mut self,
        requested: usize,
        steal: bool,
        mut extra: impl FnMut(&[i32], bool),
    ) -> Vec<i32> {
        self.flush_trash(&mut extra);
        let (bodies, first_stolen) = self.allocator.allocate(requested, steal);
        if let Some(first) = first_stolen {
            let stolen = bodies[first..].to_vec();
            self.clear(&stolen);
            extra(&stolen, false);
        }
        self.initialize(&bodies);
        extra(&bodies, true);
        self.allocated_since.extend_from_slice(&bodies);
        bodies
    }

    /// 140ff2550.
    pub fn trash(&mut self, bodies: &[i32]) {
        self.trash.extend_from_slice(bodies);
    }

    /// 140ff2620: trashed bodies that are still allocated are released in
    /// batches of 128 (140ff2390: effects' slot 9, removal from +0x58, then
    /// the allocator).
    pub fn flush_trash(&mut self, extra: &mut impl FnMut(&[i32], bool)) {
        if self.trash.is_empty() {
            return;
        }
        let trash = std::mem::take(&mut self.trash);
        let live: Vec<i32> = trash.into_iter().filter(|&body| self.allocator.is_allocated(body)).collect();
        for batch in live.chunks(128) {
            self.clear(batch);
            extra(batch, false);
            for &body in batch {
                if let Some(position) = self.allocated_since.iter().position(|&other| other == body) {
                    self.allocated_since.swap_remove(position);
                }
            }
            self.allocator.release(batch);
        }
    }
}
