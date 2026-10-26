//! Per-body 16-bit keyed values (cea9f0/cead30), including retained free-slot
//! values. Allocation ownership is separate from body reservation bitsets.

#[derive(Clone, Debug)]
struct Property<T> {
    next: u16,
    key: u16,
    value: T,
}

#[derive(Default)]
pub struct BodyProperties<T = u64> {
    heads: Vec<u16>,
    properties: Vec<Property<T>>,
    free: Option<u16>,
    deferred_release: Vec<u16>,
    dispatching: bool,
}

impl<T: Default> BodyProperties<T> {
    /// ceab10 removes the body head immediately. During the world's deferred
    /// destruction phase it retains the chain for later processing; otherwise
    /// ceaec0 destroys each value before splicing the chain into the free list.
    /// The value owner supplies ae9d00's signal destruction, not an assumed zero.
    pub fn release_body(
        &mut self,
        body: u32,
        mut destroy_value: impl FnMut(&mut T) -> Result<(), String>,
    ) -> Result<(), String> {
        let Some(head) = self.heads.get_mut((body & 0x00ff_ffff) as usize) else {
            return Ok(());
        };
        if *head == u16::MAX {
            return Ok(());
        }
        let first = *head;
        *head = u16::MAX;
        if self.dispatching {
            self.deferred_release.push(first);
        } else {
            self.release_chain(first, &mut destroy_value)?;
        }
        Ok(())
    }

    /// ceb0c0 emits global listeners, then zero, one or two body listener
    /// chains according to the event key. Releases during callbacks are deferred
    /// until all chains finish. Repeated body IDs intentionally emit twice.
    pub fn dispatch(
        &mut self,
        key: u16,
        bodies: [u32; 2],
        mut invoke: impl FnMut(&mut Self, u16, usize) -> Result<(), String>,
        mut destroy_value: impl FnMut(&mut T) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.heads.is_empty() {
            return Err("body-property dispatch has no global listener head".into());
        }
        self.dispatching = true;
        let result: Result<(), String> = (|| {
            self.dispatch_chain(0, key, 0, &mut invoke)?;
            let count = if key < 9 {
                2
            } else if key.wrapping_sub(10) < 8 {
                1
            } else {
                0
            };
            for (index, &body) in bodies.iter().take(count).enumerate() {
                self.dispatch_chain(body, key, index, &mut invoke)?;
            }
            Ok(())
        })();
        self.dispatching = false;
        result?;
        let mut index = 0;
        while index < self.deferred_release.len() {
            let first = self.deferred_release[index];
            self.release_chain(first, &mut destroy_value)?;
            index += 1;
        }
        self.deferred_release.clear();
        Ok(())
    }

    fn dispatch_chain(
        &mut self,
        body: u32,
        key: u16,
        index: usize,
        invoke: &mut impl FnMut(&mut Self, u16, usize) -> Result<(), String>,
    ) -> Result<(), String> {
        let Some(&head) = self.heads.get((body & 0x00ff_ffff) as usize) else {
            return Ok(());
        };
        let mut current = head;
        let mut visited = 0;
        while current != u16::MAX {
            if visited >= self.properties.len() {
                return Err("cyclic dispatched property chain".into());
            }
            visited += 1;
            if self
                .properties
                .get(current as usize)
                .ok_or("missing dispatched property")?
                .key
                == key
            {
                invoke(self, current, index)?;
            }
            // Reload after dispatch: callbacks may grow the table or edit links.
            current = self
                .properties
                .get(current as usize)
                .ok_or("dispatched property disappeared")?
                .next;
        }
        Ok(())
    }

    pub fn value_mut(&mut self, property: u16) -> Option<&mut T> {
        self.properties
            .get_mut(property as usize)
            .map(|p| &mut p.value)
    }

    fn release_chain(
        &mut self,
        first: u16,
        destroy_value: &mut impl FnMut(&mut T) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut current = first;
        for _ in 0..self.properties.len() {
            let property = self
                .properties
                .get_mut(current as usize)
                .ok_or("released property chain points outside storage")?;
            destroy_value(&mut property.value)?;
            if property.next == u16::MAX {
                property.next = self.free.unwrap_or(u16::MAX);
                self.free = Some(first);
                return Ok(());
            }
            current = property.next;
        }
        Err("cyclic released body-property chain".into())
    }

    pub fn get_or_create(
        &mut self,
        body_capacity: usize,
        body: u32,
        key: u16,
    ) -> Result<&mut T, String> {
        if self.heads.len() < body_capacity {
            self.heads.resize(body_capacity, u16::MAX);
        }
        let slot = (body & 0x00ff_ffff) as usize;
        let mut current = *self
            .heads
            .get(slot)
            .ok_or("property body index exceeds capacity")?;
        let mut visited = 0;
        while current != u16::MAX {
            if visited >= self.properties.len() {
                return Err("cyclic body-property chain".into());
            }
            visited += 1;
            let property = self
                .properties
                .get(current as usize)
                .ok_or("missing body-property record")?;
            if property.key == key {
                return Ok(&mut self.properties[current as usize].value);
            }
            current = property.next;
        }
        let index = if let Some(index) = self.free {
            let property = self
                .properties
                .get(index as usize)
                .ok_or("missing free body-property record")?;
            self.free = (property.next != u16::MAX).then_some(property.next);
            index
        } else {
            let index = u16::try_from(self.properties.len())
                .map_err(|_| "body-property indices exhausted")?;
            if index == u16::MAX {
                return Err("body-property indices exhausted".into());
            }
            self.properties.push(Property {
                next: u16::MAX,
                key: 0,
                value: T::default(),
            });
            index
        };
        let property = &mut self.properties[index as usize];
        property.next = self.heads[slot];
        property.key = key;
        self.heads[slot] = index;
        // cead30 clears +8 only on append; reused free records retain it.
        Ok(&mut property.value)
    }
}

#[cfg(test)]
#[path = "body_properties/tests.rs"]
mod tests;
