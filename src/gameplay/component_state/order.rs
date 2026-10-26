//! Entity component insertion order (878a50 and its 87b5d0 search).
use crate::storage::PointerList;

/// A cached class list and an uncached component walk deliberately differ on
/// equal IDs. The entity owner decides whether a class cache exists. Class
/// metadata is required for each actual component in the uncached path.
pub fn insertion_slot(
    components: &PointerList,
    class_id: i32,
    cached_classes: Option<&mut Vec<u16>>,
    mut class_of: impl FnMut(u64) -> Result<i32, String>,
) -> Result<usize, String> {
    if let Some(classes) = cached_classes {
        let key = class_id as u16;
        let slot = if classes
            .last()
            .is_none_or(|&last| i32::from(last) <= class_id)
        {
            classes.len()
        } else {
            // 87b5d0 is a lower-bound search over unsigned 16-bit IDs.
            classes.partition_point(|&value| value < key)
        };
        // 59bc90/8788d0's ordered logical insertion is Vec::insert. Engine
        // allocator ownership is supplied by the Rust container.
        classes.insert(slot, key);
        Ok(slot)
    } else {
        for (index, &component) in components.values().iter().enumerate() {
            if class_id <= class_of(component)? {
                return Ok(index);
            }
        }
        Ok(components.values().len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slots_match_complete_executed_entity_ordering() {
        let mut cases = 0;
        for line in include_str!("../../../tests/fixtures/component_order.txt").lines() {
            let mut words = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap());
            let cached = words.next().unwrap() != 0;
            let key = words.next().unwrap() as i32;
            let count = words.next().unwrap();
            let classes: Vec<_> = (0..count).map(|_| words.next().unwrap() as u16).collect();
            let mut components = PointerList::with_capacity(count, 0x40).unwrap();
            for index in 0..count {
                components.append(index.into(), false).unwrap();
            }
            let mut cache = classes.clone();
            let result = insertion_slot(&components, key, cached.then_some(&mut cache), |handle| {
                Ok(i32::from(classes[handle as usize]))
            })
            .unwrap();
            assert_eq!(result as u32, words.next().unwrap(), "case {cases}");
            let cache_count = words.next().unwrap();
            if cached {
                assert_eq!(cache.len(), cache_count as usize);
                assert_eq!(cache, words.map(|v| v as u16).collect::<Vec<_>>());
            } else {
                assert_eq!(cache_count, 0);
                assert!(words.next().is_none());
                assert_eq!(cache, classes);
            }
            cases += 1;
        }
        assert_eq!(cases, 768);
    }
}
