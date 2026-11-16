//! Native Heart TI providers read current admitted identities/components.
use super::*;
use crate::gameplay::entity_lifetime::{EntityId, LiveEntities};

impl PhysicsScene {
    ///6123e0 rejects the actual idWorldspawn class range, not every non-NPC.
    pub(in crate::app) fn heart_context_target(
        &self,
        live: &LiveEntities,
        target: Option<EntityId>,
    ) -> Result<Option<EntityId>, String> {
        let Some(target) = target else {
            return Ok(None);
        };
        let Some((_, mut class)) = live.get(target) else {
            return Ok(None);
        };
        let classes = self.types.classes();
        loop {
            let metadata = classes
                .metadata(class)
                .ok_or("Heart target class metadata absent")?;
            if classes.name_text(metadata.name) == Some(b"idWorldspawn") {
                return Ok(None);
            }
            let Some(base) = classes.hierarchy().node(class).and_then(|node| node.base) else {
                break;
            };
            class = base;
        }
        Ok(Some(target))
    }

    ///The closed current actor set has no AIIdentity/AreaLocator payloads.
    ///Check real component headers before taking the source's null branch;
    ///future admitted providers must supply their actual data, never defaults.
    pub(in crate::app) fn heart_context_component_present(
        &self,
        entity: EntityId,
        name: &[u8],
    ) -> Result<bool, String> {
        let Some(&index) = self.shared_indices.get(&entity) else {
            return Ok(false);
        };
        let actor = self
            .entities
            .get(index)
            .ok_or("Heart context actor index absent")?;
        if actor.retired {
            return Err("Heart context actor retired".into());
        }
        let classes = self.types.classes();
        for &value in actor.components.entries.values() {
            let handle = ComponentHandle::from_packed(value)?;
            let index = usize::try_from(handle.class_id)
                .map_err(|_| "Heart provider component type negative")?;
            let mut class = *classes
                .hierarchy()
                .by_id()
                .get(index)
                .ok_or("Heart provider component type absent")?;
            loop {
                let metadata = classes
                    .metadata(class)
                    .ok_or("Heart provider component metadata absent")?;
                if classes.name_text(metadata.name) == Some(name) {
                    return Ok(true);
                }
                let Some(base) = classes.hierarchy().node(class).and_then(|node| node.base) else {
                    break;
                };
                class = base;
            }
        }
        Ok(false)
    }

    ///572f70 reads idEntity+d1 bit3 (the canonical d0 flags'800 bit).
    pub(in crate::app) fn heart_context_target_alive(
        &self,
        live: &LiveEntities,
        target: Option<EntityId>,
    ) -> Result<bool, String> {
        let Some(target) = target.filter(|&id| live.get(id).is_some()) else {
            return Ok(false);
        };
        let &index = self
            .shared_indices
            .get(&target)
            .ok_or("Heart target has no actual scene header")?;
        let actor = self
            .entities
            .get(index)
            .ok_or("Heart target scene slot absent")?;
        if actor.retired {
            return Err("Heart target scene header retired".into());
        }
        Ok(actor.entity_flags & 0x800 == 0)
    }
}
