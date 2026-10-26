//! Body-buffer construction and ID reservation, cadd00/cae7f0.
//! Growth callbacks, release and property-key registration belong to the world;
//! reserve receives the registered bitsets whose bits cb8ba0 clears.
use super::collision_body::ShapeSlots;
use super::{broadphase::PackedBodyBounds, collision_body::CollisionBody};
use crate::assets::physworld::{
    body::{BodyInfo, MassDistribution},
    shape::ShapeHeader,
};
use std::collections::HashMap;
#[path = "body_storage/changes.rs"]
mod changes;
pub use changes::{BodyActivation, BodyChange};
#[path = "body_storage/activation.rs"]
mod activation;
#[path = "body_storage/transform_distance.rs"]
mod transform_distance;
#[cfg(test)]
pub(crate) use activation::tests::{initialize as activation_test_state, snapshot as activation_snapshot};

/// Resolved asset references used by caf730. Handles in BodyInfo are resource
/// indices, never process pointers; their owner resolves names/distributions.
pub struct BodyCreationAssets<'a> {
    pub shape: ShapeHeader,
    pub wrapped_shapes: Option<ShapeSlots>,
    pub name: Option<&'a [u8]>,
    pub mass_distribution: Option<MassDistribution>,
}

/// Optional creation-time overrides, shared by caedc0 and caeed0.
#[derive(Clone, Copy, Debug)]
pub struct BodyMassProperties {
    pub mass: f32,
    pub motion_properties_id: u16,
}
impl Default for BodyMassProperties {
    fn default() -> Self {
        Self {
            mass: -1.,
            motion_properties_id: u16::MAX,
        }
    }
}

pub struct BodyPropertyBits {
    bit_count: usize,
    words: Vec<u32>,
}
impl BodyPropertyBits {
    pub fn from_words(bit_count: usize, words: Vec<u32>) -> Result<Self, String> {
        if words.len() < bit_count.div_ceil(32) {
            return Err("body property bit count exceeds its buffer".into());
        }
        Ok(Self { bit_count, words })
    }
    pub fn words(&self) -> &[u32] {
        &self.words
    }
    fn clear(&mut self, index: usize) {
        if index < self.bit_count {
            self.words[index >> 5] &= !(1u32 << (index & 31));
        }
    }
}

pub struct BodyStorage {
    bodies: Vec<CollisionBody>,
    next_free: Vec<Option<usize>>,
    free_head: Option<usize>,
    active_count: u32,
    maximum_index: u32,
    allocation_marks: Vec<u8>,
    active_body_ids: Vec<u32>,
    previous_bounds: Vec<PackedBodyBounds>,
    change_indices: Vec<u32>,
    changes: Vec<BodyChange>,
    queued_active: Vec<u32>,
    queued_deactivated: Vec<u32>,
    names: Vec<Option<Box<[u8]>>>,
    mass_properties: Vec<BodyMassProperties>,
    mass_distributions: HashMap<usize, MassDistribution>,
}

impl BodyStorage {
    /// Slot zero is the world's permanent static anchor; real reservations
    /// start at slot one. The capacity is supplied by the world configuration.
    /// An anchor-only buffer has no free slots (zero acoustic volumes).
    pub fn new(capacity: usize) -> Result<Self, String> {
        if !(1..=0x00ff_ffff).contains(&capacity) {
            return Err(
                "body capacity must include the anchor and fit non-null 24-bit indices".into(),
            );
        }
        let mut bodies = vec![CollisionBody::default(); capacity];
        for body in &mut bodies {
            body.id = 0x00ff_ffff;
            body.broadphase_id = u32::MAX;
        }
        bodies[0] = CollisionBody::default();
        bodies[0].flags = 1;
        for i in 0..3 {
            bodies[0].transform[i][i] = 1.;
        }
        bodies[0].motion_to_body_orientation = [0., 0., 0., 1.];
        let next_free = (0..capacity)
            .map(|i| {
                if i > 0 && i + 1 < capacity {
                    Some(i + 1)
                } else {
                    None
                }
            })
            .collect();
        Ok(Self {
            bodies,
            next_free,
            free_head: (capacity > 1).then_some(1),
            active_count: 1,
            maximum_index: 0,
            allocation_marks: vec![u8::MAX; capacity],
            active_body_ids: Vec::with_capacity(capacity),
            previous_bounds: vec![
                PackedBodyBounds {
                    words: [0xff7f_ffff, 0x7fff_ff7f, 0, 0]
                };
                capacity
            ],
            change_indices: vec![u32::MAX; capacity],
            changes: Vec::new(),
            queued_active: Vec::new(),
            queued_deactivated: Vec::new(),
            names: Vec::new(),
            mass_properties: Vec::new(),
            mass_distributions: HashMap::new(),
        })
    }

    pub fn capacity(&self) -> usize {
        self.bodies.len()
    }
    pub fn active_count(&self) -> u32 {
        self.active_count
    }
    pub fn maximum_index(&self) -> u32 {
        self.maximum_index
    }
    pub fn has_free_slot(&self) -> bool {
        self.free_head.is_some()
    }

    /// Return an explicit error when full; the original world requests growth
    /// before this call. Reserving clears the body but retains auxiliary arrays.
    pub fn reserve(&mut self, properties: &mut [BodyPropertyBits]) -> Result<u32, String> {
        let index = self.free_head.ok_or("body buffer is full")?;
        self.free_head = self.next_free[index].take();
        self.active_count = self.active_count.wrapping_add(1);
        self.maximum_index = self.maximum_index.max(index as u32);
        let generation = ((self.bodies[index].id >> 24) as u8).wrapping_add(1);
        let id = (u32::from(generation) << 24) | index as u32;
        self.bodies[index] = CollisionBody::default();
        self.bodies[index].id = id;
        self.allocation_marks[index] = u8::MAX;
        for property in properties {
            property.clear(index);
        }
        Ok(id)
    }

    pub fn get(&self, id: u32) -> Option<&CollisionBody> {
        self.bodies
            .get((id & 0xff_ffff) as usize)
            .filter(|body| body.id == id)
    }
    /// Source body-array reads use the ID's low24 slot; hits publish the
    /// generation-tagged ID from the current body record (1288af0).
    pub fn get_by_index(&self, index: u32) -> Option<&CollisionBody> {
        self.bodies.get(index as usize).filter(|body| {
            body.id != 0x00ff_ffff && body.id & 0x00ff_ffff == index
        })
    }
    pub fn get_by_index_mut(&mut self, index: u32) -> Option<&mut CollisionBody> {
        self.bodies.get_mut(index as usize)
            .filter(|body| body.id != 0x00ff_ffff && body.id & 0x00ff_ffff == index)
    }
    pub fn get_mut(&mut self, id: u32) -> Option<&mut CollisionBody> {
        self.bodies
            .get_mut((id & 0xff_ffff) as usize)
            .filter(|body| body.id == id)
    }

    /// c92500's circular attachment list and motion-kind flags. Frame, limit,
    /// bounds and quality updates follow separately in the world activation path.
    pub fn link_motion(
        &mut self,
        id: u32,
        motion_id: u32,
        motions: &mut super::motion_storage::MotionStorage,
    ) -> Result<(), String> {
        self.get(id)
            .ok_or("motion attachment body ID is stale or missing")?;
        let motion = motions
            .get_mut(motion_id)
            .ok_or("motion attachment requires an initialized motion")?;
        let first = motion.first_body_id;
        let next = if first & 0x00ff_ffff == 0x00ff_ffff {
            motion.first_body_id = id;
            id
        } else {
            let first = self
                .get_mut(first)
                .ok_or("motion attachment ring has a stale first body")?;
            let next = first.next_attached_body_id;
            first.next_attached_body_id = id;
            next
        };
        let body = self.get_mut(id).unwrap();
        body.next_attached_body_id = next;
        body.motion_id = motion_id;
        body.flags = (body.flags & !1) | 2;
        if motion.solver_id != 0 && motion.inverse_mass == [0; 4] {
            body.flags |= 4;
        } else {
            body.flags &= !4;
        }
        Ok(())
    }

    /// caf730 initializes an already reserved body, then its optional sidecars.
    /// The caller supplies the actual motion kind, not inferred Cinfo flags.
    pub fn initialize(
        &mut self,
        id: u32,
        info: &BodyInfo,
        dynamic: bool,
        assets: BodyCreationAssets<'_>,
    ) -> Result<(), String> {
        if (info.name != 0) != assets.name.is_some()
            || (info.mass_distribution != 0) != assets.mass_distribution.is_some()
        {
            return Err("body creation references have not been resolved consistently".into());
        }
        let body = self
            .get_mut(id)
            .ok_or("body creation ID was not reserved")?;
        body.initialize(id, info, assets.shape, assets.wrapped_shapes)?;
        body.assign_motion_kind(dynamic);
        self.set_name(id, assets.name)?;
        self.set_mass(id, info.mass)?;
        self.set_motion_properties(id, info.motion_properties_id)?;
        if let Some(distribution) = assets.mass_distribution {
            self.mass_distributions
                .insert((id & 0xff_ffff) as usize, distribution);
        }
        // Null distribution deliberately preserves any existing map entry.
        Ok(())
    }
    pub fn previous_bounds(&self) -> &[PackedBodyBounds] {
        &self.previous_bounds
    }
    pub fn allocation_marks(&self) -> &[u8] {
        &self.allocation_marks
    }
    pub fn change_indices(&self) -> &[u32] {
        &self.change_indices
    }

    /// caf0a0 owns a copy of the C string. Resource-name references must be
    /// resolved before calling; their low bit is not a pointer ownership tag.
    /// A null name clears an existing entry without allocating an absent array.
    pub fn set_name(&mut self, id: u32, name: Option<&[u8]>) -> Result<(), String> {
        let index = self.sidecar_index(id)?;
        if name.is_some() && index >= self.names.len() {
            self.names.resize_with(self.capacity(), || None);
        }
        if let Some(entry) = self.names.get_mut(index) {
            *entry = name.map(|bytes| {
                let end = bytes
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(bytes.len());
                bytes[..end].into()
            });
        }
        Ok(())
    }

    /// caedc0. COMISS/JNC skips negative *and unordered* inputs only when the
    /// override array has not yet reached this index. Existing entries accept
    /// either value unchanged; signed zero is nonnegative.
    pub fn set_mass(&mut self, id: u32, mass: f32) -> Result<f32, String> {
        let index = self.sidecar_index(id)?;
        if index >= self.mass_properties.len() {
            if mass.is_nan() || mass < 0. {
                return Ok(-1.);
            }
            self.mass_properties
                .resize(self.capacity(), BodyMassProperties::default());
        }
        self.mass_properties[index].mass = mass;
        Ok(mass)
    }

    /// caeed0 shares the mass array, preserving any existing mass override.
    pub fn set_motion_properties(&mut self, id: u32, properties: u16) -> Result<(), String> {
        let index = self.sidecar_index(id)?;
        if index >= self.mass_properties.len() {
            if properties == u16::MAX {
                return Ok(());
            }
            self.mass_properties
                .resize(self.capacity(), BodyMassProperties::default());
        }
        self.mass_properties[index].motion_properties_id = properties;
        Ok(())
    }

    pub fn name(&self, id: u32) -> Option<&[u8]> {
        self.names.get((id & 0xff_ffff) as usize)?.as_deref()
    }
    pub fn mass_properties(&self, id: u32) -> Option<&BodyMassProperties> {
        self.mass_properties.get((id & 0xff_ffff) as usize)
    }
    pub fn mass_distribution(&self, id: u32) -> Option<&MassDistribution> {
        self.mass_distributions.get(&((id & 0xff_ffff) as usize))
    }
    /// caefc0 resolves the body's channel-one shape, retained mass override and
    /// optional distribution before caf5c0. Like the source manager's sidecar
    /// operations it masks the generation byte; runtime callers own ID validity.
    pub fn mass_frame(
        &self,
        id: u32,
        cached: impl FnOnce(
            usize,
        ) -> Result<
            Option<crate::assets::physworld::mass::PrincipalMassProperties>,
            String,
        >,
        calculate: impl FnOnce(
            usize,
            [u32; 3],
        ) -> Result<
            crate::assets::physworld::mass::PrincipalMassProperties,
            String,
        >,
    ) -> Result<crate::assets::physworld::mass::PrincipalMassProperties, String> {
        let index = self.sidecar_index(id)?;
        let shape = self.bodies[index].selected_shape(1)?;
        let mass = self
            .mass_properties(id)
            .map_or(-1., |properties| properties.mass);
        crate::assets::physworld::mass::creation_mass_properties(
            cached(shape)?,
            self.mass_distribution(id).copied(),
            mass,
            |config| calculate(shape, config),
        )
    }
    fn sidecar_index(&self, id: u32) -> Result<usize, String> {
        let index = (id & 0xff_ffff) as usize;
        if index >= self.capacity() {
            return Err("body sidecar index exceeds world capacity".into());
        }
        // These internal manager operations deliberately ignore generation,
        // unlike the public body lookup. caf730 also uses them while creating.
        Ok(index)
    }
}

#[cfg(test)]
#[path = "body_storage/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "body_storage/properties_tests.rs"]
mod properties_tests;

#[cfg(test)]
#[path = "body_storage/motion_link_tests.rs"]
mod motion_link_tests;

#[cfg(test)]
#[path = "body_storage/mass_frame_tests.rs"]
mod mass_frame_tests;
