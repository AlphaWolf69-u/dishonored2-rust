//! 43f9b0's ordinary-object handoff, before the player requests state84.
//! The original3fe700 queues149; it does not dispatch its listeners inline.
use super::*;
use crate::gameplay::{
    entity_events::{EntityEvents, Event, Payload},
    entity_lifetime::{EntityId, LiveEntities, Stage},
    pickable_component::CheckedComponent,
    powers::far_reach::ObjectKind,
};

/// Owned by the accepted beam transaction, never by a second entity queue.
#[derive(Default)]
pub(in crate::app) struct FarReachRelease {
    target: Option<EntityId>,
    kind: Option<ObjectKind>,
    rigid: Option<CheckedComponent>,
    payload: [u8; 28],
    converted: bool,
    converting_body: Option<u32>,
    posted: bool,
}

impl PhysicsScene {
    ///440db0/440e90/440f30 project one actual released managed target.
    ///The power owns the saved878030 entity-local timestamp and delay.
    pub(in crate::app) fn far_reach_released_targets(
        &self,
        live: &LiveEntities,
        target: EntityId,
        elapsed: f64,
        delay: f32,
    ) -> Result<crate::gameplay::player::grapple_catch::Targets, String> {
        use crate::gameplay::player::grapple_catch::Targets;
        let Some((_, class)) = live.get(target) else {
            return Ok(Targets::default());
        };
        let Some(&index) = self.shared_indices.get(&target) else {
            return Ok(Targets::default());
        };
        if self.entities[index].retired {
            return Ok(Targets::default());
        }
        //Source first subtracts the two doubles, then rounds to float before
        //its strict > comparison. Equality and unordered retain the target.
        if elapsed as f32 > delay {
            if self.grapple_dependency(index, 3, [0x28, 0])?.is_some() {
                return Err("Far Reach expiry requires actual AIPhysicsReaction150 bit4".into());
            }
            return Ok(Targets::default());
        }
        //440db0 always checks a present PhysicsSystemEvolved death timestamp,
        //including the unexpired path.440f30 separately checks its238 flags.
        //There is no admitted such payload on current ordinary props.
        if self.grapple_dependency(index, 4, [0x28, 0])?.is_some() {
            return Err(
                "Far Reach target requires actual PhysicsSystemEvolved3B8/238 state".into(),
            );
        }
        let type_id = self.types.classes().hierarchy().type_id(class)?;
        let movable = self.grapple_dependency_ranges[2]
            .is_some_and(|range| type_id >= range[0] && type_id <= range[1]);
        Ok(Targets {
            grappled: Some(target),
            movable: (movable && self.grapple_has_interactable(live, target)?).then_some(target),
            ragdoll: None,
        })
    }

    ///8f2dd0 has already checked the target's actual arkGrenade class in
    ///sensor27.43f9b0 then uses877720 mode2 and4915a0 on that same entity.
    pub(in crate::app) fn restore_grapple_grenade_delay(
        &mut self,
        live: &LiveEntities,
        target: EntityId,
    ) -> Result<(), String> {
        if live.stage(target) != Some(Stage::Spawned) {
            return Ok(());
        }
        let Some(&index) = self.shared_indices.get(&target) else {
            return Ok(());
        };
        let handle = self.entities[index].components.find_component(
            [self.classes[19]; 2],
            [0x28, 0],
            |handle| {
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: self.owner(handle)?.flags,
                        start_phases: self
                            .declarations
                            .get(&handle.packed())
                            .map(|declaration| declaration.scheduling.start_phases),
                    },
                )
            },
        )?;
        if let Some(handle) = handle {
            self.grenades
                .get_mut(handle.slot)
                .ok_or("Far Reach Grenade component slot absent")?
                .restore_delay();
        }
        Ok(())
    }

    /// Check actual headers before taking source43f9b0/7edfa0's null branches.
    /// Neither provider currently has an admitted runtime payload; a future
    /// admission must implement its real callback rather than silently skip it.
    pub(super) fn far_reach_component_present(
        &self,
        id: EntityId,
        name: &[u8],
    ) -> Result<bool, String> {
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(false);
        };
        let entity = &self.entities[index];
        let classes = self.types.classes();
        for &packed in entity.components.entries.values() {
            let handle = ComponentHandle::from_packed(packed)?;
            //877720 mode2 only sees actual eligible, started components.
            if !crate::gameplay::component_state::matches_reference_state(
                self.owner(handle)?.flags,
                [0x28, 0],
            ) {
                continue;
            }
            let class_index = usize::try_from(handle.class_id)
                .map_err(|_| "Far Reach component class is negative")?;
            let mut class = *classes
                .hierarchy()
                .by_id()
                .get(class_index)
                .ok_or("Far Reach component class is absent")?;
            loop {
                let metadata = classes
                    .metadata(class)
                    .ok_or("Far Reach component metadata is absent")?;
                if classes.name_text(metadata.name) == Some(name) {
                    return Ok(true);
                }
                match classes.hierarchy().node(class).and_then(|node| node.base) {
                    Some(base) => class = base,
                    None => break,
                }
            }
        }
        Ok(false)
    }

    /// False means the retained target/component stopped or retired; the
    /// caller cancels its beam/state transaction. True acknowledges queued149.
    /// Source then retains the reference, requests84, calls7edfa0, and emits
    /// completion notifications, in that order, in the caller's same cursor.
    pub(in crate::app) fn release_far_reach_object(
        &mut self,
        live: &LiveEntities,
        events: &mut EntityEvents,
        target: EntityId,
        kind: ObjectKind,
        linked_entity: Option<EntityId>,
        linear: [f32; 3],
        angular: [f32; 3],
        cursor: &mut FarReachRelease,
    ) -> Result<bool, String> {
        if cursor.target.is_some_and(|id| id != target) {
            return Err("Far Reach release cursor belongs to another target generation".into());
        }
        if cursor.kind.is_some_and(|retained| retained != kind) {
            return Err("Far Reach release target class changed".into());
        }
        if cursor.posted {
            return Ok(true);
        }
        //A Pickup without the real Loot47 admission takes440616, not149.
        if kind == ObjectKind::Pickup {
            return Ok(false);
        }
        if live.stage(target) != Some(Stage::Spawned) {
            return Ok(false);
        }
        let Some(&index) = self.shared_indices.get(&target) else {
            return Ok(false);
        };
        if self.entities[index].retired {
            return Ok(false);
        }

        if cursor.target.is_none() {
            //43f9b0 resolves arkMovable+360, then PickableHolder mode2.
            //This managed field is distinct from LiveEntities' binding parent.
            //7379e0 releases the holder; it is NOT cpntBreakable7edfa0.
            if let Some(holder) =
                linked_entity.filter(|&id| kind == ObjectKind::Movable && live.get(id).is_some())
            {
                if self.far_reach_component_present(holder, b"arkCpntPickableHolder")? {
                    return Err(
                        "Far Reach requires the actual PickableHolder7379e0 release owner".into(),
                    );
                }
            }
            let handle = self.entities[index].components.find_component(
                self.rigid_class_range()?,
                [0x28, 0],
                |handle| {
                    Ok(
                        crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                            flags: self.owner(handle)?.flags,
                            start_phases: self
                                .declarations
                                .get(&handle.packed())
                                .map(|declaration| declaration.scheduling.start_phases),
                        },
                    )
                },
            )?;
            cursor.rigid = handle
                .map(|handle| -> Result<_, String> {
                    Ok(CheckedComponent {
                        handle,
                        generation: self.owner(handle)?.generation,
                    })
                })
                .transpose()?;
            for (offset, value) in linear.into_iter().chain(angular).enumerate() {
                cursor.payload[offset * 4..offset * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
            cursor.payload[24] = 1;
            //Grenade's43ff84 branch shares only149, bypassing7379e0,
            //virtual180/188 and the Movable's later7edfa0 call.
            cursor.converted = kind == ObjectKind::Grenade;
            cursor.kind = Some(kind);
            cursor.target = Some(target);
        }

        if let Some(checked) = cursor.rigid {
            let component = self.rigid_component(checked.handle)?;
            if component.owner.generation != checked.generation
                || component.owner.entity != Some(index)
                || component.owner.flags & 0xc != 8
            {
                return Ok(false);
            }
            if !cursor.converted {
                let body = component
                    .body()
                    .ok_or("Far Reach actual rigid body is absent")?;
                //808e20 tests modeFE==1. The host's canonical body flags4
                //mirror that live keyframe mode, including pickup changes;
                //start_behavior is merely the cached Start request.
                let shared = self.world.clone();
                let mut guard = shared
                    .write()
                    .map_err(|_| "Far Reach body world poisoned")?;
                let world = &mut *guard;
                let keyframed = world
                    .storage
                    .bodies
                    .get(body)
                    .ok_or("Far Reach body generation retired")?
                    .flags
                    & 4
                    != 0;
                //727880 deliberately leaves bound children unchanged.
                if cursor
                    .converting_body
                    .is_some_and(|retained| retained != body)
                {
                    return Err("Far Reach conversion body generation changed".into());
                }
                if cursor.converting_body.is_some()
                    || (keyframed && self.entities[index].entity_flags & 0x80 == 0)
                {
                    let mut registry = self.registry.take().ok_or("Far Reach registry absent")?;
                    //The canonical publication owns a resumable mode cursor;
                    //the body flags can already change before its last ACK.
                    cursor.converting_body = Some(body);
                    let changed = {
                        scene_publication_events!(self, &mut registry, publication);
                        world.storage.dynamic_single_body(
                            &mut world.geometry,
                            &mut publication,
                            body,
                        )
                    };
                    self.registry = Some(registry);
                    changed?;
                    self.rigid_component_mut(checked.handle)?.start_behavior = Some(2);
                    cursor.converting_body = None;
                }
                cursor.converted = true;
            }
        } else {
            cursor.converted = true;
        }

        events.post(Event {
            id: 0x149,
            target: Some(target),
            payload: Payload::Bytes(cursor.payload.to_vec()),
        });
        cursor.posted = true;
        Ok(true)
    }
}
