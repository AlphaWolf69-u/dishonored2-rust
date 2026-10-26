//! Typed component slots following 48fa80/48fe30's chunk and roster policy.
use super::roster::ComponentRoster;

pub struct ComponentPool<T> {
    pub roster: ComponentRoster,
    pub items_per_chunk: usize,
    chunks: Vec<Vec<Option<T>>>,
}
impl<T> ComponentPool<T> {
    pub fn for_class(
        initial_count: i32,
        registry: &crate::gameplay::class_registry::ClassRegistry,
        class: usize,
    ) -> Result<Self, String> {
        Self::new(
            initial_count,
            registry
                .metadata(class)
                .ok_or("component class metadata is missing")?
                .instance_size,
        )
    }

    /// The size is the game's class-info +5c value. Rust's own representation
    /// size does not change the game's allocation/slot ordering policy.
    pub fn new(initial_count: i32, class_size: u32) -> Result<Self, String> {
        if initial_count < 0 || class_size == 0 || class_size > i32::MAX as u32 {
            return Err("invalid component pool class size or initial count".into());
        }
        let items_per_chunk = if initial_count < 100 {
            if initial_count == 0 {
                100
            } else {
                initial_count
            }
        } else if class_size
            .checked_mul(100)
            .ok_or("component pool class size overflow")?
            < 0x10000
        {
            initial_count.min(1000.min((0x10000 / class_size) as i32))
        } else {
            100
        };
        let roster = ComponentRoster::new(initial_count, items_per_chunk)?;
        let mut pool = Self {
            roster,
            items_per_chunk: items_per_chunk as usize,
            chunks: Vec::new(),
        };
        for _ in 0..pool.roster.indices.len() / pool.items_per_chunk {
            pool.add_chunk()?;
        }
        Ok(pool)
    }

    fn add_chunk(&mut self) -> Result<(), String> {
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(self.items_per_chunk)
            .map_err(|e| format!("component chunk allocation failed: {e}"))?;
        slots.resize_with(self.items_per_chunk, || None);
        self.chunks.push(slots);
        Ok(())
    }

    /// 48fe30 reserves the real roster index, allocates a chunk at its boundary,
    /// then calls the component type's required in-place constructor.
    pub fn allocate(
        &mut self,
        construct: impl FnOnce(u16) -> Result<T, String>,
    ) -> Result<u16, String> {
        let index = self.roster.reserve()?;
        let chunk = index as usize / self.items_per_chunk;
        let slot = index as usize % self.items_per_chunk;
        if chunk == self.chunks.len() {
            self.add_chunk()?;
        }
        let target = self
            .chunks
            .get_mut(chunk)
            .and_then(|v| v.get_mut(slot))
            .ok_or("component roster index has no allocated chunk")?;
        if target.is_some() {
            return Err("component roster selected an occupied slot".into());
        }
        *target = Some(construct(index)?);
        Ok(index)
    }

    pub fn get(&self, index: u16) -> Option<&T> {
        self.chunks
            .get(index as usize / self.items_per_chunk)?
            .get(index as usize % self.items_per_chunk)?
            .as_ref()
    }
    /// 48fe10/48ff60 use490030 to recover the factory slot from an address.
    /// Typed handles already retain that identity in this same pool.
    pub fn schedule(&mut self, index: u16) -> Result<(), String> {
        self.get(index)
            .ok_or("cannot schedule an unallocated component")?;
        self.roster.schedule(index)
    }
    pub fn unschedule(&mut self, index: u16) -> Result<(), String> {
        self.get(index)
            .ok_or("cannot unschedule an unallocated component")?;
        self.roster.unschedule(index)
    }
    pub fn get_mut(&mut self, index: u16) -> Option<&mut T> {
        self.chunks
            .get_mut(index as usize / self.items_per_chunk)?
            .get_mut(index as usize % self.items_per_chunk)?
            .as_mut()
    }
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
    /// Update queues use host-sized handles; the component pool's real slots
    /// remain 16-bit identities. These views never create a second object store.
    pub fn get_at(&self, index: usize) -> Option<&T> {
        self.get(u16::try_from(index).ok()?)
    }
    pub fn get_at_mut(&mut self, index: usize) -> Option<&mut T> {
        self.get_mut(u16::try_from(index).ok()?)
    }
    pub fn allocated_count(&self) -> usize {
        usize::try_from(self.roster.used).expect("component roster count is negative")
    }
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.chunks
            .iter()
            .flat_map(|chunk| chunk.iter().filter_map(Option::as_ref))
    }
}

impl<T> std::ops::Index<usize> for ComponentPool<T> {
    type Output = T;
    fn index(&self, index: usize) -> &Self::Output {
        self.get_at(index)
            .expect("component pool slot is unallocated")
    }
}
impl<T> std::ops::IndexMut<usize> for ComponentPool<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_at_mut(index)
            .expect("component pool slot is unallocated")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pools_match_executed_construction_and_component_allocation() {
        let mut cases = 0;
        for line in include_str!("../../../tests/fixtures/component_pool.txt").lines() {
            let mut words = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap());
            let initial = words.next().unwrap() as i32;
            let size = words.next().unwrap();
            let mut pool = ComponentPool::new(initial, size).unwrap();
            assert_eq!(
                [
                    pool.items_per_chunk as u32,
                    pool.roster.indices.len() as u32,
                    pool.chunk_count() as u32
                ],
                std::array::from_fn(|_| words.next().unwrap())
            );
            let steps = words.next().unwrap();
            for _ in 0..steps {
                let index = pool.allocate(|index| Ok(index)).unwrap();
                assert_eq!(
                    [
                        u32::from(index),
                        pool.roster.indices.len() as u32,
                        pool.chunk_count() as u32,
                        pool.roster.used as u32,
                        pool.roster.maximum_used as u32
                    ],
                    std::array::from_fn(|_| words.next().unwrap())
                );
                assert_eq!(pool.get(index), Some(&index));
                assert_eq!(*pool.get_mut(index).unwrap(), index);
            }
            assert!(words.next().is_none());
            cases += 1;
        }
        assert_eq!(cases, 42);
    }
}
