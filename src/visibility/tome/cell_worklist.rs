//! Indexed linked work queues from FUN_140a85790. Node zero is the sentinel.
use super::{
    CellTraversalState, CellWorkItem, LinkQueryDirection, direct_spatial_link_target,
    ready_same_spatial_cell_target,
};

#[derive(Clone, Copy, Debug)]
struct Link {
    item: CellWorkItem,
    previous: u16,
    next: u16,
}

#[derive(Clone, Debug, Default)]
pub struct CellTraversalWorklist {
    nodes: Vec<Option<Link>>,
    cell_counts: Vec<u16>,
    head: u16,
    tail: u16,
    count: usize,
}

// Allocation history is scratch state; the observable list is its ordered items.
impl PartialEq for CellTraversalWorklist {
    fn eq(&self, other: &Self) -> bool {
        self.entries().eq(other.entries())
    }
}
impl Eq for CellTraversalWorklist {}

impl CellTraversalWorklist {
    pub fn new(entries: impl IntoIterator<Item = CellWorkItem>) -> Result<Self, String> {
        let mut result = Self::default();
        for item in entries {
            if item.entry_id == 0 || item.entry_id & !CellTraversalState::ENTRY_MASK != 0 {
                return Err("Tome work-list entry is outside its 14-bit non-sentinel range".into());
            }
            if result.contains_entry(item.entry_id) {
                return Err("duplicate Tome work-list entry ID".into());
            }
            result.push_back(item);
        }
        Ok(result)
    }
    pub fn entries(&self) -> impl Iterator<Item = CellWorkItem> + '_ {
        let mut cursor = self.head;
        std::iter::from_fn(move || {
            if cursor == 0 {
                return None;
            }
            let node = self.nodes[cursor as usize]
                .as_ref()
                .expect("linked work-list node is present");
            cursor = node.next;
            Some(node.item)
        })
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn len(&self) -> usize {
        self.count
    }
    pub(super) fn contains_entry(&self, id: u16) -> bool {
        self.nodes.get(id as usize).is_some_and(Option::is_some)
    }
    pub(super) fn contains_cell(&self, cell: u16) -> bool {
        self.cell_counts
            .get(cell as usize)
            .is_some_and(|&count| count != 0)
    }
    fn reserve(&mut self, item: CellWorkItem) {
        assert!(
            !self.contains_entry(item.entry_id),
            "duplicate linked work-list node"
        );
        if self.nodes.len() <= item.entry_id as usize {
            self.nodes.resize(item.entry_id as usize + 1, None);
        }
        if self.cell_counts.len() <= item.cell_index as usize {
            self.cell_counts.resize(item.cell_index as usize + 1, 0);
        }
        self.cell_counts[item.cell_index as usize] += 1;
        self.count += 1;
    }
    pub(super) fn push_front(&mut self, item: CellWorkItem) {
        self.reserve(item);
        let previous_head = self.head;
        self.nodes[item.entry_id as usize] = Some(Link {
            item,
            previous: 0,
            next: previous_head,
        });
        if previous_head == 0 {
            self.tail = item.entry_id;
        } else {
            self.nodes[previous_head as usize]
                .as_mut()
                .unwrap()
                .previous = item.entry_id;
        }
        self.head = item.entry_id;
    }
    pub(super) fn push_back(&mut self, item: CellWorkItem) {
        self.reserve(item);
        let previous_tail = self.tail;
        self.nodes[item.entry_id as usize] = Some(Link {
            item,
            previous: previous_tail,
            next: 0,
        });
        if previous_tail == 0 {
            self.head = item.entry_id;
        } else {
            self.nodes[previous_tail as usize].as_mut().unwrap().next = item.entry_id;
        }
        self.tail = item.entry_id;
    }
    pub(super) fn pop_front(&mut self) -> Option<CellWorkItem> {
        self.remove_entry(self.head)
    }
    fn remove_entry(&mut self, id: u16) -> Option<CellWorkItem> {
        let node = self.nodes.get_mut(id as usize)?.take()?;
        if node.previous == 0 {
            self.head = node.next;
        } else {
            self.nodes[node.previous as usize].as_mut().unwrap().next = node.next;
        }
        if node.next == 0 {
            self.tail = node.previous;
        } else {
            self.nodes[node.next as usize].as_mut().unwrap().previous = node.previous;
        }
        self.count -= 1;
        self.cell_counts[node.item.cell_index as usize] -= 1;
        Some(node.item)
    }
    /// Port FUN_140a85790's same-spatial fast path and its ordered-list/state
    /// transitions. The scan limit is derived from the active list after its
    /// head is removed, matching FUN_140a85790's decrement of the query-local
    /// active count. Gate openness is assumed to have been checked by
    /// FUN_140a85bd0 before this helper is reached.
    pub fn select_same_spatial_chain<F>(
        &mut self,
        spatial_index: usize,
        direction: LinkQueryDirection,
        cell_states: &mut [u16],
        mut links_for_cell: F,
    ) -> Result<Option<CellWorkItem>, String>
    where
        F: FnMut(usize, &mut dyn FnMut([u32; 4]) -> Result<bool, String>) -> Result<(), String>,
    {
        let Some(mut current) = self.pop_front() else {
            return Ok(None);
        };

        let mut remaining = self.len();
        while remaining != 0 {
            let mut next = None;
            links_for_cell(current.cell_index as usize, &mut |link| {
                // FUN_140a85790 jumps out of the fast scan at the first
                // boundary-face or different-spatial link.
                let Some(target_spatial) = direct_spatial_link_target(link) else {
                    return Ok(false);
                };
                if target_spatial as usize != spatial_index {
                    return Ok(false);
                }
                let target_cell_index = (link[1] >> 16) as usize;
                let state = *cell_states
                    .get(target_cell_index)
                    .ok_or("Tome traversal cell-state table is too short")?;
                if let Some(target) =
                    ready_same_spatial_cell_target(link, spatial_index, direction, state)
                {
                    next = Some(target);
                    return Ok(false);
                }
                Ok(true)
            })?;

            let Some(target) = next else {
                break;
            };

            // FUN_140a85790 puts the current entry at the list head and marks
            // its cell processed before removing the queued target entry.
            self.push_front(current);
            let source_state = cell_states
                .get_mut(current.cell_index as usize)
                .ok_or("Tome traversal cell-state table is too short")?;
            *source_state = current.entry_id | 0xc000;

            current = self
                .remove_entry(target.worklist_entry)
                .ok_or("queued Tome work-list entry is absent from the active list")?;
            remaining -= 1;
        }

        // Match FUN_140a85790's list walk: requeue entries until the first
        // entry whose cell already has the queued flag.
        for item in self.entries() {
            let state = cell_states
                .get_mut(item.cell_index as usize)
                .ok_or("Tome traversal cell-state table is too short")?;
            if *state & 0xc000 == 0x8000 {
                break;
            }
            *state = item.entry_id | 0x8000;
        }

        Ok(Some(current))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn indexed_queue_preserves_order_membership_and_reused_ids() {
        let mut queue = CellTraversalWorklist::default();
        let mut reference = VecDeque::<CellWorkItem>::new();
        let mut seed = 0x9e3779b9u32;
        for step in 0..2048 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let id = (seed % 127 + 1) as u16;
            let item = CellWorkItem {
                entry_id: id,
                cell_index: ((seed >> 9) % 32) as u16,
            };
            match (seed >> 5) & 3 {
                0 | 1 if !queue.contains_entry(id) => {
                    if seed & 1 == 0 {
                        queue.push_front(item);
                        reference.push_front(item);
                    } else {
                        queue.push_back(item);
                        reference.push_back(item);
                    }
                }
                2 => assert_eq!(queue.pop_front(), reference.pop_front()),
                _ => {
                    let expected = reference
                        .iter()
                        .position(|item| item.entry_id == id)
                        .and_then(|position| reference.remove(position));
                    assert_eq!(queue.remove_entry(id), expected);
                }
            }
            assert_eq!(
                queue.entries().collect::<Vec<_>>(),
                reference.iter().copied().collect::<Vec<_>>(),
                "queue step{step}"
            );
            assert_eq!(queue.len(), reference.len());
            for cell in 0..32 {
                assert_eq!(
                    queue.contains_cell(cell),
                    reference.iter().any(|item| item.cell_index == cell)
                );
            }
            for id in 1..128 {
                assert_eq!(
                    queue.contains_entry(id),
                    reference.iter().any(|item| item.entry_id == id)
                );
            }
            assert_eq!(queue.clone(), queue);
        }
        while queue.pop_front().is_some() {}
        assert!(queue.is_empty());
        assert_eq!(queue, CellTraversalWorklist::default());
    }
}
