//! Per-body water membership and density updates (651d70/651a40/650bf0).
use super::component_state::entity::ComponentHandle;
use crate::float::{multiply, subtract};

#[derive(Clone, Copy)]
pub(crate) struct WaterContactState {
    pub flags: u32,
    pub plane: Option<[f32; 4]>,
}
impl From<&super::water_component::WaterComponent> for WaterContactState {
    fn from(water: &super::water_component::WaterComponent) -> Self {
        Self {
            flags: water.owner.flags,
            plane: water.plane(),
        }
    }
}

pub(crate) struct WaterContacts {
    /// The source may retain prior density when no shape property is available.
    /// Unpublished density remains explicit until its owner supplies a value.
    pub density: Option<[f32; 4]>,
    pub components: Vec<ComponentHandle>,
}
pub(crate) trait WaterDensityWorld {
    /// The actual collision-user-data virtual18 callback with argument true.
    fn prepare_body(&mut self, body_index: u32) -> Result<(), String>;
    /// Component declaration+70, read after prepare_body's callbacks.
    fn density_override(&self, body_index: u32) -> Result<f32, String>;
    /// Canonical body mass sidecar; an absent entry returns its source -1 value.
    fn body_mass(&self, body_index: u32) -> Result<f32, String>;
    /// arkPhysicsShapeProperty key1's signed volume vector at+40. This is not
    /// the compressed mass/inertia property at keyf100.
    fn shape_volume(&self, body_index: u32) -> Result<Option<[f32; 4]>, String>;
    fn body_label(&self, body_index: u32) -> Result<String, String>;
}
///651820/651f30 report a change in whether the first contact is started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum ContactChange {
    Activate = 0,
    Unchanged = 1,
    Deactivate = 2,
}
impl ContactChange {
    fn between(before: bool, after: bool) -> Self {
        match (before, after) {
            (false, true) => Self::Activate,
            (true, false) => Self::Deactivate,
            _ => Self::Unchanged,
        }
    }
}

impl WaterContacts {
    fn head_started(
        &self,
        state: &mut impl FnMut(ComponentHandle) -> Result<WaterContactState, String>,
    ) -> Result<bool, String> {
        self.components
            .first()
            .map(|&h| state(h).map(|s| s.flags & 0xc >= 8))
            .transpose()
            .map(|v| v.unwrap_or(false))
    }

    ///651820 inserts without deduplicating. Unstarted components go first;
    /// started components follow higher started surfaces, skipping unstarted
    /// entries. COMISS/JNC deliberately does not stop for unordered heights.
    pub fn enter(
        &mut self,
        component: ComponentHandle,
        mut state: impl FnMut(ComponentHandle) -> Result<WaterContactState, String>,
    ) -> Result<ContactChange, String> {
        let mut at = 0;
        let changed = state(component)?;
        if changed.flags & 0xc >= 8 {
            for &existing in &self.components {
                let other = state(existing)?;
                if other.flags & 0xc >= 8 {
                    let height =
                        subtract(0., changed.plane.ok_or("entered water has no plane")?[3]);
                    let other_height =
                        subtract(0., other.plane.ok_or("started water has no plane")?[3]);
                    if height >= other_height {
                        break;
                    }
                }
                at += 1;
            }
        }
        let before = self.head_started(&mut state)?;
        self.components.insert(at, component);
        Ok(ContactChange::between(
            before,
            self.head_started(&mut state)?,
        ))
    }

    ///651f30 removes only the first occurrence, retaining contact order.
    pub fn leave(
        &mut self,
        component: ComponentHandle,
        mut state: impl FnMut(ComponentHandle) -> Result<WaterContactState, String>,
    ) -> Result<ContactChange, String> {
        let Some(at) = self.components.iter().position(|&h| h == component) else {
            return Ok(ContactChange::Unchanged);
        };
        let before = self.head_started(&mut state)?;
        self.components.remove(at);
        Ok(ContactChange::between(
            before,
            self.head_started(&mut state)?,
        ))
    }

    /// Reorder the first matching component toward the list's end, preserving
    /// all other entries and duplicates. Native COMISS/JBE stops on unordered
    /// heights too; do not replace this with a whole-list sort.
    pub fn component_changed(
        &mut self,
        changed: ComponentHandle,
        mut state: impl FnMut(ComponentHandle) -> Result<WaterContactState, String>,
    ) -> Result<bool, String> {
        let Some(mut at) = self.components.iter().position(|&handle| handle == changed) else {
            return Ok(false);
        };
        while at + 1 < self.components.len() {
            let next = self.components[at + 1];
            let next_state = state(next)?;
            if next_state.flags & 0xc >= 8 {
                let next_height =
                    subtract(0., next_state.plane.ok_or("started water has no plane")?[3]);
                let height = subtract(
                    0.,
                    state(changed)?.plane.ok_or("changed water has no plane")?[3],
                );
                if !(next_height > height) {
                    break;
                }
            }
            self.components[at] = next;
            at += 1;
        }
        self.components[at] = changed;
        Ok(state(self.components[0])?.flags & 0xc >= 8)
    }

    ///650ad0 moves the first stopped component before the immediately
    /// preceding started run; membership and density remain retained.
    pub fn component_stopped(
        &mut self,
        stopped: ComponentHandle,
        mut state: impl FnMut(ComponentHandle) -> Result<WaterContactState, String>,
    ) -> Result<bool, String> {
        let Some(mut at) = self.components.iter().position(|&h| h == stopped) else {
            return Ok(false);
        };
        let before = self.head_started(&mut state)?;
        while at > 0 {
            let previous = self.components[at - 1];
            if state(previous)?.flags & 0xc < 8 {
                break;
            }
            self.components[at] = previous;
            at -= 1;
        }
        self.components[at] = stopped;
        Ok(before)
    }

    pub fn update_density(
        &mut self,
        body_index: u32,
        world: &mut impl WaterDensityWorld,
    ) -> Result<(), String> {
        world.prepare_body(body_index)?;
        let override_density = world.density_override(body_index)?;
        if override_density > 0. {
            self.density = Some([override_density; 4]);
        } else {
            let mass = world.body_mass(body_index)?;
            if let Some(volume) = world.shape_volume(body_index)? {
                self.density =
                    Some(volume.map(|v| multiply(super::constraint_solver::reciprocal(v), mass)));
            }
        }
        let density = self
            .density
            .ok_or("body water density has not been initialized")?;
        //651b0d uses UCOMISS/JNC: unordered NaNs take the diagnostic path.
        const MINIMUM_DENSITY: f32 = f32::from_bits(0x437a_0000);
        if !(density[0] >= MINIMUM_DENSITY) {
            return Err(format!(
                "water density below {MINIMUM_DENSITY} on '{}'",
                world.body_label(body_index)?
            ));
        }
        Ok(())
    }
}

/// Required collision-user-data callbacks remain on the canonical body owner.
pub(crate) trait WaterContactWorld: WaterDensityWorld {
    fn water_state(&self, component: ComponentHandle) -> Result<WaterContactState, String>;
    fn deactivate_body(&mut self, body_index: u32) -> Result<(), String>;
}

struct ContactBody {
    id: u32,
    contacts: WaterContacts,
}

/// Source650bf0/651b80/651e70/652080: hash buckets in ascending order,
/// each with a chain ordered by the body's low24 index. The caller supplies
/// the table size from its owner; this does not invent a constructor default.
pub(crate) struct WaterContactManager {
    buckets: Vec<Vec<ContactBody>>,
    mask: u32,
    count: usize,
}
impl WaterContactManager {
    ///8a3720 initializes256 empty buckets with mask255. The four deferred
    /// simulation-query queues constructed alongside these are a separate owner.
    pub fn new() -> Result<Self, String> {
        Self::with_bucket_count(256)
    }

    pub fn with_bucket_count(bucket_count: u32) -> Result<Self, String> {
        if !bucket_count.is_power_of_two() || bucket_count > i32::MAX as u32 {
            return Err("water contact bucket count must be a positive power of two".into());
        }
        let mut buckets = Vec::new();
        buckets
            .try_reserve_exact(bucket_count as usize)
            .map_err(|e| e.to_string())?;
        buckets.resize_with(bucket_count as usize, Vec::new);
        Ok(Self {
            buckets,
            mask: bucket_count - 1,
            count: 0,
        })
    }

    ///651b80 is called only after4d69d0's rigid-component RTTI gate.
    pub fn contact(
        &mut self,
        body: u32,
        water: ComponentHandle,
        entered: bool,
        world: &mut impl WaterContactWorld,
    ) -> Result<(), String> {
        let index = body & 0xffffff;
        let bucket = &mut self.buckets[(index & self.mask) as usize];
        let at = match bucket.binary_search_by_key(&index, |b| b.id & 0xffffff) {
            Ok(at) => at,
            Err(at) => {
                //651b80 initializes the density from1423ccc20: four zero words.
                // Other force/integration fields belong to the simulation owner.
                bucket.insert(
                    at,
                    ContactBody {
                        id: body,
                        contacts: WaterContacts {
                            density: Some([0.; 4]),
                            components: Vec::new(),
                        },
                    },
                );
                self.count += 1;
                at
            }
        };
        let contacts = &mut bucket[at].contacts;
        let change = if entered {
            contacts.enter(water, |h| world.water_state(h))?
        } else {
            contacts.leave(water, |h| world.water_state(h))?
        };
        match change {
            ContactChange::Activate => contacts.update_density(index, world)?,
            ContactChange::Deactivate => world.deactivate_body(index)?,
            ContactChange::Unchanged => (),
        }
        if contacts.components.is_empty() {
            self.remove_body(body);
        }
        Ok(())
    }

    ///651e70 compares the complete ID; a stale removal cannot erase a
    /// replacement generation that occupies the same low24 body slot.
    fn remove_body(&mut self, id: u32) -> bool {
        let bucket = &mut self.buckets[(id & 0xffffff & self.mask) as usize];
        if let Some(at) = bucket.iter().position(|body| body.id == id) {
            bucket.remove(at);
            self.count -= 1;
            true
        } else {
            false
        }
    }

    ///651960, subscribed by651800 to world+868's removed-body signal.
    /// No density/deactivation callbacks occur during destruction.
    pub fn remove_bodies(&mut self, bodies: &[u32]) {
        for &body in bodies {
            self.remove_body(body);
        }
    }

    ///651e30 ->650ad0, before the inherited component stop callback.
    pub fn stop_component(
        &mut self,
        water: ComponentHandle,
        world: &mut impl WaterContactWorld,
    ) -> Result<(), String> {
        for bucket in &mut self.buckets {
            for body in bucket {
                if body
                    .contacts
                    .component_stopped(water, |h| world.water_state(h))?
                {
                    world.deactivate_body(body.id & 0xffffff)?;
                }
            }
        }
        Ok(())
    }

    pub fn notify_component(
        &mut self,
        water: ComponentHandle,
        world: &mut impl WaterContactWorld,
    ) -> Result<(), String> {
        // Split immutable state lookup and mutable density callbacks by body;
        // the source rereads the world between these operations.
        for bucket in &mut self.buckets {
            for body in bucket {
                if body
                    .contacts
                    .component_changed(water, |h| world.water_state(h))?
                {
                    body.contacts.update_density(body.id & 0xffffff, world)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
