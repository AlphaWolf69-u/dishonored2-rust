//! Fixed-capacity motion IDs and the permanent static motion.
//! World resize callbacks, activation lists and release have separate owners.
use super::{motion::Motion, motion_info::MotionInfo, space_cells::SpaceCells};

pub struct MotionStorage {
    motions: Vec<Motion>,
    next_free: Vec<Option<usize>>,
    initialized: Vec<bool>,
    free_head: Option<usize>,
    active_count: u32,
    maximum_index: u32,
    cell_lists: [Vec<u32>; 32],
}
impl MotionStorage {
    /// cd77b0 appends to the selected cell and publishes the solver-list index.
    pub fn add_to_solver(&mut self, id: u32) -> Result<(), String> {
        let cell = self.get(id).ok_or("solver motion is not initialized")?.cell as usize;
        let list = self
            .cell_lists
            .get_mut(cell)
            .ok_or("motion solver cell exceeds world lists")?;
        let index = list.len() as u32;
        list.push(id);
        self.get_mut(id).unwrap().solver_id = index;
        Ok(())
    }

    /// cd7820 swap-removes the motion and repairs the moved motion's index.
    pub fn remove_from_solver(&mut self, id: u32) -> Result<(), String> {
        let motion = self.get(id).ok_or("solver motion is not initialized")?;
        let cell = motion.cell as usize;
        let index = motion.solver_id as usize;
        let list = self
            .cell_lists
            .get_mut(cell)
            .ok_or("motion solver cell exceeds world lists")?;
        if index >= list.len() {
            return Err("motion solver index exceeds its cell list".into());
        }
        let last = *list.last().unwrap();
        list.swap_remove(index);
        if index < list.len() {
            self.get_mut(last)
                .ok_or("moved solver motion is missing")?
                .solver_id = index as u32;
        }
        self.get_mut(id).unwrap().solver_id = 0x7fff_ffff;
        Ok(())
    }

    /// cd6ce0/cd6e20's defined initial storage state. Slot zero is never reserved.
    pub fn new(capacity: usize) -> Result<Self, String> {
        if !(2..0x7fff_ffff).contains(&capacity) {
            return Err("motion capacity must include its static anchor and fit valid IDs".into());
        }
        let mut motions = vec![
            Motion {
                solver_id: 0x7fff_ffff,
                ..Motion::default()
            };
            capacity
        ];
        motions[0] = Motion::default();
        motions[0].pose.orientation = [0., 0., 0., 1.];
        motions[0].cell = u8::MAX;
        motions[0].space_splitter_weight = 1;
        let mut initialized = vec![false; capacity];
        initialized[0] = true;
        Ok(Self {
            motions,
            next_free: (0..capacity)
                .map(|i| {
                    if i > 0 && i + 1 < capacity {
                        Some(i + 1)
                    } else {
                        None
                    }
                })
                .collect(),
            initialized,
            free_head: Some(1),
            active_count: 1,
            maximum_index: 0,
            cell_lists: std::array::from_fn(|_| vec![0]),
        })
    }
    /// cd7250 does not clear a reused motion record. Until ca6e30 initializes
    /// it, the safe reader below withholds its unspecified creation fields.
    pub fn reserve(&mut self) -> Result<u32, String> {
        let index = self.free_head.ok_or("motion buffer is full")?;
        self.free_head = self.next_free[index].take();
        self.motions[index].first_body_id = 0x00ff_ffff;
        self.motions[index].space_splitter_weight = 1;
        self.initialized[index] = false;
        self.active_count = self.active_count.wrapping_add(1);
        self.maximum_index = self.maximum_index.max(index as u32);
        Ok(index as u32)
    }
    pub fn initialize(
        &mut self,
        id: u32,
        info: &MotionInfo,
        cells: &SpaceCells,
    ) -> Result<(), String> {
        let index = id as usize;
        if index == 0
            || self
                .motions
                .get(index)
                .is_none_or(|motion| motion.space_splitter_weight == 0)
        {
            return Err("motion initialization requires a reserved non-static ID".into());
        }
        self.motions[index] = Motion::initialize(info, cells)?;
        self.initialized[index] = true;
        Ok(())
    }
    pub fn get(&self, id: u32) -> Option<&Motion> {
        let index = id as usize;
        self.motions
            .get(index)
            .filter(|motion| motion.space_splitter_weight != 0 && self.initialized[index])
    }
    pub(crate) fn get_mut(&mut self, id: u32) -> Option<&mut Motion> {
        let index = id as usize;
        self.motions
            .get_mut(index)
            .filter(|motion| motion.space_splitter_weight != 0 && self.initialized[index])
    }
    /// cd6c80 starts at one, skips inactive records and excludes the anchor.
    pub fn active_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.motions
            .iter()
            .enumerate()
            .take(self.maximum_index as usize + 1)
            .skip(1)
            .filter_map(|(index, motion)| {
                (motion.space_splitter_weight != 0).then_some(index as u32)
            })
    }
    pub fn capacity(&self) -> usize {
        self.motions.len()
    }
    pub fn has_free_slot(&self) -> bool {
        self.free_head.is_some()
    }
    pub fn active_count(&self) -> u32 {
        self.active_count
    }
    pub fn maximum_index(&self) -> u32 {
        self.maximum_index
    }
    pub fn cell_lists(&self) -> &[Vec<u32>; 32] {
        &self.cell_lists
    }
}

#[cfg(test)]
#[path = "motion_storage/tests.rs"]
mod tests;
#[cfg(test)]
pub(crate) use tests::snapshot as motion_snapshot;
