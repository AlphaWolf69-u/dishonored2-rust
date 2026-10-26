//! Base component admission flags and scheduler routing (7e7670).
pub mod declaration;
pub mod dependencies;
pub mod entity;
pub mod entity_lookup;
pub mod entity_setup;
pub mod lifecycle;
pub mod order;
pub mod pool;
pub mod roster;
pub mod types;

/// Common component fields initialized by 7e71e0 and linked by 7e7210.
/// Handles replace process pointers; the level owns the referenced values.
pub struct ComponentOwner {
    pub declaration: Option<usize>,
    pub entity: Option<usize>,
    pub generation: u32,
    pub flags: u32,
}

/// Component types retain one common owner record, shared by factories,
/// lifecycle queues and generation-tagged references.
pub trait ComponentInstance {
    fn owner(&self) -> &ComponentOwner;
    fn owner_mut(&mut self) -> &mut ComponentOwner;
}
impl ComponentInstance for ComponentOwner {
    fn owner(&self) -> &ComponentOwner {
        self
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        self
    }
}
impl Default for ComponentOwner {
    fn default() -> Self {
        Self {
            declaration: None,
            entity: None,
            generation: u32::MAX,
            flags: 0,
        }
    }
}

impl ComponentOwner {
    ///7e7400's base initialization transition; retain every other owner field.
    pub fn mark_initialized(&mut self) {
        self.flags = (self.flags & !1) | 2;
    }

    /// 7e7ab0: retain a requested update independently of automatic admission.
    /// Bit0 controls the retained request; the low byte controls the special
    /// bit30 counter path, matching the source's distinct tests.
    pub fn request_update(&mut self, request: u32) {
        self.flags = (self.flags & !(1 << 27)) | ((request & 1) << 27);
        if self.flags & (1 << 30) != 0 {
            if request as u8 != 0 {
                let count = ((self.flags >> 6) & 0x1f_ffff).max(1);
                self.flags = (self.flags & !0x07ff_ffc0) | (count << 6);
            } else {
                self.flags &= 0xf800_003f;
            }
        }
    }
}

/// The manager's ordered registration list and generation allocator. Its
/// enclosing level supplies construction counters until its ctor is verified.
pub struct ComponentRegistry {
    pub next_generation: u32,
    pub maximum_registered: i32,
    pub registered: crate::storage::PointerList,
    pub pending_start: crate::storage::PointerList,
    pub pending_stop: crate::storage::PointerList,
    pub pending_release: crate::storage::PointerList,
    /// Entries below this cursor are already being processed and cannot be
    /// cancelled by 7ec530. It is a signed source field, not an array index.
    pub processed_stop_count: i32,
    pub processed_initialization_count: i32,
    pub processed_start_count: i32,
    pub queue_reset_count: i32,
}
impl ComponentRegistry {
    /// The registration/phase-queue fields initialized by7e7e10. Declaration
    /// configuration and typed factory pools remain with their level owner.
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            next_generation: 0,
            maximum_registered: 0,
            registered: crate::storage::PointerList::owned_with_capacity(64)?,
            pending_start: crate::storage::PointerList::owned_with_capacity(64)?,
            pending_stop: crate::storage::PointerList::owned_with_capacity(64)?,
            pending_release: crate::storage::PointerList::owned_with_capacity(64)?,
            processed_stop_count: 0,
            processed_initialization_count: 0,
            processed_start_count: 0,
            queue_reset_count: 0,
        })
    }

    /// The shared declaration-factory path (978b70): construct in the real
    /// component pool first, then assign its generation through this manager.
    pub fn allocate<T: ComponentInstance>(
        &mut self,
        pool: &mut pool::ComponentPool<T>,
        construct: impl FnOnce(u16, &Self) -> Result<T, String>,
    ) -> Result<u16, String> {
        let slot = pool.allocate(|slot| construct(slot, self))?;
        self.assign_generation(
            pool.get_mut(slot)
                .ok_or("newly allocated component slot is missing")?
                .owner_mut(),
        );
        Ok(slot)
    }

    /// Complete7ec530, including the missing-entry comparison against -1.
    pub fn cancel_pending_stop(&mut self, component: usize) -> Result<bool, String> {
        let index = self
            .pending_stop
            .values()
            .iter()
            .position(|&v| v == component as u64)
            .map_or(-1, |i| i as i32);
        if self.processed_stop_count <= index {
            self.pending_stop.remove_at(index)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Complete7e7b10. The caller supplies the component's real virtual38
    /// queued-start notification; it runs after enqueueing and may mutate owner state.
    pub fn request_start(
        &mut self,
        owner: &mut ComponentOwner,
        component: usize,
        grow_one_at_a_time: bool,
        mut on_start_queued: impl FnMut(&mut Self, &mut ComponentOwner) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.queue_start(owner, component, grow_one_at_a_time)? {
            on_start_queued(self, owner)?;
        }
        Ok(())
    }

    /// Typed virtual dispatch uses the same transition as the common owner
    /// projection. The handler receives the actual component, not copied flags.
    pub fn request_component_start<T: ComponentInstance>(
        &mut self,
        value: &mut T,
        component: usize,
        grow_one_at_a_time: bool,
        mut on_start_queued: impl FnMut(&mut Self, &mut T) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.queue_start(value.owner_mut(), component, grow_one_at_a_time)? {
            on_start_queued(self, value)?;
        }
        Ok(())
    }

    fn queue_start(
        &mut self,
        owner: &mut ComponentOwner,
        component: usize,
        grow_one_at_a_time: bool,
    ) -> Result<bool, String> {
        if owner.flags & 3 == 3 {
            // The diagnostic 166830 returns; 7e7b10 then releases the lock and
            // leaves this detach-pending component unchanged. It is not fatal.
            return Ok(false);
        }
        let phase = (owner.flags >> 2) & 3;
        if phase != 1 {
            if phase == 3 && self.cancel_pending_stop(component)? {
                owner.flags = (owner.flags & !4) | 8;
            }
            if owner.flags & 12 == 0 {
                owner.flags = (owner.flags & !8) | 4;
                self.pending_start
                    .append(component as u64, grow_one_at_a_time)?;
                self.maximum_registered = self
                    .maximum_registered
                    .max(self.pending_start.count() as i32);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Complete 7eb510. Generation wrap, including ffffffff, is preserved.
    pub fn assign_generation(&mut self, owner: &mut ComponentOwner) {
        owner.generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1);
    }

    /// 7e7210/7eb2f0. Exclusive ownership replaces the manager's spinlock;
    /// entries use the shared packed list rather than a second container.
    pub fn register(
        &mut self,
        owner: &mut ComponentOwner,
        component: usize,
        declaration: usize,
        entity: usize,
        grow_one_at_a_time: bool,
    ) -> Result<u32, String> {
        owner.flags = (owner.flags & !2) | 1;
        owner.declaration = Some(declaration);
        owner.entity = Some(entity);
        let index = self
            .registered
            .append(component as u64, grow_one_at_a_time)?;
        self.maximum_registered = self.maximum_registered.max(self.registered.count() as i32);
        Ok(index)
    }
}

/// 7e7390's reference-state mask test. The owner supplies the selected table
/// row as (required, rejected); bits0..5 are reordered before either test.
/// Object-reference resolution uses the verified policy0 row (0,0).
pub fn matches_reference_state(flags: u32, masks: [u32; 2]) -> bool {
    let state = ((flags >> 2 & 3) | (flags & 3) << 2) << 2 | (flags >> 4 & 3);
    state & masks[0] == masks[0] && state & masks[1] == 0
}

/// Inputs come from the component's +60 virtual, declaration priority +64 and
/// owning entity priority +294. Return true for the +88/48fe10 scheduler path.
/// The caller owns that scheduler; this function never substitutes a no-op.
pub fn prepare_admission(
    flags: &mut u32,
    enabled: u8,
    declaration_priority: i32,
    entity_priority: i32,
) -> bool {
    *flags = (*flags & !4) | 8;
    let eligible =
        enabled != 0 && declaration_priority <= 7 && entity_priority >= declaration_priority;
    *flags = (*flags & 0x7fff_ffff) | (u32::from(enabled) << 31);
    if *flags & (1 << 30) == 0 {
        if *flags & (1 << 28) != 0 {
            *flags &= !(1 << 28);
            return false;
        }
        if eligible || *flags & (1 << 27) != 0 {
            let count = ((*flags >> 27) & 1) + u32::from(eligible);
            *flags = (*flags & !0x07ff_ffd0) | (count << 6) | 0x20;
            return false;
        }
    } else if *flags & 0x07ff_ffc0 != 0 {
        *flags = (*flags & !0x10) | 0x20;
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn update_requests_match_executed_counter_and_request_flags() {
        for row in include_str!("../../tests/fixtures/component_update_request.txt").lines() {
            let words: Vec<_> = row
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let mut owner = super::ComponentOwner {
                flags: words[0],
                ..Default::default()
            };
            owner.request_update(words[1]);
            assert_eq!(owner.flags, words[2]);
        }
    }
    #[test]
    fn starts_match_complete_executed_component_lifecycle() {
        let mut cases = 0;
        for line in include_str!("../../tests/fixtures/component_start.txt").lines() {
            let mut words = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap());
            let flags = words.next().unwrap();
            let cursor = words.next().unwrap() as i32;
            let maximum = words.next().unwrap() as i32;
            let mutation = words.next().unwrap();
            let capacity = words.next().unwrap();
            let mut pending_start =
                crate::storage::PointerList::with_capacity(capacity, 0x40).unwrap();
            let count = words.next().unwrap();
            for _ in 0..count {
                pending_start
                    .append(words.next().unwrap().into(), false)
                    .unwrap();
            }
            let mut pending_stop = crate::storage::PointerList::with_capacity(16, 0x40).unwrap();
            let count = words.next().unwrap();
            for _ in 0..count {
                pending_stop
                    .append(words.next().unwrap().into(), false)
                    .unwrap();
            }
            let mut registry = super::ComponentRegistry {
                next_generation: 0,
                maximum_registered: maximum,
                registered: Default::default(),
                pending_start,
                pending_stop,
                processed_stop_count: cursor,
                processed_initialization_count: 0,
                processed_start_count: 0,
                queue_reset_count: 0,
                pending_release: crate::storage::PointerList::owned_with_capacity(64).unwrap(),
            };
            let mut owner = super::ComponentOwner {
                flags,
                ..Default::default()
            };
            let mut events = Vec::new();
            registry
                .request_start(&mut owner, 0, false, |registry, owner| {
                    events.extend([owner.flags, registry.pending_start.count()]);
                    owner.flags ^= mutation;
                    Ok(())
                })
                .unwrap();
            let mut actual = vec![owner.flags, registry.maximum_registered as u32];
            for list in [&registry.pending_start, &registry.pending_stop] {
                actual.push(list.count());
                actual.extend(list.values().iter().map(|&v| v as u32));
            }
            actual.push(events.len() as u32);
            actual.extend(events);
            assert_eq!(actual, words.collect::<Vec<_>>(), "case {cases}");
            cases += 1;
        }
        assert_eq!(cases, 768);
    }

    #[test]
    fn registration_matches_executed_owner_and_manager_routines() {
        for line in include_str!("../../tests/fixtures/component_registration.txt").lines() {
            let words: Vec<_> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let mut registry = super::ComponentRegistry {
                next_generation: words[0],
                maximum_registered: words[1] as i32,
                registered: crate::storage::PointerList::with_capacity(words[2], 0x40).unwrap(),
                pending_start: Default::default(),
                pending_stop: Default::default(),
                processed_stop_count: 0,
                processed_initialization_count: 0,
                processed_start_count: 0,
                queue_reset_count: 0,
                pending_release: crate::storage::PointerList::owned_with_capacity(64).unwrap(),
            };
            let count = words[3] as usize;
            let mut at = 4;
            for index in 0..count {
                let mut owner = super::ComponentOwner {
                    flags: words[at],
                    ..Default::default()
                };
                registry.assign_generation(&mut owner);
                let result = registry
                    .register(&mut owner, index, 0x100 + index, 0x200 + index, false)
                    .unwrap();
                assert_eq!(
                    result,
                    if words[2] == 0 {
                        u32::MAX
                    } else {
                        index as u32
                    }
                );
                assert_eq!(
                    [
                        owner.declaration.unwrap() as u32,
                        owner.entity.unwrap() as u32,
                        owner.generation,
                        owner.flags
                    ],
                    words[at + 1..at + 5]
                );
                at += 5;
            }
            assert_eq!(
                [
                    registry.next_generation,
                    registry.maximum_registered as u32,
                    registry.registered.count()
                ],
                words[at..at + 3]
            );
            assert_eq!(
                registry.registered.values(),
                words[at + 3..]
                    .iter()
                    .map(|&v| u64::from(v))
                    .collect::<Vec<_>>()
            );
        }
    }
    #[test]
    fn reference_state_masks_match_executed_component_gate() {
        let mut count = 0;
        for row in include_str!("../../tests/fixtures/component_reference_state.txt").lines() {
            let w: Vec<_> = row
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            assert_eq!(
                super::matches_reference_state(w[0], [w[1], w[2]]),
                w[3] != 0,
                "case {count}"
            );
            count += 1;
        }
        assert_eq!(count, 1536);
    }
    #[test]
    fn component_admission_matches_executed_base_routine() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/component_admission.txt").lines() {
            let v: Vec<_> = line
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let mut flags = v[0];
            let schedule =
                super::prepare_admission(&mut flags, v[1] as u8, v[2] as i32, v[3] as i32);
            assert_eq!([flags, u32::from(schedule)], v[4..], "case {count}");
            count += 1;
        }
        assert_eq!(count, 2048);
    }
}
