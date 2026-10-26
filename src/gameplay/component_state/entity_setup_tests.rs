use super::*;
use crate::{gameplay::component_state::order::insertion_slot, storage::PointerList};

struct World {
    declarations: Vec<Option<usize>>,
    flags: [u8; 6],
    owners: Vec<ComponentOwner>,
    events: Vec<u32>,
    mode: u32,
    prefix: i32,
    calls: usize,
}
impl World {
    fn state(entity: &EntityComponents) -> [u32; 3] {
        [
            u32::from(entity.cached_classes.is_some()),
            entity.cached_classes.as_ref().map_or(0, |c| c.len() as u32),
            entity.entries.count(),
        ]
    }
}
impl EntityComponentSetup for World {
    fn declaration_count(&self, _: &EntityComponents) -> Result<usize, String> {
        Ok(self.declarations.len())
    }
    fn declaration_at(&self, _: &EntityComponents, index: usize) -> Result<Option<usize>, String> {
        Ok(self.declarations[index])
    }
    fn declaration_flags(&self, declaration: usize) -> Result<u8, String> {
        Ok(self.flags[declaration])
    }
    fn create_component(
        &mut self,
        entity: &mut EntityComponents,
        declaration: usize,
        immediate: bool,
    ) -> Result<Option<ComponentHandle>, String> {
        self.events
            .extend([0xf0, declaration as u32, u32::from(immediate)]);
        self.events.extend(Self::state(entity));
        if entity.creation_disabled {
            return Ok(None);
        }
        let slot = self.owners.len() as u16;
        let class_id = 100 + i32::from(slot);
        let handle = ComponentHandle { class_id, slot };
        self.owners.push(ComponentOwner {
            declaration: Some(declaration),
            entity: Some(entity.identity),
            generation: u32::from(slot),
            flags: 1 | if immediate { 4 } else { 0 },
        });
        let position = insertion_slot(
            &entity.entries,
            class_id,
            entity.cached_classes.as_mut(),
            |value| Ok(ComponentHandle::from_packed(value)?.class_id),
        )?;
        entity
            .entries
            .insert(position as u32, handle.packed(), false)?;
        Ok(Some(handle))
    }
    fn component_prefix_count(&mut self, entity: &mut EntityComponents) -> Result<i32, String> {
        let count = self.prefix + (self.calls % 3) as i32;
        self.calls += 1;
        self.events.extend([0xe1, count as u32]);
        if self.mode & 1 != 0 && self.calls == 1 && self.declarations.len() > 1 {
            *self.declarations.last_mut().unwrap() = None;
        }
        if self.mode & 2 != 0 {
            entity.start_deferred = !entity.start_deferred;
        }
        Ok(count)
    }
    fn component_owner_mut(
        &mut self,
        component: ComponentHandle,
    ) -> Result<&mut ComponentOwner, String> {
        Ok(&mut self.owners[component.slot as usize])
    }
    fn find_reference_actor(
        &mut self,
        _: &mut EntityComponents,
    ) -> Result<Option<ComponentHandle>, String> {
        self.events.extend([0xe2, self.owners.len() as u32]);
        Ok(self.owners.len().checked_sub(1).map(|n| ComponentHandle {
            class_id: 100 + n as i32,
            slot: n as u16,
        }))
    }
}

#[test]
fn declaration_setup_matches_complete_executed_entity_bootstrap() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/entity_component_setup.txt").lines() {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let mode = words.next().unwrap();
        let prefix = words.next().unwrap() as i32;
        let deferred = words.next().unwrap() != 0;
        let disabled = words.next().unwrap() != 0;
        let cache_capacity = words.next().unwrap();
        let entry_capacity = words.next().unwrap();
        let count = words.next().unwrap();
        let declarations = (0..count)
            .map(|_| {
                let index = words.next().unwrap();
                (index != u32::MAX).then_some(index as usize)
            })
            .collect();
        let flags = std::array::from_fn(|_| words.next().unwrap() as u8);
        let mut entity = EntityComponents::new(42);
        entity.start_deferred = deferred;
        entity.creation_disabled = disabled;
        entity.entries = PointerList::with_capacity(entry_capacity, 0x3f).unwrap();
        entity.cached_classes =
            (cache_capacity > 0).then(|| Vec::with_capacity(cache_capacity as usize));
        let mut world = World {
            declarations,
            flags,
            owners: Vec::new(),
            events: Vec::new(),
            mode,
            prefix,
            calls: 0,
        };
        entity.setup_components(&mut world).unwrap();
        let mut actual = vec![u32::from(entity.start_deferred)];
        actual.extend(World::state(&entity));
        if let Some(classes) = entity.cached_classes {
            actual.extend(classes.into_iter().map(u32::from));
        }
        actual.extend(
            entity
                .entries
                .values()
                .iter()
                .map(|&v| u32::from(ComponentHandle::from_packed(v).unwrap().slot)),
        );
        actual.push(world.owners.len() as u32);
        actual.extend(world.owners.iter().map(|o| o.flags));
        actual.push(
            entity
                .reference_actor
                .map_or(u32::MAX, |c| u32::from(c.slot)),
        );
        actual.push(world.events.len() as u32);
        actual.extend(world.events);
        assert_eq!(actual, words.collect::<Vec<_>>(), "case {cases}");
        cases += 1;
    }
    assert_eq!(cases, 512);
}
