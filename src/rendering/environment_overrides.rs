//! The environment manager's 16 stable override slots (890150, 890970,
//! 8911a0 and 88e220). Entity/script controllers supply the environments.

pub struct EnvironmentOverrides<T> {
    slots: [Slot<T>; 16],
}
struct Slot<T> {
    environment: Option<T>,
    weight: f32,
    influence: f32,
    retiring: bool,
}
impl<T> Default for EnvironmentOverrides<T> {
    fn default() -> Self {
        Self {
            // 88b9f0 initializes each record's second factor to one.
            slots: std::array::from_fn(|_| Slot {
                environment: None,
                weight: 0.,
                influence: 1.,
                retiring: false,
            }),
        }
    }
}
impl<T> EnvironmentOverrides<T> {
    /// Reserve the first empty slot. Duplicate environments keep separate
    /// slots. An absent environment leaves its returned slot reusable.
    /// None means exhaustion; the original caller reports a fatal capacity
    /// failure, which an integrating owner must surface rather than ignore.
    pub fn register(&mut self, environment: Option<T>) -> Option<usize> {
        let index = self
            .slots
            .iter()
            .position(|slot| slot.environment.is_none())?;
        self.slots[index].environment = environment;
        self.slots[index].weight = 0.;
        Some(index)
    }

    /// Null and foreign handles are ignored. Empty slots remain addressable,
    /// just as the manager's stable pointer handles do.
    pub fn set_weights(&mut self, id: Option<usize>, weight: f32, influence: f32) {
        if let Some(slot) = id.and_then(|id| self.slots.get_mut(id)) {
            slot.weight = weight;
            slot.influence = influence;
        }
    }

    pub fn retire(&mut self, id: Option<usize>) {
        if let Some(slot) = id.and_then(|id| self.slots.get_mut(id)) {
            slot.retiring = true;
        }
    }

    /// Visit in slot order. A retiring environment participates once with
    /// weight zero before its slot becomes empty; the other factor survives.
    /// A retirement flag on an empty slot survives until it has an environment.
    pub fn visit_frame(&mut self, mut visit: impl FnMut(&T, f32, f32)) {
        for slot in &mut self.slots {
            if slot.environment.is_none() {
                continue;
            }
            if slot.retiring {
                let environment = slot.environment.take().unwrap();
                slot.weight = 0.;
                slot.retiring = false;
                visit(&environment, slot.weight, slot.influence);
            } else {
                visit(
                    slot.environment.as_ref().unwrap(),
                    slot.weight,
                    slot.influence,
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "environment_overrides/tests.rs"]
mod tests;
