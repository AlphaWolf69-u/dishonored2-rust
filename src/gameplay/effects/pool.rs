//! 221b80's shared free-list to group-list transfer. The owner retains the
//! pool's lock; mutable borrows prevent concurrent edits through this function.

/// Links belong to the shared pool, not to a single running group. Initialization
/// is supplied by its owner; this type does not guess a free-chain layout.
pub struct EffectPoolLinks {
    pub next: Vec<u16>,
    pub free_head: u16,
    pub allocated_count: u16,
}
impl EffectPoolLinks {
    /// 19a1d0 -> 19f3d0 initializes this family's 64 slots in ascending order.
    /// Other FX families have their own capacities and must not reuse this limit.
    pub fn environment() -> Self {
        Self {
            next: (1..64).chain(std::iter::once(u16::MAX)).collect(),
            free_head: 0,
            allocated_count: 0,
        }
    }

    pub fn acquire(&mut self, group_head: &mut u16) -> Result<bool, String> {
        acquire(
            &mut self.free_head,
            group_head,
            &mut self.next,
            &mut self.allocated_count,
        )
    }

    /// 2225f0's head release. Returned storage is pushed onto the free chain.
    pub fn release_head(&mut self, group_head: &mut u16) -> Result<(), String> {
        let index = *group_head;
        let next = self.release(index)?;
        *group_head = next;
        Ok(())
    }

    /// The same transfer is used for an inactive interior link by 21eec0;
    /// its caller reconnects the previous node to this returned successor.
    pub(super) fn release(&mut self, index: u16) -> Result<u16, String> {
        let link = self
            .next
            .get_mut(usize::from(index))
            .ok_or_else(|| format!("FX release index {index} exceeds pool capacity"))?;
        let next = *link;
        *link = self.free_head;
        self.free_head = index;
        self.allocated_count = self.allocated_count.wrapping_sub(1);
        Ok(next)
    }
}
pub fn acquire(
    free_head: &mut u16,
    group_head: &mut u16,
    links: &mut [u16],
    allocated_count: &mut u16,
) -> Result<bool, String> {
    if *free_head == u16::MAX {
        return Ok(false);
    }
    let index = *free_head;
    let link = links
        .get_mut(usize::from(index))
        .ok_or_else(|| format!("FX free-list index {index} exceeds pool capacity"))?;
    *free_head = *link;
    *link = *group_head;
    *group_head = index;
    *allocated_count = allocated_count.wrapping_add(1);
    Ok(true)
}
