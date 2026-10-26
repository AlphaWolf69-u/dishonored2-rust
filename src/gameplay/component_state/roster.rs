//! Component slot reservation and roster growth (48fd60/490160/4900d0).

pub struct ComponentRoster {
    pub indices: Vec<u16>,
    pub block_size: i32,
    pub cursor: i32,
    pub sorted_start: i32,
    pub used: i32,
    pub maximum_used: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(roster: &ComponentRoster) -> Vec<u32> {
        let mut out = vec![
            roster.indices.len() as u32,
            roster.block_size as u32,
            roster.cursor as u32,
            roster.sorted_start as u32,
            roster.used as u32,
            roster.maximum_used as u32,
        ];
        out.extend(roster.indices.iter().map(|&v| u32::from(v)));
        out
    }

    #[test]
    fn update_partitions_match_executed_scheduling_and_removal() {
        let mut cases = 0;
        for line in include_str!("../../../tests/fixtures/component_scheduling.txt").lines() {
            let mut words = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap());
            let mut roster = ComponentRoster::new(0, 7).unwrap();
            let steps = words.next().unwrap();
            for _ in 0..steps {
                let op = words.next().unwrap();
                let slot = words.next().unwrap() as u16;
                match op {
                    0 => assert_eq!(roster.reserve().unwrap(), slot),
                    1 => roster.schedule(slot).unwrap(),
                    2 => roster.unschedule(slot).unwrap(),
                    _ => panic!("invalid fixture operation"),
                }
                let actual = snapshot(&roster);
                let expected: Vec<_> = words.by_ref().take(actual.len()).collect();
                assert_eq!(actual, expected, "case {cases}");
            }
            assert!(words.next().is_none());
            cases += 1;
        }
        assert_eq!(cases, 128);
    }

    #[test]
    fn rosters_match_executed_construction_relocation_and_slot_reservation() {
        let mut cases = 0;
        for line in include_str!("../../../tests/fixtures/component_roster.txt").lines() {
            let mut words = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap());
            let initial = words.next().unwrap() as i32;
            let capacity = words.next().unwrap();
            let block = words.next().unwrap() as i32;
            let mut roster = ComponentRoster::new(initial, block).unwrap();
            assert_eq!(roster.indices.len(), capacity as usize);
            assert_eq!(
                roster.indices,
                (0..capacity).map(|v| v as u16).collect::<Vec<_>>()
            );
            roster.cursor = words.next().unwrap() as i32;
            roster.sorted_start = words.next().unwrap() as i32;
            roster.used = words.next().unwrap() as i32;
            roster.maximum_used = words.next().unwrap() as i32;
            for index in &mut roster.indices {
                *index = words.next().unwrap() as u16;
            }
            let steps = words.next().unwrap();
            for _ in 0..steps {
                assert_eq!(u32::from(roster.reserve().unwrap()), words.next().unwrap());
                let actual = snapshot(&roster);
                let expected: Vec<_> = words.by_ref().take(actual.len()).collect();
                assert_eq!(actual, expected, "case {cases}");
            }
            assert!(words.next().is_none());
            cases += 1;
        }
        assert_eq!(cases, 256);
    }
}
impl ComponentRoster {
    /// 490280 moves an allocated slot from the sorted inactive suffix into
    /// the sorted update prefix. The source count at +28 and pointer at +18
    /// describe the same boundary, represented here by sorted_start.
    pub fn schedule(&mut self, slot: u16) -> Result<(), String> {
        let boundary =
            usize::try_from(self.sorted_start).map_err(|_| "negative component update boundary")?;
        let used = usize::try_from(self.used).map_err(|_| "negative component count")?;
        let suffix = self
            .indices
            .get(boundary..used)
            .ok_or("component update boundary exceeds allocated roster")?;
        let mut at = boundary
            + suffix
                .binary_search(&slot)
                .map_err(|_| "component is not in the inactive roster")?;
        while at > 0 {
            if at - 1 < boundary && self.indices[at - 1] <= slot {
                break;
            }
            self.indices[at] = self.indices[at - 1];
            at -= 1;
        }
        self.indices[at] = slot;
        self.sorted_start += 1;
        Ok(())
    }

    /// 490320 reverses the transition without freeing the component slot.
    pub fn unschedule(&mut self, slot: u16) -> Result<(), String> {
        let boundary =
            usize::try_from(self.sorted_start).map_err(|_| "negative component update boundary")?;
        let cursor = usize::try_from(self.cursor).map_err(|_| "negative component cursor")?;
        let prefix = self
            .indices
            .get(..boundary)
            .ok_or("component update boundary exceeds roster storage")?;
        let mut at = prefix
            .binary_search(&slot)
            .map_err(|_| "component is not in the update roster")?;
        if boundary > cursor || cursor > self.indices.len() {
            return Err("component update boundary exceeds allocation cursor".into());
        }
        while at + 1 < cursor {
            if at + 1 >= boundary && slot <= self.indices[at + 1] {
                break;
            }
            self.indices[at] = self.indices[at + 1];
            at += 1;
        }
        self.indices[at] = slot;
        self.sorted_start -= 1;
        Ok(())
    }

    /// 4900d0 resets storage; 48fa80 initializes both cursors and counts.
    pub fn new(initial_capacity: i32, block_size: i32) -> Result<Self, String> {
        if initial_capacity < 0 || block_size <= 0 {
            return Err("invalid component roster capacity or growth block".into());
        }
        let mut roster = Self {
            indices: Vec::new(),
            block_size,
            cursor: 0,
            sorted_start: 0,
            used: 0,
            maximum_used: 0,
        };
        if initial_capacity > 0 {
            roster.grow(initial_capacity)?;
        }
        Ok(roster)
    }

    /// The game's allocator/refcounted buffer becomes Vec ownership. Existing
    /// indices survive relocation and the new tail contains sequential u16 IDs.
    pub fn grow(&mut self, requested: i32) -> Result<(), String> {
        if self.block_size <= 0 || requested < 0 {
            return Err("invalid component roster growth request".into());
        }
        let rounded = requested
            .checked_add(self.block_size - 1)
            .ok_or("component roster capacity overflow")?;
        let capacity = (rounded - rounded % self.block_size) as usize;
        if capacity < self.indices.len() {
            return Err("component roster growth would truncate live storage".into());
        }
        self.indices
            .try_reserve_exact(capacity - self.indices.len())
            .map_err(|e| format!("component roster allocation failed: {e}"))?;
        self.indices
            .extend((self.indices.len()..capacity).map(|i| i as u16));
        Ok(())
    }

    /// 48fd60 grows for used+1 (confirmed EDX at the machine-code call), then
    /// inserts the selected slot into the sorted active range. Cursors are
    /// offsets, so relocating the owned buffer cannot leave dangling pointers.
    pub fn reserve(&mut self) -> Result<u16, String> {
        if self.used < 0
            || self.sorted_start < 0
            || self.sorted_start > self.used
            || self.cursor < 0
            || self.cursor as usize > self.indices.len()
            || self.used as usize > self.indices.len()
        {
            return Err("component roster cursors disagree with storage".into());
        }
        let requested = self
            .used
            .checked_add(1)
            .ok_or("component roster count overflow")?;
        if (self.indices.len() as i32) < requested {
            self.grow(requested)?;
        }
        let index = self.used as usize;
        let value = self.indices[index];
        let mut at = index;
        while at > self.sorted_start as usize && self.indices[at - 1] > value {
            self.indices[at] = self.indices[at - 1];
            at -= 1;
        }
        self.indices[at] = value;
        self.used = requested;
        self.cursor += 1;
        self.maximum_used = self.maximum_used.max(self.used);
        Ok(value)
    }
}
