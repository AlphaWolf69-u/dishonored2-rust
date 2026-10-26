//!877720 component lookup, including cached search order and inactive fallback.
use super::{
    entity::{ComponentHandle, EntityComponents},
    matches_reference_state,
};

pub struct ComponentLookupState {
    pub flags: u32,
    /// A missing declaration is an error only after reference-state admission.
    pub start_phases: Option<u32>,
}

impl EntityComponents {
    /// `class_range` is the requested class's inclusive hierarchy ID range;
    /// `masks` is the selected7e7390 reference-policy table row. This does not
    /// create components. The source prefers a phase match, otherwise retaining
    /// the last reference-eligible candidate encountered in its search order.
    pub fn find_component(
        &self,
        class_range: [i32; 2],
        masks: [u32; 2],
        mut state: impl FnMut(ComponentHandle) -> Result<ComponentLookupState, String>,
    ) -> Result<Option<ComponentHandle>, String> {
        let phase = 1u32 << (self.phase & 31);
        let mut fallback = None;
        let mut candidate = |index: usize| -> Result<Option<ComponentHandle>, String> {
            let word = *self
                .entries
                .values()
                .get(index)
                .ok_or("class cache has no component entry")?;
            let handle = ComponentHandle::from_packed(word)?;
            let component = state(handle)?;
            if matches_reference_state(component.flags, masks) {
                fallback = Some(handle);
                if component
                    .start_phases
                    .ok_or("lookup component declaration is missing")?
                    & phase
                    != 0
                {
                    return Ok(Some(handle));
                }
            }
            Ok(None)
        };
        if let Some(classes) = &self.cached_classes {
            let mut low = 0;
            let mut high = classes.len();
            // Source high is inclusive; half-open bounds here select the same
            // midpoint floor((low + inclusiveHigh)/2), including even lengths.
            while low < high {
                let mid = (low + high - 1) / 2;
                let class = i32::from(classes[mid]);
                if class < class_range[0] {
                    low = mid + 1;
                } else if class > class_range[1] {
                    high = mid;
                } else {
                    for index in mid..high {
                        if i32::from(classes[index]) > class_range[1] {
                            break;
                        }
                        if let Some(handle) = candidate(index)? {
                            return Ok(Some(handle));
                        }
                    }
                    for index in (low..mid).rev() {
                        if i32::from(classes[index]) < class_range[0] {
                            break;
                        }
                        if let Some(handle) = candidate(index)? {
                            return Ok(Some(handle));
                        }
                    }
                    break;
                }
            }
        } else {
            for (index, &word) in self.entries.values().iter().enumerate() {
                let handle = ComponentHandle::from_packed(word)?;
                if handle.class_id >= class_range[0] && handle.class_id <= class_range[1] {
                    if let Some(handle) = candidate(index)? {
                        return Ok(Some(handle));
                    }
                }
            }
        }
        Ok(fallback)
    }
}

#[cfg(test)]
mod tests;
